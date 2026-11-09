# CONTRACT
"""Shared vocabulary of the vhs-phys GUI.

The backend (no Qt) and the window (Qt) both import this module, and so do the
tests. It is frozen: change it only deliberately, because both sides depend on it.
"""
from __future__ import annotations

from dataclasses import dataclass, field

# The core works at NTSC frame timing whatever the frame rate of the input is.
FPS_NUM = 30000
FPS_DEN = 1001
FPS = FPS_NUM / FPS_DEN

# A frame needs two frames of context on each side, so the first frames of a fresh
# render are not representative. A render that starts mid-file therefore begins this
# many frames early and discards them.
LEAD_FRAMES = 3

PREVIEW_MAX_SIZE = (640, 400)  # largest preview frame (width, height)
PREVIEW_DEFAULT_SECONDS = 2.0
LOG_CHARS = 4000  # how much of a process's stderr JobState.log keeps


def frame_time(index: int) -> float:
    """Start time in seconds of frame `index` at the NTSC rate."""
    return index * FPS_DEN / FPS_NUM


class GuiError(Exception):
    """A problem the user can act on; str(error) is shown to them as it is."""


# ---------------------------------------------------------------------------
# The condition: the high-level knobs of the core (`[condition]` in a config file)
# ---------------------------------------------------------------------------

GROUPS = ("Tape", "Heads and drum", "Playback deck", "Transport and tracking", "Recording deck")


@dataclass(frozen=True)
class FieldSpec:
    """One control of the condition: how to label it and which range it takes."""

    key: str  # name in the core's [condition] table
    label: str
    group: str  # one of GROUPS
    kind: str  # "float" or "bool"
    lo: float = 0.0
    hi: float = 1.0
    step: float = 0.01
    unit: str = ""
    help: str = ""


CONDITION_FIELDS: tuple[FieldSpec, ...] = (
    # ---- tape ----
    FieldSpec("tape_age_years", "Tape age", "Tape", "float", 0, 50, 0.5, "years", "Years since the recording was made."),
    FieldSpec("tape_storage", "Storage", "Tape", "float", 0, 1, 0.01, "", "0 = cool dry shelf, 1 = hot humid attic."),
    FieldSpec("tape_quality", "Tape quality", "Tape", "float", 0, 1, 0.01, "", "0 = cheap tape, 1 = premium (smoother, better calendered coating)."),
    FieldSpec("tape_passes", "Earlier plays", "Tape", "float", 0, 200, 1, "plays", "How often the tape has been played already; loose debris wears off."),
    FieldSpec("tape_stretch", "Stretch and edge damage", "Tape", "float", 0, 1, 0.01, "", "0 = fine, 1 = stretched, creased and edge damaged."),
    FieldSpec("tape_position_m", "Position on tape", "Tape", "float", 0, 400, 1, "m", "Where the clip sits on the tape. Other positions give other defects."),
    # ---- heads and drum ----
    FieldSpec("head_wear", "Head wear", "Heads and drum", "float", 0, 1, 0.01, "", "Worn heads lose signal and resolution and drift in azimuth."),
    FieldSpec("head_clog", "Head clog", "Heads and drum", "float", 0, 1, 0.01, "", "Debris on one head: one noisy field and dropouts."),
    FieldSpec("drum_wear", "Drum wear", "Heads and drum", "float", 0, 1, 0.01, "", "Bearing wear: timebase wobble."),
    # ---- playback deck ----
    FieldSpec("deck_age_years", "Deck age", "Playback deck", "float", 0, 50, 0.5, "years", "Ages the electrolytic capacitors and resistors of the playback deck."),
    FieldSpec("deck_temp_c", "Capacitor temperature", "Playback deck", "float", 20, 90, 1, "°C", "Temperature around the capacitors; their life halves per 10 °C."),
    FieldSpec("deck_humidity", "Humidity", "Playback deck", "float", 0, 1, 0.01, "", "Drifts resistor values: emphasis network, detuned discriminator."),
    FieldSpec("psu_smps", "Switch-mode supply", "Playback deck", "bool", help="Switch-mode instead of a linear supply: fine herringbone instead of hum bars."),
    FieldSpec("mains_hz", "Mains frequency", "Playback deck", "float", 50, 60, 10, "Hz", "Frequency of the hum."),
    FieldSpec("mains_sag", "Mains sag", "Playback deck", "float", 0, 0.2, 0.005, "", "Fraction the mains is low (0.1 = 10 %); tips a tired supply into ripple."),
    # ---- transport and tracking ----
    FieldSpec("tracking_error_um", "Tracking error", "Transport and tracking", "float", -40, 40, 0.5, "µm", "Head offset from the track centre: noise band and crosstalk."),
    FieldSpec("tracking_slope_um", "Tracking slope", "Transport and tracking", "float", -100, 100, 1, "µm", "Change of the offset along each track: sweeps a noise band across the picture."),
    FieldSpec("tension_error", "Tape tension error", "Transport and tracking", "float", -1, 1, 0.05, "", "Negative = slack (noise and flagging at the top), positive = too tight."),
    # ---- recording deck ----
    FieldSpec("rec_deck_age_years", "Recorder age", "Recording deck", "float", 0, 50, 0.5, "years", "Age of the deck that made the recording."),
    FieldSpec("rec_deck_temp_c", "Recorder temperature", "Recording deck", "float", 20, 90, 1, "°C", "Temperature around that deck's capacitors."),
)


