# CONTRACT
"""Acceptance tests for vhsgui.backend (no Qt involved).

They define the backend API: `find_binary`, `list_presets`, `preset_condition`, `probe`,
`snap_start`, `preview_plan`, `merge_config`, `validate_advanced`, `run_args`,
`parse_progress`, `partial_path`, `decode_args` and the `Job` runner.
"""
from __future__ import annotations

import os
import subprocess
import sys
import textwrap
import time
import tomllib
from pathlib import Path

import numpy as np
import pytest

from vhsgui import backend
from vhsgui.model import (
    CONDITION_FIELDS,
    FPS,
    LEAD_FRAMES,
    Frames,
    GuiError,
    Look,
    MediaInfo,
    Progress,
    RunRequest,
    frame_time,
)

INFO = MediaInfo("x.mp4", 1920, 1080, FPS, 10.0, True)


# ---------------------------------------------------------------- locating the tools


def _fake_exe(path: Path) -> Path:
    path.write_text("#!/bin/sh\nexit 0\n")
    path.chmod(0o755)
    return path


def test_find_binary_prefers_the_argument_then_the_environment(tmp_path, monkeypatch):
    a, b = _fake_exe(tmp_path / "a"), _fake_exe(tmp_path / "b")
    monkeypatch.setenv("VHSPHYS_BIN", str(b))
    assert backend.find_binary(str(a)) == a
    assert backend.find_binary() == b


def test_find_binary_reports_a_bad_explicit_path(tmp_path):
    with pytest.raises(GuiError, match="not found"):
        backend.find_binary(str(tmp_path / "nope"))


def test_find_binary_finds_the_build(binary):
    assert binary.is_file() and binary.name.startswith("vhsphys")


# ---------------------------------------------------------------- presets and the condition


def test_presets_are_listed_in_order_with_a_summary(binary):
    presets = backend.list_presets(binary)
    names = [p.name for p in presets]
    assert names[:5] == ["pristine", "good", "worn", "bad", "ruined"]
    assert {"dry-caps", "dirty-heads", "mistracked", "stretched", "humid-attic"} <= set(names)
    assert all(p.summary.strip() for p in presets)


def test_preset_condition_has_every_field_the_window_offers(binary):
    cond = backend.preset_condition(binary, "worn")
    assert set(cond) == {f.key for f in CONDITION_FIELDS}
    for spec in CONDITION_FIELDS:
        assert isinstance(cond[spec.key], bool) == (spec.kind == "bool"), spec.key
        if spec.kind == "choice":
            assert cond[spec.key] in spec.choices, spec.key
    assert cond["head_wear"] == pytest.approx(0.4)
    assert cond["psu_smps"] is False


def test_every_preset_fits_the_ranges_of_the_controls(binary):
    for preset in backend.list_presets(binary):
        cond = backend.preset_condition(binary, preset.name)
        for spec in CONDITION_FIELDS:
            if spec.kind == "float":
                assert spec.lo <= cond[spec.key] <= spec.hi, f"{preset.name}: {spec.key}={cond[spec.key]}"


def test_unknown_preset_is_a_gui_error(binary):
    with pytest.raises(GuiError, match="unknown preset"):
        backend.preset_condition(binary, "no-such-preset")


# ---------------------------------------------------------------- probing media


def test_probe_reads_size_rate_and_duration(clip):
    info = backend.probe(clip)
    assert (info.width, info.height) == (320, 240)
    assert info.fps == pytest.approx(FPS, abs=0.05)
    assert info.duration == pytest.approx(4.0, abs=0.1)
    assert info.has_audio is False
    assert abs(info.frames - 120) <= 3


def test_probe_detects_audio(clip_audio):
    assert backend.probe(clip_audio).has_audio is True


def test_probe_applies_rotation_metadata(clip, ffmpeg, tmp_path):
    rotated = tmp_path / "rotated.mp4"
    made = subprocess.run([ffmpeg, "-v", "error", "-y", "-display_rotation", "90", "-i", str(clip), "-c", "copy", str(rotated)], capture_output=True)
    if made.returncode != 0:
        pytest.skip("this ffmpeg cannot set a display rotation")
    info = backend.probe(rotated)
    assert (info.width, info.height) == (240, 320)


def test_probe_errors_are_gui_errors(tmp_path):
    with pytest.raises(GuiError):
        backend.probe(tmp_path / "missing.mp4")
    text = tmp_path / "notes.txt"
    text.write_text("not a video")
    with pytest.raises(GuiError):
        backend.probe(text)


