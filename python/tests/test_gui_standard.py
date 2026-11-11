# CONTRACT
"""The television standard and the camera tube in the GUI: command lines, frame timing, look files, controls."""
from __future__ import annotations

import pytest

from vhsgui import backend
from vhsgui.model import CAMERA_TUBES, STANDARDS, Look, MediaInfo, RunRequest, fps_of, frame_time


def _req(**look) -> RunRequest:
    return RunRequest(input="in.mp4", output="out.mp4", look=Look(**look))


def test_frame_timing_follows_the_standard():
    assert fps_of("pal") == 25.0
    assert fps_of("pal60") == fps_of("ntsc") == 30000 / 1001
    assert frame_time(25, "pal") == pytest.approx(1.0)
    assert frame_time(30, "ntsc") == pytest.approx(1.001)
    info = MediaInfo("x.mp4", 1920, 1080, 30.0, 4.0, False)
    assert info.frames_at("pal") == 100 and info.frames_at("ntsc") == info.frames == 120


def test_run_args_carry_the_standard_only_when_it_is_not_ntsc():
    plain = backend.run_args("vhsphys", _req(), None)
    assert "--standard" not in plain
    for std in ("pal", "pal60"):
        args = backend.run_args("vhsphys", _req(standard=std), None)
        i = args.index("--standard")
        assert args[i + 1] == std


def test_run_args_carry_the_camera_only_with_a_tube():
    assert "--camera-tube" not in backend.run_args("vhsphys", _req(iris_stops=2.0), None)
    args = backend.run_args("vhsphys", _req(camera_tube="plumbicon-act", iris_stops=1.5), None)
    assert args[args.index("--camera-tube") + 1] == "plumbicon-act"
    assert float(args[args.index("--iris-stops") + 1]) == 1.5


def test_a_pal_start_snaps_to_pal_frames():
    frame, seconds = backend.snap_start(1.01, "pal")
    assert frame == 25 and seconds == pytest.approx(1.0)
    req = RunRequest(input="in.mp4", output="out.mp4", look=Look(standard="pal"), start=2.0)
    args = backend.run_args("vhsphys", req, None)
    assert float(args[args.index("--start") + 1]) == pytest.approx(2.0, abs=1e-6)


def test_pal_previews_and_decoding_run_at_25():
    info = MediaInfo("x.mp4", 1280, 720, 30.0, 10.0, False)
    plan = backend.preview_plan(info, 4.0, 2.0, standard="pal")
    assert plan.start_frame == 100 and plan.frames == 50
    assert plan.duration == pytest.approx((plan.lead_frames + plan.frames) / 25)
    args = backend.decode_args("ffmpeg", "x.mp4", (320, 240), standard="pal")
    vf = args[args.index("-vf") + 1]
    assert vf.startswith("fps=25/1,")
    assert backend.decode_args("ffmpeg", "x.mp4", (320, 240))[args.index("-vf") + 1].startswith("fps=30000/1001,")


def test_look_files_keep_the_standard_and_the_camera():
    look = Look(standard="pal", camera_tube="plumbicon", iris_stops=2.5)
    back = backend.load_look(backend.save_look(look))
    assert (back.standard, back.camera_tube, back.iris_stops) == ("pal", "plumbicon", 2.5)
    old = backend.load_look(backend.save_look(Look()))
    assert (old.standard, old.camera_tube, old.iris_stops) == ("ntsc", "off", 0.0)


def test_look_files_refuse_unknown_values():
    with pytest.raises(Exception):
        backend.load_look('[look]\nstandard = "secam"\n')
    with pytest.raises(Exception):
        backend.load_look('[look]\ncamera_tube = "vidicon"\n')


@pytest.fixture
def window(binary, ffmpeg, ffprobe, tmp_path):
    from PySide6.QtWidgets import QApplication

    from vhsgui.app import MainWindow

    app = QApplication.instance() or QApplication([])
    win = MainWindow(binary=binary, workdir=tmp_path / "work")
    win.show()
    yield win
    win.close()
    app.processEvents()


def test_the_window_has_the_controls(window):
    from PySide6.QtWidgets import QComboBox, QDoubleSpinBox

    std = window.findChild(QComboBox, "field_standard")
    tube = window.findChild(QComboBox, "field_camera_tube")
    iris = window.findChild(QDoubleSpinBox, "field_iris_stops")
    assert std is not None and tube is not None and iris is not None
    # the item data are the values the CLI takes; the texts are for people
    assert [std.itemData(i) for i in range(std.count())] == list(STANDARDS)
    assert [tube.itemData(i) for i in range(tube.count())] == list(CAMERA_TUBES)
    std.setCurrentIndex(STANDARDS.index("pal"))
    tube.setCurrentIndex(CAMERA_TUBES.index("plumbicon"))
    iris.setValue(2.0)
    look = window.look()
    assert (look.standard, look.camera_tube, look.iris_stops) == ("pal", "plumbicon", 2.0)