# ---------------------------------------------------------------------------
# Data passed between the backend and the window
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class PresetInfo:
    name: str
    summary: str  # the description printed by `vhsphys presets`


@dataclass(frozen=True)
class MediaInfo:
    path: str
    width: int  # display size: rotation metadata is already applied
    height: int
    fps: float  # average frame rate; 0.0 when unknown
    duration: float  # seconds; 0.0 when unknown
    has_audio: bool

    @property
    def frames(self) -> int:
        """Frames the core will see: the duration at the NTSC rate."""
        return int(round(self.duration * FPS))


@dataclass
class Look:
    """Everything that defines the look. Preview and final render use the same one."""

    preset: str = "worn"
    seed: int = 1
    # Overrides of the preset's condition: any subset of the CONDITION_FIELDS keys.
    condition: dict[str, float | bool] = field(default_factory=dict)
    # TOML text with raw parameter tables, e.g. "[tape]\ncreases_per_m = 0.5\n".
    advanced: str = ""


@dataclass
class RunRequest:
    input: str
    output: str
    look: Look
    start: float = 0.0  # seconds into the input; the backend snaps it to a frame
    duration: float | None = None  # seconds; None renders to the end
    size: tuple[int, int] | None = None  # output (width, height); None keeps the input size
    crf: int = 18
    threads: int | None = None


@dataclass(frozen=True)
class Progress:
    frames: int  # frames written so far
    fps: float  # processing speed


@dataclass(frozen=True)
class JobState:
    running: bool
    returncode: int | None  # None until the process has exited
    cancelled: bool
    progress: Progress | None
    log: str  # the end of the process's stderr, at most LOG_CHARS characters


@dataclass(frozen=True)
class PreviewPlan:
    """What to render and show for one preview, derived from a position and a length."""

    start_frame: int  # absolute index of the first frame shown
    lead_frames: int  # rendered before it and thrown away (LEAD_FRAMES, fewer near the start of the file)
    frames: int  # frames shown, at least 1
    start: float  # seconds to hand to the CLI: frame_time(start_frame - lead_frames)
    # seconds to render: (lead_frames + frames) frame periods. ffmpeg ends a clip at the nearest
    # whole frame, so an exact number of periods is the value that survives rounding.
    duration: float
    size: tuple[int, int]  # width, height of every preview frame: even, input aspect, within the maximum


@dataclass(frozen=True)
class Frames:
    """Decoded video: packed RGB24 frames one after another."""

    width: int
    height: int
    data: bytes

    @property
    def count(self) -> int:
        return len(self.data) // (self.width * self.height * 3)

    def frame(self, index: int) -> bytes:
        size = self.width * self.height * 3
        if not 0 <= index < self.count:
            raise IndexError(f"frame {index} of {self.count}")
        return self.data[index * size : (index + 1) * size]

    def skip(self, n: int) -> "Frames":
        """The same video without its first `n` frames."""
        return Frames(self.width, self.height, self.data[n * self.width * self.height * 3 :])