# ---------------------------------------------------------------- frame alignment


def test_snap_start_lands_on_a_frame():
    assert backend.snap_start(0.0) == (0, 0.0)
    assert backend.snap_start(-3.0) == (0, 0.0)
    frame, seconds = backend.snap_start(5.0)
    assert frame == 150 and seconds == pytest.approx(frame_time(150))


def test_preview_plan_in_the_middle_of_a_file():
    p = backend.preview_plan(INFO, at=5.0, length=2.0)
    assert (p.start_frame, p.lead_frames, p.frames) == (150, LEAD_FRAMES, 60)
    assert p.start == pytest.approx(frame_time(150 - LEAD_FRAMES))
    assert p.duration == pytest.approx(frame_time(LEAD_FRAMES + 60))
    assert p.size == (640, 360)


def test_preview_plan_near_the_start_has_a_shorter_lead_in():
    p = backend.preview_plan(INFO, at=0.0, length=1.0)
    assert (p.start_frame, p.lead_frames, p.start) == (0, 0, 0.0)
    p = backend.preview_plan(INFO, at=0.07, length=1.0)  # frame 2
    assert (p.start_frame, p.lead_frames) == (2, 2)


def test_preview_plan_is_clamped_to_the_media():
    short = MediaInfo("x.mp4", 320, 240, FPS, 3.0, False)  # 90 frames
    p = backend.preview_plan(short, at=2.5, length=2.0)
    assert (p.start_frame, p.frames) == (75, 15)
    p = backend.preview_plan(short, at=99.0, length=2.0)
    assert (p.start_frame, p.frames) == (89, 1)
    assert backend.preview_plan(short, at=1.0, length=0).frames == 1


@pytest.mark.parametrize("w,h", [(1920, 1080), (1080, 1920), (1000, 999), (3840, 2160), (640, 400), (320, 240)])
def test_preview_size_is_even_fits_and_keeps_the_aspect(w, h):
    info = MediaInfo("x.mp4", w, h, FPS, 5.0, False)
    pw, ph = backend.preview_plan(info, at=1.0, length=1.0).size
    assert pw % 2 == 0 and ph % 2 == 0
    assert pw <= 640 and ph <= 400 and pw <= w and ph <= h
    assert abs(pw / ph - w / h) / (w / h) < 0.02


def test_preview_size_honours_a_custom_maximum():
    info = MediaInfo("x.mp4", 320, 240, FPS, 5.0, False)
    assert backend.preview_plan(info, 1.0, 1.0, max_size=(160, 160)).size == (160, 120)


# ---------------------------------------------------------------- configuration text


def test_merge_config_writes_the_condition_and_the_advanced_tables():
    look = Look(
        condition={"head_wear": 0.7, "psu_smps": True, "tape_age_years": 12.0},
        advanced="[tape]\ncreases_per_m = 0.5\n[head1]\nextra_spacing_um = 0.9\n",
    )
    doc = tomllib.loads(backend.merge_config(look))
    assert doc["condition"] == {"head_wear": 0.7, "psu_smps": True, "tape_age_years": 12.0}
    assert doc["tape"] == {"creases_per_m": 0.5}
    assert doc["head1"] == {"extra_spacing_um": 0.9}


def test_advanced_text_wins_over_the_controls():
    look = Look(condition={"head_wear": 0.7}, advanced="[condition]\nhead_wear = 0.9\nhead_clog = 0.2\n")
    assert tomllib.loads(backend.merge_config(look))["condition"] == {"head_wear": 0.9, "head_clog": 0.2}


def test_merge_config_round_trips_arrays_strings_and_booleans():
    advanced = '[deck]\neq_peaks = [[5.4e6, 0.9, 6.0], [7.2e6, 1.2, 3.0]]\nagc_enabled = false\n[extra]\nname = "a \\"b\\""\nn = 3\n'
    doc = tomllib.loads(backend.merge_config(Look(advanced=advanced)))
    assert doc["deck"]["eq_peaks"] == [[5.4e6, 0.9, 6.0], [7.2e6, 1.2, 3.0]]
    assert doc["deck"]["agc_enabled"] is False
    assert doc["extra"] == {"name": 'a "b"', "n": 3}


