# CONTRACT
"""Acceptance tests for the phosphor schematic widget (Qt, offscreen).

They fix the public surface of `vhsgui.schematic.view.SchematicView`, which draws the two data
files described in docs/design/FORMAT.md and turns the lines of the drawing into controls:

    SchematicView(parent=None, *, data_dir=None)      data_dir: folder with layout.json and glyphs.json
    valueChanged(id, value)                           Signal(str, object): float, int, bool or str
    resetRequested(id)                                Signal(str): the user double clicked a line
    control_ids() / value(id) / values() / set_value(id, value, emit=False)
    choices(id) / set_choices(id, names)              the words of a choice line; the window passes the presets
    set_baseline(values) / is_edited(id)              the preset's values; an edited line is marked
    control_rect(id) / control_at(pos)                widget coordinates of a control's row (18 canvas px tall)
    hover_control() / current_control() / set_current_control(id)
    set_help({id: text}) / hint_text()                the explanation printed in the hint band
    set_screen_image(QImage | None) / screen_image() / screen_rect() / set_tv_zoom(bool) / tv_zoomed()
                                                      screen_image() returns exactly the image it was given
    set_animated(bool) / is_animated() / is_ticking() / advance(seconds) / clock()
    start_boot() / skip_boot() / boot_progress()
    set_effects(glow=True, scanlines=True, vignette=True) / set_flicker(bool)
    render_to_image(width) -> QImage                  deterministic, no flicker; height = width * 620 / 1180
    overflowing_controls() -> list[str]               rows whose text does not fit at the real font

A new view is settled (boot_progress() == 1.0), animated, and every control is at its smallest value
(the first choice, False, the minimum). The animation clock runs on `advance(dt)` and, while the view
is visible and animated, on its own timer. `set_animated(False)` stops only decorative motion (pulses,
reels, TV snow, flicker), which then rest at their phase at clock 0; boot and value flashes still follow
the clock. `control_rect`, `screen_rect` and the mouse positions are in widget coordinates. The keys act
on `current_control()` (set by a press or by `set_current_control`): Left/Right step (Shift: ten steps),
Home/End go to the ends, Space flips a toggle, Up/Down move to the nearest row above/below whose extent
overlaps more than half of the current row's width. The wheel and the mouse act on the row under the pointer.

Also specified here: the widget has strong focus and mouse tracking; sizeHint() is the canvas size and
minimumSizeHint() at least 900 px wide. Sliders: the whole row is the track; Shift multiplies a wheel or key
step by ten. Steppers: a click on the left (right) half of the row steps down (up), Shift makes it ten, the
value is clamped. Toggles flip on a click or Space. Choices: a left click cycles forward and wraps; a right
click or a Shift click goes back and wraps; the wheel and the arrow keys step without wrapping. A double
click emits resetRequested and nothing else. set_choices() keeps the current word if it is still offered and
falls back to the first one otherwise. A click on the TV screen zooms it; any click or Esc puts it back and
nothing else reacts while it is zoomed. hint_text() is the help of the hovered line, else of the current
line, else the layout's `hint.default`.
"""
from __future__ import annotations

import json
import math
import os
import time
from pathlib import Path

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np
import pytest

pytest.importorskip("PySide6")
from PySide6.QtCore import QEvent, QPoint, QPointF, QRect, QRectF, Qt  # noqa: E402
from PySide6.QtGui import QImage, QKeyEvent, QMouseEvent, QPainter, QWheelEvent  # noqa: E402
from PySide6.QtWidgets import QApplication  # noqa: E402

from vhsgui.schematic.view import SchematicView  # noqa: E402

DATA = Path(__file__).resolve().parents[1] / "vhsgui" / "schematic"
LAYOUT = json.loads((DATA / "layout.json").read_text())
GLYPHS = json.loads((DATA / "glyphs.json").read_text())["glyphs"]
CONTROLS = {c["id"]: c for c in LAYOUT["controls"]}
CW, CH = LAYOUT["canvas"]["w"], LAYOUT["canvas"]["h"]
ROW_H = 18
ANIM = LAYOUT["animation"]
FLASH_S = ANIM["refresh_flash_ms"] / 1000
BOOT_S = ANIM["boot_ms"] / 1000
NO_EFFECTS = dict(glow=False, scanlines=False, vignette=False)

LEFT, RIGHT = Qt.MouseButton.LeftButton, Qt.MouseButton.RightButton
NOBTN = Qt.MouseButton.NoButton
SHIFT, NOMOD = Qt.KeyboardModifier.ShiftModifier, Qt.KeyboardModifier.NoModifier


# ---------------------------------------------------------------- helpers


@pytest.fixture(scope="session")
def app():
    return QApplication.instance() or QApplication([])


@pytest.fixture
def view(app):
    v = SchematicView()
    v.resize(CW, CH)
    yield v
    v.hide()
    v.deleteLater()


