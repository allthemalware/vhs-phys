# CONTRACT
"""Acceptance tests for the window (Qt, offscreen).

They fix the public surface of `vhsgui.app.MainWindow`: the object names of the
widgets, the `look()` / `open_input()` / `set_output()` methods, the `busy`,
`view_mode`, `preview_stale`, `original_frames` and `processed_frames` attributes and the
`confirm_overwrite(path)` hook. Everything else about the layout is free.
"""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import struct
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import pytest

pytest.importorskip("PySide6")
from PySide6.QtWidgets import (  # noqa: E402
    QAbstractButton,
    QApplication,
    QCheckBox,
    QComboBox,
    QDoubleSpinBox,
    QLabel,
    QLineEdit,
    QPlainTextEdit,
    QProgressBar,
    QPushButton,
    QSlider,
    QSpinBox,
    QWidget,
)

from vhsgui import backend  # noqa: E402
from vhsgui.model import CONDITION_FIELDS  # noqa: E402

PYTHON_DIR = Path(__file__).resolve().parents[1]


@pytest.fixture(scope="session")
def app():
    return QApplication.instance() or QApplication([])


@pytest.fixture
def window(app, binary, ffmpeg, ffprobe, tmp_path):
    from vhsgui.app import MainWindow

    win = MainWindow(binary=binary, workdir=tmp_path / "work")
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


# ---------------------------------------------------------------- settings


def test_the_fields_follow_the_preset(app, window):
    combo = child(window, QComboBox, "presetCombo")
    names = [combo.itemText(i) for i in range(combo.count())]
    assert len(names) >= 10 and {"worn", "ruined", "pristine"} <= set(names)
    for spec in CONDITION_FIELDS:
        assert window.findChild(QWidget, f"field_{spec.key}") is not None, spec.key

    wear = child(window, QDoubleSpinBox, "field_head_wear")
    smps = child(window, QCheckBox, "field_psu_smps")
    combo.setCurrentText("ruined")
    assert wear.value() == pytest.approx(0.9)
    combo.setCurrentText("worn")
    assert wear.value() == pytest.approx(0.4) and not smps.isChecked()

    wear.setValue(0.75)
    smps.setChecked(True)
    look = window.look()
    assert look.preset == "worn"
    assert look.condition["head_wear"] == pytest.approx(0.75) and look.condition["psu_smps"] is True
    assert set(look.condition) == {f.key for f in CONDITION_FIELDS}

    speed = child(window, QComboBox, "field_tape_speed")
    assert speed.currentData() == "sp" and window.look().condition["tape_speed"] == "sp"
    speed.setCurrentIndex(speed.findData("ep"))
    assert window.look().condition["tape_speed"] == "ep"

    child(window, QAbstractButton, "resetButton").click()
    assert wear.value() == pytest.approx(0.4) and not smps.isChecked()
    assert speed.currentData() == "sp"


def test_the_seed_is_part_of_the_look(app, window):
    seed = child(window, QSpinBox, "seedSpin")
    seed.setValue(1234)
    assert window.look().seed == 1234
    child(window, QAbstractButton, "randomSeedButton").click()
    assert 0 <= window.look().seed <= seed.maximum()


def test_invalid_advanced_settings_block_running(app, window, clip):
    window.open_input(str(clip))
    render, preview = child(window, QPushButton, "renderButton"), child(window, QPushButton, "previewButton")
    assert render.isEnabled() and preview.isEnabled()
    edit = child(window, QPlainTextEdit, "advancedEdit")
    edit.setPlainText("[tape\n")
    assert child(window, QLabel, "advancedStatus").text().strip()
    assert not render.isEnabled() and not preview.isEnabled()
    edit.setPlainText("[tape]\ncreases_per_m = 0.5\n")
    assert render.isEnabled() and preview.isEnabled()
    assert window.look().advanced.strip().startswith("[tape]")


# ---------------------------------------------------------------- input and preview


def test_nothing_can_run_without_an_input(app, window):
    assert not child(window, QPushButton, "renderButton").isEnabled()
    assert not child(window, QPushButton, "previewButton").isEnabled()
    assert window.busy is False


def test_opening_a_file_shows_its_details_and_first_frame(app, window, clip):
    window.open_input(str(clip))
    info = child(window, QLabel, "infoLabel").text()
    assert "320" in info and "240" in info
    out = child(window, QLineEdit, "outputEdit").text()
    assert out.endswith("_vhs.mp4") and Path(out).parent == clip.parent
    view = child(window, QWidget, "previewView")
    wait_for(app, lambda: view.image() is not None)
    assert (view.image().width(), view.image().height()) == (320, 240)


