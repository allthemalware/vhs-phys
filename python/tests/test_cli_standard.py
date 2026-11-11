# CONTRACT
"""The command line tool's television standard: NTSC, PAL (625/50) and PAL-60."""
from __future__ import annotations

import json
import subprocess


def run_cli(binary, clip, out, *extra):
    cmd = [str(binary), "run", str(clip), str(out), "--size", "320x240", "--crf", "30", "-t", "1", "--audio", "none", *extra]
    return subprocess.run(cmd, capture_output=True, text=True, timeout=300)


def video_stream(path):
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "v:0", "-count_frames",
         "-show_entries", "stream=r_frame_rate,nb_read_frames", "-of", "json", str(path)],
        capture_output=True, text=True, check=True,
    ).stdout
    return json.loads(out)["streams"][0]


def test_ntsc_is_the_default_at_29_97(binary, clip, tmp_path):
    r = run_cli(binary, clip, tmp_path / "n.mp4")
    assert r.returncode == 0, r.stderr
    s = video_stream(tmp_path / "n.mp4")
    assert s["r_frame_rate"] == "30000/1001"
    assert abs(int(s["nb_read_frames"]) - 30) <= 1


def test_pal_runs_at_25_frames(binary, clip, tmp_path):
    r = run_cli(binary, clip, tmp_path / "p.mp4", "--standard", "pal")
    assert r.returncode == 0, r.stderr
    s = video_stream(tmp_path / "p.mp4")
    assert s["r_frame_rate"] == "25/1"
    assert abs(int(s["nb_read_frames"]) - 25) <= 1


def test_pal60_is_an_ntsc_tape_at_29_97(binary, clip, tmp_path):
    r = run_cli(binary, clip, tmp_path / "p60.mp4", "--standard", "pal60")
    assert r.returncode == 0, r.stderr
    assert video_stream(tmp_path / "p60.mp4")["r_frame_rate"] == "30000/1001"


def test_a_config_file_can_choose_the_standard(binary, clip, tmp_path):
    cfg = tmp_path / "pal.toml"
    cfg.write_text('standard = "pal"\n')
    r = run_cli(binary, clip, tmp_path / "c.mp4", "--config", str(cfg))
    assert r.returncode == 0, r.stderr
    assert video_stream(tmp_path / "c.mp4")["r_frame_rate"] == "25/1"


def test_bad_combinations_are_refused(binary, clip, tmp_path):
    assert run_cli(binary, clip, tmp_path / "x.mp4", "--standard", "secam").returncode != 0
    cfg = tmp_path / "ep.toml"
    cfg.write_text('[condition]\ntape_speed = "ep"\n')
    r = run_cli(binary, clip, tmp_path / "y.mp4", "--standard", "pal", "--config", str(cfg))
    assert r.returncode != 0
    assert "EP" in r.stderr


def test_the_camera_tube_runs_in_both_engines(binary, clip, tmp_path):
    for std in ("ntsc", "pal"):
        r = run_cli(binary, clip, tmp_path / f"cam_{std}.mp4", "--standard", std, "--camera-tube", "plumbicon", "--iris-stops", "2")
        assert r.returncode == 0, r.stderr


def test_show_prints_the_engine_s_parameters(binary):
    pal = subprocess.run([str(binary), "show", "worn", "--standard", "pal"], capture_output=True, text=True, timeout=60)
    assert pal.returncode == 0, pal.stderr
    assert 'standard = "pal"' in pal.stdout
    ntsc = subprocess.run([str(binary), "show", "worn"], capture_output=True, text=True, timeout=60)
    assert 'standard = "ntsc"' in ntsc.stdout
