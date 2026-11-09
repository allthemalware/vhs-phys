from __future__ import annotations

import math
import time
from pathlib import Path
from typing import Any

from PySide6.QtCore import QPointF, QRectF, QTimer, Qt, Signal
from PySide6.QtGui import QColor, QFont, QFontMetricsF, QImage, QKeyEvent, QMouseEvent, QPainter, QPaintEvent
from PySide6.QtWidgets import QSizePolicy, QWidget

from .data import Control, Primitive, SchematicData, load_data
from .paint import PaintContext, color, draw_block, draw_control, draw_hint, draw_primitive, draw_reel_motion, draw_scanlines, draw_screen, draw_vignette, draw_wire, set_font, text


# The canvas is fixed by FORMAT.md; only its presentation scales.
CANVAS_WIDTH = 1180
CANVAS_HEIGHT = 620
ROW_HEIGHT = 18
TIMER_INTERVAL_MS = 40
TV_MARGIN = 20
NOISE_TILE_SIZE = 96
NOISE_PERIOD = 24


class SchematicView(QWidget):
    valueChanged = Signal(str, object)
    resetRequested = Signal(str)

    def __init__(self, parent: QWidget | None = None, *, data_dir: str | Path | None = None) -> None:
        super().__init__(parent)
        self._data: SchematicData = load_data(data_dir)
        self._controls = {control.id: control for control in self._data.controls}
        self._values: dict[str, Any] = {control.id: self._default(control) for control in self._data.controls}
        self._baseline: dict[str, Any] = {}
        self._choices: dict[str, list[str]] = {control.id: list(control.choices) for control in self._data.controls if control.kind == "choice"}
        self._help: dict[str, str] = {}
        self._hover: str | None = None
        self._current: str | None = None
        self._dragging: str | None = None
        self._press_snapshot: tuple[str, Any] | None = None
        self._flash_until: dict[str, float] = {}
        self._screen_image: QImage | None = None
        self._zoomed = False
        self._animated = True
        self._flicker = True
        self._clock = 0.0
        self._last_real = 0.0
        self._boot_start: float | None = None
        self._boot_done = True
        self._effects = {"glow": True, "scanlines": True, "vignette": True}
        self._cache_revision = 0
        self._base_cache: QImage | None = None
        self._base_cache_key: tuple[Any, ...] | None = None
        self._effect_cache: QImage | None = None
        self._effect_cache_key: tuple[Any, ...] | None = None
        self._screen_cache: QImage | None = None
        self._screen_cache_key: tuple[Any, ...] | None = None
        self._noise_tile = self._make_noise_tile()
        self._timer = QTimer(self)
        self._timer.setInterval(TIMER_INTERVAL_MS)
        self._timer.timeout.connect(self._on_timer)
        self.setFocusPolicy(Qt.FocusPolicy.StrongFocus)
        self.setMouseTracking(True)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)

    def _make_noise_tile(self) -> QImage:
        tile = QImage(NOISE_TILE_SIZE, NOISE_TILE_SIZE, QImage.Format.Format_RGB32)
        for y in range(NOISE_TILE_SIZE):
            for x in range(NOISE_TILE_SIZE):
                value = (x * 37 + y * 61 + x * y * 13) % 32
                tile.setPixelColor(x, y, QColor(8 + value // 3, 38 + value, 20 + value // 2))
        return tile

    def _touch(self) -> None:
        self._cache_revision += 1
        self.update()

    def _default(self, control: Control) -> Any:
        if control.kind == "choice":
            return control.choices[0]
        if control.kind == "toggle":
            return False
        if control.kind == "stepper":
            return int(control.minimum or 0)
        return float(control.minimum or 0)

    def control_ids(self) -> list[str]:
        return [control.id for control in self._data.controls]

    def value(self, control_id: str) -> Any:
        if control_id not in self._controls:
            raise KeyError(control_id)
        return self._values[control_id]

    def values(self) -> dict[str, Any]:
        return dict(self._values)

    def _snap(self, control: Control, value: Any) -> Any:
        if control.kind == "choice":
            if not isinstance(value, str) or value not in self._choices[control.id]:
                raise ValueError(f"unknown choice for {control.id}")
            return value
        if control.kind == "toggle":
            if not isinstance(value, (bool, int)):
                raise TypeError(f"{control.id} expects a boolean")
            return bool(value)
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise TypeError(f"{control.id} expects a number")
        minimum = float(control.minimum)
        maximum = float(control.maximum)
        step = float(control.step or 1)
        result = min(maximum, max(minimum, minimum + round((float(value) - minimum) / step) * step))
        return int(result) if control.kind == "stepper" else float(result)

    def set_value(self, control_id: str, value: Any, emit: bool = False) -> None:
        if control_id not in self._controls:
            raise KeyError(control_id)
        snapped = self._snap(self._controls[control_id], value)
        if snapped == self._values[control_id]:
            return
        self._values[control_id] = snapped
        self._flash_until[control_id] = self._clock + self._data.animation.refresh_flash_ms / 1000.0
        self._touch()
        if emit:
            self.valueChanged.emit(control_id, snapped)

    def choices(self, control_id: str) -> list[str]:
        if control_id not in self._controls or self._controls[control_id].kind != "choice":
            raise KeyError(control_id)
        return list(self._choices[control_id])

    def set_choices(self, control_id: str, names: list[str]) -> None:
        if control_id not in self._controls or self._controls[control_id].kind != "choice":
            raise ValueError(f"{control_id} is not a choice")
        if not names or not all(isinstance(name, str) for name in names):
            raise ValueError("choices must be non-empty text")
        self._choices[control_id] = list(names)
        if self._values[control_id] not in names:
            self._values[control_id] = names[0]
        self._touch()

    def set_baseline(self, values: dict[str, Any]) -> None:
        self._baseline = {key: values[key] for key in values if key in self._controls}
        self._touch()

    def is_edited(self, control_id: str) -> bool:
        if control_id not in self._controls:
            raise KeyError(control_id)
        if control_id not in self._baseline:
            return False
        current, baseline = self._values[control_id], self._baseline[control_id]
        return not (isinstance(current, float) and isinstance(baseline, (int, float)) and math.isclose(current, float(baseline), abs_tol=1e-9)) and current != baseline

    def set_current_control(self, control_id: str | None) -> None:
        if control_id is not None and control_id not in self._controls:
            raise KeyError(control_id)
        self._current = control_id
        self._touch()

    def current_control(self) -> str | None:
        return self._current

    def hover_control(self) -> str | None:
        return self._hover

    def set_help(self, values: dict[str, str]) -> None:
        self._help = dict(values)
        self._touch()

    def hint_text(self) -> str:
        control_id = self._hover or self._current
        return self._help.get(control_id, self._data.hint.default) if control_id else self._data.hint.default

    def _scale(self) -> tuple[float, float, float]:
        scale = min(self.width() / CANVAS_WIDTH if self.width() else 1.0, self.height() / CANVAS_HEIGHT if self.height() else 1.0)
        return scale, (self.width() - CANVAS_WIDTH * scale) / 2, (self.height() - CANVAS_HEIGHT * scale) / 2

    def _canvas_point(self, point: QPointF) -> QPointF:
        scale, ox, oy = self._scale()
        return QPointF((point.x() - ox) / scale, (point.y() - oy) / scale)

    def _canvas_rect(self, x: float, y: float, width: float, height: float) -> QRectF:
        scale, ox, oy = self._scale()
        return QRectF(ox + x * scale, oy + y * scale, width * scale, height * scale)

    def control_rect(self, control_id: str) -> QRectF:
        if control_id not in self._controls:
            raise KeyError(control_id)
        control = self._controls[control_id]
        return self._canvas_rect(control.x, control.y, control.width, ROW_HEIGHT)

    def control_at(self, position: QPointF) -> str | None:
        point = self._canvas_point(position)
        for control in self._data.controls:
            if control.x <= point.x() <= control.x + control.width and control.y <= point.y() <= control.y + ROW_HEIGHT:
                return control.id
        return None

    def _screen_canvas_rect(self) -> QRectF:
        for instance in self._data.instances:
            glyph = self._data.glyphs[instance.glyph]
            for primitive in glyph.primitives:
                if primitive.kind == "screen":
                    args = primitive.args
                    return QRectF(instance.x + args[0] * instance.scale, instance.y + args[1] * instance.scale, args[2] * instance.scale, args[3] * instance.scale)
        return QRectF()

    def screen_rect(self) -> QRectF | None:
        screen = self._screen_canvas_rect()
        if screen.isNull():
            return None
        if self._zoomed:
            aspect = self._screen_aspect()
            available = QRectF(TV_MARGIN, TV_MARGIN, CANVAS_WIDTH - 2 * TV_MARGIN, CANVAS_HEIGHT - 2 * TV_MARGIN)
            if available.width() / available.height() > aspect:
                height = available.height()
                width = height * aspect
            else:
                width = available.width()
                height = width / aspect
            screen = QRectF(0, 0, width, height)
            screen.moveCenter(QPointF(CANVAS_WIDTH / 2, CANVAS_HEIGHT / 2))
        scale, ox, oy = self._scale()
        return QRectF(ox + screen.x() * scale, oy + screen.y() * scale, screen.width() * scale, screen.height() * scale)

    def _screen_aspect(self) -> float:
        if self._screen_image is not None and self._screen_image.height() > 0:
            return self._screen_image.width() / self._screen_image.height()
        rect = self._screen_canvas_rect()
        return rect.width() / rect.height() if rect.height() else 4 / 3

    def screen_image(self) -> QImage | None:
        return self._screen_image

    def set_screen_image(self, image: QImage | None) -> None:
        self._screen_image = image
        self._screen_cache = None
        self._screen_cache_key = None
        self.update()

    def set_tv_zoom(self, zoomed: bool) -> None:
        self._zoomed = bool(zoomed)
        if self._zoomed:
            self._hover = None
        self.update()

    def tv_zoomed(self) -> bool:
        return self._zoomed

    def set_effects(self, glow: bool = True, scanlines: bool = True, vignette: bool = True) -> None:
        self._effects = {"glow": glow, "scanlines": scanlines, "vignette": vignette}
        self._touch()

    def set_flicker(self, enabled: bool) -> None:
        self._flicker = bool(enabled)
        self.update()

    def clock(self) -> float:
        return self._clock

    def advance(self, seconds: float) -> None:
        self._clock += max(0.0, float(seconds))
        self.update(self._moving_update_rect())

    def set_animated(self, animated: bool) -> None:
        self._animated = bool(animated)
        self._refresh_timer()
        self.update()

    def is_animated(self) -> bool:
        return self._animated

    def is_ticking(self) -> bool:
        return self._timer.isActive()

    def start_boot(self) -> None:
        self._boot_start = self._clock
        self._boot_done = False
        self._touch()

    def skip_boot(self) -> None:
        self._boot_done = True
        self._boot_start = None
        self._touch()

    def boot_progress(self) -> float:
        if self._boot_done or self._boot_start is None:
            return 1.0
        duration = self._data.animation.boot_ms / 1000.0
        return min(1.0, max(0.0, (self._clock - self._boot_start) / duration))

    def overflowing_controls(self) -> list[str]:
        metrics = QFontMetricsF(self._font())
        result = []
        for control in self._data.controls:
            if control.kind == "choice":
                widest = max(self._choices[control.id], key=len)
                value = widest.upper()
            elif control.kind == "toggle":
                value = "OFF"
            elif control.kind == "stepper":
                value = str(int(control.maximum or 0))
            else:
                values = (float(control.minimum), float(control.maximum))
                value = max((f"{number:.{control.decimals}f}{(' ' + control.unit) if control.unit else ''}" for number in values), key=len)
            needed = metrics.horizontalAdvance(control.label + "..." + value)
            if needed > control.width - 4:
                result.append(control.id)
        return result

    def _font(self, size: float | None = None) -> QFont:
        font = QFont()
        font.setFamilies(list(self._data.theme.font_family))
        font.setPixelSize(round(size if size is not None else self._data.theme.font_size))
        font.setLetterSpacing(QFont.SpacingType.AbsoluteSpacing, self._data.theme.tracking)
        return font

    def sizeHint(self) -> Any:
        from PySide6.QtCore import QSize

        return QSize(CANVAS_WIDTH, CANVAS_HEIGHT)

    def minimumSizeHint(self) -> Any:
        from PySide6.QtCore import QSize

        return QSize(900, round(900 * CANVAS_HEIGHT / CANVAS_WIDTH))

    def _fraction(self, control: Control, value: Any) -> float:
        if control.kind == "toggle":
            return 1.0 if value else 0.0
        if control.kind == "choice":
            return (self._choices[control.id].index(value) + 1) / len(self._choices[control.id])
        return (float(value) - float(control.minimum)) / max(float(control.maximum) - float(control.minimum), 1e-9)

    def _set_pointer_value(self, control: Control, point: QPointF) -> None:
        if control.kind != "slider":
            return
        fraction = min(1.0, max(0.0, (point.x() - control.x) / control.width))
        value = float(control.minimum) + fraction * (float(control.maximum) - float(control.minimum))
        self.set_value(control.id, value, emit=True)

    def _step(self, control: Control, amount: int, wrap: bool = False) -> None:
        amount = int(amount)
        if control.kind == "toggle":
            if amount:
                self.set_value(control.id, not self._values[control.id], emit=True)
            return
        if control.kind == "choice":
            choices = self._choices[control.id]
            index = choices.index(self._values[control.id]) + amount
            if wrap:
                index %= len(choices)
            else:
                index = min(len(choices) - 1, max(0, index))
            self.set_value(control.id, choices[index], emit=True)
            return
        self.set_value(control.id, float(self._values[control.id]) + amount * float(control.step or 1), emit=True)

    def _row_move(self, direction: int) -> None:
        if self._current is None:
            return
        current = self._controls[self._current]
        candidates = []
        for control in self._data.controls:
            if direction < 0 and control.y >= current.y - 1:
                continue
            if direction > 0 and control.y <= current.y + 1:
                continue
            overlap = min(control.x + control.width, current.x + current.width) - max(control.x, current.x)
            if overlap > current.width * 0.5:
                candidates.append(control)
        if candidates:
            chosen = (max if direction < 0 else min)(candidates, key=lambda item: item.y)
            self._current = chosen.id
            self.update()

    def mousePressEvent(self, event: QMouseEvent) -> None:
        point = event.position()
        screen = self.screen_rect()
        if self._zoomed:
            self.set_tv_zoom(False)
            event.accept()
            return
        if screen is not None and screen.contains(point):
            self.set_tv_zoom(True)
            event.accept()
            return
        control_id = self.control_at(point)
        if control_id is None:
            return
        control = self._controls[control_id]
        if self._current != control_id:
            self._current = control_id
            self._touch()
        self._press_snapshot = (control_id, self._values[control_id])
        canvas = self._canvas_point(point)
        if control.kind == "slider":
            self._dragging = control_id
            self._set_pointer_value(control, canvas)
        elif control.kind == "stepper":
            amount = 1 if canvas.x() >= control.x + control.width / 2 else -1
            self._step(control, amount * (10 if event.modifiers() & Qt.KeyboardModifier.ShiftModifier else 1))
        elif control.kind == "toggle":
            self._step(control, 1)
        else:
            amount = -1 if event.button() == Qt.MouseButton.RightButton or event.modifiers() & Qt.KeyboardModifier.ShiftModifier else 1
            self._step(control, amount, wrap=True)
        self.setFocus(Qt.FocusReason.MouseFocusReason)
        self.update()

    def mouseMoveEvent(self, event: QMouseEvent) -> None:
        if self._dragging is not None:
            self._set_pointer_value(self._controls[self._dragging], self._canvas_point(event.position()))
            return
        if self._zoomed:
            self._hover = None
            return
        hover = self.control_at(event.position())
        if hover != self._hover:
            self._hover = hover
            self._touch()

    def mouseReleaseEvent(self, event: QMouseEvent) -> None:
        self._dragging = None
        self._press_snapshot = None
        event.accept()

    def mouseDoubleClickEvent(self, event: QMouseEvent) -> None:
        control_id = self.control_at(event.position())
        if control_id is not None:
            if self._press_snapshot and self._press_snapshot[0] == control_id:
                self._values[control_id] = self._press_snapshot[1]
                self._flash_until.pop(control_id, None)
            self.resetRequested.emit(control_id)
            self._dragging = None
            self._press_snapshot = None
            self.update()
        event.accept()

    def wheelEvent(self, event: Any) -> None:
        if self._zoomed:
            return
        control_id = self.control_at(event.position())
        if control_id is None:
            return
        self._current = control_id
        amount = 1 if event.angleDelta().y() > 0 else -1
        if event.modifiers() & Qt.KeyboardModifier.ShiftModifier:
            amount *= 10
        self._step(self._controls[control_id], amount)
        event.accept()

    def keyPressEvent(self, event: QKeyEvent) -> None:
        if event.key() == Qt.Key.Key_Escape and self._zoomed:
            self.set_tv_zoom(False)
            return
        if self._zoomed or self._current is None:
            return
        control = self._controls[self._current]
        if event.key() == Qt.Key.Key_Up:
            self._row_move(-1)
        elif event.key() == Qt.Key.Key_Down:
            self._row_move(1)
        elif event.key() == Qt.Key.Key_Space and control.kind == "toggle":
            self._step(control, 1)
        elif event.key() in (Qt.Key.Key_Left, Qt.Key.Key_Right):
            amount = 1 if event.key() == Qt.Key.Key_Right else -1
            if event.modifiers() & Qt.KeyboardModifier.ShiftModifier:
                amount *= 10
            self._step(control, amount)
        elif event.key() == Qt.Key.Key_Home:
            if control.kind == "choice":
                self.set_value(control.id, self._choices[control.id][0], emit=True)
            elif control.kind != "toggle":
                self.set_value(control.id, control.minimum, emit=True)
        elif event.key() == Qt.Key.Key_End:
            if control.kind == "choice":
                self.set_value(control.id, self._choices[control.id][-1], emit=True)
            elif control.kind != "toggle":
                self.set_value(control.id, control.maximum, emit=True)
        else:
            super().keyPressEvent(event)

    def leaveEvent(self, event: Any) -> None:
        if self._hover is not None:
            self._hover = None
            self._touch()
        super().leaveEvent(event)

    def showEvent(self, event: Any) -> None:
        self._last_real = time.monotonic()
        self._refresh_timer()
        super().showEvent(event)

    def hideEvent(self, event: Any) -> None:
        self._timer.stop()
        super().hideEvent(event)

    def _refresh_timer(self) -> None:
        if self.isVisible() and self._animated:
            self._last_real = time.monotonic()
            if not self._timer.isActive():
                self._timer.start()
        else:
            self._timer.stop()

    def _on_timer(self) -> None:
        now = time.monotonic()
        elapsed = max(0.0, min(0.2, now - self._last_real))
        self._last_real = now
        self._clock += elapsed
        self.update(self._moving_update_rect())

    def _moving_update_rect(self) -> Any:
        points = [point for wire in self._data.wires for point in wire.points]
        screen = self._zoom_canvas_rect() if self._zoomed else self._screen_canvas_rect()
        if not screen.isNull():
            points.extend([(screen.left(), screen.top()), (screen.right(), screen.bottom())])
        for instance in self._data.instances:
            for primitive in self._data.glyphs[instance.glyph].primitives:
                if primitive.kind == "reel":
                    radius = primitive.args[2] * instance.scale + 8
                    center_x = instance.x + primitive.args[0] * instance.scale
                    center_y = instance.y + primitive.args[1] * instance.scale
                    points.extend([(center_x - radius, center_y - radius), (center_x + radius, center_y + radius)])
        if not points:
            return self.rect()
        left = min(point[0] for point in points) - 12
        top = min(point[1] for point in points) - 12
        right = max(point[0] for point in points) + 12
        bottom = max(point[1] for point in points) + 12
        return self._canvas_rect(left, top, right - left, bottom - top).toAlignedRect()

    def paintEvent(self, event: QPaintEvent) -> None:
        painter = QPainter(self)
        painter.fillRect(event.rect(), color(self._data.theme, "bg"))
        scale, ox, oy = self._scale()
        painter.translate(ox, oy)
        self._paint_output(painter, round(CANVAS_WIDTH * scale * self.devicePixelRatioF()), flicker=True)
        painter.end()

    def render_to_image(self, width: int) -> QImage:
        height = round(width * CANVAS_HEIGHT / CANVAS_WIDTH)
        image = QImage(width, height, QImage.Format.Format_ARGB32_Premultiplied)
        image.fill(color(self._data.theme, "bg"))
        painter = QPainter(image)
        self._paint_output(painter, width, flicker=False)
        painter.end()
        return image

    def _context(self) -> PaintContext:
        return PaintContext(self._data.theme, self._effects["glow"])

    def _theme_key(self) -> tuple[Any, ...]:
        theme = self._data.theme
        return (theme.bg, theme.faint, theme.dim, theme.phosphor, theme.hot, theme.warn, tuple(theme.line.items()), theme.glow, theme.scanline_period, theme.scanline_alpha, theme.vignette, theme.font_family, theme.font_size, theme.tracking)

    def _flash_signature(self) -> tuple[str, ...]:
        return tuple(sorted(control_id for control_id, until in self._flash_until.items() if until > self._clock))

    def _ensure_base_cache(self, width: int) -> QImage:
        key = (width, round(width * CANVAS_HEIGHT / CANVAS_WIDTH), self._cache_revision, self._flash_signature(), self._theme_key(), self._effects["glow"])
        if self._base_cache is not None and self._base_cache_key == key:
            return self._base_cache
        height = key[1]
        image = QImage(width, height, QImage.Format.Format_ARGB32_Premultiplied)
        image.fill(color(self._data.theme, "bg"))
        painter = QPainter(image)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing, True)
        painter.setRenderHint(QPainter.RenderHint.TextAntialiasing, True)
        painter.scale(width / CANVAS_WIDTH, width / CANVAS_WIDTH)
        ctx = self._context()
        text(painter, ctx, self._data.title.text, self._data.title.x, self._data.title.y, "hot", self._data.title.size)
        for block in self._data.blocks:
            draw_block(painter, ctx, block)
        for wire in self._data.wires:
            draw_wire(painter, ctx, wire, 0.0, wire.kind == "dub", pulses=False)
            if wire.label:
                text(painter, ctx, wire.label, wire.label_at[0], wire.label_at[1], "norm", self._data.theme.font_size)
        for label in self._data.labels:
            text(painter, ctx, label.text, label.x, label.y, label.brightness, label.size, label.align)
        for instance in self._data.instances:
            glyph = self._data.glyphs[instance.glyph]
            for primitive in glyph.primitives:
                draw_primitive(painter, ctx, primitive, instance)
        active_ids = {self._hover, self._current}
        flashing_ids = self._flash_signature()
        for control in self._data.controls:
            baseline = self._baseline.get(control.id)
            baseline_fraction = self._fraction(control, baseline) if baseline is not None else None
            draw_control(painter, ctx, control, self._values[control.id], self._fraction(control, self._values[control.id]), control.id in active_ids, self.is_edited(control.id), control.id in flashing_ids, baseline_fraction)
        draw_hint(painter, ctx, self._data.hint, self._fit_hint(self.hint_text()))
        painter.end()
        self._base_cache = image
        self._base_cache_key = key
        return image

    def _ensure_effect_cache(self, width: int) -> QImage:
        height = round(width * CANVAS_HEIGHT / CANVAS_WIDTH)
        key = (width, height, self._theme_key(), self._effects["scanlines"], self._effects["vignette"])
        if self._effect_cache is not None and self._effect_cache_key == key:
            return self._effect_cache
        image = QImage(width, height, QImage.Format.Format_ARGB32_Premultiplied)
        image.fill(QColor(0, 0, 0, 0))
        painter = QPainter(image)
        if self._effects["scanlines"]:
            draw_scanlines(painter, self._data.theme, width, height)
        if self._effects["vignette"]:
            draw_vignette(painter, self._data.theme, width, height)
        painter.end()
        self._effect_cache = image
        self._effect_cache_key = key
        return image

    def _fitted_screen(self, rect: QRectF, width: int) -> QImage | None:
        if self._screen_image is None or self._screen_image.isNull():
            return None
        target_width = max(1, round(rect.width() * width / CANVAS_WIDTH))
        target_height = max(1, round(rect.height() * width / CANVAS_WIDTH))
        key = (self._screen_image.cacheKey(), target_width, target_height)
        if self._screen_cache is None or self._screen_cache_key != key:
            self._screen_cache = self._screen_image.scaled(target_width, target_height, Qt.AspectRatioMode.KeepAspectRatio, Qt.TransformationMode.SmoothTransformation)
            self._screen_cache_key = key
        return self._screen_cache

    def _paint_output(self, painter: QPainter, width: int, flicker: bool) -> None:
        height = round(width * CANVAS_HEIGHT / CANVAS_WIDTH)
        painter.save()
        painter.setRenderHint(QPainter.RenderHint.Antialiasing, True)
        painter.setRenderHint(QPainter.RenderHint.TextAntialiasing, True)
        painter.scale(width / CANVAS_WIDTH, width / CANVAS_WIDTH)
        base = self._ensure_base_cache(width)
        progress = self.boot_progress()
        if progress > 0.0:
            painter.save()
            if progress < 1.0:
                painter.setClipRect(QRectF(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT * progress), Qt.ClipOperation.ReplaceClip)
            painter.drawImage(QRectF(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT), base)
            painter.restore()
        if progress <= 0.0 or progress < 1.0:
            lead_y = CANVAS_HEIGHT * progress
            painter.setPen(color(self._data.theme, "hot"))
            painter.drawLine(QPointF(0, lead_y), QPointF(CANVAS_WIDTH, lead_y))
        if progress > 0.0:
            painter.save()
            if progress < 1.0:
                painter.setClipRect(QRectF(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT * progress), Qt.ClipOperation.IntersectClip)
            self._draw_moving(painter, width)
            painter.restore()
        if self._zoomed:
            painter.fillRect(QRectF(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT), color(self._data.theme, "bg", 150))
            self._draw_tv(painter, self._zoom_canvas_rect(), width)
        painter.drawImage(QRectF(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT), self._ensure_effect_cache(width))
        if flicker and self._flicker and self._animated:
            amount = 0.5 * (1.0 + math.sin(self._clock * 2.0)) * self._data.animation.flicker
            painter.fillRect(QRectF(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT), color(self._data.theme, "bg", round(amount * 255)))
        painter.restore()

    def _draw_moving(self, painter: QPainter, width: int) -> None:
        # The cached wire and reel bodies already provide the broad phosphor halo;
        # the small moving marks need only their crisp core on each tick.
        ctx = PaintContext(self._data.theme, False)
        for index, wire in enumerate(self._data.wires):
            phase = (self._clock if self._animated else 0.0) * self._data.animation.pulse_speed + index * self._data.animation.pulse_spacing / 2
            draw_wire(painter, ctx, wire, phase, wire.kind == "dub", pulses=True, base=False)
        reel_phase = self._clock * self._data.animation.reel_rpm * math.pi / 30 if self._animated else 0.0
        for instance in self._data.instances:
            for primitive in self._data.glyphs[instance.glyph].primitives:
                if primitive.kind == "reel":
                    draw_reel_motion(painter, ctx, primitive, instance, reel_phase)
        rect = self._screen_canvas_rect()
        self._draw_tv(painter, rect, width)

    def _draw_tv(self, painter: QPainter, rect: QRectF, width: int) -> None:
        if rect.isNull():
            return
        seed = int(self._clock * NOISE_PERIOD) if self._animated else 0
        draw_screen(painter, self._context(), rect, self._fitted_screen(rect, width), self._noise_tile, seed)

    def _fit_hint(self, value: str) -> str:
        metrics = QFontMetricsF(self._font(self._data.hint.size))
        if metrics.horizontalAdvance(value) <= self._data.hint.width:
            return value
        ellipsis = "..."
        while value and metrics.horizontalAdvance(value + ellipsis) > self._data.hint.width:
            value = value[:-1]
        return value + ellipsis

    def _zoom_canvas_rect(self) -> QRectF:
        aspect = self._screen_aspect()
        available = QRectF(TV_MARGIN, TV_MARGIN, CANVAS_WIDTH - 2 * TV_MARGIN, CANVAS_HEIGHT - 2 * TV_MARGIN)
        if available.width() / available.height() > aspect:
            height = available.height()
            width = height * aspect
        else:
            width = available.width()
            height = width / aspect
        rect = QRectF(0, 0, width, height)
        rect.moveCenter(QPointF(CANVAS_WIDTH / 2, CANVAS_HEIGHT / 2))
        return rect