def test_merge_config_of_an_empty_look_is_an_empty_document():
    assert tomllib.loads(backend.merge_config(Look())) == {}


def test_merge_config_rejects_bad_toml_with_the_line_number():
    with pytest.raises(GuiError, match="line 1"):
        backend.merge_config(Look(advanced="[tape\n"))


def test_validate_advanced():
    assert backend.validate_advanced("") is None
    assert backend.validate_advanced("  \n") is None
    assert backend.validate_advanced("[tape]\ncreases_per_m = 1\n") is None
    message = backend.validate_advanced("[tape\n")
    assert isinstance(message, str) and message.strip()


# ---------------------------------------------------------------- command lines


def _options(args: list[str]) -> dict[str, str]:
    """Every flag of the run command takes one value."""
    return dict(zip(args[0::2], args[1::2]))


def test_run_args_with_the_minimum():
    req = RunRequest(input="in.mp4", output="out.mp4", look=Look(preset="good", seed=5))
    args = backend.run_args("/x/vhsphys", req, None)
    assert args[:4] == ["/x/vhsphys", "run", "in.mp4", "out.mp4"]
    assert _options(args[4:]) == {"--preset": "good", "--seed": "5", "--crf": "18"}


def test_run_args_with_every_option():
    req = RunRequest("in.mp4", "out.mp4", Look(preset="bad", seed=9), start=5.0, duration=1.5, size=(640, 360), crf=20, threads=2)
    args = backend.run_args(Path("/x/vhsphys"), req, Path("/tmp/look.toml"))
    assert all(isinstance(a, str) for a in args)
    opts = _options(args[4:])
    assert {k: opts[k] for k in ("--preset", "--seed", "--config", "-t", "--size", "--crf", "--threads")} == {
        "--preset": "bad", "--seed": "9", "--config": "/tmp/look.toml", "-t": "1.5", "--size": "640x360", "--crf": "20", "--threads": "2",
    }
    # the start is a frame boundary, cut to whole microseconds so a seek never skips that frame
    snapped = frame_time(150)
    assert snapped - 2e-6 < float(opts["--start"]) <= snapped + 1e-12


def test_parse_progress_reads_the_cli_output():
    assert backend.parse_progress("\r  15 frames  12.34 fps   ") == Progress(15, 12.34)
    text = "preset 'worn' seed 1  1920x1080 -> 1920x1080 (out.mp4)\n\r  30 frames  11.00 fps   \r  45 frames  11.50 fps   "
    assert backend.parse_progress(text) == Progress(45, 11.5)
    assert backend.parse_progress("\r  899 frames in 79.1s (11.37 fps)          ") == Progress(899, 11.37)
    assert backend.parse_progress("Error: ffmpeg encoder failed") is None
    assert backend.parse_progress("") is None


def test_partial_path_keeps_the_extension_and_hides_the_file():
    assert backend.partial_path("/tmp/x/out.mp4") == Path("/tmp/x/.out.partial.mp4")
    assert backend.partial_path(Path("clip.mkv")) == Path(".clip.partial.mkv")


# ---------------------------------------------------------------- background jobs


def _py(code: str) -> list[str]:
    return [sys.executable, "-c", textwrap.dedent(code)]


def test_job_reports_progress_and_the_exit_code():
    job = backend.Job(
        _py(
            """
            import sys, time
            for n in (15, 30, 45):
                sys.stderr.write(f"\\r  {n} frames  12.50 fps   ")
                sys.stderr.flush()
                time.sleep(0.05)
            sys.stderr.write("\\r  45 frames in 3.6s (12.50 fps)          \\n")
            """
        )
    ).start()
    state = job.wait(timeout=20)
    assert (state.running, state.returncode, state.cancelled) == (False, 0, False)
    assert state.progress == Progress(45, 12.5)


def test_job_poll_does_not_block_and_wait_times_out():
    job = backend.Job(_py("import time; time.sleep(3)")).start()
    assert job.poll().running is True
    with pytest.raises(TimeoutError):
        job.wait(timeout=0.2)
    job.cancel()
    job.wait(timeout=10)


def test_job_keeps_the_end_of_stderr_when_it_fails():
    state = backend.Job(_py("import sys; sys.stderr.write('Error: boom\\n'); sys.exit(3)")).start().wait(timeout=20)
    assert state.returncode == 3 and not state.cancelled
    assert "boom" in state.log