def rgb(image: QImage) -> np.ndarray:
    img = image.convertToFormat(QImage.Format.Format_RGB32)
    w, h = img.width(), img.height()
    raw = np.frombuffer(img.constBits(), np.uint8).reshape(h, img.bytesPerLine() // 4, 4)[:, :w]
    return raw[:, :, [2, 1, 0]].astype(np.float64)


def lum(a: np.ndarray) -> np.ndarray:
    return a @ np.array([0.2126, 0.7152, 0.0722])


def shot(view: SchematicView, width: int = CW) -> np.ndarray:
    image = view.render_to_image(width)
    assert (image.width(), image.height()) == (width, round(width * CH / CW))
    return rgb(image)


def crop(a: np.ndarray, r: QRectF, grow: float = 0) -> np.ndarray:
    x0, y0 = max(0, int(math.floor(r.left() - grow))), max(0, int(math.floor(r.top() - grow)))
    x1, y1 = int(math.ceil(r.right() + grow)), int(math.ceil(r.bottom() + grow))
    return a[y0:y1, x0:x1]


def mouse(view, kind, pos, button=LEFT, buttons=None, mods=NOMOD):
    if buttons is None:
        buttons = NOBTN if kind == QEvent.Type.MouseButtonRelease else button
    p = QPointF(pos)
    QApplication.sendEvent(view, QMouseEvent(kind, p, QPointF(view.mapToGlobal(p.toPoint())), button, buttons, mods))


def press(view, pos, button=LEFT, mods=NOMOD):
    mouse(view, QEvent.Type.MouseButtonPress, pos, button, mods=mods)


def release(view, pos, button=LEFT, mods=NOMOD):
    mouse(view, QEvent.Type.MouseButtonRelease, pos, button, mods=mods)


def drag(view, pos, button=LEFT, mods=NOMOD):
    mouse(view, QEvent.Type.MouseMove, pos, NOBTN, buttons=button, mods=mods)


def hover(view, pos):
    mouse(view, QEvent.Type.MouseMove, pos, NOBTN, buttons=NOBTN)


def click(view, pos, button=LEFT, mods=NOMOD):
    press(view, pos, button, mods)
    release(view, pos, button, mods)


def double_click(view, pos):
    click(view, pos)
    mouse(view, QEvent.Type.MouseButtonDblClick, pos)
    release(view, pos)


def wheel(view, pos, delta, mods=NOMOD):
    p = QPointF(pos)
    QApplication.sendEvent(view, QWheelEvent(p, QPointF(view.mapToGlobal(p.toPoint())), QPoint(0, 0), QPoint(0, delta), NOBTN, mods, Qt.ScrollPhase.NoScrollPhase, False))


def key(view, k, mods=NOMOD):
    QApplication.sendEvent(view, QKeyEvent(QEvent.Type.KeyPress, k, mods))
    QApplication.sendEvent(view, QKeyEvent(QEvent.Type.KeyRelease, k, mods))


def at(view, cid, frac=0.5):
    """A point on the text part of a control's row, `frac` of the way along it."""
    r = view.control_rect(cid)
    return QPointF(r.left() + frac * r.width(), r.top() + 0.4 * r.height())


def snap(cid, v):
    c = CONTROLS[cid]
    return min(c["max"], max(c["min"], c["min"] + round((v - c["min"]) / c["step"]) * c["step"]))


def picture(w=320, h=240) -> QImage:
    """Blue grows to the right, green downwards, a white square at the top left (no phosphor is blue)."""
    a = np.zeros((h, w, 4), np.uint8)
    a[..., 3] = 255
    a[..., 0] = np.linspace(0, 255, w, dtype=np.uint8)[None, :]
    a[..., 1] = np.linspace(0, 255, h, dtype=np.uint8)[:, None]
    a[h // 6 : h // 2, w // 8 : w // 3, :3] = 255
    return QImage(a.data, w, h, w * 4, QImage.Format.Format_RGB32).copy()


def thumb(image: QImage, rect: QRectF | None = None) -> np.ndarray:
    if rect is not None:
        image = image.copy(QRect(round(rect.x()), round(rect.y()), round(rect.width()), round(rect.height())))
    small = image.scaled(40, 30, Qt.AspectRatioMode.IgnoreAspectRatio, Qt.TransformationMode.SmoothTransformation)
    return lum(rgb(small)).ravel()


def lit(a: np.ndarray, level=60) -> int:
    return int((a.max(axis=2) > level).sum())


def default_of(c):
    return {"slider": lambda: float(c["min"]), "stepper": lambda: int(c["min"]), "toggle": lambda: False, "choice": lambda: c["choices"][0]}[c["kind"]]()


def letterboxed(pic: QImage) -> np.ndarray:
    """What a 4:3 screen shows of `pic` when it keeps its shape: the picture with black bars."""
    canvas = QImage(40, 30, QImage.Format.Format_RGB32)
    canvas.fill(0)
    fitted = pic.scaled(40, 30, Qt.AspectRatioMode.KeepAspectRatio, Qt.TransformationMode.SmoothTransformation)
    painter = QPainter(canvas)
    painter.drawImage((40 - fitted.width()) // 2, (30 - fitted.height()) // 2, fitted)
    painter.end()
    return lum(rgb(canvas)).ravel()


# ---------------------------------------------------------------- the controls


def test_a_new_view_is_settled_and_knows_every_control(view):
    assert view.control_ids() == [c["id"] for c in LAYOUT["controls"]]
    assert view.boot_progress() == 1.0 and view.is_animated()
    for cid, c in CONTROLS.items():
        expected = default_of(c)
        assert view.value(cid) == expected and type(view.value(cid)) is type(expected), cid
    assert view.values() == {cid: view.value(cid) for cid in view.control_ids()}
    assert view.focusPolicy() == Qt.FocusPolicy.StrongFocus and view.hasMouseTracking()
    assert (view.sizeHint().width(), view.sizeHint().height()) == (CW, CH) and view.minimumSizeHint().width() >= 900


def test_set_value_snaps_clamps_and_keeps_the_type(view):
    view.set_value("head_wear", 0.337)
    assert view.value("head_wear") == pytest.approx(0.34, abs=1e-9) and isinstance(view.value("head_wear"), float)
    view.set_value("head_wear", 7)
    assert view.value("head_wear") == pytest.approx(1.0)
    view.set_value("head_wear", -3)
    assert view.value("head_wear") == pytest.approx(0.0)
    view.set_value("generations", 3.7)
    assert view.value("generations") == 4 and type(view.value("generations")) is int
    view.set_value("generations", 99)
    assert view.value("generations") == 8
    view.set_value("psu_smps", 1)
    assert view.value("psu_smps") is True
    view.set_value("tape_speed", "lp")
    assert view.value("tape_speed") == "lp"
    view.set_value("tape_age_years", 12.34)  # a step that is not a power of ten
    c = CONTROLS["tape_age_years"]
    assert view.value("tape_age_years") == pytest.approx(snap("tape_age_years", 12.34), abs=1e-9)
    assert (view.value("tape_age_years") - c["min"]) / c["step"] == pytest.approx(round((view.value("tape_age_years") - c["min"]) / c["step"]), abs=1e-6)


def test_unknown_controls_and_choices_are_errors(view):
    with pytest.raises(KeyError):
        view.value("nothing")
    with pytest.raises(KeyError):
        view.set_value("nothing", 1)
    with pytest.raises(ValueError):
        view.set_value("tape_speed", "warp")
    with pytest.raises((TypeError, ValueError)):
        view.set_value("head_wear", "much")
    assert view.value("tape_speed") == "sp" and view.value("head_wear") == 0.0


def test_value_changed_is_emitted_only_when_asked_and_only_on_a_change(view):
    seen = []
    view.valueChanged.connect(lambda cid, v: seen.append((cid, v)))
    view.set_value("head_wear", 0.5)  # emit defaults to False
    assert seen == []
    view.set_value("head_wear", 0.5, emit=True)  # no change
    assert seen == []
    view.set_value("head_wear", 0.551, emit=True)
    assert seen == [("head_wear", pytest.approx(0.55, abs=1e-9))]
    view.set_value("psu_smps", True, emit=True)
    view.set_value("tape_speed", "ep", emit=True)
    view.set_value("generations", 3, emit=True)
    assert [s[0] for s in seen[1:]] == ["psu_smps", "tape_speed", "generations"]
    assert seen[1][1] is True and seen[2][1] == "ep" and seen[3][1] == 3 and type(seen[3][1]) is int


def test_values_round_trip(view):
    wanted = {}
    for cid, c in CONTROLS.items():
        if c["kind"] == "slider":
            wanted[cid] = snap(cid, c["min"] + 0.3 * (c["max"] - c["min"]))
        elif c["kind"] == "stepper":
            wanted[cid] = min(c["max"], c["min"] + 2)
        elif c["kind"] == "toggle":
            wanted[cid] = True
        else:
            wanted[cid] = c["choices"][-1]
        view.set_value(cid, wanted[cid])
    for cid, v in wanted.items():
        if isinstance(v, float):
            assert view.value(cid) == pytest.approx(v, abs=1e-9), cid
        else:
            assert view.value(cid) == v, cid


def test_the_words_of_a_choice_can_be_replaced(view):
    assert view.choices("tape_speed") == ["sp", "lp", "ep"] and view.choices("preset") == CONTROLS["preset"]["choices"]
    view.set_value("preset", CONTROLS["preset"]["choices"][2])
    view.set_choices("preset", ["alpha", "beta"])
    assert view.choices("preset") == ["alpha", "beta"] and view.value("preset") == "alpha", "a value that is gone falls back to the first word"
    view.set_value("preset", "beta")
    view.set_choices("preset", ["beta", "gamma", "delta"])
    assert view.value("preset") == "beta"
    click(view, at(view, "preset"))
    assert view.value("preset") == "gamma"
    with pytest.raises(ValueError):
        view.set_choices("preset", [])
    with pytest.raises(ValueError):
        view.set_choices("head_wear", ["a", "b"])
    with pytest.raises(ValueError):
        view.set_value("preset", "alpha")
    assert view.overflowing_controls() == []
    view.set_choices("preset", ["a-name-that-is-far-too-long-to-fit-on-the-line"])
    assert view.overflowing_controls() == ["preset"]


# ---------------------------------------------------------------- geometry


def test_rows_sit_where_the_layout_puts_them_at_scale_one(view):
    for cid, c in CONTROLS.items():
        r = view.control_rect(cid)
        assert (r.x(), r.y(), r.width(), r.height()) == pytest.approx((c["x"], c["y"], c["w"], ROW_H), abs=0.6), cid


def test_the_canvas_scales_and_stays_centred(view):
    c = CONTROLS["head_wear"]
    view.resize(2 * CW, 2 * CH)
    r = view.control_rect("head_wear")
    assert (r.x(), r.y(), r.width(), r.height()) == pytest.approx((2 * c["x"], 2 * c["y"], 2 * c["w"], 2 * ROW_H), abs=1.0)
    view.resize(2000, 400)  # short and wide: scaled to the height, centred across
    s = 400 / CH
    r = view.control_rect("head_wear")
    assert r.width() == pytest.approx(c["w"] * s, abs=1.0) and r.height() == pytest.approx(ROW_H * s, abs=0.5)
    assert r.x() == pytest.approx((2000 - CW * s) / 2 + c["x"] * s, abs=1.0) and r.y() == pytest.approx(c["y"] * s, abs=1.0)
    view.resize(600, 1000)  # narrow and tall: scaled to the width, centred down
    s = 600 / CW
    r = view.control_rect("head_wear")
    assert r.width() == pytest.approx(c["w"] * s, abs=1.0) and r.y() == pytest.approx((1000 - CH * s) / 2 + c["y"] * s, abs=1.0)


def test_control_at_finds_the_row_under_a_point(view):
    for cid in view.control_ids():
        r = view.control_rect(cid)
        assert view.control_at(r.center()) == cid
        assert view.control_at(QPointF(r.left() + 1, r.bottom() - 0.5)) == cid, "the gauge line belongs to its row"
    assert view.control_at(QPointF(1, 1)) is None
    assert view.control_at(QPointF(-50, 20)) is None


def test_every_row_of_text_fits_at_the_real_font(view):
    assert view.overflowing_controls() == []


# ---------------------------------------------------------------- interaction: sliders


def test_pressing_and_dragging_along_a_slider_row_sets_the_value_from_the_position(view):
    seen = []
    view.valueChanged.connect(lambda cid, v: seen.append((cid, v)))
    c = CONTROLS["head_wear"]
    span = c["max"] - c["min"]
    tol = max(2 * c["step"], 0.02 * span)
    press(view, at(view, "head_wear", 0.25))
    assert view.value("head_wear") == pytest.approx(c["min"] + 0.25 * span, abs=tol)
    assert view.current_control() == "head_wear"
    drag(view, at(view, "head_wear", 0.75))
    assert view.value("head_wear") == pytest.approx(c["min"] + 0.75 * span, abs=tol)
    r = view.control_rect("head_wear")
    drag(view, QPointF(r.left() + 0.5 * r.width(), r.top() - 60))  # off the row, the drag goes on
    assert view.value("head_wear") == pytest.approx(c["min"] + 0.5 * span, abs=tol)
    drag(view, QPointF(r.right() + 80, r.top() - 60))
    assert view.value("head_wear") == pytest.approx(c["max"])
    drag(view, QPointF(r.left() - 80, r.bottom() + 40))
    assert view.value("head_wear") == pytest.approx(c["min"])
    release(view, QPointF(r.left() - 80, r.bottom() + 40))
    hover(view, at(view, "head_wear", 0.5))  # no button: nothing moves
    assert view.value("head_wear") == pytest.approx(c["min"])
    assert len(seen) >= 4 and all(cid == "head_wear" and isinstance(v, float) and c["min"] <= v <= c["max"] for cid, v in seen)
    assert seen[-1][1] == pytest.approx(c["min"])


def test_a_slider_snaps_to_its_step_while_dragging(view):
    c = CONTROLS["tape_age_years"]
    press(view, at(view, "tape_age_years", 0.37))
    drag(view, at(view, "tape_age_years", 0.613))
    release(view, at(view, "tape_age_years", 0.613))
    n = (view.value("tape_age_years") - c["min"]) / c["step"]
    assert n == pytest.approx(round(n), abs=1e-6) and 0.5 * (c["max"] - c["min"]) < view.value("tape_age_years") - c["min"] < 0.7 * (c["max"] - c["min"])


def test_the_wheel_and_the_keys_step_a_slider(view):
    c = CONTROLS["head_wear"]
    step = c["step"]
    view.set_value("head_wear", 0.5)
    wheel(view, at(view, "head_wear"), 120)
    assert view.value("head_wear") == pytest.approx(0.5 + step, abs=1e-9)
    wheel(view, at(view, "head_wear"), -120)
    wheel(view, at(view, "head_wear"), -120)
    assert view.value("head_wear") == pytest.approx(0.5 - step, abs=1e-9)
    wheel(view, at(view, "head_wear"), 120, SHIFT)
    assert view.value("head_wear") == pytest.approx(0.5 - step + 10 * step, abs=1e-9)
    view.set_value("head_wear", 0.5)
    view.set_current_control("head_wear")
    assert view.current_control() == "head_wear"
    key(view, Qt.Key.Key_Right)
    assert view.value("head_wear") == pytest.approx(0.5 + step, abs=1e-9)
    key(view, Qt.Key.Key_Left)
    key(view, Qt.Key.Key_Left)
    assert view.value("head_wear") == pytest.approx(0.5 - step, abs=1e-9)
    key(view, Qt.Key.Key_Right, SHIFT)
    assert view.value("head_wear") == pytest.approx(0.5 - step + 10 * step, abs=1e-9)
    key(view, Qt.Key.Key_End)
    assert view.value("head_wear") == pytest.approx(c["max"])
    key(view, Qt.Key.Key_Right)
    assert view.value("head_wear") == pytest.approx(c["max"]), "clamped"
    key(view, Qt.Key.Key_Home)
    assert view.value("head_wear") == pytest.approx(c["min"])


def test_the_wheel_acts_on_the_row_under_the_pointer_only(view):
    view.set_value("head_wear", 0.5)
    view.set_current_control("head_wear")
    wheel(view, QPointF(2, 2), 120)
    assert view.value("head_wear") == pytest.approx(0.5)


# ---------------------------------------------------------------- interaction: steppers, toggles, choices


def test_a_stepper_steps_by_clicks_on_its_halves_the_wheel_and_the_keys(view):
    top = CONTROLS["generations"]["max"]
    click(view, at(view, "generations", 0.2))
    assert view.value("generations") == 1, "already at the minimum"
    click(view, at(view, "generations", 0.8))
    assert view.value("generations") == 2 and type(view.value("generations")) is int
    click(view, at(view, "generations", 0.8))
    click(view, at(view, "generations", 0.2))
    assert view.value("generations") == 2
    click(view, at(view, "generations", 0.8), mods=SHIFT)
    assert view.value("generations") == top, "Shift steps by ten and clamps"
    click(view, at(view, "generations", 0.2), mods=SHIFT)
    assert view.value("generations") == 1
    wheel(view, at(view, "generations"), 120)
    assert view.value("generations") == 2
    view.set_current_control("generations")
    key(view, Qt.Key.Key_Right)
    key(view, Qt.Key.Key_Right)
    key(view, Qt.Key.Key_Left)
    assert view.value("generations") == 3
    click(view, at(view, "seed", 0.8), mods=SHIFT)
    assert view.value("seed") == 10


def test_a_toggle_flips_on_a_click_or_the_space_bar(view):
    assert view.value("psu_smps") is False
    click(view, at(view, "psu_smps", 0.3))
    assert view.value("psu_smps") is True
    click(view, at(view, "psu_smps", 0.9))
    assert view.value("psu_smps") is False
    view.set_current_control("psu_smps")
    key(view, Qt.Key.Key_Space)
    assert view.value("psu_smps") is True


def test_a_choice_cycles_on_clicks_and_steps_with_the_wheel_and_the_keys(view):
    choices = CONTROLS["tape_speed"]["choices"]
    assert choices == ["sp", "lp", "ep"]
    click(view, at(view, "tape_speed"))
    assert view.value("tape_speed") == "lp"
    click(view, at(view, "tape_speed"))
    click(view, at(view, "tape_speed"))
    assert view.value("tape_speed") == "sp", "a click wraps round"
    click(view, at(view, "tape_speed"), RIGHT)
    assert view.value("tape_speed") == "ep", "right click goes back, and wraps"
    click(view, at(view, "tape_speed"), mods=SHIFT)
    assert view.value("tape_speed") == "lp", "Shift click goes back"
    wheel(view, at(view, "tape_speed"), 120)
    assert view.value("tape_speed") == "ep"
    wheel(view, at(view, "tape_speed"), 120)
    assert view.value("tape_speed") == "ep", "the wheel does not wrap"
    wheel(view, at(view, "tape_speed"), -120)
    assert view.value("tape_speed") == "lp"
    view.set_current_control("tape_speed")
    key(view, Qt.Key.Key_Left)
    key(view, Qt.Key.Key_Left)
    assert view.value("tape_speed") == "sp", "the keys do not wrap"
    key(view, Qt.Key.Key_End)
    assert view.value("tape_speed") == "ep"
    key(view, Qt.Key.Key_Home)
    assert view.value("tape_speed") == "sp"
    many = CONTROLS["preset"]["choices"]
    for _ in range(len(many)):
        click(view, at(view, "preset"))
    assert view.value("preset") == many[0]


def test_every_change_by_the_user_is_announced(view):
    seen = []
    view.valueChanged.connect(lambda cid, v: seen.append((cid, v)))
    click(view, at(view, "tape_speed"))
    click(view, at(view, "psu_smps"))
    click(view, at(view, "generations", 0.8))
    wheel(view, at(view, "head_wear"), 120)
    key_target = "tape_age_years"
    view.set_current_control(key_target)
    key(view, Qt.Key.Key_Right)
    assert [cid for cid, _ in seen] == ["tape_speed", "psu_smps", "generations", "head_wear", key_target]


def test_a_double_click_asks_for_a_reset(view):
    asked = []
    view.resetRequested.connect(asked.append)
    double_click(view, at(view, "head_clog"))
    assert asked == ["head_clog"]
    double_click(view, at(view, "psu_smps"))
    assert asked == ["head_clog", "psu_smps"]
    double_click(view, QPointF(2, 2))
    assert len(asked) == 2


# ---------------------------------------------------------------- pointer and keyboard focus


def test_hover_and_the_current_line(view):
    assert view.hover_control() is None and view.current_control() is None
    hover(view, view.control_rect("head_clog").center())
    assert view.hover_control() == "head_clog"
    hover(view, QPointF(2, 2))
    assert view.hover_control() is None
    hover(view, view.control_rect("head_clog").center())
    QApplication.sendEvent(view, QEvent(QEvent.Type.Leave))
    assert view.hover_control() is None
    with pytest.raises(KeyError):
        view.set_current_control("nothing")


def test_hovering_a_row_brightens_it_and_nothing_else(view):
    view.set_animated(False)
    view.set_screen_image(picture())
    idle = shot(view)
    r = view.control_rect("head_clog")
    hover(view, r.center())
    over = shot(view)
    assert lum(crop(over, r, 3)).sum() > lum(crop(idle, r, 3)).sum() * 1.03
    mask = np.ones(idle.shape[:2], bool)
    h = LAYOUT["hint"]
    mask[max(0, int(r.top() - 12)) : int(r.bottom() + 12), max(0, int(r.left() - 40)) : int(r.right() + 40)] = False
    mask[int(h["y"] - h["size"] - 4) : int(h["y"] + 8), :] = False  # the hint band changes too
    assert np.abs(over - idle)[mask].max() < 1.0, "only the row and the hint band change"


def test_the_arrow_keys_move_between_the_rows_of_a_block(view):
    ids = view.control_ids()
    rects = {cid: view.control_rect(cid) for cid in ids}

    def overlaps(r, o):
        return min(rects[o].right(), r.right()) - max(rects[o].left(), r.left()) > 0.5 * r.width()

    def below(cid):
        r = rects[cid]
        return min((o for o in ids if rects[o].top() > r.top() + 1 and overlaps(r, o)), key=lambda o: rects[o].top(), default=None)

    def above(cid):
        r = rects[cid]
        return max((o for o in ids if rects[o].top() < r.top() - 1 and overlaps(r, o)), key=lambda o: rects[o].top(), default=None)

    movable = [cid for cid in ids if below(cid) is not None]
    assert len(movable) >= 12
    for cid in movable[:6]:
        view.set_current_control(cid)
        key(view, Qt.Key.Key_Down)
        assert view.current_control() == below(cid), cid
        key(view, Qt.Key.Key_Up)
        assert view.current_control() == above(below(cid)), cid
    last = next(cid for cid in ids if below(cid) is None)
    view.set_current_control(last)
    key(view, Qt.Key.Key_Down)
    assert view.current_control() == last


def test_the_hint_line_explains_the_line_under_the_pointer(view):
    view.set_animated(False)
    h = LAYOUT["hint"]
    view.set_help({"head_wear": "THE TEST SENTENCE ABOUT HEAD WEAR"})
    assert view.hint_text() == h["default"]
    idle = shot(view)
    hover(view, view.control_rect("head_wear").center())
    assert view.hint_text() == "THE TEST SENTENCE ABOUT HEAD WEAR"
    band = QRectF(h["x"], h["y"] - h["size"] - 2, h["w"], h["size"] + 8)
    assert (np.abs(crop(shot(view), band) - crop(idle, band)).sum(axis=2) > 30).sum() >= 20, "the band shows the new text"
    hover(view, view.control_rect("head_clog").center())
    assert view.hint_text() == h["default"], "a line without help text shows the default hint"
    hover(view, QPointF(2, 2))
    assert view.hint_text() == h["default"]


# ---------------------------------------------------------------- edited marks and flashes


def test_the_edited_mark_follows_the_baseline(view):
    view.set_animated(False)
    view.set_screen_image(picture())
    view.set_value("head_wear", 0.7)
    view.set_baseline({"head_wear": 0.7, "tape_speed": "sp"})
    assert not view.is_edited("head_wear") and not view.is_edited("tape_speed") and not view.is_edited("seed")
    same = shot(view)
    view.set_baseline({"head_wear": 0.5, "tape_speed": "sp"})
    assert view.is_edited("head_wear") and not view.is_edited("tape_speed")
    edited = shot(view)
    ys, xs = np.nonzero(np.abs(edited - same).sum(axis=2) > 30)
    r = view.control_rect("head_wear")
    assert len(xs) >= 6, "an edited line shows a mark"
    assert ((xs >= r.left() - 24) & (xs <= r.right() + 24) & (ys >= r.top() - 4) & (ys <= r.bottom() + 4)).all(), "the mark belongs to its own line"
    view.set_value("tape_speed", "lp")
    assert view.is_edited("tape_speed")
    view.set_baseline({})
    assert not any(view.is_edited(cid) for cid in view.control_ids())


def test_a_changed_value_flashes_and_then_settles(view):
    view.set_animated(False)
    view.set_screen_image(picture())
    view.set_value("head_wear", 0.3)
    flash = shot(view)
    view.advance(FLASH_S + 0.3)
    settled = shot(view)
    r = view.control_rect("head_wear")
    assert lum(crop(flash, r, 4)).sum() >= lum(crop(settled, r, 4)).sum() * 1.03, "the changed value is brighter at first"
    mask = np.ones(flash.shape[:2], bool)
    mask[max(0, int(r.top() - 12)) : int(r.bottom() + 12), max(0, int(r.left() - 12)) : int(r.right() + 12)] = False
    assert np.abs(flash - settled)[mask].max() < 1.0, "nothing else changes"
    again = shot(view)
    view.advance(1.0)
    assert np.array_equal(again, shot(view)), "and then the picture is still"


# ---------------------------------------------------------------- the TV


def test_the_tv_shows_the_picture_inside_its_screen(view):
    pic = picture()
    assert view.screen_image() is None
    view.set_screen_image(pic)
    assert view.screen_image() == pic
    screen = view.screen_rect()
    assert screen is not None and screen.width() >= 200 and screen.height() >= 150
    assert screen.width() / screen.height() == pytest.approx(4 / 3, abs=0.12)
    image = view.render_to_image(CW)
    a = rgb(image)
    assert np.corrcoef(thumb(image, screen), thumb(pic))[0, 1] > 0.85, "the picture is what the screen shows"
    blue = (a[..., 2] > 100) & (a[..., 2] > 1.4 * a[..., 1]) & (a[..., 2] > 1.4 * a[..., 0])
    blue[int(screen.top() - 10) : int(screen.bottom() + 10), int(screen.left() - 10) : int(screen.right() + 10)] = False
    assert not blue.any(), "the picture stays inside its screen"
    view.set_screen_image(None)
    assert view.screen_image() is None
    empty = shot(view)
    assert np.abs(crop(empty, screen) - crop(a, screen)).mean() > 10 and crop(empty, screen).max() > 40, "no signal still lights the screen"


def test_a_picture_of_another_shape_is_not_stretched(view):
    pic = picture(640, 400)
    view.set_screen_image(pic)
    screen = view.screen_rect()
    got = thumb(view.render_to_image(CW), screen)
    shaped = np.corrcoef(got, letterboxed(pic))[0, 1]
    stretched = np.corrcoef(got, thumb(pic))[0, 1]
    assert shaped > 0.8 and shaped > stretched + 0.01, (shaped, stretched)


def test_clicking_the_tv_enlarges_the_picture(view):
    view.set_animated(False)
    pic = picture()
    view.set_screen_image(pic)
    before = view.screen_rect()
    assert not view.tv_zoomed()
    click(view, before.center())
    assert view.tv_zoomed()
    big = view.screen_rect()
    assert big.height() >= 1.8 * before.height() and big.width() >= 1.8 * before.width()
    assert QRectF(view.rect()).adjusted(4, 4, -4, -4).contains(big)
    assert np.corrcoef(thumb(view.render_to_image(CW), big), thumb(pic))[0, 1] > 0.85
    hover(view, view.control_rect("head_clog").center())
    assert view.hover_control() is None, "nothing behind the picture reacts"
    seen = []
    view.valueChanged.connect(lambda cid, v: seen.append(cid))
    click(view, at(view, "head_wear", 0.9))
    assert not view.tv_zoomed() and seen == [] and view.value("head_wear") == 0.0, "any click puts it back and does nothing else"
    assert view.screen_rect().height() == pytest.approx(before.height(), abs=1.0)
    view.set_tv_zoom(True)
    assert view.tv_zoomed()
    key(view, Qt.Key.Key_Escape)
    assert not view.tv_zoomed()
    click(view, before.center())
    click(view, before.center())
    assert not view.tv_zoomed()


# ---------------------------------------------------------------- how it looks


def test_it_looks_like_a_phosphor_screen(view):
    a = shot(view)
    assert np.median(lum(a)) < 0.06 * 255, "a dark screen"
    mask = a.max(axis=2) > 60
    assert 0.02 <= mask.mean() <= 0.45, "drawn, neither blank nor flooded"
    tv = view.screen_rect().adjusted(-4, -4, 4, 4)
    mask[max(0, int(tv.top())) : int(tv.bottom()), max(0, int(tv.left())) : int(tv.right())] = False  # the picture has its own colours
    r, g, b = a[mask].sum(axis=0)
    assert g > 1.6 * r and g > 1.3 * b, "green phosphor"


def test_the_glow_adds_light_around_the_strokes_and_never_takes_any_away(view):
    view.set_animated(False)
    view.set_screen_image(picture())
    tv = view.screen_rect().adjusted(-8, -8, 8, 8)
    outside = np.ones((CH, CW), bool)  # the picture is not part of the drawing's glow
    outside[max(0, int(tv.top())) : int(tv.bottom()), max(0, int(tv.left())) : int(tv.right())] = False
    view.set_effects(**NO_EFFECTS)
    crisp = lum(shot(view))
    view.set_effects(glow=True, scanlines=False, vignette=False)
    glow = lum(shot(view))
    assert glow[outside].sum() >= crisp[outside].sum() * 1.10
    assert ((glow > 8) & outside).sum() >= ((crisp > 8) & outside).sum() * 1.15
    assert (glow >= crisp - 2).all()


def test_the_scanlines_darken_every_nth_row(view):
    view.set_animated(False)
    view.set_screen_image(picture())
    period = LAYOUT["theme"]["scanline"]["period"]

    def spread(a):
        means = [lum(a)[k::period].mean() for k in range(period)]
        return max(means) / min(means)

    view.set_effects(**NO_EFFECTS)
    off = spread(shot(view))
    view.set_effects(glow=False, scanlines=True, vignette=False)
    on = spread(shot(view))
    assert on >= off * 1.05, (on, off)


def test_the_effects_switch_independently(view):
    view.set_animated(False)
    view.set_screen_image(picture())
    view.set_effects(**NO_EFFECTS)
    base = shot(view)
    for only in ("glow", "scanlines", "vignette"):
        view.set_effects(**{**NO_EFFECTS, only: True})
        assert not np.array_equal(shot(view), base), f"{only} changes the picture"
    view.set_effects()
    assert not np.array_equal(shot(view), base)
    view.set_effects(**NO_EFFECTS)
    assert np.array_equal(shot(view), base)


def test_the_painted_widget_matches_the_rendered_image(view):
    view.set_animated(False)
    view.set_flicker(False)
    view.set_screen_image(picture())
    painted = rgb(view.grab().toImage())
    rendered = shot(view)
    assert painted.shape == rendered.shape
    assert np.abs(painted - rendered).mean() < 1.0


def test_a_render_at_another_width_is_the_same_picture(view):
    """The TV picture, the rows and the wires must scale together: only the pixel count changes."""
    view.set_animated(False)
    view.set_screen_image(picture())
    screen = view.screen_rect()
    base = view.render_to_image(CW)
    small = lambda img: lum(rgb(img.scaled(236, 124, Qt.AspectRatioMode.IgnoreAspectRatio, Qt.TransformationMode.SmoothTransformation))).ravel()  # noqa: E731
    for width in (590, 886, 2360):
        other = view.render_to_image(width)
        k = width / CW
        tv = QRectF(screen.x() * k, screen.y() * k, screen.width() * k, screen.height() * k)
        assert np.corrcoef(thumb(other, tv), thumb(base, screen))[0, 1] > 0.9, f"at width {width} the TV shows a different picture"
        assert np.corrcoef(small(other), small(base))[0, 1] > 0.95, f"at width {width} the drawing is not the same drawing"


def test_rendering_works_at_other_sizes(view):
    for width in (590, 1180, 2360):
        a = shot(view, width)
        assert lit(a) / a[..., 0].size > 0.01, width


# ---------------------------------------------------------------- how it moves


def _near(points, polylines, tol):
    d = np.full(len(points), np.inf)
    for pts in polylines:
        for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
            vx, vy = x1 - x0, y1 - y0
            n = max(vx * vx + vy * vy, 1e-9)
            t = np.clip(((points[:, 0] - x0) * vx + (points[:, 1] - y0) * vy) / n, 0, 1)
            d = np.minimum(d, np.hypot(points[:, 0] - (x0 + t * vx), points[:, 1] - (y0 + t * vy)))
    return d <= tol


def _reels():
    out = []
    for i in LAYOUT["instances"]:
        for p in GLYPHS[i["glyph"]]["prims"]:
            if p[0] == "reel":
                out.append((i["x"] + p[1] * i["scale"], i["y"] + p[2] * i["scale"], p[3] * i["scale"]))
    return out


def test_pulses_run_along_the_wires_and_text_stands_still(view):
    view.set_effects(**NO_EFFECTS)
    view.set_screen_image(picture())
    a0 = shot(view)
    view.advance(0.37)
    a1 = shot(view)
    ys, xs = np.nonzero(np.abs(a1 - a0).sum(axis=2) > 24)
    assert len(xs) >= 30, "the pulses move"
    pts = np.stack([xs, ys], axis=1).astype(float)
    ok = _near(pts, [w["points"] for w in LAYOUT["wires"]], 14)
    for cx, cy, r in _reels():
        ok |= np.hypot(pts[:, 0] - cx, pts[:, 1] - cy) <= r + 6
    assert ok.all(), f"{(~ok).sum()} changed pixels are neither on a wire nor on a reel"
    for cid in view.control_ids():
        r = view.control_rect(cid)
        label = QRectF(r.left() + 6, r.top(), 0.35 * r.width() - 6, r.height())
        assert np.array_equal(crop(a0, label), crop(a1, label)), f"the label of {cid} moved"


def test_the_reels_turn(view):
    reels = _reels()
    assert reels
    view.set_effects(**NO_EFFECTS)
    view.set_screen_image(picture())
    a0 = shot(view)
    view.advance(0.07 * 60 / ANIM["reel_rpm"])
    a1 = shot(view)
    for cx, cy, r in reels:
        box = QRectF(cx - r, cy - r, 2 * r, 2 * r)
        assert (np.abs(crop(a1, box) - crop(a0, box)).sum(axis=2) > 24).sum() >= 8, f"the reel at ({cx:.0f}, {cy:.0f}) does not turn"


def test_a_view_that_is_not_animated_keeps_still(view):
    view.set_animated(False)
    assert not view.is_animated()
    view.set_screen_image(picture())
    a0 = shot(view)
    view.advance(1.3)
    assert np.array_equal(a0, shot(view))
    view.set_screen_image(None)
    s0 = shot(view)
    view.advance(0.4)
    assert np.array_equal(s0, shot(view)), "the snow on a dead TV is decoration too"
    view.set_animated(True)
    view.advance(0.4)
    assert not np.array_equal(s0, shot(view))
    assert view.clock() >= 2.1


def test_the_view_only_ticks_while_it_is_visible_and_animated(view):
    assert not view.is_ticking()
    view.show()
    assert view.is_ticking()
    view.hide()
    assert not view.is_ticking()
    view.set_animated(False)
    view.show()
    assert not view.is_ticking()
    view.set_animated(True)
    assert view.is_ticking()


def test_the_screen_draws_itself_line_by_line_at_start(view):
    view.set_animated(False)
    view.set_effects(**NO_EFFECTS)
    view.set_screen_image(picture())
    final = shot(view)
    view.start_boot()
    assert view.boot_progress() == 0.0
    start = lit(shot(view))
    assert start <= 0.15 * lit(final), "at the start only the sweep's leading line is lit"
    view.advance(BOOT_S * 0.5)
    half = lit(shot(view))
    assert 0 < view.boot_progress() < 1 and start < half < lit(final)
    last = half
    for _ in range(4):
        view.advance(BOOT_S * 0.1)
        now = lit(shot(view))
        assert now >= last
        last = now
    view.advance(BOOT_S)
    assert view.boot_progress() == 1.0
    assert np.array_equal(shot(view), final)
    view.start_boot()
    view.skip_boot()
    assert view.boot_progress() == 1.0 and np.array_equal(shot(view), final)


# ---------------------------------------------------------------- robustness


def test_bad_data_is_refused_clearly(app, tmp_path):
    with pytest.raises(FileNotFoundError):
        SchematicView(data_dir=tmp_path)
    for name in ("layout.json", "glyphs.json"):
        (tmp_path / name).write_text((DATA / name).read_text())
    SchematicView(data_dir=tmp_path).deleteLater()
    broken = json.loads((tmp_path / "layout.json").read_text())
    broken["version"] = 2
    (tmp_path / "layout.json").write_text(json.dumps(broken))
    with pytest.raises(ValueError, match="version"):
        SchematicView(data_dir=tmp_path)


_DPR_SCRIPT = """
import sys
import numpy as np
from PySide6.QtGui import QImage
from PySide6.QtWidgets import QApplication
app = QApplication([])
from vhsgui.schematic.view import SchematicView
def rgb(img):
    img = img.convertToFormat(QImage.Format.Format_RGB32)
    w, h = img.width(), img.height()
    return np.frombuffer(img.constBits(), np.uint8).reshape(h, img.bytesPerLine() // 4, 4)[:, :w, [2, 1, 0]].astype(float)
v = SchematicView()
v.set_animated(False)
v.set_flicker(False)
v.resize(590, 310)
painted = rgb(v.grab().toImage())
rendered = rgb(v.render_to_image(round(590 * v.devicePixelRatioF())))
assert painted.shape == rendered.shape, (painted.shape, rendered.shape)
print(float(np.abs(painted - rendered).mean()))
"""


@pytest.mark.parametrize("scale", ["1.25", "1.5", "2"])
def test_a_high_dpi_screen_shows_the_same_picture(scale):
    """On a screen with a scale factor the widget is painted at device resolution, once, not twice."""
    import subprocess
    import sys

    env = dict(os.environ, QT_QPA_PLATFORM="offscreen", QT_SCALE_FACTOR=scale, PYTHONPATH=str(DATA.parents[1]))
    result = subprocess.run([sys.executable, "-c", _DPR_SCRIPT], env=env, capture_output=True, text=True, timeout=120)
    assert result.returncode == 0, result.stderr
    assert float(result.stdout.strip().splitlines()[-1]) < 2.0, f"at scale factor {scale} the painted widget differs from the rendered picture"


def test_the_page_of_the_outside_world_loads_and_draws_like_the_circuit(app):
    """A second drawing in its own folder: the same engine, its own controls, a picture on its own TV."""
    folder = DATA / "outside"
    layout = json.loads((folder / "layout.json").read_text())
    other = SchematicView(data_dir=folder)
    other.resize(CW, CH)
    assert other.control_ids() == [c["id"] for c in layout["controls"]]
    assert not set(other.control_ids()) & set(SchematicView().control_ids())
    other.set_screen_image(picture())
    other.set_animated(False)
    a = shot(other)
    assert lit(a) / a[..., 0].size > 0.01 and other.screen_rect() is not None
    r = other.control_rect(other.control_ids()[3])
    seen = []
    other.valueChanged.connect(lambda cid, v: seen.append(cid))
    press(other, QPointF(r.left() + 0.9 * r.width(), r.top() + 0.4 * r.height()))
    release(other, QPointF(r.left() + 0.9 * r.width(), r.top() + 0.4 * r.height()))
    assert seen == [other.control_ids()[3]]
    other.deleteLater()


def test_painting_is_fast_enough_to_animate(view):
    view.set_screen_image(picture())
    view.grab()
    start = time.perf_counter()
    for i in range(20):
        view.advance(0.03)
        view.grab()
    assert time.perf_counter() - start < 6.0, "20 repaints took over 6 s: no blur filters, cache what does not move"


def _best_ms(fn, runs=15):
    fn()
    best = float("inf")
    for _ in range(runs):
        start = time.perf_counter()
        fn()
        best = min(best, (time.perf_counter() - start) * 1000)
    return best


def test_a_frame_has_a_time_budget(view):
    """The view stays open all day: static content is painted once and cached, only what moves is redrawn.

    Budgets are for a 1180 x 620 view on a desktop machine, best of fifteen (a full repaint of everything
    takes about 14 ms; the budgets ask for a fraction of that)."""
    view.set_screen_image(picture())
    view.set_animated(False)
    view.advance(5.0)
    still = _best_ms(view.grab)
    view.set_animated(True)

    def tick():
        view.advance(0.033)
        view.grab()

    moving = _best_ms(tick)
    assert still <= 5.0, f"a repaint of a scene that did not change took {still:.1f} ms (budget 5 ms): cache it"
    assert moving <= 8.0, f"a repaint with the pulses and reels moving took {moving:.1f} ms (budget 8 ms): redraw only what moves"


def test_the_labels_follow_the_standard(view):
    """The spec labels say what the chosen standard records: lines, frame rate, FM band."""
    view.set_value("standard", "ntsc")
    ntsc = " / ".join(view.visible_labels())
    view.set_value("standard", "pal")
    pal = " / ".join(view.visible_labels())
    assert "525" in ntsc and "3.4-4.4" in ntsc and "625" not in ntsc
    assert "625" in pal and "3.8-4.8" in pal and "525" not in pal
    view.set_value("standard", "pal60")
    assert "525" in " / ".join(view.visible_labels())


def test_a_label_change_reaches_the_picture(view):
    view.set_value("standard", "ntsc")
    a = rgb(view.render_to_image(CW))
    view.set_value("standard", "pal")
    b = rgb(view.render_to_image(CW))
    assert np.abs(a - b).sum() > 0, "the standard's labels are drawn"

