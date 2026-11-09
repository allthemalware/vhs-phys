from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any


# The format and row geometry are versioned with the design files.
EXPECTED_VERSION = 1
ROW_HEIGHT = 18
# Primitive arities exclude their optional trailing style object.
PRIMITIVE_ARGUMENTS = {
    "line": 4,
    "poly": 1,
    "loop": 1,
    "rect": 4,
    "circle": 3,
    "arc": 5,
    "dot": 2,
    "text": 5,
    "reel": 3,
    "screen": 4,
}
# These are the only interactions the view can expose.
CONTROL_KINDS = {"slider", "stepper", "toggle", "choice"}
STROKE_WEIGHTS = {"hair", "thin", "bold"}
BRIGHTNESS_TIERS = {"faint", "dim", "norm", "hot"}
FILL_TIERS = {"none", "faint", "dim"}


@dataclass(frozen=True)
class Theme:
    bg: str
    faint: str
    dim: str
    phosphor: str
    hot: str
    warn: str
    line: dict[str, float]
    glow: tuple[tuple[float, float], ...]
    scanline_period: int
    scanline_alpha: float
    vignette: float
    font_family: tuple[str, ...]
    font_size: float
    tracking: float


@dataclass(frozen=True)
class Canvas:
    width: int
    height: int


@dataclass(frozen=True)
class Primitive:
    kind: str
    args: tuple[Any, ...]
    style: dict[str, Any]


@dataclass(frozen=True)
class Glyph:
    name: str
    width: float
    height: float
    primitives: tuple[Primitive, ...]


@dataclass(frozen=True)
class Block:
    id: str
    title: str
    x: float
    y: float
    width: float
    height: float
    frame: str
    caption: str


@dataclass(frozen=True)
class Instance:
    glyph: str
    x: float
    y: float
    scale: float
    block: str


@dataclass(frozen=True)
class Wire:
    id: str
    kind: str
    points: tuple[tuple[float, float], ...]
    label: str
    label_at: tuple[float, float]
    arrow: bool


@dataclass(frozen=True)
class Label:
    text: str
    x: float
    y: float
    size: float
    align: str
    brightness: str


@dataclass(frozen=True)
class Hint:
    x: float
    y: float
    width: float
    size: float
    default: str


@dataclass(frozen=True)
class Control:
    id: str
    block: str
    label: str
    x: float
    y: float
    width: float
    kind: str
    minimum: float | int | None = None
    maximum: float | int | None = None
    step: float | int | None = None
    unit: str = ""
    decimals: int = 0
    choices: tuple[str, ...] = ()


@dataclass(frozen=True)
class Animation:
    pulse_speed: float
    pulse_spacing: float
    reel_rpm: float
    flicker: float
    boot_ms: float
    refresh_flash_ms: float


@dataclass(frozen=True)
class SchematicData:
    canvas: Canvas
    theme: Theme
    title: Label
    blocks: tuple[Block, ...]
    instances: tuple[Instance, ...]
    wires: tuple[Wire, ...]
    labels: tuple[Label, ...]
    hint: Hint
    controls: tuple[Control, ...]
    glyphs: dict[str, Glyph]
    animation: Animation


def _error(path: str, message: str) -> ValueError:
    return ValueError(f"{path}: {message}")


