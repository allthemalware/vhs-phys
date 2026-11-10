"""Everything the window needs that is not Qt: locating the tools, asking the CLI about
presets, probing media, frame arithmetic, configuration text, command lines and background
processes. Standard library only, so it can be tested and reused without a display."""
from __future__ import annotations

import codecs
import datetime
import json
import math
import os
import re
import shutil
import signal
import subprocess
import threading
import time
import tomllib
from collections.abc import Sequence
from pathlib import Path
from typing import Any

from vhsgui.model import (
    FPS,
    FPS_DEN,
    FPS_NUM,
    LEAD_FRAMES,
    LOG_CHARS,
    PREVIEW_MAX_SIZE,
    AUDIO_MODES,
    MAX_GENERATIONS,
    SIGNALS,
    GuiError,
    Look,
    MediaInfo,
    PresetInfo,
    PreviewPlan,
    Progress,
    JobState,
    RunRequest,
    frame_time,
)

BINARY_NAME = "vhsphys"
ENV_BINARY = "VHSPHYS_BIN"
REPO_DEPTH = 2  # this file is <repo>/python/vhsgui/backend.py
TOOL_TIMEOUT_S = 30.0  # the CLI and ffprobe answer these small questions at once
ROTATION_SWAP = (90, 270)  # display rotations (degrees) that exchange width and height
ERROR_PREFIX = "Error: "  # what the CLI puts in front of a failure message
MICROSECONDS = 1_000_000  # seeks are cut to this resolution
ASPECT_TOLERANCE_STEPS = 8  # even widths tried below the largest one while fitting the aspect ratio
DEFAULT_SIGNAL = "svideo"  # the deck's own Y/C: no composite stage is configured
DEFAULT_AUDIO = "process"  # the core's default, so no flag is needed
EVEN = 2  # video encoders need even frame dimensions
ASPECT_EPSILON = 1e-9  # guards the floor of sizes that are exact in theory but not in floats

READ_CHUNK = 65536  # bytes asked of a pipe per read
PROGRESS_WINDOW = 400  # characters of recent stderr searched for a progress update
DRAIN_GRACE_S = 2.0  # how long readers may outlive the process (a stray child can hold a pipe)
KILL_GRACE_S = 2.0  # time between SIGTERM and SIGKILL on cancel
KILL_POLL_S = 0.05  # how often the helper checks whether the process group is gone
CANCELLED_RETURNCODE = -1  # reported when a job that was cancelled exits with 0 anyway

# `\r  15 frames  12.34 fps   ` while running, `\r  899 frames in 79.1s (11.37 fps)` at the end.
_PROGRESS_RE = re.compile(r"(?<!\d)(\d+) frames\s+(?:in\s+[\d.]+s\s+\(([\d.]+) fps\)|([\d.]+) fps)")
_BARE_KEY_RE = re.compile(r"[A-Za-z0-9_-]+")
_TOML_ESCAPES = {
    '"': '\\"',
    "\\": "\\\\",
    "\b": "\\b",
    "\t": "\\t",
    "\n": "\\n",
    "\f": "\\f",
    "\r": "\\r",
}
_DEL = 0x7F  # TOML does not allow it unescaped in a string
_FIRST_PRINTABLE = 0x20


# ---------------------------------------------------------------- locating the tools


def find_binary(explicit: str | None = None) -> Path | None:
    """Find the `vhsphys` executable, or None when there is none."""
    if explicit:
        path = Path(explicit)
        if not path.is_file():
            raise GuiError(f"vhsphys not found: {explicit}")
        return path
    env = os.environ.get(ENV_BINARY)
    if env and Path(env).is_file():
        return Path(env)
    repo = Path(__file__).resolve().parents[REPO_DEPTH]
    names = [BINARY_NAME]
    if os.name == "nt":
        names.append(BINARY_NAME + ".exe")
    for build in ("release", "debug"):
        for name in names:
            candidate = repo / "target" / build / name
            if candidate.is_file():
                return candidate
    for name in names:
        found = shutil.which(name)
        if found:
            return Path(found)
    return None