def test_job_captures_stdout_while_it_runs():
    job = backend.Job(_py("import sys; sys.stdout.buffer.write(bytes(range(256)) * 4000)"), capture_stdout=True).start()
    job.wait(timeout=20)
    assert job.stdout == bytes(range(256)) * 4000


def test_starting_a_missing_program_is_a_gui_error():
    with pytest.raises(GuiError):
        backend.Job(["/nonexistent/program"]).start()


@pytest.mark.skipif(os.name != "posix", reason="process groups")
def test_job_cancel_stops_the_whole_process_tree():
    job = backend.Job(
        _py(
            """
            import subprocess, sys, time
            child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
            print(child.pid, flush=True)
            time.sleep(60)
            """
        ),
        capture_stdout=True,
    ).start()
    deadline = time.time() + 10
    while b"\n" not in job.stdout and time.time() < deadline:
        time.sleep(0.05)
    child_pid = int(job.stdout.split()[0])
    job.cancel()
    state = job.wait(timeout=10)
    assert state.cancelled and not state.running
    assert state.returncode not in (None, 0)
    deadline = time.time() + 5
    while time.time() < deadline:
        try:
            os.kill(child_pid, 0)
        except ProcessLookupError:
            return
        time.sleep(0.1)
    pytest.fail("the child of the cancelled job is still running")


# ---------------------------------------------------------------- decoding and the real tool


def _run(binary, req: RunRequest):
    state = backend.Job(backend.run_args(binary, req, None)).start().wait(timeout=300)
    assert state.returncode == 0, state.log


def _decode(ffmpeg: str, path: Path, size, start: float = 0.0, frames: int | None = None) -> Frames:
    job = backend.Job(backend.decode_args(ffmpeg, path, size, start=start, frames=frames), capture_stdout=True).start()
    assert job.wait(timeout=120).returncode == 0
    return Frames(size[0], size[1], job.stdout)


def _psnr(a: bytes, b: bytes) -> float:
    x, y = np.frombuffer(a, np.uint8).astype(np.float64), np.frombuffer(b, np.uint8).astype(np.float64)
    mse = float(np.mean((x - y) ** 2))
    return 99.0 if mse == 0 else 10 * np.log10(255.0**2 / mse)


def test_decode_args_yield_exactly_the_requested_frames_from_the_requested_place(ffmpeg, clip):
    everything = _decode(ffmpeg, clip, (160, 120))
    assert abs(everything.count - 120) <= 1
    five = _decode(ffmpeg, clip, (160, 120), start=frame_time(30), frames=5)
    assert five.count == 5
    assert five.frame(0) == everything.frame(30)


def test_a_preview_window_matches_the_full_render(binary, ffmpeg, clip, tmp_path):
    """The point of a preview: the same tape, wherever the render starts."""
    look, size = Look(preset="bad", seed=3), (320, 240)
    info = backend.probe(clip)
    full_file, window_file = tmp_path / "full.mp4", tmp_path / "window.mp4"
    _run(binary, RunRequest(str(clip), str(full_file), look, size=size, crf=10))
    plan = backend.preview_plan(info, at=2.0, length=1.0, max_size=size)
    _run(binary, RunRequest(str(clip), str(window_file), look, start=plan.start, duration=plan.duration, size=plan.size, crf=10))
    full = _decode(ffmpeg, full_file, size)
    window = _decode(ffmpeg, window_file, plan.size).skip(plan.lead_frames)
    assert window.count == plan.frames

    def mean_psnr(shift: int) -> float:
        return float(np.mean([_psnr(window.frame(j), full.frame(plan.start_frame + j + shift)) for j in range(5, 20)]))

    assert mean_psnr(0) > 35.0  # only codec noise separates them
    assert mean_psnr(1) < 28.0  # a frame off would be a different piece of tape


def test_the_core_accepts_what_merge_config_writes(binary, ffmpeg, clip, tmp_path):
    look = Look(preset="good", condition={"head_wear": 0.5, "psu_smps": True}, advanced="[tape]\ncreases_per_m = 0.5\n")
    config = tmp_path / "look.toml"
    config.write_text(backend.merge_config(look))
    out = tmp_path / "out.mp4"
    req = RunRequest(str(clip), str(out), look, duration=0.5, size=(320, 240))
    state = backend.Job(backend.run_args(binary, req, config)).start().wait(timeout=120)
    assert state.returncode == 0, state.log
    assert out.stat().st_size > 1000
