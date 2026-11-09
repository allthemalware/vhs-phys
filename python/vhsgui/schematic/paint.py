from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Any

from PySide6.QtCore import QPointF, QRectF, Qt
from PySide6.QtGui import QBrush, QColor, QFont, QFontMetricsF, QImage, QPainter, QPainterPath, QPen, QRadialGradient

from .data import Block, Control, Hint, Instance, Primitive, Theme, Wire


# The design has one phosphor palette and a small vocabulary of wire treatments.
TIER_NAMES = {"norm": "phosphor"}
WIRE_COLORS = {"video": "phosphor", "audio": "dim", "dub": "hot", "power": "faint"}
WIRE_DASHES = {"video": (), "audio": (5.0, 5.0), "dub": (12.0, 5.0), "power": (2.0, 7.0)}
ARROW_CLEARANCE = 11.0
PULSE_SPACING = 70.0
# A flashing edited value stays amber while moving toward the hot tier.
EDIT_FLASH_MIX = 0.45


@dataclass(frozen=True)
class PaintContext:
    theme: Theme
    glow: bool = True


def color(theme: Theme, tier: str | QColor, alpha: int = 255) -> QColor:
    if isinstance(tier, QColor):
        result = QColor(tier)
    else:
        name = TIER_NAMES.get(tier, tier)
        result = QColor(getattr(theme, name))
    result.setAlpha(max(0, min(255, alpha)))
    return result


def _font(theme: Theme, size: float | None = None) -> QFont:
    font = QFont()
    font.setFamilies(list(theme.font_family))
    font.setPixelSize(round(size if size is not None else theme.font_size))
    font.setLetterSpacing(QFont.SpacingType.AbsoluteSpacing, theme.tracking)
    return font


def set_font(painter: QPainter, theme: Theme, size: float | None = None) -> None:
    painter.setFont(_font(theme, size))


def _pen(ctx: PaintContext, tier: str, width: float, dash: tuple[float, float] = ()) -> QPen:
    pen = QPen(color(ctx.theme, tier), width)
    pen.setCapStyle(Qt.PenCapStyle.RoundCap)
    pen.setJoinStyle(Qt.PenJoinStyle.RoundJoin)
    if dash:
        pen.setDashPattern([part / max(width, 0.01) for part in dash])
    return pen


def draw_path(
    painter: QPainter,
    ctx: PaintContext,
    path: QPainterPath,
    tier: str | QColor = "norm",
    weight: str = "thin",
    dashed: tuple[float, float] = (),
) -> None:
    theme = ctx.theme
    core_width = theme.line.get(weight, theme.line["thin"])
    if ctx.glow:
        painter.save()
        painter.setCompositionMode(QPainter.CompositionMode.CompositionMode_Plus)
        for glow_width, opacity in theme.glow:
            painter.setPen(_pen(ctx, tier, glow_width, dashed))
            painter.setOpacity(opacity)
            painter.drawPath(path)
        painter.restore()
    painter.setOpacity(1.0)
    painter.setPen(_pen(ctx, tier, core_width, dashed))
    painter.drawPath(path)


def text(
    painter: QPainter,
    ctx: PaintContext,
    value: str,
    x: float,
    baseline: float,
    tier: str = "norm",
    size: float | None = None,
    align: str = "l",
) -> None:
    set_font(painter, ctx.theme, size)
    painter.setPen(color(ctx.theme, tier))
    width = QFontMetricsF(painter.font()).horizontalAdvance(value)
    left = x - width / 2 if align == "c" else x - width if align == "r" else x
    painter.drawText(QPointF(left, baseline), value)


def _primitive_path(primitive: Primitive, instance: Instance) -> QPainterPath:
    args = primitive.args
    scale = instance.scale
    ox, oy = instance.x, instance.y
    path = QPainterPath()
    if primitive.kind == "line":
        path.moveTo(ox + args[0] * scale, oy + args[1] * scale)
        path.lineTo(ox + args[2] * scale, oy + args[3] * scale)
    elif primitive.kind in {"poly", "loop"}:
        points = args[0]
        path.moveTo(ox + points[0][0] * scale, oy + points[0][1] * scale)
        for x, y in points[1:]:
            path.lineTo(ox + x * scale, oy + y * scale)
        if primitive.kind == "loop":
            path.closeSubpath()
    elif primitive.kind == "rect":
        path.addRect(QRectF(ox + args[0] * scale, oy + args[1] * scale, args[2] * scale, args[3] * scale))
    elif primitive.kind in {"circle", "reel"}:
        path.addEllipse(QPointF(ox + args[0] * scale, oy + args[1] * scale), args[2] * scale, args[2] * scale)
    elif primitive.kind == "arc":
        box = QRectF(ox + (args[0] - args[2]) * scale, oy + (args[1] - args[2]) * scale, 2 * args[2] * scale, 2 * args[2] * scale)
        path.arcMoveTo(box, -args[3])
        path.arcTo(box, -args[3], -args[4])
    elif primitive.kind == "dot":
        path.addEllipse(QPointF(ox + args[0] * scale, oy + args[1] * scale), 2 * scale, 2 * scale)
    return path