def _run_tool(args: Sequence[str]) -> str:
    """Run a short command and return its stdout; failures become a GuiError."""
    try:
        done = subprocess.run(
            [str(a) for a in args],
            capture_output=True,
            stdin=subprocess.DEVNULL,
            encoding="utf-8",
            errors="replace",
            timeout=TOOL_TIMEOUT_S,
        )
    except FileNotFoundError:
        raise GuiError(f"{args[0]} not found") from None
    except subprocess.TimeoutExpired:
        raise GuiError(f"{args[0]} did not answer in {TOOL_TIMEOUT_S:.0f} s") from None
    except OSError as err:
        raise GuiError(f"cannot run {args[0]}: {err}") from None
    if done.returncode != 0:
        message = done.stderr.strip()
        if message.startswith(ERROR_PREFIX):
            message = message[len(ERROR_PREFIX) :]
        raise GuiError(message or f"{args[0]} failed with exit code {done.returncode}")
    return done.stdout


# ---------------------------------------------------------------- presets and the condition


def list_presets(binary: Path | str) -> list[PresetInfo]:
    """The presets of `vhsphys presets`, in the order the CLI lists them."""
    presets: list[PresetInfo] = []
    for line in _run_tool([str(binary), "presets"]).splitlines():
        if not line.startswith("  ") or not line.strip():
            continue
        parts = line.strip().split(None, 1)
        presets.append(PresetInfo(parts[0], parts[1].strip() if len(parts) > 1 else ""))
    return presets


def preset_condition(binary: Path | str, name: str) -> dict[str, float | bool]:
    """The `[condition]` table of a preset."""
    text = _run_tool([str(binary), "show", name, "--condition"])
    try:
        doc = tomllib.loads(text)
    except tomllib.TOMLDecodeError as err:
        raise GuiError(f"Unreadable condition of preset '{name}': {err}") from None
    # the tool prints the table under its `[condition]` header; older builds printed the keys bare
    table = doc.get("condition", doc)
    if not isinstance(table, dict):
        raise GuiError(f"Unreadable condition of preset '{name}': no [condition] table")
    return table


# ---------------------------------------------------------------- probing media


def _parse_rate(text: Any) -> float:
    """A frame rate such as `30000/1001`; 0.0 for zero or anything unreadable."""
    try:
        if isinstance(text, str) and "/" in text:
            num, den = text.split("/", 1)
            return float(num) / float(den) if float(den) != 0 else 0.0
        return float(text)
    except (TypeError, ValueError):
        return 0.0


def _parse_seconds(value: Any) -> float | None:
    try:
        seconds = float(value)
    except (TypeError, ValueError):
        return None
    return seconds if math.isfinite(seconds) and seconds > 0 else None


def _rotation(stream: dict[str, Any]) -> int:
    """Display rotation in degrees from the side data or the old `rotate` tag."""
    candidates: list[Any] = [side.get("rotation") for side in stream.get("side_data_list", []) if isinstance(side, dict)]
    tags = stream.get("tags")
    if isinstance(tags, dict):
        candidates.append(tags.get("rotate"))
    for value in candidates:
        try:
            return int(round(float(value))) % 360
        except (TypeError, ValueError):
            continue
    return 0