def _object(value: Any, path: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise _error(path, "expected an object")
    return value


def _number(value: Any, path: str) -> float | int:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise _error(path, "expected a number")
    return value


def _text(value: Any, path: str) -> str:
    if not isinstance(value, str):
        raise _error(path, "expected text")
    return value


def _field(obj: dict[str, Any], name: str, path: str) -> Any:
    if name not in obj:
        raise _error(f"{path}.{name}", "is missing")
    return obj[name]


def _version(obj: dict[str, Any], path: str) -> None:
    if obj.get("version") != EXPECTED_VERSION:
        raise ValueError(f"{path}.version: expected {EXPECTED_VERSION}")


def _parse_style(value: Any, path: str) -> dict[str, Any]:
    if value is None:
        return {"w": "thin", "b": "norm", "fill": "none"}
    style = _object(value, path)
    allowed = {"w", "b", "dash", "fill"}
    if set(style) - allowed:
        raise _error(path, "contains an unknown style key")
    result = {"w": style.get("w", "thin"), "b": style.get("b", "norm"), "fill": style.get("fill", "none")}
    if result["w"] not in STROKE_WEIGHTS or result["b"] not in BRIGHTNESS_TIERS or result["fill"] not in FILL_TIERS:
        raise _error(path, "contains an unknown style value")
    if "dash" in style:
        dash = style["dash"]
        if not isinstance(dash, list) or len(dash) != 2 or not all(isinstance(v, (int, float)) and v > 0 for v in dash):
            raise _error(f"{path}.dash", "expected two positive numbers")
        result["dash"] = (float(dash[0]), float(dash[1]))
    return result


def _parse_primitive(value: Any, path: str) -> Primitive:
    if not isinstance(value, list) or not value or not isinstance(value[0], str):
        raise _error(path, "expected a primitive array")
    kind = value[0]
    if kind not in PRIMITIVE_ARGUMENTS:
        raise _error(f"{path}[0]", f"unknown primitive {kind!r}")
    args = value[1:]
    style = None
    if args and isinstance(args[-1], dict):
        style = args.pop()
    expected = PRIMITIVE_ARGUMENTS[kind]
    if len(args) != expected:
        raise _error(path, f"{kind} takes {expected} arguments")
    if kind in {"poly", "loop"}:
        points = args[0]
        minimum = 2 if kind == "poly" else 3
        if not isinstance(points, list) or len(points) < minimum or any(not isinstance(p, list) or len(p) != 2 for p in points):
            raise _error(f"{path}[1]", f"{kind} needs at least {minimum} points")
        args[0] = tuple((float(_number(p[0], f"{path}[1]")), float(_number(p[1], f"{path}[1]"))) for p in points)
    else:
        for index, arg in enumerate(args):
            if kind == "text" and index == 2:
                _text(arg, f"{path}[{index + 1}]")
            elif kind == "text" and index == 4:
                if arg not in {"l", "c", "r"}:
                    raise _error(f"{path}[{index + 1}]", "text alignment must be l, c or r")
            elif not isinstance(arg, str):
                _number(arg, f"{path}[{index + 1}]")
    return Primitive(kind, tuple(args), _parse_style(style, f"{path}.style"))


def _parse_glyphs(raw: Any, path: str) -> dict[str, Glyph]:
    obj = _object(raw, path)
    _version(obj, path)
    values = _object(_field(obj, "glyphs", path), f"{path}.glyphs")
    result: dict[str, Glyph] = {}
    for name, value in values.items():
        item_path = f"{path}.glyphs[{name!r}]"
        glyph = _object(value, item_path)
        width = _number(_field(glyph, "w", item_path), f"{item_path}.w")
        height = _number(_field(glyph, "h", item_path), f"{item_path}.h")
        primitives_raw = _field(glyph, "prims", item_path)
        if width <= 0 or height <= 0 or not isinstance(primitives_raw, list):
            raise _error(item_path, "invalid dimensions or primitives")
        result[name] = Glyph(name, float(width), float(height), tuple(_parse_primitive(p, f"{item_path}.prims[{i}]") for i, p in enumerate(primitives_raw)))
    return result


def _parse_theme(raw: Any, path: str) -> Theme:
    obj = _object(raw, path)
    strings = {name: _text(_field(obj, name, path), f"{path}.{name}") for name in ("bg", "faint", "dim", "phosphor", "hot", "warn")}
    line_raw = _object(_field(obj, "line", path), f"{path}.line")
    line = {name: float(_number(_field(line_raw, name, f"{path}.line"), f"{path}.line.{name}")) for name in ("hair", "thin", "bold")}
    glow_raw = _field(obj, "glow", path)
    if not isinstance(glow_raw, list):
        raise _error(f"{path}.glow", "expected a list")
    glow = []
    for index, value in enumerate(glow_raw):
        item = _object(value, f"{path}.glow[{index}]")
        glow.append((float(_number(_field(item, "w", f"{path}.glow[{index}]"), f"{path}.glow[{index}].w")), float(_number(_field(item, "a", f"{path}.glow[{index}]"), f"{path}.glow[{index}].a"))))
    scanline = _object(_field(obj, "scanline", path), f"{path}.scanline")
    font = _object(_field(obj, "font", path), f"{path}.font")
    families = font.get("family")
    if isinstance(families, str):
        families = (families,)
    elif isinstance(families, list) and all(isinstance(item, str) for item in families):
        families = tuple(families)
    else:
        raise _error(f"{path}.font.family", "expected a string or list of strings")
    return Theme(
        **strings,
        line=line,
        glow=tuple(glow),
        scanline_period=int(_number(_field(scanline, "period", f"{path}.scanline"), f"{path}.scanline.period")),
        scanline_alpha=float(_number(_field(scanline, "alpha", f"{path}.scanline"), f"{path}.scanline.alpha")),
        vignette=float(_number(_field(obj, "vignette", path), f"{path}.vignette")),
        font_family=families,
        font_size=float(_number(_field(font, "size", f"{path}.font"), f"{path}.font.size")),
        tracking=float(_number(_field(font, "tracking", f"{path}.font"), f"{path}.font.tracking")),
    )


def _parse_layout(raw: Any, path: str, glyphs: dict[str, Glyph]) -> SchematicData:
    obj = _object(raw, path)
    _version(obj, path)
    canvas_raw = _object(_field(obj, "canvas", path), f"{path}.canvas")
    canvas = Canvas(int(_number(_field(canvas_raw, "w", f"{path}.canvas"), f"{path}.canvas.w")), int(_number(_field(canvas_raw, "h", f"{path}.canvas"), f"{path}.canvas.h")))
    theme = _parse_theme(_field(obj, "theme", path), f"{path}.theme")

    def rect_item(value: Any, item_path: str) -> tuple[str, float, float, float, float]:
        item = _object(value, item_path)
        return (_text(_field(item, "id", item_path), f"{item_path}.id"), float(_number(_field(item, "x", item_path), f"{item_path}.x")), float(_number(_field(item, "y", item_path), f"{item_path}.y")), float(_number(_field(item, "w", item_path), f"{item_path}.w")), float(_number(_field(item, "h", item_path), f"{item_path}.h")))

    blocks = []
    for i, value in enumerate(_field(obj, "blocks", path)):
        item_path = f"{path}.blocks[{i}]"
        item = _object(value, item_path)
        ident, x, y, width, height = rect_item(value, item_path)
        frame = _text(_field(item, "frame", item_path), f"{item_path}.frame")
        if frame not in {"box", "bracket", "none"}:
            raise _error(f"{item_path}.frame", "unknown frame")
        blocks.append(Block(ident, _text(_field(item, "title", item_path), f"{item_path}.title"), x, y, width, height, frame, _text(item.get("caption", ""), f"{item_path}.caption")))

    instances = []
    for i, value in enumerate(_field(obj, "instances", path)):
        item_path = f"{path}.instances[{i}]"
        item = _object(value, item_path)
        name = _text(_field(item, "glyph", item_path), f"{item_path}.glyph")
        if name not in glyphs:
            raise _error(f"{item_path}.glyph", "unknown glyph")
        instances.append(Instance(name, float(_number(_field(item, "x", item_path), f"{item_path}.x")), float(_number(_field(item, "y", item_path), f"{item_path}.y")), float(_number(_field(item, "scale", item_path), f"{item_path}.scale")), _text(_field(item, "block", item_path), f"{item_path}.block")))

    wires = []
    for i, value in enumerate(_field(obj, "wires", path)):
        item_path = f"{path}.wires[{i}]"
        item = _object(value, item_path)
        points_raw = _field(item, "points", item_path)
        if not isinstance(points_raw, list) or len(points_raw) < 2:
            raise _error(f"{item_path}.points", "a wire needs at least two points")
        points = []
        for j, point in enumerate(points_raw):
            if not isinstance(point, list) or len(point) != 2:
                raise _error(f"{item_path}.points[{j}]", "expected an x/y pair")
            points.append((float(_number(point[0], f"{item_path}.points[{j}]")), float(_number(point[1], f"{item_path}.points[{j}]"))))
        wires.append(Wire(_text(_field(item, "id", item_path), f"{item_path}.id"), _text(_field(item, "kind", item_path), f"{item_path}.kind"), tuple(points), _text(item.get("label", ""), f"{item_path}.label"), tuple(float(_number(v, f"{item_path}.label_at")) for v in item.get("label_at", [0, 0])), bool(item.get("arrow", False))))

    labels = []
    for i, value in enumerate(obj.get("labels", [])):
        item_path = f"{path}.labels[{i}]"
        item = _object(value, item_path)
        labels.append(Label(_text(_field(item, "text", item_path), f"{item_path}.text"), float(_number(_field(item, "x", item_path), f"{item_path}.x")), float(_number(_field(item, "y", item_path), f"{item_path}.y")), float(_number(_field(item, "size", item_path), f"{item_path}.size")), item.get("align", "l"), item.get("b", "norm")))
    title_raw = _object(_field(obj, "title", path), f"{path}.title")
    title = Label(_text(_field(title_raw, "text", f"{path}.title"), f"{path}.title.text"), float(_number(_field(title_raw, "x", f"{path}.title"), f"{path}.title.x")), float(_number(_field(title_raw, "y", f"{path}.title"), f"{path}.title.y")), float(_number(_field(title_raw, "size", f"{path}.title"), f"{path}.title.size")), "l", "norm")
    hint_raw = _object(_field(obj, "hint", path), f"{path}.hint")
    hint = Hint(float(_number(_field(hint_raw, "x", f"{path}.hint"), f"{path}.hint.x")), float(_number(_field(hint_raw, "y", f"{path}.hint"), f"{path}.hint.y")), float(_number(_field(hint_raw, "w", f"{path}.hint"), f"{path}.hint.w")), float(_number(_field(hint_raw, "size", f"{path}.hint"), f"{path}.hint.size")), _text(_field(hint_raw, "default", f"{path}.hint"), f"{path}.hint.default"))

    controls = []
    seen: set[str] = set()
    for i, value in enumerate(_field(obj, "controls", path)):
        item_path = f"{path}.controls[{i}]"
        item = _object(value, item_path)
        ident = _text(_field(item, "id", item_path), f"{item_path}.id")
        if ident in seen:
            raise _error(f"{item_path}.id", "is duplicated")
        seen.add(ident)
        kind = _text(_field(item, "kind", item_path), f"{item_path}.kind")
        if kind not in CONTROL_KINDS:
            raise _error(f"{item_path}.kind", f"unknown control kind {kind!r}")
        choices = tuple(item.get("choices", ()))
        if kind == "choice" and (not choices or not all(isinstance(choice, str) for choice in choices)):
            raise _error(f"{item_path}.choices", "choice controls need non-empty text choices")
        controls.append(Control(ident, _text(_field(item, "block", item_path), f"{item_path}.block"), _text(_field(item, "label", item_path), f"{item_path}.label"), float(_number(_field(item, "x", item_path), f"{item_path}.x")), float(_number(_field(item, "y", item_path), f"{item_path}.y")), float(_number(_field(item, "w", item_path), f"{item_path}.w")), kind, item.get("min"), item.get("max"), item.get("step"), item.get("unit", ""), int(item.get("decimals", 0)), choices))

    animation_raw = _object(_field(obj, "animation", path), f"{path}.animation")
    animation = Animation(*(float(_number(_field(animation_raw, name, f"{path}.animation"), f"{path}.animation.{name}")) for name in ("pulse_speed", "pulse_spacing", "reel_rpm", "flicker", "boot_ms", "refresh_flash_ms")))
    return SchematicData(canvas, theme, title, tuple(blocks), tuple(instances), tuple(wires), tuple(labels), hint, tuple(controls), glyphs, animation)


def load_data(data_dir: str | Path | None = None) -> SchematicData:
    folder = Path(data_dir) if data_dir is not None else Path(__file__).resolve().parent
    layout_path = folder / "layout.json"
    glyph_path = folder / "glyphs.json"
    if not layout_path.is_file():
        raise FileNotFoundError(layout_path)
    if not glyph_path.is_file():
        raise FileNotFoundError(glyph_path)
    try:
        glyphs_raw = json.loads(glyph_path.read_text())
        layout_raw = json.loads(layout_path.read_text())
    except (json.JSONDecodeError, OSError) as exc:
        raise ValueError(f"{folder}: invalid schematic JSON: {exc}") from exc
    glyphs = _parse_glyphs(glyphs_raw, str(glyph_path))
    return _parse_layout(layout_raw, str(layout_path), glyphs)