def _reel_parts(ctx: PaintContext, painter: QPainter, primitive: Primitive, instance: Instance, phase: float | None) -> None:
    style = primitive.style
    draw_path(painter, ctx, _primitive_path(primitive, instance), style["b"], style["w"], tuple(style.get("dash", ())))
    cx = instance.x + primitive.args[0] * instance.scale
    cy = instance.y + primitive.args[1] * instance.scale
    radius = primitive.args[2] * instance.scale
    hub = QPainterPath()
    hub.addEllipse(QPointF(cx, cy), radius * 0.18, radius * 0.18)
    draw_path(painter, ctx, hub, "hot", "thin")
    if phase is None:
        return
    for spoke in range(6):
        angle = phase + spoke * math.pi / 3
        line = QPainterPath()
        line.moveTo(cx + math.cos(angle) * radius * 0.2, cy + math.sin(angle) * radius * 0.2)
        line.lineTo(cx + math.cos(angle) * radius * 0.78, cy + math.sin(angle) * radius * 0.78)
        draw_path(painter, ctx, line, style["b"], style["w"])


def draw_primitive(painter: QPainter, ctx: PaintContext, primitive: Primitive, instance: Instance, reel_phase: float | None = None) -> None:
    style = primitive.style
    if primitive.kind == "text":
        args = primitive.args
        text(painter, ctx, args[2], instance.x + args[0] * instance.scale, instance.y + args[1] * instance.scale, style["b"], args[3] * instance.scale, args[4])
        return
    if primitive.kind == "screen":
        return
    if primitive.kind == "reel":
        _reel_parts(ctx, painter, primitive, instance, reel_phase)
        return
    path = _primitive_path(primitive, instance)
    if style["fill"] != "none":
        painter.fillPath(path, color(ctx.theme, style["fill"], 180))
    draw_path(painter, ctx, path, style["b"], style["w"], tuple(style.get("dash", ())))


def draw_block(painter: QPainter, ctx: PaintContext, block: Block) -> None:
    if block.frame == "none":
        return
    rect = QRectF(block.x, block.y, block.width, block.height)
    if block.frame == "box":
        path = QPainterPath()
        path.addRect(rect)
        draw_path(painter, ctx, path, "faint", "thin")
    else:
        corner = 14.0
        path = QPainterPath()
        path.moveTo(block.x + corner, block.y)
        path.lineTo(block.x, block.y)
        path.lineTo(block.x, block.y + corner)
        path.moveTo(block.x + block.width - corner, block.y)
        path.lineTo(block.x + block.width, block.y)
        path.lineTo(block.x + block.width, block.y + corner)
        path.moveTo(block.x, block.y + block.height - corner)
        path.lineTo(block.x, block.y + block.height)
        path.lineTo(block.x + corner, block.y + block.height)
        path.moveTo(block.x + block.width - corner, block.y + block.height)
        path.lineTo(block.x + block.width, block.y + block.height)
        path.lineTo(block.x + block.width, block.y + block.height - corner)
        draw_path(painter, ctx, path, "faint", "thin")
    text(painter, ctx, block.title, block.x + 8, block.y + 15, "norm", ctx.theme.font_size)
    if block.caption:
        text(painter, ctx, block.caption, block.x + 8, block.y + block.height - 7, "dim", ctx.theme.font_size - 1)


def _wire_path(wire: Wire) -> QPainterPath:
    path = QPainterPath()
    path.moveTo(QPointF(*wire.points[0]))
    for point in wire.points[1:]:
        path.lineTo(QPointF(*point))
    return path


def _wire_length(wire: Wire) -> float:
    return sum(math.hypot(b[0] - a[0], b[1] - a[1]) for a, b in zip(wire.points, wire.points[1:]))


