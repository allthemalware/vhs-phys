# CONTRACT
"""Shared fixtures for the GUI tests: the built binary and small synthetic clips."""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

PYTHON_DIR = Path(__file__).resolve().parents[1]
if str(PYTHON_DIR) not in sys.path:
    sys.path.insert(0, str(PYTHON_DIR))


def _unavailable(what: str):
    """Skip, unless the run is not allowed to pass without it."""
    if os.environ.get("VHSGUI_REQUIRE_BINARY"):
        pytest.fail(f"{what} (VHSGUI_REQUIRE_BINARY is set)")
    pytest.skip(what)


@pytest.fixture(scope="session")
def binary() -> Path:
    from vhsgui import backend

    found = backend.find_binary()
    if found is None:
        _unavailable("vhsphys is not built (cargo build --release)")
    return found


@pytest.fixture(scope="session")
def ffmpeg() -> str:
    path = shutil.which("ffmpeg")
    if path is None:
        _unavailable("ffmpeg is not installed")
    return path


@pytest.fixture(scope="session")
def ffprobe() -> str:
    path = shutil.which("ffprobe")
    if path is None:
        _unavailable("ffprobe is not installed")
    return path


def make_clip(path: Path, ffmpeg: str, audio: bool = False) -> Path:
    """A 4 s, 320x240, 29.97 fps test pattern with motion (and a tone when `audio`)."""
    cmd = [ffmpeg, "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30000/1001:duration=4"]
    if audio:
        cmd += ["-f", "lavfi", "-i", "sine=frequency=440:duration=4", "-c:a", "aac"]
    cmd += ["-pix_fmt", "yuv420p", "-c:v", "libx264", "-crf", "12", str(path)]
    subprocess.run(cmd, check=True)
    return path


@pytest.fixture(scope="session")
def clip(tmp_path_factory, ffmpeg) -> Path:
    return make_clip(tmp_path_factory.mktemp("clips") / "clip.mp4", ffmpeg)


@pytest.fixture(scope="session")
def clip_audio(tmp_path_factory, ffmpeg) -> Path:
    return make_clip(tmp_path_factory.mktemp("clips_audio") / "clip_audio.mp4", ffmpeg, audio=True)