def test_moving_the_position_shows_another_source_frame(app, window, clip):
    window.open_input(str(clip))
    view = child(window, QWidget, "previewView")
    wait_for(app, lambda: view.image() is not None)
    first = view.image().copy()
    child(window, QSlider, "positionSlider").setValue(20)  # tenths of a second
    wait_for(app, lambda: view.image() is not None and view.image() != first)


def test_a_preview_renders_the_processed_frames(app, window, clip):
    window.open_input(str(clip))
    child(window, QSlider, "positionSlider").setValue(10)
    child(window, QDoubleSpinBox, "lengthSpin").setValue(1.0)
    child(window, QPushButton, "previewButton").click()
    assert window.busy
    wait_for(app, lambda: not window.busy and window.processed_frames is not None)

    original, processed = window.original_frames, window.processed_frames
    assert original.count == processed.count == 30
    assert (original.width, original.height) == (processed.width, processed.height) == (320, 240)
    a = np.frombuffer(original.frame(5), np.uint8).astype(float)
    b = np.frombuffer(processed.frame(5), np.uint8).astype(float)
    assert np.abs(a - b).mean() > 1.0  # the effect really changes the picture
    assert window.view_mode == "processed" and window.preview_stale is False

    child(window, QAbstractButton, "originalButton").click()
    assert window.view_mode == "original"
    child(window, QAbstractButton, "processedButton").click()
    assert window.view_mode == "processed"

    wear = child(window, QDoubleSpinBox, "field_head_wear")
    wear.setValue(min(wear.maximum(), wear.value() + 0.1))
    assert window.preview_stale is True


# ---------------------------------------------------------------- rendering to a file


def test_render_writes_the_chosen_file(app, window, clip, tmp_path):
    window.open_input(str(clip))
    out = tmp_path / "result.mp4"
    window.set_output(str(out))
    child(window, QDoubleSpinBox, "startSpin").setValue(1.0)
    child(window, QDoubleSpinBox, "durationSpin").setValue(1.0)
    child(window, QPushButton, "renderButton").click()
    assert window.busy
    wait_for(app, lambda: not window.busy)

    assert out.is_file() and out.stat().st_size > 1000
    assert not list(tmp_path.glob(".*partial*"))
    info = backend.probe(out)
    assert (info.width, info.height) == (320, 240) and 0.9 <= info.duration <= 1.2
    bar = child(window, QProgressBar, "progressBar")
    assert bar.value() == bar.maximum()
    assert "done" in child(window, QLabel, "statusLabel").text().lower()


def test_cancel_stops_the_render_and_leaves_an_existing_file_alone(app, window, clip, tmp_path):
    out = tmp_path / "keep.mp4"
    out.write_bytes(b"previous contents")
    window.confirm_overwrite = lambda path: True
    window.open_input(str(clip))
    window.set_output(str(out))
    child(window, QPushButton, "renderButton").click()
    bar = child(window, QProgressBar, "progressBar")
    wait_for(app, lambda: bar.value() > 0, timeout=60)
    child(window, QPushButton, "cancelButton").click()
    wait_for(app, lambda: not window.busy)

    assert out.read_bytes() == b"previous contents"
    assert not list(tmp_path.glob(".*partial*"))
    assert "cancel" in child(window, QLabel, "statusLabel").text().lower()


# ---------------------------------------------------------------- the module entry point


def _run_module(*args: str, timeout: float = 120):
    env = dict(os.environ, QT_QPA_PLATFORM="offscreen", PYTHONPATH=str(PYTHON_DIR))
    return subprocess.run([sys.executable, "-m", "vhsgui", *args], env=env, capture_output=True, text=True, timeout=timeout)


def test_module_help_does_not_open_a_window():
    result = _run_module("--help")
    assert result.returncode == 0 and "usage" in result.stdout.lower()


def test_module_can_save_a_screenshot_of_the_window(binary, clip, tmp_path):
    png = tmp_path / "window.png"
    result = _run_module("--screenshot", str(png), str(clip))
    assert result.returncode == 0, result.stderr
    data = png.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    width, height = struct.unpack(">II", data[16:24])
    assert width >= 900 and height >= 600