def _point_on_wire(wire: Wire, distance: float) -> tuple[float, float] | None:
    left = distance
    for a, b in zip(wire.points, wire.points[1:]):
        length = math.hypot(b[0] - a[0], b[1] - a[1])
        if left <= length:
            ratio = left / max(length, 1e-6)
            return a[0] + (b[0] - a[0]) * ratio, a[1] + (b[1] - a[1]) * ratio
        left -= length
    return None


def draw_wire(painter: QPainter, ctx: PaintContext, wire: Wire, phase: float, hot: bool = False, pulses: bool = True, base: bool = True) -> None:
    if base:
        draw_path(painter, ctx, _wire_path(wire), "hot" if hot else WIRE_COLORS.get(wire.kind, "norm"), "thin", WIRE_DASHES.get(wire.kind, ()))
    if base and wire.arrow:
        end = wire.points[-1]
        before = wire.points[-2]
        angle = math.atan2(end[1] - before[1], end[0] - before[0])
        arrow = QPainterPath()
        arrow.moveTo(QPointF(*end))
        arrow.lineTo(QPointF(end[0] - 8 * math.cos(angle - 0.5), end[1] - 8 * math.sin(angle - 0.5)))
        arrow.moveTo(QPointF(*end))
        arrow.lineTo(QPointF(end[0] - 8 * math.cos(angle + 0.5), end[1] - 8 * math.sin(angle + 0.5)))
        draw_path(painter, ctx, arrow, "hot", "thin")
    if not pulses:
        return
    total = _wire_length(wire)
    distance = phase % max(total, 1.0)
    pulse_tier = "hot" if hot else WIRE_COLORS.get(wire.kind, "norm")
    pulses_path = QPainterPath()
    for _ in range(3):
        if distance < max(0.0, total - ARROW_CLEARANCE):
            point = _point_on_wire(wire, distance)
            if point is not None:
                pulses_path.addEllipse(QPointF(*point), 2.1, 2.1)
        distance += PULSE_SPACING
    if not pulses_path.isEmpty():
        draw_path(painter, ctx, pulses_path, pulse_tier, "hair")


def draw_reel_motion(painter: QPainter, ctx: PaintContext, primitive: Primitive, instance: Instance, phase: float) -> None:
    style = primitive.style
    cx = instance.x + primitive.args[0] * instance.scale
    cy = instance.y + primitive.args[1] * instance.scale
    radius = primitive.args[2] * instance.scale
    spokes = QPainterPath()
    for spoke in range(6):
        angle = phase + spoke * math.pi / 3
        spokes.moveTo(cx + math.cos(angle) * radius * 0.2, cy + math.sin(angle) * radius * 0.2)
        spokes.lineTo(cx + math.cos(angle) * radius * 0.78, cy + math.sin(angle) * radius * 0.78)
    draw_path(painter, ctx, spokes, style["b"], style["w"])


def _control_text(control: Control, value: Any) -> str:
    if control.kind == "choice":
        return str(value).upper()
    if control.kind == "toggle":
        return "ON" if value else "OFF"
    if control.kind == "stepper":
        return str(int(value))
    suffix = f" {control.unit}" if control.unit else ""
    return f"{float(value):.{control.decimals}f}{suffix}"


