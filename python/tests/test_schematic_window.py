# CONTRACT
"""Acceptance tests for the schematic mode of the window (Qt, offscreen).

The schematic is a second way to see and change the same look. The editors of the window stay the
one place where the look lives (`MainWindow.look()` keeps working); the schematic mirrors them and
every change made on it goes through them. The surface fixed here:

    MainWindow.schematic_mode            True while the schematic is shown (read only)
    MainWindow.set_schematic_mode(on)    what the two buttons call
    MainWindow.schematic                 the SchematicView of the circuit, also found by the object name "schematicView"
    MainWindow.outside                   the SchematicView of the outside world (phone, radio, mains, magnets),
                                         object name "outsideView"; both mirror the same editors, both show the picture on their TV
    MainWindow.schematic_page            "circuit" or "outside": which of the two is shown in schematic mode
    MainWindow.set_schematic_page(name)  what the two page buttons call
    "panelsButton", "schematicButton"    checkable, exclusive, visible in both modes
    "circuitButton", "outsideButton"     checkable, exclusive, visible in schematic mode only
    python -m vhsgui                     starts in schematic mode (as does --schematic; --panels starts with the
                                         panels); with --screenshot the picture shows the settled drawing (no
                                         half-drawn boot); `--page outside` shows the other page

In schematic mode the editors and the preview picture are hidden (the TV shows the picture) and the rest
of the window stays reachable below the schematic. The window starts in panels mode.
"""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import pytest

pytest.importorskip("PySide6")
from PySide6.QtCore import QEvent, QPoint, QPointF, QRect, Qt  # noqa: E402
from PySide6.QtGui import QImage, QMouseEvent  # noqa: E402
from PySide6.QtWidgets import (  # noqa: E402
    QAbstractButton,
    QApplication,
    QCheckBox,
    QComboBox,
    QDoubleSpinBox,
    QLabel,
    QLineEdit,
    QProgressBar,
    QPushButton,
    QSlider,
    QSpinBox,
    QWidget,
)

from vhsgui import backend  # noqa: E402
from vhsgui.model import CONDITION_FIELDS  # noqa: E402
from vhsgui.schematic.view import SchematicView  # noqa: E402

PYTHON_DIR = Path(__file__).resolve().parents[1]
LEFT, NOBTN = Qt.MouseButton.LeftButton, Qt.MouseButton.NoButton
HIDDEN_IN_SCHEMATIC = [("field_head_wear", QDoubleSpinBox), ("presetCombo", QComboBox), ("advancedEdit", QWidget), ("previewView", QWidget)]
KEPT_IN_SCHEMATIC = [
    ("inputEdit", QLineEdit),
    ("browseInputButton", QPushButton),
    ("outputEdit", QLineEdit),
    ("browseOutputButton", QPushButton),
    ("startSpin", QDoubleSpinBox),
    ("durationSpin", QDoubleSpinBox),
    ("sizeCombo", QComboBox),
    ("crfSpin", QSpinBox),
    ("renderButton", QPushButton),
    ("cancelButton", QPushButton),
    ("progressBar", QProgressBar),
    ("statusLabel", QLabel),
    ("originalButton", QAbstractButton),
    ("processedButton", QAbstractButton),
    ("splitButton", QAbstractButton),
    ("saveFrameButton", QPushButton),
    ("playButton", QPushButton),
    ("frameSlider", QSlider),
    ("positionSlider", QSlider),
    ("lengthSpin", QDoubleSpinBox),
    ("previewButton", QPushButton),
    ("panelsButton", QAbstractButton),
    ("schematicButton", QAbstractButton),
]


@pytest.fixture(scope="session")
def app():
    return QApplication.instance() or QApplication([])


@pytest.fixture
def window(app, binary, ffmpeg, ffprobe, tmp_path):
    from vhsgui.app import MainWindow

    win = MainWindow(binary=binary, workdir=tmp_path / "work")
    win.resize(1280, 900)
    win.show()
    yield win
    win.close()
    app.processEvents()


