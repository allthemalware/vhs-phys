# CONTRACT
"""The command line tool's sound: simulated by the tape and deck, kept, or removed."""
from __future__ import annotations

import json
import subprocess

import numpy as np
import pytest

RATE = 48_000
TONE = 440.0


def run_cli(binary, clip, out, *extra):
    cmd = [str(binary), "run", str(clip), str(out), "--size", "320x240", "--crf", "30", "-t", "2.5", *extra]
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
    return result


def streams(path):
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "stream=codec_type,duration", "-of", "json", str(path)],
        capture_output=True, text=True, check=True,
    ).stdout
    return json.loads(out)["streams"]


def pcm(path):
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", str(path), "-vn", "-ac", "1", "-ar", str(RATE), "-f", "f32le", "pipe:1"],
        capture_output=True, check=True,
    ).stdout
    return np.frombuffer(raw, np.float32).astype(np.float64)


def tone_snr_db(x):
    """SNR of a 440 Hz tone: the power of the best-fit sinusoid over the power of the rest."""
    n = np.arange(len(x))
    basis = np.stack([np.cos(2 * np.pi * TONE * n / RATE), np.sin(2 * np.pi * TONE * n / RATE)])
    coeff = basis @ x * 2 / len(x)
    fit = coeff @ basis
    return 10 * np.log10(np.mean(fit**2) / max(np.mean((x - fit) ** 2), 1e-20))


def tone_amplitude(x):
    n = np.arange(len(x))
    return 2 * abs(np.mean(x * np.exp(-2j * np.pi * TONE * n / RATE)))


def test_the_sound_can_be_removed(binary, clip_audio, tmp_path):
    out = tmp_path / "none.mp4"
    assert run_cli(binary, clip_audio, out, "--preset", "worn", "--audio", "none").returncode == 0
    assert [s["codec_type"] for s in streams(out)] == ["video"]


def test_the_sound_can_be_kept_untouched(binary, clip_audio, tmp_path):
    out = tmp_path / "copy.mp4"
    assert run_cli(binary, clip_audio, out, "--preset", "worn", "--audio", "copy").returncode == 0
    assert "audio" in [s["codec_type"] for s in streams(out)]
    assert tone_snr_db(pcm(out)[RATE // 4 : RATE * 2]) > 25.0


def test_the_sound_is_simulated_by_default_and_by_request(binary, clip_audio, tmp_path):
    kept = tmp_path / "copy.mp4"
    assert run_cli(binary, clip_audio, kept, "--preset", "worn", "--audio", "copy").returncode == 0
    clean = tone_snr_db(pcm(kept)[RATE // 4 : RATE * 2])
    for name, extra in [("default", []), ("process", ["--audio", "process"])]:
        out = tmp_path / f"{name}.mp4"
        result = run_cli(binary, clip_audio, out, "--preset", "worn", *extra)
        assert result.returncode == 0, result.stderr
        assert "audio" in [s["codec_type"] for s in streams(out)], name
        x = pcm(out)
        assert tone_snr_db(x[RATE // 4 : RATE * 2]) < clean - 8.0, f"{name}: a worn tape should be noisier than the original"
        assert 0.05 < tone_amplitude(x[RATE // 4 : RATE * 2]) < 1.0, f"{name}: the tone must still be there"


def test_the_sound_stays_aligned_with_the_picture(binary, clip_audio, tmp_path):
    out = tmp_path / "aligned.mp4"
    assert run_cli(binary, clip_audio, out, "--preset", "good", "--audio", "process").returncode == 0
    info = {s["codec_type"]: float(s["duration"]) for s in streams(out)}
    assert abs(info["audio"] - info["video"]) < 0.08, info
    x = pcm(out)
    first, middle = tone_amplitude(x[: RATE // 10]), tone_amplitude(x[RATE : RATE + RATE // 10])
    assert first > 0.6 * middle, f"the tone must be there from the very start (latency compensated): {first:.3f} vs {middle:.3f}"


def test_a_clip_without_sound_still_renders_and_a_bad_mode_is_refused(binary, clip, tmp_path):
    out = tmp_path / "silent.mp4"
    assert run_cli(binary, clip, out, "--preset", "worn", "--audio", "process").returncode == 0
    assert [s["codec_type"] for s in streams(out)] == ["video"]
    assert run_cli(binary, clip, tmp_path / "x.mp4", "--audio", "loud").returncode != 0