def probe(path: str | Path, ffprobe: str = "ffprobe") -> MediaInfo:
    """Size, rate and length of a media file, with rotation metadata applied."""
    if not Path(path).is_file():
        raise GuiError(f"File not found: {path}")
    entries = "stream=codec_type,width,height,avg_frame_rate,duration:stream_side_data=rotation:stream_tags=rotate:format=duration"
    try:
        out = _run_tool([ffprobe, "-v", "error", "-show_entries", entries, "-of", "json", str(path)])
    except GuiError as err:
        raise GuiError(f"Cannot read {path}: {err}") from None
    try:
        doc = json.loads(out)
    except ValueError:
        raise GuiError(f"Cannot read {path}: unreadable ffprobe output") from None
    streams = [s for s in doc.get("streams", []) if isinstance(s, dict)]
    video = next((s for s in streams if s.get("codec_type") == "video"), None)
    if video is None:
        raise GuiError(f"{path} has no video stream")
    width, height = video.get("width"), video.get("height")
    if not isinstance(width, int) or not isinstance(height, int) or width <= 0 or height <= 0:
        raise GuiError(f"{path} has no usable video size")
    if _rotation(video) in ROTATION_SWAP:
        width, height = height, width
    duration = _parse_seconds((doc.get("format") or {}).get("duration"))
    if duration is None:
        duration = _parse_seconds(video.get("duration"))
    return MediaInfo(
        path=str(path),
        width=width,
        height=height,
        fps=_parse_rate(video.get("avg_frame_rate")),
        duration=duration or 0.0,
        has_audio=any(s.get("codec_type") == "audio" for s in streams),
    )


# ---------------------------------------------------------------- frame alignment


def snap_start(seconds: float) -> tuple[int, float]:
    """The frame at or nearest to `seconds`, and its start time."""
    frame = max(0, round(seconds * FPS))
    return frame, frame_time(frame)