def wait_for(app, predicate, timeout: float = 90.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        app.processEvents()
        if predicate():
            return
        time.sleep(0.02)
    pytest.fail("timed out waiting for the window")


def child(win, cls, name):
    found = win.findChild(cls, name)
    assert found is not None, f"no {cls.__name__} named {name}"
    return found


def to_schematic(app, window):
    child(window, QAbstractButton, "schematicButton").click()
    app.processEvents()
    assert window.schematic_mode
    return window.schematic


def mouse(view, kind, pos, button=LEFT, buttons=None):
    if buttons is None:
        buttons = NOBTN if kind == QEvent.Type.MouseButtonRelease else button
    p = QPointF(pos)
    QApplication.sendEvent(view, QMouseEvent(kind, p, QPointF(view.mapToGlobal(p.toPoint())), button, buttons, Qt.KeyboardModifier.NoModifier))


def click(view, pos):
    mouse(view, QEvent.Type.MouseButtonPress, pos)
    mouse(view, QEvent.Type.MouseButtonRelease, pos)


def double_click(view, pos):
    click(view, pos)
    mouse(view, QEvent.Type.MouseButtonDblClick, pos)
    mouse(view, QEvent.Type.MouseButtonRelease, pos)


def hover(view, pos):
    mouse(view, QEvent.Type.MouseMove, pos, NOBTN, NOBTN)


def at(view, cid, frac=0.5):
    r = view.control_rect(cid)
    return QPointF(r.left() + frac * r.width(), r.top() + 0.4 * r.height())


def on_grid(spec, fraction):
    n = round((spec.hi - spec.lo) / spec.step)
    return spec.lo + round(fraction * n) * spec.step


def shown_look(window):
    """The schematic's idea of the look, as the window's own look() would say it."""
    look = window.look()
    return {**look.condition, "preset": look.preset, "seed": look.seed, "generations": look.generations, "signal": look.signal, "audio": look.audio,
            "standard": look.standard, "camera_tube": look.camera_tube, "iris_stops": look.iris_stops}


def views(window):
    return [window.schematic, window.outside]


def view_of(window, cid):
    for v in views(window):
        if cid in v.control_ids():
            return v
    raise AssertionError(f"no page has a line for {cid}")


def show_page_of(app, window, cid):
    """Go to the page that holds `cid` and return its view."""
    v = view_of(window, cid)
    window.set_schematic_page("outside" if v is window.outside else "circuit")
    app.processEvents()
    return v


def assert_mirrors(window):
    want = shown_look(window)
    seen = set()
    for view in views(window):
        for cid in view.control_ids():
            seen.add(cid)
            got = view.value(cid)
            if isinstance(want[cid], float):
                assert got == pytest.approx(want[cid], abs=1e-6), cid
            else:
                assert got == want[cid], cid
    assert seen == set(want), "every setting of the window has a line on one of the pages"


# ---------------------------------------------------------------- the two views


def test_the_window_starts_with_the_panels(app, window):
    assert window.schematic_mode is False
    panels, schematic = child(window, QAbstractButton, "panelsButton"), child(window, QAbstractButton, "schematicButton")
    assert panels.isCheckable() and schematic.isCheckable() and panels.isChecked() and not schematic.isChecked()
    view = child(window, SchematicView, "schematicView")
    outside = child(window, SchematicView, "outsideView")
    assert window.schematic is view and window.outside is outside
    assert not view.isVisibleTo(window) and not view.is_ticking() and not outside.isVisibleTo(window) and not outside.is_ticking()
    assert child(window, QDoubleSpinBox, "field_head_wear").isVisibleTo(window)
    assert child(window, QDoubleSpinBox, "field_ground_loop_mv").isVisibleTo(window), "the interference settings have their panels too"
    assert not child(window, QAbstractButton, "circuitButton").isVisibleTo(window) and not child(window, QAbstractButton, "outsideButton").isVisibleTo(window)
    assert window.schematic_page == "circuit"
    assert child(window, QWidget, "previewView").isVisibleTo(window)


def test_the_buttons_switch_the_view_and_the_rest_stays_reachable(app, window):
    panels, schematic = child(window, QAbstractButton, "panelsButton"), child(window, QAbstractButton, "schematicButton")
    schematic.click()
    app.processEvents()
    assert window.schematic_mode and schematic.isChecked() and not panels.isChecked()
    assert window.schematic.isVisibleTo(window)
    for name, cls in HIDDEN_IN_SCHEMATIC:
        assert not child(window, cls, name).isVisibleTo(window), f"{name} should give way to the schematic"
    for name, cls in KEPT_IN_SCHEMATIC:
        assert child(window, cls, name).isVisibleTo(window), f"{name} must stay on screen"
    panels.click()
    app.processEvents()
    assert not window.schematic_mode and panels.isChecked() and not schematic.isChecked()
    assert not window.schematic.isVisibleTo(window)
    for name, cls in HIDDEN_IN_SCHEMATIC + KEPT_IN_SCHEMATIC:
        if name not in ("panelsButton", "schematicButton"):
            assert child(window, cls, name).isVisibleTo(window), f"{name} comes back"
    window.set_schematic_mode(True)
    assert window.schematic_mode and child(window, QAbstractButton, "schematicButton").isChecked()
    window.set_schematic_mode(False)
    assert not window.schematic_mode and child(window, QAbstractButton, "panelsButton").isChecked()


def test_the_schematic_has_room_at_a_normal_window_size(app, window):
    view = to_schematic(app, window)
    window.resize(1280, 900)
    app.processEvents()
    assert view.width() >= 1180 and view.height() >= 600, (view.width(), view.height())
    drawing = QRect(view.mapTo(window, QPoint(0, 0)), view.size())
    for name, cls in KEPT_IN_SCHEMATIC:
        w = child(window, cls, name)
        box = QRect(w.mapTo(window, QPoint(0, 0)), w.size())
        assert window.rect().contains(box), f"{name} is cut off by the window edge"
        assert not drawing.intersects(box), f"{name} overlaps the schematic"


def test_the_schematic_only_draws_while_it_is_shown_and_boots_once(app, window):
    view = window.schematic
    assert not view.is_ticking()
    window.set_schematic_mode(True)
    app.processEvents()
    assert view.is_ticking() and view.boot_progress() < 1.0, "the first visit plays the start-up sequence"
    view.skip_boot()
    window.set_schematic_mode(False)
    app.processEvents()
    assert not view.is_ticking()
    window.set_schematic_mode(True)
    app.processEvents()
    assert view.is_ticking() and view.boot_progress() == 1.0, "and only the first visit"


# ---------------------------------------------------------------- one look, two views


def test_the_schematic_starts_as_the_window_does(app, window):
    assert set(window.schematic.control_ids()) | set(window.outside.control_ids()) == {f.key for f in CONDITION_FIELDS} | {
        "preset", "seed", "generations", "signal", "audio", "standard", "camera_tube", "iris_stops"
    }
    assert not set(window.schematic.control_ids()) & set(window.outside.control_ids())
    combo = child(window, QComboBox, "presetCombo")
    assert window.schematic.choices("preset") == [combo.itemText(i) for i in range(combo.count())], "the presets are the ones the program has"
    assert_mirrors(window)


def test_the_editors_drive_the_schematic(app, window):
    for spec in CONDITION_FIELDS:
        if spec.kind == "float":
            child(window, QDoubleSpinBox, f"field_{spec.key}").setValue(on_grid(spec, 0.6))
        elif spec.kind == "bool":
            box = child(window, QCheckBox, f"field_{spec.key}")
            box.setChecked(not box.isChecked())
        else:
            combo = child(window, QComboBox, f"field_{spec.key}")
            combo.setCurrentIndex(combo.count() - 1)
    assert_mirrors(window)
    child(window, QSpinBox, "seedSpin").setValue(1234)
    child(window, QSpinBox, "generationsSpin").setValue(3)
    signal, audio = child(window, QComboBox, "signalCombo"), child(window, QComboBox, "audioCombo")
    signal.setCurrentIndex(signal.findData("comb1h"))
    audio.setCurrentIndex(audio.findData("copy"))
    assert_mirrors(window)
    assert window.schematic.value("seed") == 1234 and window.schematic.value("generations") == 3
    assert window.schematic.value("signal") == "comb1h" and window.schematic.value("audio") == "copy"


def test_a_preset_puts_all_of_its_values_on_the_schematic(app, window):
    preset = child(window, QComboBox, "presetCombo")
    for name in ("ruined", "pristine", "dry-caps"):
        preset.setCurrentIndex(preset.findText(name))
        assert window.schematic.value("preset") == name
        assert_mirrors(window)
        assert not any(v.is_edited(c) for v in views(window) for c in v.control_ids())


def test_the_schematic_drives_the_editors(app, window):
    view = to_schematic(app, window)
    window.resize(1280, 900)
    app.processEvents()
    spec = next(f for f in CONDITION_FIELDS if f.key == "head_wear")
    spin = child(window, QDoubleSpinBox, "field_head_wear")
    r = view.control_rect("head_wear")
    mouse(view, QEvent.Type.MouseButtonPress, QPointF(r.right() - 1, r.top() + 6))
    mouse(view, QEvent.Type.MouseButtonRelease, QPointF(r.right() - 1, r.top() + 6))
    assert spin.value() >= spec.hi - 2 * spec.step and view.value("head_wear") == pytest.approx(spin.value(), abs=1e-6)
    assert window.preview_stale is True
    click(view, at(view, "tape_speed"))
    assert child(window, QComboBox, "field_tape_speed").currentData() == "lp"
    smps = child(window, QCheckBox, "field_psu_smps")
    before = smps.isChecked()
    click(view, at(view, "psu_smps"))
    assert smps.isChecked() is (not before)
    click(view, at(view, "generations", 0.8))
    assert child(window, QSpinBox, "generationsSpin").value() == 2
    click(view, at(view, "signal"))
    assert child(window, QComboBox, "signalCombo").currentData() == "notch"
    click(view, at(view, "audio"))
    assert child(window, QComboBox, "audioCombo").currentData() == "copy"
    first = child(window, QComboBox, "presetCombo").currentText()
    click(view, at(view, "preset"))
    assert child(window, QComboBox, "presetCombo").currentText() != first
    assert_mirrors(window)


def test_edits_are_marked_against_the_preset_and_the_defaults(app, window):
    view = window.schematic
    assert not any(v.is_edited(c) for v in views(window) for c in v.control_ids())
    ground = child(window, QDoubleSpinBox, "field_ground_loop_mv")
    ground.setValue(ground.value() + 100.0)
    assert window.outside.is_edited("ground_loop_mv") and not window.outside.is_edited("mains_sag")
    child(window, QPushButton, "resetButton").click()
    assert not window.outside.is_edited("ground_loop_mv") and window.outside.value("ground_loop_mv") == pytest.approx(ground.value(), abs=1e-6)
    wear = child(window, QDoubleSpinBox, "field_head_wear")
    original = wear.value()
    wear.setValue(original + 0.2 if original <= 0.8 else original - 0.2)
    assert view.is_edited("head_wear") and not view.is_edited("head_clog")
    child(window, QPushButton, "resetButton").click()
    assert not view.is_edited("head_wear") and view.value("head_wear") == pytest.approx(original, abs=1e-6)
    seed, gens = child(window, QSpinBox, "seedSpin"), child(window, QSpinBox, "generationsSpin")
    start_seed = seed.value()
    seed.setValue(start_seed + 5)
    gens.setValue(2)
    signal = child(window, QComboBox, "signalCombo")
    signal.setCurrentIndex(signal.findData("comb2h"))
    assert all(view.is_edited(c) for c in ("seed", "generations", "signal")) and not view.is_edited("audio")
    seed.setValue(start_seed)
    gens.setValue(1)
    signal.setCurrentIndex(signal.findData("svideo"))
    assert not any(view.is_edited(c) for c in ("seed", "generations", "signal"))
    preset = child(window, QComboBox, "presetCombo")
    preset.setCurrentIndex(preset.findText("ruined"))
    assert not view.is_edited("head_wear")
    ruined = wear.value()
    wear.setValue(ruined - 0.1 if ruined >= 0.1 else ruined + 0.1)
    assert view.is_edited("head_wear"), "edits are measured against the preset that is chosen now"
    preset.setCurrentIndex(preset.findText("pristine"))
    assert not any(v.is_edited(c) for v in views(window) for c in v.control_ids())
    assert not view.is_edited("preset")


def test_a_double_click_puts_one_line_back(app, window):
    view = to_schematic(app, window)
    window.resize(1280, 900)
    app.processEvents()
    wear, clog = child(window, QDoubleSpinBox, "field_head_wear"), child(window, QDoubleSpinBox, "field_head_clog")
    want_wear, want_clog = wear.value(), clog.value()
    wear.setValue(0.9 if want_wear < 0.5 else 0.1)
    clog.setValue(0.9 if want_clog < 0.5 else 0.1)
    double_click(view, at(view, "head_wear", 0.5))
    assert wear.value() == pytest.approx(want_wear, abs=1e-6)
    assert clog.value() != pytest.approx(want_clog, abs=1e-6), "only the line that was clicked"
    gens, seed = child(window, QSpinBox, "generationsSpin"), child(window, QSpinBox, "seedSpin")
    start_seed = seed.value()
    gens.setValue(4)
    seed.setValue(start_seed + 7)
    double_click(view, at(view, "generations", 0.8))
    assert gens.value() == 1 and seed.value() == start_seed + 7
    double_click(view, at(view, "seed", 0.8))
    assert seed.value() == start_seed
    signal = child(window, QComboBox, "signalCombo")
    double_click(view, at(view, "signal"))
    assert signal.currentData() == "svideo"
    double_click(view, at(view, "preset"))  # a preset has nothing to go back to: it must simply not break
    assert_mirrors(window)


def test_looks_that_are_loaded_show_on_the_schematic(app, window, tmp_path):
    view = to_schematic(app, window)
    wear = child(window, QDoubleSpinBox, "field_head_wear")
    wear.setValue(0.8)
    child(window, QSpinBox, "generationsSpin").setValue(3)
    path = tmp_path / "mine.toml"
    window.save_look(str(path))
    wear.setValue(0.1)
    child(window, QSpinBox, "generationsSpin").setValue(1)
    assert view.value("head_wear") == pytest.approx(0.1)
    window.load_look(str(path))
    assert view.value("head_wear") == pytest.approx(0.8) and view.value("generations") == 3
    assert_mirrors(window)


def test_every_line_has_a_help_text_from_the_window(app, window):
    view = to_schematic(app, window)
    window.resize(1280, 900)
    app.processEvents()
    default = view.hint_text()
    for spec in CONDITION_FIELDS:
        v = show_page_of(app, window, spec.key)
        hover(v, v.control_rect(spec.key).center())
        assert v.hint_text() == spec.help, spec.key
    for cid in ("preset", "seed", "generations", "signal", "audio", "standard", "camera_tube", "iris_stops"):
        hover(view, view.control_rect(cid).center())
        assert view.hint_text() not in ("", default), cid


def test_the_two_pages_switch_and_each_starts_up_once(app, window):
    circuit_btn, outside_btn = child(window, QAbstractButton, "circuitButton"), child(window, QAbstractButton, "outsideButton")
    assert circuit_btn.isCheckable() and outside_btn.isCheckable() and circuit_btn.isChecked() and not outside_btn.isChecked()
    window.set_schematic_mode(True)
    app.processEvents()
    assert circuit_btn.isVisibleTo(window) and outside_btn.isVisibleTo(window)
    assert window.schematic.isVisibleTo(window) and not window.outside.isVisibleTo(window) and window.schematic_page == "circuit"
    assert window.schematic.is_ticking() and not window.outside.is_ticking()
    window.schematic.skip_boot()
    outside_btn.click()
    app.processEvents()
    assert window.schematic_page == "outside" and outside_btn.isChecked() and not circuit_btn.isChecked()
    assert window.outside.isVisibleTo(window) and not window.schematic.isVisibleTo(window)
    assert window.outside.is_ticking() and not window.schematic.is_ticking()
    assert window.outside.boot_progress() < 1.0, "the second page plays its own start-up the first time"
    window.outside.skip_boot()
    circuit_btn.click()
    outside_btn.click()
    app.processEvents()
    assert window.outside.boot_progress() == 1.0 and window.schematic.boot_progress() == 1.0
    window.set_schematic_page("circuit")
    assert window.schematic_page == "circuit" and circuit_btn.isChecked()
    window.set_schematic_mode(False)
    assert not circuit_btn.isVisibleTo(window) and not outside_btn.isVisibleTo(window)
    assert not window.schematic.is_ticking() and not window.outside.is_ticking()


def test_the_outside_page_changes_the_same_editors(app, window):
    window.set_schematic_mode(True)
    window.set_schematic_page("outside")
    window.resize(1280, 900)
    app.processEvents()
    view = window.outside
    spec = next(f for f in CONDITION_FIELDS if f.key == "ground_loop_mv")
    spin = child(window, QDoubleSpinBox, "field_ground_loop_mv")
    r = view.control_rect("ground_loop_mv")
    mouse(view, QEvent.Type.MouseButtonPress, QPointF(r.right() - 1, r.top() + 6))
    mouse(view, QEvent.Type.MouseButtonRelease, QPointF(r.right() - 1, r.top() + 6))
    assert spin.value() >= spec.hi - 2 * spec.step and view.value("ground_loop_mv") == pytest.approx(spin.value(), abs=1e-6)
    assert window.preview_stale is True
    click(view, at(view, "phone_mode"))
    assert child(window, QComboBox, "field_phone_mode").currentData() == "gsm"
    click(view, at(view, "crt_magnet"))
    assert child(window, QComboBox, "field_crt_magnet").currentData() == "fridge"
    double_click(view, at(view, "ground_loop_mv", 0.5))
    assert spin.value() == pytest.approx(0.0, abs=1e-6), "double click returns to the preset's value"
    assert window.look().condition["phone_mode"] == "gsm" and window.look().condition["crt_magnet"] == "fridge"
    assert_mirrors(window)


def test_an_interference_preset_shows_on_the_outside_page_and_is_not_an_edit(app, window):
    preset = child(window, QComboBox, "presetCombo")
    preset.setCurrentIndex(preset.findText("ground-loop"))
    assert window.outside.value("ground_loop_mv") > 100.0 and abs(window.outside.value("mains_hz") - 60.0) > 0.1
    assert not any(window.outside.is_edited(c) for c in window.outside.control_ids())
    preset.setCurrentIndex(preset.findText("magnet-head"))
    assert window.outside.value("head_magnet") == "neodymium" and window.outside.value("ground_loop_mv") == 0.0
    assert_mirrors(window)


# ---------------------------------------------------------------- the TV and the work flow


def test_the_tv_shows_what_the_preview_shows(app, window, clip):
    window.open_input(str(clip))
    view = to_schematic(app, window)
    preview = child(window, QWidget, "previewView")
    wait_for(app, lambda: preview.image() is not None)
    assert view.screen_image() is not None and view.screen_image() == preview.image()
    assert window.outside.screen_image() == preview.image(), "the other page has a TV too"
    first = view.screen_image().copy()
    child(window, QDoubleSpinBox, "lengthSpin").setValue(0.0)
    child(window, QPushButton, "previewButton").click()
    wait_for(app, lambda: not window.busy and window.processed_frames is not None)
    child(window, QAbstractButton, "processedButton").click()
    assert view.screen_image() == preview.image() and view.screen_image() != first
    child(window, QAbstractButton, "splitButton").click()
    assert view.screen_image() == preview.image() and window.outside.screen_image() == preview.image()
    child(window, QAbstractButton, "originalButton").click()
    assert view.screen_image() == preview.image()


def test_a_render_can_be_started_and_cancelled_from_the_schematic(app, window, clip, tmp_path):
    window.open_input(str(clip))
    to_schematic(app, window)
    window.set_output(str(tmp_path / "out.mp4"))
    child(window, QPushButton, "renderButton").click()
    assert window.busy
    wait_for(app, lambda: child(window, QProgressBar, "progressBar").value() > 0, timeout=60)
    child(window, QPushButton, "cancelButton").click()
    wait_for(app, lambda: not window.busy)
    assert "cancel" in child(window, QLabel, "statusLabel").text().lower()
    assert window.schematic.isVisibleTo(window)


# ---------------------------------------------------------------- the module entry point


def _run_module(*args: str, timeout: float = 120):
    env = dict(os.environ, QT_QPA_PLATFORM="offscreen", PYTHONPATH=str(PYTHON_DIR))
    return subprocess.run([sys.executable, "-m", "vhsgui", *args], env=env, capture_output=True, text=True, timeout=timeout)


def test_the_module_can_start_in_schematic_mode_and_save_a_picture_of_it(binary, tmp_path):
    png = tmp_path / "schematic.png"
    result = _run_module("--schematic", "--screenshot", str(png))
    assert result.returncode == 0, result.stderr
    image = QImage(str(png))
    assert not image.isNull() and image.width() >= 1000 and image.height() >= 700
    a = image.convertToFormat(QImage.Format.Format_RGB32)
    raw = np.frombuffer(a.constBits(), np.uint8).reshape(a.height(), a.bytesPerLine() // 4, 4)[:, : a.width()].astype(float)
    r, g, b = raw[..., 2], raw[..., 1], raw[..., 0]
    phosphor = (g > 120) & (g > 1.6 * r) & (g > 1.3 * b)
    assert phosphor.mean() > 0.01, "the schematic is on the picture and fully drawn"  # measured on the whole window: 3.3 % with the finished drawing, 1.5 % with the placeholder, about 0 % mid-boot
    default = tmp_path / "default.png"
    assert _run_module("--screenshot", str(default)).returncode == 0
    d = QImage(str(default)).convertToFormat(QImage.Format.Format_RGB32)
    draw = np.frombuffer(d.constBits(), np.uint8).reshape(d.height(), d.bytesPerLine() // 4, 4)[:, : d.width()].astype(float)
    assert ((draw[..., 1] > 120) & (draw[..., 1] > 1.6 * draw[..., 2]) & (draw[..., 1] > 1.3 * draw[..., 0])).mean() > 0.01, "the default is the schematic"
    plain = tmp_path / "panels.png"
    assert _run_module("--panels", "--screenshot", str(plain)).returncode == 0
    p = QImage(str(plain)).convertToFormat(QImage.Format.Format_RGB32)
    praw = np.frombuffer(p.constBits(), np.uint8).reshape(p.height(), p.bytesPerLine() // 4, 4)[:, : p.width()].astype(float)
    assert ((praw[..., 1] > 120) & (praw[..., 1] > 1.6 * praw[..., 2]) & (praw[..., 1] > 1.3 * praw[..., 0])).mean() < phosphor.mean() / 3, "--panels starts with the panels"


def test_the_module_can_start_on_the_outside_page(binary, tmp_path):
    png = tmp_path / "outside.png"
    result = _run_module("--schematic", "--page", "outside", "--screenshot", str(png))
    assert result.returncode == 0, result.stderr
    image = QImage(str(png)).convertToFormat(QImage.Format.Format_RGB32)
    assert image.width() >= 1000 and image.height() >= 700
    raw = np.frombuffer(image.constBits(), np.uint8).reshape(image.height(), image.bytesPerLine() // 4, 4)[:, : image.width()].astype(float)
    r, g, b = raw[..., 2], raw[..., 1], raw[..., 0]
    assert ((g > 120) & (g > 1.6 * r) & (g > 1.3 * b)).mean() > 0.01
    assert _run_module("--page", "outside", "--screenshot", str(tmp_path / "y.png")).returncode == 0, "the schematic is the default, so --page alone is enough"
    assert _run_module("--panels", "--page", "outside", "--screenshot", str(tmp_path / "x.png")).returncode != 0, "a page with --panels makes no sense"


def test_the_standard_and_the_camera_drive_and_follow_the_editors(app, window):
    view = to_schematic(app, window)
    window.resize(1280, 900)
    app.processEvents()
    std = child(window, QComboBox, "field_standard")
    tube = child(window, QComboBox, "field_camera_tube")
    click(view, at(view, "standard"))
    assert std.currentData() == "pal" and window.look().standard == "pal"
    click(view, at(view, "camera_tube"))
    assert tube.currentData() == "plumbicon"
    std.setCurrentIndex(std.findData("pal60"))
    tube.setCurrentIndex(tube.findData("plumbicon-act"))
    app.processEvents()
    assert view.value("standard") == "pal60" and view.value("camera_tube") == "plumbicon-act"
    assert_mirrors(window)