def draw_control(painter: QPainter, ctx: PaintContext, control: Control, value: Any, fraction: float, active: bool, edited: bool, flashing: bool, baseline_fraction: float | None) -> None:
    tier = "hot" if active or flashing else "norm"
    value_tier: str | QColor = tier
    if edited:
        value_tier = color(ctx.theme, "warn")
        if flashing:
            warn = color(ctx.theme, "warn")
            hot = color(ctx.theme, "hot")
            value_tier = QColor(
                round(warn.red() * (1 - EDIT_FLASH_MIX) + hot.red() * EDIT_FLASH_MIX),
                round(warn.green() * (1 - EDIT_FLASH_MIX) + hot.green() * EDIT_FLASH_MIX),
                round(warn.blue() * (1 - EDIT_FLASH_MIX) + hot.blue() * EDIT_FLASH_MIX),
            )
    value_text = _control_text(control, value)
    set_font(painter, ctx.theme)
    metrics = QFontMetricsF(painter.font())
    value_width = metrics.horizontalAdvance(value_text)
    label_width = metrics.horizontalAdvance(control.label)
    text(painter, ctx, control.label, control.x, control.y + 12, tier)
    value_x = control.x + control.width
    painter.setPen(color(ctx.theme, "dim" if not active else "norm"))
    dot_start = control.x + label_width + 6
    dot_end = value_x - value_width - 7
    if dot_end > dot_start:
        painter.drawText(QPointF(dot_start, control.y + 12), "." * max(1, int((dot_end - dot_start) / max(metrics.horizontalAdvance("."), 1))))
    text(painter, ctx, value_text, value_x, control.y + 12, value_tier, align="r")
    track_y = control.y + 16.5
    track = QPainterPath()
    track.moveTo(control.x, track_y)
    track.lineTo(control.x + control.width, track_y)
    draw_path(painter, ctx, track, "dim" if active else "faint", "hair")
    bright = QPainterPath()
    bright.moveTo(control.x, track_y)
    bright.lineTo(control.x + control.width * max(0.0, min(1.0, fraction)), track_y)
    draw_path(painter, ctx, bright, tier, "bold" if flashing else "thin")
    if active:
        ticks = QPainterPath()
        ticks.moveTo(control.x, control.y + 13)
        ticks.lineTo(control.x, control.y + 18)
        ticks.moveTo(control.x + control.width, control.y + 13)
        ticks.lineTo(control.x + control.width, control.y + 18)
        draw_path(painter, ctx, ticks, "hot", "hair")
    if edited and baseline_fraction is not None:
        base = QPainterPath()
        x = control.x + control.width * baseline_fraction
        base.moveTo(x, track_y - 4)
        base.lineTo(x, track_y + 3)
        draw_path(painter, ctx, base, "warn", "thin")


def draw_hint(painter: QPainter, ctx: PaintContext, hint: Hint, value: str) -> None:
    text(painter, ctx, value, hint.x, hint.y, "dim", hint.size)


def draw_vignette(painter: QPainter, theme: Theme, width: float, height: float) -> None:
    gradient = QRadialGradient(QPointF(width / 2, height / 2), math.hypot(width, height) * 0.62)
    gradient.setColorAt(0.0, QColor(0, 0, 0, 0))
    gradient.setColorAt(0.78, QColor(0, 0, 0, 0))
    gradient.setColorAt(1.0, QColor(0, 0, 0, round(theme.vignette * 255)))
    painter.fillRect(QRectF(0, 0, width, height), gradient)
    painter.setBrush(QColor(0, 0, 0, 28))
    painter.setPen(Qt.PenStyle.NoPen)
    corner = 34.0
    for x, y in ((0, 0), (width - corner, 0), (0, height - corner), (width - corner, height - corner)):
        painter.drawRect(QRectF(x, y, corner, corner))


def draw_scanlines(painter: QPainter, theme: Theme, width: float, height: float) -> None:
    painter.setPen(QPen(QColor(0, 0, 0, round(theme.scanline_alpha * 255)), 1))
    for y in range(0, math.ceil(height), theme.scanline_period):
        painter.drawLine(QPointF(0, y + 0.5), QPointF(width, y + 0.5))


def draw_screen(painter: QPainter, ctx: PaintContext, rect: QRectF, fitted_image: QImage | None, snow_tile: QImage, snow_offset: int) -> None:
    if rect.isNull():
        return
    painter.save()
    painter.setClipRect(rect)
    painter.fillRect(rect, QColor(0, 0, 0))
    if fitted_image is not None and not fitted_image.isNull():
        image_width = fitted_image.width() / fitted_image.devicePixelRatio()
        image_height = fitted_image.height() / fitted_image.devicePixelRatio()
        painter.drawImage(QPointF(rect.x() + (rect.width() - image_width) / 2, rect.y() + (rect.height() - image_height) / 2), fitted_image)
    else:
        painter.setBrushOrigin(QPointF(rect.x() - snow_offset, rect.y() - snow_offset))
        painter.fillRect(rect, QBrush(snow_tile))
        plate = rect.adjusted(rect.width() * 0.25, rect.height() * 0.40, -rect.width() * 0.25, -rect.height() * 0.40)
        painter.fillRect(plate.adjusted(-5, -4, 5, 4), color(ctx.theme, "bg", 220))
        text(painter, ctx, "NO SIGNAL", rect.center().x(), rect.center().y(), "dim", ctx.theme.font_size, "c")
    painter.restore()
    edge = QPainterPath()
    edge.addRect(rect)
    draw_path(painter, ctx, edge, "faint", "hair")