def _preview_size(info: MediaInfo, max_size: tuple[int, int]) -> tuple[int, int]:
    """Largest even size inside `max_size` and the input that keeps the input's aspect ratio."""
    max_w, max_h = max_size
    cap_h = max(EVEN, min(max_h, info.height) // EVEN * EVEN)
    widest = min(max_w, info.width, max_h * info.width / info.height)
    top = max(EVEN, int(widest + ASPECT_EPSILON) // EVEN * EVEN)
    aspect = info.width / info.height
    best: tuple[float, int, int] | None = None
    for step in range(ASPECT_TOLERANCE_STEPS):
        w = top - step * EVEN
        if w < EVEN:
            break
        h = min(cap_h, max(EVEN, round(w / aspect / EVEN) * EVEN))
        error = abs(w / h - aspect)
        if best is None or error < best[0]:
            best = (error, w, h)
    assert best is not None  # the loop always runs once: top >= EVEN
    return best[1], best[2]


def preview_plan(info: MediaInfo, at: float, length: float, max_size: tuple[int, int] = PREVIEW_MAX_SIZE) -> PreviewPlan:
    """Which frames to render and show for a preview of `length` seconds around `at`."""
    total = info.frames if info.duration > 0 else None
    start_frame = max(0, round(at * FPS))
    if total is not None:
        start_frame = min(start_frame, max(0, total - 1))
    frames = max(1, round(length * FPS))
    if total is not None:
        frames = max(1, min(frames, total - start_frame))
    lead = min(LEAD_FRAMES, start_frame)
    return PreviewPlan(
        start_frame=start_frame,
        lead_frames=lead,
        frames=frames,
        start=frame_time(start_frame - lead),
        duration=frame_time(lead + frames),
        size=_preview_size(info, max_size),
    )


# ---------------------------------------------------------------- configuration text


def _toml_string(text: str) -> str:
    out = ['"']
    for ch in text:
        if ch in _TOML_ESCAPES:
            out.append(_TOML_ESCAPES[ch])
        elif ord(ch) < _FIRST_PRINTABLE or ord(ch) == _DEL:
            out.append(f"\\u{ord(ch):04x}")
        else:
            out.append(ch)
    out.append('"')
    return "".join(out)


def _toml_key(key: str) -> str:
    return key if _BARE_KEY_RE.fullmatch(key) else _toml_string(key)


def _toml_value(value: Any) -> str:
    if isinstance(value, bool):  # before int: bool is an int
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        return repr(value)  # always has a '.', an exponent, inf or nan, so it reads back as a float
    if isinstance(value, str):
        return _toml_string(value)
    if isinstance(value, (datetime.datetime, datetime.date, datetime.time)):
        return value.isoformat()
    if isinstance(value, (list, tuple)):
        return "[" + ", ".join(_toml_value(v) for v in value) + "]"
    if isinstance(value, dict):
        return "{" + ", ".join(f"{_toml_key(str(k))} = {_toml_value(v)}" for k, v in value.items()) + "}"
    raise GuiError(f"Cannot write a value of type {type(value).__name__} to the configuration")


def _emit_table(table: dict[str, Any], path: tuple[str, ...], lines: list[str]) -> None:
    """Scalars of a table first, then its sub-tables as `[a]` / `[a.b]` sections."""
    scalars = {k: v for k, v in table.items() if not isinstance(v, dict)}
    tables = {k: v for k, v in table.items() if isinstance(v, dict)}
    if path and (scalars or not tables):
        if lines:
            lines.append("")
        lines.append("[" + ".".join(_toml_key(p) for p in path) + "]")
    for key, value in scalars.items():
        lines.append(f"{_toml_key(key)} = {_toml_value(value)}")
    for key, sub in tables.items():
        _emit_table(sub, path + (key,), lines)


def _deep_merge(base: dict[str, Any], over: dict[str, Any]) -> None:
    for key, value in over.items():
        if isinstance(value, dict) and isinstance(base.get(key), dict):
            _deep_merge(base[key], value)
        else:
            base[key] = value


def merge_config(look: Look) -> str:
    """TOML for the core: the condition controls with the advanced tables merged on top."""
    try:
        advanced = tomllib.loads(look.advanced)
    except tomllib.TOMLDecodeError as err:
        raise GuiError(f"Advanced settings: {err}") from None
    if look.signal not in SIGNALS:
        raise GuiError(f"Unknown output signal '{look.signal}'")
    doc: dict[str, Any] = {"condition": dict(look.condition)} if look.condition else {}
    if look.signal != DEFAULT_SIGNAL:
        doc["composite"] = {"enabled": True, "decoder": look.signal}
    _deep_merge(doc, advanced)  # after the signal, so the advanced text wins
    lines: list[str] = []
    _emit_table(doc, (), lines)
    return "\n".join(lines) + "\n" if lines else ""


def save_look(look: Look) -> str:
    """A look file: the `[look]` table of the GUI followed by the core config derived from it.

    The core ignores the unknown `[look]` table, so the file works as `--config` too."""
    config = merge_config(look)  # validates the advanced text and the signal first
    fields = {
        "preset": look.preset,
        "seed": look.seed,
        "generations": look.generations,
        "signal": look.signal,
        "audio": look.audio,
        "advanced": look.advanced,
    }
    lines = ["[look]"] + [f"{key} = {_toml_value(value)}" for key, value in fields.items()]
    text = "\n".join(lines) + "\n"
    if config:
        text += "\n" + config
    return text


def _is_int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def load_look(text: str) -> Look:
    """The inverse of `save_look`. Only `[look]` and `[condition]` are read; the rest is derived."""
    try:
        doc = tomllib.loads(text)
    except tomllib.TOMLDecodeError as err:
        raise GuiError(f"Not a look file: {err}") from None
    section = doc.get("look")
    if not isinstance(section, dict):
        raise GuiError("Not a look file: there is no [look] table")
    defaults = Look()
    preset = section.get("preset", defaults.preset)
    if not isinstance(preset, str) or not preset:
        raise GuiError("Look file: preset must be a non-empty string")
    seed = section.get("seed", defaults.seed)
    if not _is_int(seed) or seed < 0:
        raise GuiError("Look file: seed must be an integer of 0 or more")
    generations = section.get("generations", defaults.generations)
    if not _is_int(generations) or not 1 <= generations <= MAX_GENERATIONS:
        raise GuiError(f"Look file: generations must be an integer from 1 to {MAX_GENERATIONS}")
    output_signal = section.get("signal", defaults.signal)
    if output_signal not in SIGNALS:
        raise GuiError(f"Look file: signal must be one of {', '.join(SIGNALS)}")
    audio = section.get("audio", defaults.audio)
    if audio not in AUDIO_MODES:
        raise GuiError(f"Look file: audio must be one of {', '.join(AUDIO_MODES)}")
    advanced = section.get("advanced", defaults.advanced)
    if not isinstance(advanced, str):
        raise GuiError("Look file: advanced must be a string")
    raw = doc.get("condition", {})
    if not isinstance(raw, dict):
        raise GuiError("Look file: [condition] must be a table")
    condition: dict[str, float | bool] = {}
    for key, value in raw.items():
        if not isinstance(value, (int, float, bool, str)):
            raise GuiError(f"Look file: condition value '{key}' must be a number, a bool or a string")
        condition[key] = value
    return Look(
        preset=preset,
        seed=seed,
        condition=condition,
        advanced=advanced,
        generations=generations,
        signal=output_signal,
        audio=audio,
    )


def validate_advanced(text: str) -> str | None:
    """None when `text` is empty or valid TOML, otherwise the parser's message."""
    if not text.strip():
        return None
    try:
        tomllib.loads(text)
    except tomllib.TOMLDecodeError as err:
        return str(err)
    return None


# ---------------------------------------------------------------- command lines


def _micros(seconds: float) -> str:
    """Seconds cut (not rounded) to whole microseconds, so a seek never skips the frame it aims at."""
    return f"{math.floor(seconds * MICROSECONDS) / MICROSECONDS:.6f}"


def run_args(binary: Path | str, req: RunRequest, config_path: Path | str | None) -> list[str]:
    """Command line of `vhsphys run` for a request."""
    args = [str(binary), "run", str(req.input), str(req.output), "--preset", req.look.preset, "--seed", str(req.look.seed)]
    if req.look.generations > 1:
        args += ["--generations", str(req.look.generations)]
    if req.look.audio != DEFAULT_AUDIO:
        args += ["--audio", req.look.audio]
    if config_path is not None:
        args += ["--config", str(config_path)]
    frame, seconds = snap_start(req.start)
    if frame > 0:
        args += ["--start", _micros(seconds)]
    if req.duration is not None:
        args += ["-t", repr(float(req.duration))]
    if req.size is not None:
        args += ["--size", f"{req.size[0]}x{req.size[1]}"]
    args += ["--crf", str(req.crf)]
    if req.threads is not None:
        args += ["--threads", str(req.threads)]
    return args


def parse_progress(text: str) -> Progress | None:
    """The last progress update in the CLI's stderr, if any."""
    last: re.Match[str] | None = None
    for last in _PROGRESS_RE.finditer(text):
        pass
    if last is None:
        return None
    return Progress(int(last.group(1)), float(last.group(2) or last.group(3)))


def partial_path(output: str | Path) -> Path:
    """Where a render is written until it is complete. The extension stays: it picks the encoder."""
    path = Path(output)
    return path.with_name(f".{path.stem}.partial{path.suffix}")


def decode_args(ffmpeg: str, path: str | Path, size: tuple[int, int], start: float = 0.0, frames: int | None = None) -> list[str]:
    """An ffmpeg command writing packed RGB24 frames of `size` to stdout, from `start` on."""
    args = [ffmpeg, "-v", "error"]
    if start > 0:
        args += ["-ss", _micros(start)]
    args += ["-i", str(path), "-an", "-vf", f"fps={FPS_NUM}/{FPS_DEN},scale={size[0]}:{size[1]}:flags=bicubic"]
    if frames is not None:
        args += ["-frames:v", str(frames)]
    args += ["-pix_fmt", "rgb24", "-f", "rawvideo", "pipe:1"]
    return args


# ---------------------------------------------------------------- background jobs


class Job:
    """A background process. Nothing here blocks except `wait`, so a timer can poll it."""

    def __init__(self, args: Sequence[str], *, capture_stdout: bool = False) -> None:
        self._args = [str(a) for a in args]
        self._capture = capture_stdout
        self._proc: subprocess.Popen[bytes] | None = None
        self._lock = threading.Lock()
        self._done = threading.Event()
        self._readers: list[threading.Thread] = []
        self._log = ""
        self._progress: Progress | None = None
        self._stdout = bytearray()
        self._cancelled = False

    def start(self) -> Job:
        if self._proc is not None:
            raise GuiError("The process was already started")
        options: dict[str, Any] = {}
        if os.name == "posix":
            options["start_new_session"] = True  # own process group, so cancel reaches the children
        try:
            self._proc = subprocess.Popen(
                self._args,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE if self._capture else subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                **options,
            )
        except (OSError, ValueError) as err:
            raise GuiError(f"Cannot start {self._args[0] if self._args else 'the process'}: {err}") from None
        assert self._proc.stderr is not None
        self._readers.append(threading.Thread(target=self._read_stderr, args=(self._proc.stderr.fileno(),), daemon=True))
        if self._capture:
            assert self._proc.stdout is not None
            self._readers.append(threading.Thread(target=self._read_stdout, args=(self._proc.stdout.fileno(),), daemon=True))
        for reader in self._readers:
            reader.start()
        threading.Thread(target=self._watch, daemon=True).start()
        return self

    # -- worker threads

    def _read_stderr(self, fd: int) -> None:
        # Progress updates end in "\r", not "\n", so the pipe is read in chunks, not lines.
        decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
        while True:
            try:
                chunk = os.read(fd, READ_CHUNK)
            except OSError:
                break
            text = decoder.decode(chunk, final=not chunk).replace("\r", "\n")
            if text:
                with self._lock:
                    self._log = (self._log + text)[-LOG_CHARS:]
                    progress = parse_progress(self._log[-PROGRESS_WINDOW:])
                    if progress is not None:
                        self._progress = progress
            if not chunk:
                break

    def _read_stdout(self, fd: int) -> None:
        while True:
            try:
                chunk = os.read(fd, READ_CHUNK)
            except OSError:
                break
            if not chunk:
                break
            with self._lock:
                self._stdout.extend(chunk)

    def _watch(self) -> None:
        proc = self._proc
        assert proc is not None
        proc.wait()
        deadline = time.monotonic() + DRAIN_GRACE_S
        for reader in self._readers:
            reader.join(max(0.0, deadline - time.monotonic()))
        # A reader that is still blocked owns its descriptor: closing it under the read could hit a reused fd.
        if not any(reader.is_alive() for reader in self._readers):
            for pipe in (proc.stderr, proc.stdout):
                if pipe is not None:
                    try:
                        pipe.close()
                    except OSError:
                        pass
        self._done.set()

    # -- public interface

    def poll(self) -> JobState:
        with self._lock:
            log, progress, cancelled = self._log, self._progress, self._cancelled
        finished = self._done.is_set()
        returncode = self._proc.returncode if finished and self._proc is not None else None
        if cancelled and returncode == 0:
            returncode = CANCELLED_RETURNCODE
        return JobState(
            running=not finished,
            returncode=returncode,
            cancelled=cancelled,
            progress=progress,
            log=log,
        )

    def wait(self, timeout: float | None = None) -> JobState:
        if self._proc is None:
            raise GuiError("The process was not started")
        if not self._done.wait(timeout):
            raise TimeoutError(f"the process did not finish within {timeout} s")
        return self.poll()

    def cancel(self) -> None:
        with self._lock:
            proc = self._proc
            if proc is None or self._cancelled or self._done.is_set():
                return
            self._cancelled = True
        if os.name == "posix":
            threading.Thread(target=self._terminate_group, args=(proc.pid,), daemon=True).start()
        else:
            try:
                proc.kill()
            except OSError:
                pass

    @staticmethod
    def _terminate_group(pgid: int) -> None:
        """SIGTERM the process group, SIGKILL whatever is still there after a grace period."""
        try:
            os.killpg(pgid, signal.SIGTERM)
        except OSError:
            return  # already gone
        deadline = time.monotonic() + KILL_GRACE_S
        while time.monotonic() < deadline:
            try:
                os.killpg(pgid, 0)
            except OSError:
                return
            time.sleep(KILL_POLL_S)
        try:
            os.killpg(pgid, signal.SIGKILL)
        except OSError:
            pass

    @property
    def stdout(self) -> bytes:
        with self._lock:
            return bytes(self._stdout)
