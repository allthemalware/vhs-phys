"""The vhs-phys window: pick a video, choose a look, preview it and render it to a file.

All process handling and arithmetic lives in `vhsgui.backend`; this module only holds
widgets and the glue that polls background jobs from a timer.
"""
from __future__ import annotations

import math
import os
import random
import shutil
import tempfile
from dataclasses import dataclass, replace
from pathlib import Path

from PySide6.QtCore import QEventLoop, QPointF, QRect, QRectF, Qt, QTimer, QUrl, Signal
from PySide6.QtGui import (
    QBrush,
    QCloseEvent,
    QColor,
    QDesktopServices,
    QFont,
    QFontDatabase,
    QImage,
    QKeyEvent,
    QKeySequence,
    QMouseEvent,
    QPainter,
    QPaintEvent,
    QPen,
    QPolygonF,
    QShortcut,
)
from PySide6.QtWidgets import (
    QButtonGroup,
    QCheckBox,
    QComboBox,
    QDialog,
    QDoubleSpinBox,
    QFileDialog,
    QGridLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QMainWindow,
    QMessageBox,
    QPlainTextEdit,
    QProgressBar,
    QPushButton,
    QScrollArea,
    QSizePolicy,
    QSlider,
    QSpinBox,
    QSplitter,
    QTabWidget,
    QVBoxLayout,
    QWidget,
)

from vhsgui import backend
from vhsgui.model import (
    AUDIO_MODES,
    CAMERA_TUBES,
    CONDITION_FIELDS,
    DEFAULT_STANDARD,
    GROUPS,
    IRIS_STOPS_RANGE,
    MAX_GENERATIONS,
    PREVIEW_DEFAULT_SECONDS,
    SIGNALS,
    STANDARDS,
    FieldSpec,
    Frames,
    GuiError,
    JobState,
    Look,
    MediaInfo,
    PreviewPlan,
    RunRequest,
    fps_of,
    frame_time,
)
from vhsgui.schematic.view import SchematicView

WINDOW_TITLE = "vhs-phys"
WINDOW_SIZE = (1180, 760)  # default size of the window
WINDOW_MIN_SIZE = (900, 600)
LEFT_WIDTH = 440  # initial width of the controls column
LEFT_MIN_WIDTH = 340  # the controls scroll below this width instead of squeezing further
PREVIEW_MIN_SIZE = (320, 200)
PREVIEW_BACKGROUND = "#141414"
DETACHED_PREVIEW_SIZE = (760, 520)  # default size of the detached preview window
ABOUT_DIALOG_SIZE = (980, 680)  # dimensions of the about dialog with interactive flow canvas
ABOUT_TITLE = "About vhs-phys"
KOFI_URL = "https://ko-fi.com/allthemalware"
GITHUB_URL = "https://github.com/allthemalware/vhs-phys"
PHOSPHOR_BG = "#010804"
PHOSPHOR_FAINT = "#0e4a22"
PHOSPHOR_DIM = "#1c9a45"
PHOSPHOR_NORM = "#39ff6e"
PHOSPHOR_HOT = "#d8ffe3"
PHOSPHOR_WARN = "#ffb347"
PHOSPHOR_GRID = "#061a0c"
ERROR_STYLE = "color: #c0392b;"

POLL_MS = 100  # how often running jobs are looked at
DEBOUNCE_MS = 150  # wait after the last move of the position slider before decoding a frame
MS_PER_SECOND = 1000  # preview playback ticks once per frame period of the television standard
CLOSE_WAIT_MS = 5000  # longest wait for killed processes when the window closes
CLOSE_POLL_MS = 50

SEED_MAX = 2**31 - 1  # largest value a QSpinBox holds
DEFAULT_PRESET = "worn"
POSITION_STEPS_PER_S = 10  # the position slider counts tenths of a second
TIME_STEP_S = 0.1  # step of the start and length boxes
TIME_DECIMALS = 2
UNKNOWN_DURATION_MAX_S = 86400.0  # limit of the start and length boxes when the input has no duration
LENGTH_MAX_S = 10.0  # longest preview
LENGTH_STEP_S = 0.5
PREVIEW_CRF = 18  # previews are decoded again at once, so quality is not worth waiting for
DEFAULT_CRF = 18
CRF_MAX = 51
PROGRESS_FULL = 100
PROGRESS_RUNNING_CAP = 99  # the bar only reaches 100 when the work is really done
SECONDS_PER_MINUTE = 60
TENTHS_PER_MINUTE = 600
MEGABYTE = 1024 * 1024
LOG_LINES = 3  # lines of a process's log shown in the status line
RGB_BYTES = 3  # bytes per pixel of the decoded frames
PREVIEW_FILE = "preview.mp4"
CONFIG_FILE = "look.toml"

# Menu texts, in the order of SIGNALS and AUDIO_MODES.
SIGNAL_LABELS = ("S-Video", "Composite, notch", "Composite, 1-line comb", "Composite, 2-line comb")
AUDIO_LABELS = ("Simulate", "Keep original", "Remove")
STANDARD_LABELS = ("NTSC", "PAL", "PAL-60")  # in the order of STANDARDS
CAMERA_TUBE_LABELS = ("None", "Plumbicon", "Plumbicon with light bias", "Plumbicon, anti-comet-tail gun")  # in the order of CAMERA_TUBES
IRIS_STEP = 0.5  # stops per click of the iris box
DEFAULT_IRIS_STOPS = 0.0  # normal exposure: the iris neither over- nor underexposes
IRIS_DECIMALS = 1
PREVIEW_AUDIO = "none"  # a preview is only looked at, so the sound is never rendered for it

SPLIT_DEFAULT = 0.5  # the divider starts in the middle
SPLIT_MIN = 0.02  # the divider stays inside the picture so it can always be grabbed again
SPLIT_MAX = 0.98
DIVIDER_COLOR = "#ffffff"
DIVIDER_WIDTH = 1

VIDEO_FILTER = "Video (*.mp4 *.mov *.mkv *.avi *.webm *.m4v *.ts);;All files (*)"
SAVE_FILTER = "MP4 (*.mp4);;Matroska (*.mkv);;QuickTime (*.mov)"
DEFAULT_SUFFIX = ".mp4"
LOOK_FILTER = "Look (*.toml)"
LOOK_SUFFIX = ".toml"
FRAME_FILTER = "PNG (*.png)"
FRAME_SUFFIX = ".png"
FRAME_FORMAT = "PNG"
FILTER_SUFFIXES = {"MP4 (*.mp4)": ".mp4", "Matroska (*.mkv)": ".mkv", "QuickTime (*.mov)": ".mov"}
SIZE_CHOICES: tuple[tuple[str, tuple[int, int] | None], ...] = (
    ("Same as input", None),
    ("1280 x 720", (1280, 720)),
    ("960 x 540", (960, 540)),
    ("854 x 480", (854, 480)),
    ("640 x 480", (640, 480)),
)

MODE_ORIGINAL = "original"
MODE_PROCESSED = "processed"
MODE_SPLIT = "split"
TASK_PREVIEW = "preview"
TASK_RENDER = "render"

# The schematic mirrors these editor lines and prints the same help text as their tooltips. A text has to
# fit the schematic's one-line hint band (about 140 characters), or the band cuts it short.
STANDARD_HELP = "The set's television standard: NTSC is 525 lines at 29.97 fps, PAL 625 at 25 fps, PAL-60 plays an NTSC tape on a PAL set."
CAMERA_TUBE_HELP = "The camera's pickup tube: a Plumbicon lags and smears bright lights into comet tails; the bias and gun variants cure it."
IRIS_HELP = "The camera's iris opening past normal, in stops (each doubles the light): wide, it overexposes and shows comet tails. Needs a tube."
LOOK_HELP = {
    "preset": "The starting condition of the tape and decks; the lines below start from its values.",
    "seed": "Seeds the random defects: the same seed always gives the same damage.",
    "generations": "How many times the tape is copied: each generation is a different deck and tape",
    "signal": "How the picture is separated again: S-Video keeps luma and chroma apart, notch and comb filters mix them the way a TV does.",
    "audio": "The sound track: simulated with the same wear, copied unchanged, or removed.",
    "standard": STANDARD_HELP,
    "camera_tube": CAMERA_TUBE_HELP,
    "iris_stops": IRIS_HELP,
}
SCHEMATIC_MIN_WIDTH = 1240
SCHEMATIC_MIN_HEIGHT = 880
SCHEMATIC_BOTTOM_MAX_HEIGHT = 250
# Page names are shared by the buttons and the command-line entry point.
SCHEMATIC_CIRCUIT = "circuit"
SCHEMATIC_OUTSIDE = "outside"
SCHEMATIC_PAGE_NAMES = (SCHEMATIC_CIRCUIT, SCHEMATIC_OUTSIDE)


def _clock(seconds: float) -> str:
    """`m:ss.s`, rounded to a tenth of a second."""
    minutes, tenths = divmod(round(max(0.0, seconds) * POSITION_STEPS_PER_S), TENTHS_PER_MINUTE)
    return f"{minutes}:{tenths / POSITION_STEPS_PER_S:04.1f}"


def _remaining(seconds: float) -> str:
    """`m:ss` for a time that is only an estimate."""
    minutes, rest = divmod(round(max(0.0, seconds)), SECONDS_PER_MINUTE)
    return f"{minutes}:{rest:02d}"


def _decimals(step: float) -> int:
    if step >= 1:
        return 0
    return 2 if step >= 0.01 else 3


def _tail(log: str) -> str:
    """The last few meaningful lines of a process's log."""
    lines = [line.strip() for line in log.splitlines() if line.strip() and backend.parse_progress(line) is None]
    return "\n".join(lines[-LOG_LINES:]) or "The process failed without a message."


def _describe(info: MediaInfo) -> str:
    parts = [f"{info.width} x {info.height}"]
    if info.fps > 0:
        parts.append(f"{info.fps:.2f} fps")
    if info.duration > 0:
        parts.append(_clock(info.duration))
    parts.append("audio" if info.has_audio else "no audio")
    return " | ".join(parts)


def _image_from(frames: Frames, index: int) -> QImage:
    raw = frames.frame(index)
    # The copy owns its pixels, so the bytes need not outlive this call.
    return QImage(raw, frames.width, frames.height, RGB_BYTES * frames.width, QImage.Format.Format_RGB888).copy()


def _suggest_output(source: Path) -> Path:
    """`<stem>_vhs.mp4` next to the input, numbered when that name is taken."""
    candidate = source.with_name(f"{source.stem}_vhs{DEFAULT_SUFFIX}")
    number = 2
    while candidate.exists():
        candidate = source.with_name(f"{source.stem}_vhs{number}{DEFAULT_SUFFIX}")
        number += 1
    return candidate


def _wrapping_label(name: str, text: str = "") -> QLabel:
    """A label that wraps and never asks for more width than the column has."""
    label = QLabel(text)
    label.setObjectName(name)
    label.setWordWrap(True)
    label.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Preferred)
    return label


class PreviewView(QWidget):
    """Paints one image, scaled to fit, on a dark background.

    In split mode it holds two frames and shows the first left of a divider and the second
    right of it; `image()` then returns that composition at the native frame size."""

    image_changed = Signal()

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._image: QImage | None = None
        self._pair: tuple[QImage, QImage] | None = None  # original and processed, same size
        self._split = SPLIT_DEFAULT
        self.setMinimumSize(*PREVIEW_MIN_SIZE)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)

    @property
    def split(self) -> float:
        return self._split

    def set_split(self, x: float) -> None:
        self._split = max(SPLIT_MIN, min(SPLIT_MAX, float(x)))
        if self._pair is not None:
            self._image = self._compose(*self._pair)
            self.image_changed.emit()
        self.update()

    def set_image(self, image: QImage | None) -> None:
        self._pair = None
        self._image = image
        self.image_changed.emit()
        self.update()

    def set_split_images(self, original: QImage, processed: QImage) -> None:
        self._pair = (original, processed)
        self._image = self._compose(original, processed)
        self.image_changed.emit()
        self.update()

    def image(self) -> QImage | None:
        return self._image

    def _compose(self, original: QImage, processed: QImage) -> QImage:
        composed = processed.convertToFormat(QImage.Format.Format_RGB32)
        left = QRect(0, 0, round(self._split * composed.width()), composed.height())
        painter = QPainter(composed)
        painter.drawImage(left, original, left)
        painter.end()
        return composed

    def _target(self) -> QRect | None:
        """Where the image is painted, or None without one."""
        image = self._image
        if image is None or image.isNull():
            return None
        fitted = image.size().scaled(self.size(), Qt.AspectRatioMode.KeepAspectRatio)
        target = QRect(0, 0, fitted.width(), fitted.height())
        target.moveCenter(self.rect().center())
        return target

    def _drag_divider(self, event: QMouseEvent) -> None:
        target = self._target()
        if self._pair is None or target is None or target.width() <= 0:
            return
        self.set_split((event.position().x() - target.left()) / target.width())

    def mousePressEvent(self, event: QMouseEvent) -> None:
        if event.button() == Qt.MouseButton.LeftButton:
            self._drag_divider(event)

    def mouseMoveEvent(self, event: QMouseEvent) -> None:
        if event.buttons() & Qt.MouseButton.LeftButton:
            self._drag_divider(event)

    def paintEvent(self, event: QPaintEvent) -> None:
        painter = QPainter(self)
        painter.fillRect(self.rect(), QColor(PREVIEW_BACKGROUND))
        target = self._target()
        if target is not None and self._image is not None:
            painter.setRenderHint(QPainter.RenderHint.SmoothPixmapTransform)
            painter.drawImage(target, self._image)
            if self._pair is not None:
                x = target.left() + round(self._split * target.width())
                painter.setPen(QPen(QColor(DIVIDER_COLOR), DIVIDER_WIDTH))
                painter.drawLine(x, target.top(), x, target.bottom())
        painter.end()


class DetachedPreviewWindow(QWidget):
    """Floating window hosting the video preview, allowing independent positioning across displays."""

    def __init__(self, main_window: MainWindow) -> None:
        super().__init__(None, Qt.WindowType.Window)
        self._main = main_window
        self.setObjectName("detachedPreviewWindow")
        self.setWindowTitle(f"{WINDOW_TITLE} — Preview")
        self.resize(*DETACHED_PREVIEW_SIZE)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(6, 6, 6, 6)
        layout.setSpacing(4)
        bar = QHBoxLayout()
        bar.addStretch(1)
        self.dock_button = QPushButton("Dock to main window")
        self.dock_button.setObjectName("dockPreviewButton")
        self.dock_button.clicked.connect(self._main._dock_preview)
        bar.addWidget(self.dock_button)
        layout.addLayout(bar)
        self._container = layout

    def set_preview_widget(self, widget: QWidget) -> None:
        self._container.insertWidget(0, widget, 1)

    def closeEvent(self, event: QCloseEvent) -> None:
        self._main._dock_preview()
        event.accept()


@dataclass(frozen=True)
class FlowSection:
    key: str
    number: str
    title: str
    domain: str
    summary: str
    parts: tuple[tuple[str, str], ...]
    equations: tuple[str, ...]
    parameters: tuple[tuple[str, str], ...]


FLOW_SECTIONS: tuple[FlowSection, ...] = (
    FlowSection(
        key="camera",
        number="01",
        title="SOURCE & CAMERA STAGE",
        domain="Photoelectric Target & Rasterization",
        summary="Converts scene irradiance through photoconductive pickup tubes into gamma-corrected baseband video signals.",
        parts=(
            ("Photoconductive Target", "Semiconductor target (PbO/Sb2S3) integrating photon charge per pixel element."),
            ("ACT Electron Gun", "Anti-comet-tail electron beam discharging overload highlights during flyback."),
            ("Light Bias LED", "Sub-threshold illumination flooding target to eliminate dark-state transit lag."),
            ("Gamma OETF & Matrix", "Applies transfer exponent and matrices RGB into Y, Cu, Cv baseband with sync."),
        ),
        equations=(
            "i_photo = I_0 * OETF^-1(V_in) * 2^(iris_stops)",
            "dq/dt = i_photo - i_beam(q)",
            "V_out = OETF((s_read - black) / peak_na)",
        ),
        parameters=(
            ("camera.tube", "Tube selection: plumbicon, plumbicon-bias, plumbicon-act, vidicon, saticon"),
            ("camera.iris_stops", "Optical exposure offset (stops; positive overexposes to highlight comet tails)"),
            ("camera.beam_ratio", "ACT beam reserve capacity multiplier above peak white (nominal: 2.0)"),
            ("standard", "Raster standard: NTSC (525/29.97), PAL (625/25), or PAL-60"),
        ),
    ),
    FlowSection(
        key="modulator",
        number="02",
        title="FM LUMA & COLOUR-UNDER",
        domain="Direct FM Synthesis & Heterodyne Chroma",
        summary="Synthesizes direct FM RF carrier for luminance and heterodyne downconverted subcarrier for chrominance.",
        parts=(
            ("Pre-Emphasis Network", "High-frequency boost (+8 dB main shelf, sub-emphasis at LP/EP) for FM noise immunity."),
            ("FM Video Modulator", "Astable multivibrator mapping active IRE level to instantaneous RF carrier frequency."),
            ("Colour-Under Mixer", "Heterodynes 3.58/4.43 MHz subcarrier down to 629.37 kHz with alternating +/-90 deg rotation."),
            ("Direct Duty Combiner", "Superimposes colour-under chroma as duty-cycle modulation of the saturated FM carrier."),
        ),
        equations=(
            "f_carrier(t) = f_sync + IRE(t) * (f_white - f_sync)",
            "f_chroma = 40 * f_line = 629.37 kHz (NTSC / PAL)",
            "s_rec(t) = sgn(sin(phi_FM(t)) + k_chroma * s_chroma(t))",
        ),
        parameters=(
            ("record.luma_carrier_mhz", "Nominal sync-tip/white carrier (3.4-4.4 MHz NTSC, 3.8-4.8 MHz PAL)"),
            ("record.sub_emphasis", "Sub-emphasis network for LP/EP tape speeds"),
            ("record.chroma_level", "Colour-under amplitude injection ratio"),
            ("speed", "Tape transport speed: SP, LP, EP (sets subcarrier interleave & track pitch)"),
        ),
    ),
    FlowSection(
        key="head_record",
        number="03",
        title="RECORDING HEAD & COERCIVITY",
        domain="Electromagnetic Saturation & Transition Relay",
        summary="Drives ferrite pole tips to magnetic saturation, recording transition reversals at coercive boundaries.",
        parts=(
            ("Ferrite Core Gap", "Concentrates recording flux into tape coating layer past magnetic saturation."),
            ("Coercive Relay Model", "Resolves sub-sample boundary points x_i where head fringe field equals coercivity H_c."),
            ("Self-Demagnetisation", "Opposing magnetic poles broaden transition width: a = (2 * M_r * d) / H_c."),
            ("Azimuth Angle Offset", "Helical head gaps angled at +6 deg and -6 deg to eliminate need for guard bands."),
        ),
        equations=(
            "H_head(x, y) >= H_c ==> M(x) = +M_s (or -M_s)",
            "a = (2 * M_r * d) / H_c (Transition width parameter)",
            "theta_azimuth = +/- 6.0 deg",
        ),
        parameters=(
            ("tape.coercivity_oe", "Tape magnetic coercivity (650-900 Oe)"),
            ("heads[i].width_um", "Physical recording head pole width"),
            ("speed.adjacent_track_offset_rf", "Subcarrier advance between adjacent helical passes"),
        ),
    ),
    FlowSection(
        key="tape_medium",
        number="04",
        title="2D HELICAL TAPE MEDIUM",
        domain="Spatial Helical Geometry & Defect Fields",
        summary="Represents the physical magnetic tape surface in 2D helical coordinates with oxide voids and mechanical creasing.",
        parts=(
            ("Helical Track Matrix", "2D surface coordinates mapped onto 3.1 deg (NTSC) / 3.3 deg (PAL) angled tracks."),
            ("Oxide Defect Field", "Spatial Poisson point-process generating oxide dropouts, coating pinholes, and flake-offs."),
            ("Tape Creases & Crinkles", "Mechanical edge crinkles and transport wrinkles generating periodic carrier loss."),
            ("Magnetic Spacing Profile", "Surface roughness and microscopic debris creating dynamic spacing fluctuations."),
        ),
        equations=(
            "P(k defects in area A) = ((lambda * A)^k * e^(-lambda * A)) / k!",
            "theta_track = 3.1 deg (NTSC) / 3.3 deg (PAL)",
            "Track length = half drum circumference = 97.4 mm",
        ),
        parameters=(
            ("tape.defects_per_mm2", "Oxide defect density triggering dropout compensation"),
            ("tape.crinkle_density_per_m", "Longitudinal creasing density per meter of tape"),
            ("tape.roughness_um", "Surface roughness inducing magnetic spacing noise"),
        ),
    ),
    FlowSection(
        key="readback",
        number="05",
        title="READBACK & KARLQVIST RECIPROCITY",
        domain="Lorentzian Pulse Summation & Spacing Loss",
        summary="Integrates playback flux using Karlqvist reciprocity, Lorentzian pulse summation, Wallace spacing, and azimuth rejection.",
        parts=(
            ("Karlqvist Reciprocity", "Integrates magnetic transition reversals via head fringe field potential."),
            ("Lorentzian Pulses", "Each isolated transition produces an impulse: e(t) = v * h_0 / (h_0^2 + (v*t - x)^2)."),
            ("Wallace Spacing Loss", "Exponential attenuation of short-wavelength RF: 54.6 * d / lambda dB."),
            ("Azimuth Crosstalk Law", "Sinc attenuation law cancels opposite-azimuth crosstalk from adjacent tracks."),
            ("Head Preamplifier", "Resonant LC tank (head coil + rotary transformer) with thermal Johnson-Nyquist noise."),
        ),
        equations=(
            "e(t) = sum_i (v * h_0) / (h_0^2 + (v*t - x_i)^2)",
            "Loss_spacing = 54.6 * (d / lambda) dB",
            "Atten_azimuth = |sinc(pi * W * tan(2*theta) / lambda)|",
        ),
        parameters=(
            ("heads[i].extra_spacing_um", "Dynamic head gap debris / clogging spacing loss"),
            ("heads[i].h0_um", "Nominal pulse half-width (gap + transition width)"),
            ("heads[i].cnr_db", "Head carrier-to-noise ratio in 6 MHz bandwidth"),
            ("tracking.offset_um", "Mean lateral head-to-track displacement (um)"),
            ("tracking.wander_um", "Capstan servo wandering / tracking jitter rms"),
        ),
    ),
    FlowSection(
        key="mechanics",
        number="06",
        title="TIMEBASE & MECHANICAL TRANSPORT",
        domain="Capstan Servo, Flutter & Drum Phase",
        summary="Applies multi-band mechanical wow, flutter, drum rotational jitter, tape tension skew, and head switching.",
        parts=(
            ("Drum & Capstan Servos", "Multi-band colored noise (0.1-100 Hz) inducing timebase perturbation e(t)."),
            ("Head Switching Pulse", "Switches active playback head 6.5 lines ahead of vertical sync, producing bottom glitch."),
            ("Tape Tension & Skew", "Back-tension mismatch warping line duration at top of frame (flagging)."),
            ("Capstan Slip & Wander", "Low-frequency speed drift causing slow vertical roll and tracking wander."),
        ),
        equations=(
            "t_playback = t_nominal + e(t)",
            "e(t) = sum_k A_k * sin(2*pi*f_k*t + phi_k) + Wander(t)",
            "Switch time = 6.5 H before vertical blanking",
        ),
        parameters=(
            ("mech.flutter_rms", "Wow and flutter root-mean-square amplitude"),
            ("mech.tension_offset", "Longitudinal tape tension offset (causes top flagging)"),
            ("mech.head_switch_delay", "Drum head switch trigger position"),
        ),
    ),
    FlowSection(
        key="deck_rf",
        number="07",
        title="DECK RF ELECTRONICS & DEMOD",
        domain="RF Equalisation, DOC & Limiter Discriminator",
        summary="Reconstructs luminance via RF peaking biquads, 1H dropout recirculation, limiter, and delay-line discriminator.",
        parts=(
            ("RF Peaking Equaliser", "Equalises high-frequency carrier loss with resonant biquad filters."),
            ("1H Dropout Compensator", "Detects carrier amplitude loss below threshold and recirculates previous line."),
            ("Saturating Limiter", "Strips AM noise from the FM carrier before demodulation."),
            ("FM Delay-Line Discriminator", "Phase-multiplies carrier with 90-degree delayed copy to linearize frequency-to-IRE."),
            ("Main De-Emphasis", "Restores original flat baseband spectrum, inverting +8 dB high-pass pre-emphasis."),
        ),
        equations=(
            "V_demod(t) = k * (d phi / dt) = 2*pi * f(t)",
            "DOC(t) = s_RF(t) if A_carrier >= V_thresh else s_RF(t - 1H)",
        ),
        parameters=(
            ("deck.fm_gain", "RF preamplifier gain into limiter stage"),
            ("deck.doc_threshold", "Dropout compensator trigger sensitivity threshold"),
            ("deck.eq_peaks", "Equaliser biquad peaking frequencies, Q, and gain"),
            ("deck.limiter_drive", "Limiter clipping drive level"),
        ),
    ),
    FlowSection(
        key="display",
        number="08",
        title="CHROMA DEMOD & CRT DISPLAY",
        domain="2H Comb Decoder, Sync AFC & Phosphor Decay",
        summary="Separates Y/C, restores chroma subcarrier, extracts sync with AFC flywheel, and emulates CRT phosphor decay.",
        parts=(
            ("Colour-Under Demodulator", "Heterodynes 629 kHz back to 3.58/4.43 MHz and reverses +/-90 deg line phase rotation."),
            ("1H/2H Comb & Notch Decoder", "Cancels adjacent-track chroma crosstalk and separates Y/C."),
            ("Sync Slicer & AFC Flywheel", "Slices sync tips and flywheel-stabilizes horizontal deflection."),
            ("CRT Shadow Mask & Phosphor", "Emulates RGB phosphor triads and multi-exponential phosphorescence decay across fields."),
        ),
        equations=(
            "I_phosphor(t) = I_0 * sum_j (c_j * e^(-t / tau_j))",
            "Comb_PAL(t) = 0.5 * (s(t) + s(t - 2H))",
            "Comb_NTSC(t) = 0.5 * (s(t) - s(t - 1H))",
        ),
        parameters=(
            ("composite.decoder", "Chroma separation filter: 'notch', 'comb1h', 'comb2h'"),
            ("decode.sync_slice_level", "Sync slicer voltage threshold"),
            ("CRT Phosphor Decay", "P22 red (0.4 ms), green (1.5 ms), and blue (0.1 ms) decay lag"),
        ),
    ),
)


def _draw_flow_arrow(painter: QPainter, p1: QPointF, p2: QPointF, color: QColor) -> None:
    painter.setPen(QPen(color, 1.8))
    painter.drawLine(p1, p2)
    dx = p2.x() - p1.x()
    dy = p2.y() - p1.y()
    length = math.hypot(dx, dy)
    if length > 0.001:
        ux = dx / length
        uy = dy / length
        hl = 7.0
        hw = 4.0
        px = -uy
        py = ux
        p_left = QPointF(p2.x() - hl * ux + hw * px, p2.y() - hl * uy + hw * py)
        p_right = QPointF(p2.x() - hl * ux - hw * px, p2.y() - hl * uy - hw * py)
        painter.setBrush(QBrush(color))
        painter.drawPolygon(QPolygonF([p2, p_left, p_right]))


class PhosphorFlowView(QWidget):
    """Interactive phosphor green flow canvas displaying the first-principles simulation pipeline.

    Clicking any stage zooms into that section to reveal internal physical mechanisms,
    governing equations, and exact simulation parameters.
    """

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("phosphorFlowView")
        self.setMouseTracking(True)
        self.setFocusPolicy(Qt.FocusPolicy.StrongFocus)
        self._selected_idx: int | None = None
        self._hovered_idx: int | None = None
        self._block_rects: list[QRectF] = [QRectF() for _ in range(len(FLOW_SECTIONS))]
        self._back_rect = QRectF()

    def zoom_to_section(self, index: int | None) -> None:
        self._selected_idx = index
        self.update()

    def mousePressEvent(self, event: QMouseEvent) -> None:
        pos = event.position()
        if self._selected_idx is None:
            for idx, r in enumerate(self._block_rects):
                if r.contains(pos):
                    self._selected_idx = idx
                    self.update()
                    event.accept()
                    return
        else:
            if self._back_rect.contains(pos):
                self._selected_idx = None
                self.update()
                event.accept()
                return
        super().mousePressEvent(event)

    def mouseMoveEvent(self, event: QMouseEvent) -> None:
        pos = event.position()
        if self._selected_idx is None:
            hovered = None
            for idx, r in enumerate(self._block_rects):
                if r.contains(pos):
                    hovered = idx
                    break
            if hovered != self._hovered_idx:
                self._hovered_idx = hovered
                self.setCursor(Qt.CursorShape.PointingHandCursor if hovered is not None else Qt.CursorShape.ArrowCursor)
                self.update()
        else:
            if self._back_rect.contains(pos):
                self.setCursor(Qt.CursorShape.PointingHandCursor)
            else:
                self.setCursor(Qt.CursorShape.ArrowCursor)
        super().mouseMoveEvent(event)

    def keyPressEvent(self, event: QKeyEvent) -> None:
        if event.key() == Qt.Key.Key_Escape and self._selected_idx is not None:
            self._selected_idx = None
            self.update()
            event.accept()
            return
        super().keyPressEvent(event)

    def paintEvent(self, event: QPaintEvent) -> None:
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        w, h = float(self.width()), float(self.height())

        # Background & CRT scanline grid
        painter.fillRect(QRectF(0, 0, w, h), QColor(PHOSPHOR_BG))
        painter.setPen(QPen(QColor(10, 42, 18, 60), 1.0))
        for gy in range(0, int(h), 22):
            painter.drawLine(0, gy, int(w), gy)
        for gx in range(0, int(w), 32):
            painter.drawLine(gx, 0, gx, int(h))

        if self._selected_idx is None:
            self._paint_overview(painter, w, h)
        else:
            self._paint_zoomed(painter, w, h, self._selected_idx)
        painter.end()

    def _paint_overview(self, painter: QPainter, w: float, h: float) -> None:
        font_mono = QFontDatabase.systemFont(QFontDatabase.SystemFont.FixedFont)
        font_mono.setPointSize(9)
        painter.setFont(font_mono)

        # Header title
        painter.setPen(QPen(QColor(PHOSPHOR_HOT)))
        painter.drawText(
            QRectF(20, 14, w - 40, 22),
            Qt.AlignmentFlag.AlignCenter,
            "--- VHS-PHYS PHYSICAL SIMULATION PIPELINE (CLICK ANY STAGE TO ZOOM IN) ---",
        )

        margin_x = 22.0
        margin_y = 44.0
        gap_x = 18.0
        gap_y = 38.0
        cols = 4
        rows = 2
        block_w = (w - 2 * margin_x - (cols - 1) * gap_x) / cols
        block_h = (h - margin_y - 48.0 - gap_y) / rows

        # Assign grid cells:
        # Row 0 (L -> R): stages 0, 1, 2, 3
        # Row 1 (R -> L): stages 4, 5, 6, 7
        grid_pos = [
            (0, 0),  # 0
            (1, 0),  # 1
            (2, 0),  # 2
            (3, 0),  # 3
            (3, 1),  # 4
            (2, 1),  # 5
            (1, 1),  # 6
            (0, 1),  # 7
        ]

        for idx, (c, r) in enumerate(grid_pos):
            bx = margin_x + c * (block_w + gap_x)
            by = margin_y + r * (block_h + gap_y)
            self._block_rects[idx] = QRectF(bx, by, block_w, block_h)

        # Connecting arrows between consecutive stages
        arrow_color = QColor(PHOSPHOR_DIM)
        for i in range(len(FLOW_SECTIONS) - 1):
            r1 = self._block_rects[i]
            r2 = self._block_rects[i + 1]
            if i in (0, 1, 2):
                p1 = QPointF(r1.right() + 2, r1.center().y())
                p2 = QPointF(r2.left() - 2, r2.center().y())
            elif i == 3:
                p1 = QPointF(r1.center().x(), r1.bottom() + 2)
                p2 = QPointF(r2.center().x(), r2.top() - 2)
            else:
                p1 = QPointF(r1.left() - 2, r1.center().y())
                p2 = QPointF(r2.right() + 2, r2.center().y())
            _draw_flow_arrow(painter, p1, p2, arrow_color)

        # Draw stage blocks
        for idx, s in enumerate(FLOW_SECTIONS):
            r = self._block_rects[idx]
            is_hovered = self._hovered_idx == idx

            bg_col = QColor(14, 56, 26, 210) if is_hovered else QColor(5, 20, 10, 180)
            border_col = QColor(PHOSPHOR_NORM if is_hovered else PHOSPHOR_DIM)
            border_width = 2.0 if is_hovered else 1.2

            painter.setBrush(QBrush(bg_col))
            painter.setPen(QPen(border_col, border_width))
            painter.drawRoundedRect(r, 6.0, 6.0)

            # Badge
            badge_rect = QRectF(r.left() + 8, r.top() + 8, 38, 18)
            painter.setBrush(QBrush(QColor(PHOSPHOR_FAINT)))
            painter.setPen(QPen(QColor(PHOSPHOR_DIM), 1.0))
            painter.drawRoundedRect(badge_rect, 3.0, 3.0)
            painter.setPen(QPen(QColor(PHOSPHOR_HOT)))
            painter.drawText(badge_rect, Qt.AlignmentFlag.AlignCenter, s.number)

            # Title
            painter.setPen(QPen(QColor(PHOSPHOR_NORM if not is_hovered else PHOSPHOR_HOT)))
            f_title = QFont(font_mono)
            f_title.setBold(True)
            painter.setFont(f_title)
            title_rect = QRectF(r.left() + 52, r.top() + 8, r.width() - 58, 20)
            painter.drawText(title_rect, Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter, s.title)

            # Domain
            f_domain = QFont(font_mono)
            f_domain.setPointSize(8)
            painter.setFont(f_domain)
            painter.setPen(QPen(QColor(PHOSPHOR_DIM if not is_hovered else PHOSPHOR_NORM)))
            domain_rect = QRectF(r.left() + 8, r.top() + 30, r.width() - 16, 16)
            painter.drawText(domain_rect, Qt.AlignmentFlag.AlignLeft, s.domain)

            # Summary snippet
            painter.setPen(QPen(QColor("#a2e6b2")))
            summary_rect = QRectF(r.left() + 8, r.top() + 50, r.width() - 16, r.height() - 74)
            painter.drawText(summary_rect, Qt.AlignmentFlag.AlignLeft | Qt.TextFlag.TextWordWrap, s.summary)

            # Click hint
            painter.setPen(QPen(QColor(PHOSPHOR_HOT if is_hovered else PHOSPHOR_FAINT)))
            hint_rect = QRectF(r.left() + 8, r.bottom() - 20, r.width() - 16, 16)
            painter.drawText(hint_rect, Qt.AlignmentFlag.AlignRight, "[CLICK TO INSPECT]" if is_hovered else "[INSPECT]")

        # Footer
        painter.setFont(font_mono)
        painter.setPen(QPen(QColor(PHOSPHOR_DIM)))
        painter.drawText(
            QRectF(20, h - 30, w - 40, 20),
            Qt.AlignmentFlag.AlignCenter,
            "FIRST-PRINCIPLES PHYSICAL SIMULATION • CONTINUOUS EMERGENCE • NO HEURISTIC FILTERS",
        )

    def _paint_zoomed(self, painter: QPainter, w: float, h: float, idx: int) -> None:
        s = FLOW_SECTIONS[idx]
        font_mono = QFontDatabase.systemFont(QFontDatabase.SystemFont.FixedFont)
        font_mono.setPointSize(9)
        painter.setFont(font_mono)

        # Back button
        self._back_rect = QRectF(20, 14, 210, 28)
        painter.setBrush(QBrush(QColor(14, 52, 24)))
        painter.setPen(QPen(QColor(PHOSPHOR_NORM), 1.5))
        painter.drawRoundedRect(self._back_rect, 4.0, 4.0)
        painter.setPen(QPen(QColor(PHOSPHOR_HOT)))
        painter.drawText(self._back_rect, Qt.AlignmentFlag.AlignCenter, "◄ BACK TO OVERVIEW (ESC)")

        # Breadcrumbs
        f_crumb = QFont(font_mono)
        f_crumb.setBold(True)
        painter.setFont(f_crumb)
        painter.setPen(QPen(QColor(PHOSPHOR_HOT)))
        painter.drawText(
            QRectF(244, 18, w - 264, 20),
            Qt.AlignmentFlag.AlignLeft,
            f"OVERVIEW  >>  STAGE {s.number}: {s.title}",
        )

        # Main columns
        top_y = 50.0
        bottom_y = h - 32.0
        col_h = bottom_y - top_y
        gap = 16.0
        col_w1 = (w - 40.0 - gap) * 0.54
        col_w2 = (w - 40.0 - gap) * 0.46
        x1 = 20.0
        x2 = x1 + col_w1 + gap

        # Left Column: Circuit Architecture & Components
        r_left = QRectF(x1, top_y, col_w1, col_h)
        painter.setBrush(QBrush(QColor(4, 18, 8, 200)))
        painter.setPen(QPen(QColor(PHOSPHOR_DIM), 1.2))
        painter.drawRoundedRect(r_left, 6.0, 6.0)

        cy = top_y + 12.0
        painter.setPen(QPen(QColor(PHOSPHOR_NORM)))
        painter.drawText(QRectF(x1 + 14, cy, col_w1 - 28, 18), Qt.AlignmentFlag.AlignLeft, f"DOMAIN: {s.domain}")
        cy += 20.0

        painter.setPen(QPen(QColor("#c8ffd8")))
        painter.drawText(QRectF(x1 + 14, cy, col_w1 - 28, 36), Qt.AlignmentFlag.AlignLeft | Qt.TextFlag.TextWordWrap, s.summary)
        cy += 42.0

        painter.setPen(QPen(QColor(PHOSPHOR_HOT)))
        painter.drawText(QRectF(x1 + 14, cy, col_w1 - 28, 16), Qt.AlignmentFlag.AlignLeft, "INTERNAL STAGE COMPONENTS:")
        cy += 20.0

        f_small = QFont(font_mono)
        f_small.setPointSize(8)
        for part_name, part_desc in s.parts:
            painter.setFont(font_mono)
            painter.setPen(QPen(QColor(PHOSPHOR_NORM)))
            painter.drawText(QRectF(x1 + 18, cy, col_w1 - 36, 16), Qt.AlignmentFlag.AlignLeft, f"► {part_name}")
            cy += 16.0
            painter.setFont(f_small)
            painter.setPen(QPen(QColor("#a8e2b8")))
            painter.drawText(QRectF(x1 + 28, cy, col_w1 - 46, 30), Qt.AlignmentFlag.AlignLeft | Qt.TextFlag.TextWordWrap, part_desc)
            cy += 32.0

        # Governing Equations Box
        eq_box = QRectF(x1 + 14, r_left.bottom() - 110, col_w1 - 28, 96)
        painter.setBrush(QBrush(QColor(2, 10, 4, 230)))
        painter.setPen(QPen(QColor(PHOSPHOR_FAINT), 1.0))
        painter.drawRoundedRect(eq_box, 4.0, 4.0)

        painter.setFont(f_small)
        painter.setPen(QPen(QColor(PHOSPHOR_HOT)))
        painter.drawText(QRectF(eq_box.left() + 8, eq_box.top() + 6, eq_box.width() - 16, 16), Qt.AlignmentFlag.AlignLeft, "GOVERNING EQUATIONS:")
        eq_y = eq_box.top() + 24.0
        painter.setPen(QPen(QColor(PHOSPHOR_NORM)))
        for eq in s.equations:
            painter.drawText(QRectF(eq_box.left() + 12, eq_y, eq_box.width() - 20, 16), Qt.AlignmentFlag.AlignLeft, f"● {eq}")
            eq_y += 18.0

        # Right Column: Controlling Simulation Parameters
        r_right = QRectF(x2, top_y, col_w2, col_h)
        painter.setBrush(QBrush(QColor(4, 18, 8, 200)))
        painter.setPen(QPen(QColor(PHOSPHOR_DIM), 1.2))
        painter.drawRoundedRect(r_right, 6.0, 6.0)

        py = top_y + 12.0
        painter.setFont(font_mono)
        painter.setPen(QPen(QColor(PHOSPHOR_HOT)))
        painter.drawText(QRectF(x2 + 14, py, col_w2 - 28, 18), Qt.AlignmentFlag.AlignLeft, "SIMULATION PARAMETERS & HOOKS:")
        py += 22.0

        painter.setFont(f_small)
        painter.setPen(QPen(QColor(PHOSPHOR_DIM)))
        painter.drawText(QRectF(x2 + 14, py, col_w2 - 28, 16), Qt.AlignmentFlag.AlignLeft, "Configuration keys mapped to this stage:")
        py += 22.0

        for param_key, param_info in s.parameters:
            badge_r = QRectF(x2 + 14, py, col_w2 - 28, 22)
            painter.setBrush(QBrush(QColor(8, 36, 16)))
            painter.setPen(QPen(QColor(PHOSPHOR_FAINT), 1.0))
            painter.drawRoundedRect(badge_r, 4.0, 4.0)

            painter.setFont(font_mono)
            painter.setPen(QPen(QColor(PHOSPHOR_HOT)))
            painter.drawText(QRectF(badge_r.left() + 8, badge_r.top() + 3, badge_r.width() - 16, 16), Qt.AlignmentFlag.AlignLeft, f"● {param_key}")
            py += 26.0

            painter.setFont(f_small)
            painter.setPen(QPen(QColor("#a8e8b8")))
            painter.drawText(QRectF(x2 + 22, py, col_w2 - 36, 32), Qt.AlignmentFlag.AlignLeft | Qt.TextFlag.TextWordWrap, param_info)
            py += 36.0

        # Footer
        painter.setFont(font_mono)
        painter.setPen(QPen(QColor(PHOSPHOR_DIM)))
        painter.drawText(
            QRectF(20, h - 26, w - 40, 20),
            Qt.AlignmentFlag.AlignCenter,
            f"STAGE {s.number} OF 08 • PRESS ESC OR CLICK '◄ BACK' TO RETURN TO PIPELINE OVERVIEW",
        )


class AboutDialog(QDialog):
    """Information dialog presenting the physical simulation details, links, and support options."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("aboutDialog")
        self.setWindowTitle(ABOUT_TITLE)
        self.resize(*ABOUT_DIALOG_SIZE)

        layout = QVBoxLayout(self)
        layout.setContentsMargins(10, 10, 10, 10)
        layout.setSpacing(8)

        tabs = QTabWidget(self)
        tabs.setObjectName("aboutTabs")
        tabs.setStyleSheet(
            "QTabWidget::pane { border: 1px solid #1c9a45; background: #010804; } "
            "QTabBar::tab { background: #081a0e; color: #1c9a45; padding: 7px 18px; font-weight: bold; border: 1px solid #0e4a22; margin-right: 2px; } "
            "QTabBar::tab:selected { background: #0e4a22; color: #39ff6e; border-bottom: 2px solid #39ff6e; }"
        )

        # Tab 1: Interactive Phosphor Flow Canvas
        self.flow_view = PhosphorFlowView()
        tabs.addTab(self.flow_view, "Physical Simulation Pipeline")

        # Tab 2: Overview & Project Links
        info_tab = QWidget()
        info_tab.setObjectName("infoTab")
        info_layout = QVBoxLayout(info_tab)
        info_layout.setContentsMargins(24, 20, 24, 20)
        info_layout.setSpacing(12)

        header = QLabel("<h2>vhs-phys</h2>")
        header.setObjectName("aboutHeader")
        header.setStyleSheet("color: #39ff6e;")
        info_layout.addWidget(header)

        subtitle = QLabel("<b>A first-principles physical model of VHS recording, tape transport, and RF playback</b>")
        subtitle.setWordWrap(True)
        subtitle.setStyleSheet("color: #d8ffe3;")
        info_layout.addWidget(subtitle)

        body_text = (
            "<p style='color: #c0f4d0;'>A complete physical simulation of the helical-scan tape chain:</p>"
            "<ul style='color: #a0e0b0; line-height: 1.5;'>"
            "<li><b>RF recording:</b> Direct FM luminance synthesis, colour-under heterodyne downconversion, and sub-sample magnetic transition timing.</li>"
            "<li><b>Tape &amp; head physics:</b> Spacing loss (Wallace law), self-demagnetisation, head pole-tip wear, azimuth crosstalk, and oxide dropouts.</li>"
            "<li><b>Camera optics:</b> Vidicon, Plumbicon, Saticon, and Chalnicon pickup tube lag, target saturation, and highlight comet tails.</li>"
            "<li><b>Standards:</b> Full raster processing for NTSC (525/29.97), PAL (625/25), and PAL-60.</li>"
            "<li><b>Display:</b> CRT beam scanning, shadow mask, and phosphor persistence emulation.</li>"
            "</ul>"
        )
        body = QLabel(body_text)
        body.setWordWrap(True)
        info_layout.addWidget(body)

        info_layout.addStretch(1)

        links_box = QHBoxLayout()
        kofi_btn = QPushButton("Support on Ko-fi")
        kofi_btn.setObjectName("kofiButton")
        kofi_btn.clicked.connect(lambda: QDesktopServices.openUrl(QUrl(KOFI_URL)))
        links_box.addWidget(kofi_btn)

        github_btn = QPushButton("GitHub Repository")
        github_btn.setObjectName("githubButton")
        github_btn.clicked.connect(lambda: QDesktopServices.openUrl(QUrl(GITHUB_URL)))
        links_box.addWidget(github_btn)

        links_box.addStretch(1)
        info_layout.addLayout(links_box)

        tabs.addTab(info_tab, "Project Overview & Links")

        layout.addWidget(tabs, 1)

        bottom_bar = QHBoxLayout()
        bottom_bar.addStretch(1)
        close_btn = QPushButton("Close")
        close_btn.setObjectName("closeAboutButton")
        close_btn.clicked.connect(self.accept)
        bottom_bar.addWidget(close_btn)
        layout.addLayout(bottom_bar)


@dataclass
class _SourceRequest:
    """The fetch of one source frame; an answer is used only while it is still the current one."""

    job: backend.Job
    size: tuple[int, int]


class MainWindow(QMainWindow):
    def __init__(
        self,
        binary: Path | str,
        workdir: Path | str | None = None,
        ffmpeg: str = "ffmpeg",
        ffprobe: str = "ffprobe",
    ) -> None:
        super().__init__()
        self._binary = binary
        self._ffmpeg = ffmpeg
        self._ffprobe = ffprobe
        self._owns_workdir = workdir is None
        if workdir is None:
            self._workdir = Path(tempfile.mkdtemp(prefix="vhsgui-"))
        else:
            self._workdir = Path(workdir)
            self._workdir.mkdir(parents=True, exist_ok=True)

        self._info: MediaInfo | None = None
        self._view_mode = MODE_ORIGINAL
        self._stale = False
        self._loading = False  # true while the editors are being filled from a preset
        self._advanced_error: str | None = None
        self._closed = False
        self._schematic_mode = False
        self._schematic_page = SCHEMATIC_CIRCUIT
        self._schematic_booted: set[str] = set()
        self._sync_guard = False
        self._schematic_baseline: dict[str, object] = {}
        self.original_frames: Frames | None = None
        self.processed_frames: Frames | None = None
        self._frame_index = 0
        self._frames_standard = DEFAULT_STANDARD  # the standard the shown frames were rendered at
        self._playing = False
        self._source: _SourceRequest | None = None

        self._task: str | None = None
        self._cancel_requested = False
        self._render_job: backend.Job | None = None  # the preview render, or the final render
        self._orig_job: backend.Job | None = None
        self._decode_job: backend.Job | None = None
        self._plan: PreviewPlan | None = None
        self._preview_look: Look | None = None
        self._partial: Path | None = None
        self._render_out: Path | None = None
        self._render_expected = 0
        self._rendered_path: Path | None = None

        self._spins: dict[str, QDoubleSpinBox] = {}
        self._checks: dict[str, QCheckBox] = {}
        self._choice_boxes: dict[str, QComboBox] = {}
        self._field_labels: list[QLabel] = []
        self._input_group: QGroupBox
        self._output_group: QGroupBox
        self._playback_row: QWidget
        self._position_row: QWidget
        self._preview_row: QWidget
        self._is_preview_detached = False
        self._detached_window: DetachedPreviewWindow | None = None
        self._preview_column: QWidget
        self._preview_layout: QVBoxLayout
        self._preview_placeholder: QWidget

        self.setWindowTitle(WINDOW_TITLE)
        self.resize(*WINDOW_SIZE)
        self.setMinimumSize(*WINDOW_MIN_SIZE)
        self._build_ui()
        self._fill_presets()

        self._poll_timer = QTimer(self)
        self._poll_timer.setInterval(POLL_MS)
        self._poll_timer.timeout.connect(self._poll)
        self._poll_timer.start()
        self._source_timer = QTimer(self)
        self._source_timer.setSingleShot(True)
        self._source_timer.setInterval(DEBOUNCE_MS)
        self._source_timer.timeout.connect(self._request_source)
        self._play_timer = QTimer(self)  # its interval is set where playback starts
        self._play_timer.timeout.connect(self._advance_frame)

        space = QShortcut(QKeySequence(Qt.Key.Key_Space), self._panels_area)
        space.setContext(Qt.ShortcutContext.WidgetWithChildrenShortcut)
        space.activated.connect(self._toggle_play)
        QShortcut(QKeySequence(Qt.Key.Key_O), self).activated.connect(self._toggle_view)
        self._update_controls()

    # ------------------------------------------------------------ public surface

    @property
    def busy(self) -> bool:
        """A preview or a render is running (fetching a source frame does not count)."""
        return self._task is not None

    @property
    def view_mode(self) -> str:
        return self._view_mode

    @property
    def preview_stale(self) -> bool:
        return self._stale

    @property
    def media(self) -> MediaInfo | None:
        return self._info

    @property
    def source_pending(self) -> bool:
        """A source frame has been asked for and has not arrived yet."""
        return self._source is not None or self._source_timer.isActive()

    @property
    def schematic_mode(self) -> bool:
        return self._schematic_mode

    @property
    def schematic_page(self) -> str:
        return self._schematic_page

    def set_schematic_page(self, name: str) -> None:
        if name not in SCHEMATIC_PAGE_NAMES:
            raise ValueError(f"unknown schematic page: {name}")
        self._schematic_page = name
        view = self._schematic_pages[name]
        for page_name, page_view in self._schematic_pages.items():
            page_view.setVisible(self._schematic_mode and page_name == name)
        self.circuit_button.setChecked(name == SCHEMATIC_CIRCUIT)
        self.outside_button.setChecked(name == SCHEMATIC_OUTSIDE)
        if self._schematic_mode:
            if name not in self._schematic_booted:
                view.start_boot()
                self._schematic_booted.add(name)
            view.setFocus(Qt.FocusReason.OtherFocusReason)

    def set_schematic_mode(self, enabled: bool) -> None:
        enabled = bool(enabled)
        if enabled == self._schematic_mode:
            return
        if enabled:
            self._grow_for_schematic()
        self._schematic_mode = enabled
        self.panels_button.setChecked(not enabled)
        self.schematic_button.setChecked(enabled)
        self.circuit_button.setVisible(enabled)
        self.outside_button.setVisible(enabled)
        self._move_schematic_widgets(enabled)
        self._panels_area.setVisible(not enabled)
        self._schematic_bottom.setVisible(enabled)
        if enabled:
            self.set_schematic_page(self._schematic_page)
        else:
            for view in self._schematic_views:
                view.setVisible(False)

    def look(self) -> Look:
        return Look(
            preset=self.preset_combo.currentText(),
            seed=self.seed_spin.value(),
            condition=self._condition(),
            advanced=self.advanced_edit.toPlainText(),
            generations=self.generations_spin.value(),
            signal=self.signal_combo.currentData(),
            audio=self.audio_combo.currentData(),
            standard=self.standard_combo.currentData(),
            camera_tube=self.camera_tube_combo.currentData(),
            iris_stops=self.iris_spin.value(),
        )

    def open_input(self, path: str) -> None:
        self._open(path, interactive=False)

    def set_output(self, path: str) -> None:
        self.output_edit.setText(path)

    def confirm_overwrite(self, path: str) -> bool:
        answer = QMessageBox.question(
            self,
            WINDOW_TITLE,
            f"{path} already exists. Replace it?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No,
        )
        return answer == QMessageBox.StandardButton.Yes

    # ------------------------------------------------------------ building the window

    def _build_ui(self) -> None:
        self._input_group = self._build_input_group()
        look_group = self._build_look_group()
        condition_groups = self._build_condition_groups()
        advanced_group = self._build_advanced_group()
        self._output_group = self._build_output_group()

        controls = QWidget()
        column = QVBoxLayout(controls)
        column.addWidget(self._input_group)
        column.addWidget(look_group)
        for group in condition_groups:
            column.addWidget(group)
        column.addWidget(advanced_group)
        column.addStretch(1)

        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        scroll.setWidget(controls)

        left = QWidget()
        left.setMinimumWidth(LEFT_MIN_WIDTH)
        left_layout = QVBoxLayout(left)
        left_layout.setContentsMargins(0, 0, 0, 0)
        left_layout.setSpacing(6)
        left_layout.addWidget(scroll, 1)
        left_layout.addWidget(self._output_group, 0)

        self._panels_area = QSplitter(Qt.Orientation.Horizontal)
        self._panels_area.setChildrenCollapsible(False)
        self._panels_area.addWidget(left)
        self._panels_area.addWidget(self._build_preview_column())
        self._panels_area.setStretchFactor(0, 0)
        self._panels_area.setStretchFactor(1, 1)
        self._panels_area.setSizes([LEFT_WIDTH, WINDOW_SIZE[0] - LEFT_WIDTH])

        self._build_mode_bar()
        self.schematic = SchematicView()
        self.schematic.setObjectName("schematicView")
        self.schematic.setVisible(False)
        outside_dir = Path(__file__).resolve().parent / "schematic" / "outside"
        self.outside = SchematicView(data_dir=outside_dir)
        self.outside.setObjectName("outsideView")
        self.outside.setVisible(False)
        self._schematic_views = [self.schematic, self.outside]
        self._schematic_pages = {
            SCHEMATIC_CIRCUIT: self.schematic,
            SCHEMATIC_OUTSIDE: self.outside,
        }
        for view in self._schematic_views:
            view.valueChanged.connect(self._on_schematic_value_changed)
            view.resetRequested.connect(self._reset_schematic_value)
        self.preview.image_changed.connect(self._sync_screen_image)
        self._sync_screen_image()

        self._build_schematic_bottom()
        self._schematic_bottom.setVisible(False)
        root = QWidget()
        root_layout = QVBoxLayout(root)
        root_layout.setContentsMargins(6, 6, 6, 6)
        root_layout.setSpacing(4)
        root_layout.addWidget(self._mode_bar)
        root_layout.addWidget(self._panels_area, 1)
        root_layout.addWidget(self.schematic, 1)
        root_layout.addWidget(self.outside, 1)
        root_layout.addWidget(self._schematic_bottom, 0)
        self.setCentralWidget(root)

    def _build_mode_bar(self) -> None:
        self._mode_bar = QWidget()
        layout = QHBoxLayout(self._mode_bar)
        layout.setContentsMargins(0, 0, 0, 0)
        self.about_button = QPushButton("About")
        self.about_button.setObjectName("aboutButton")
        self.about_button.clicked.connect(self.show_about_dialog)
        layout.addWidget(self.about_button)
        layout.addStretch(1)
        self.panels_button = QPushButton("Panels")
        self.panels_button.setObjectName("panelsButton")
        self.panels_button.setCheckable(True)
        self.schematic_button = QPushButton("Schematic")
        self.schematic_button.setObjectName("schematicButton")
        self.schematic_button.setCheckable(True)
        modes = QButtonGroup(self)
        modes.setExclusive(True)
        modes.addButton(self.panels_button)
        modes.addButton(self.schematic_button)
        self.panels_button.setChecked(True)
        self.panels_button.clicked.connect(lambda: self.set_schematic_mode(False))
        self.schematic_button.clicked.connect(lambda: self.set_schematic_mode(True))
        layout.addWidget(self.panels_button)
        layout.addWidget(self.schematic_button)
        self.circuit_button = QPushButton("Circuit")
        self.circuit_button.setObjectName("circuitButton")
        self.circuit_button.setCheckable(True)
        self.outside_button = QPushButton("Outside")
        self.outside_button.setObjectName("outsideButton")
        self.outside_button.setCheckable(True)
        pages = QButtonGroup(self)
        pages.setExclusive(True)
        pages.addButton(self.circuit_button)
        pages.addButton(self.outside_button)
        self.circuit_button.setChecked(True)
        self.circuit_button.clicked.connect(lambda: self.set_schematic_page(SCHEMATIC_CIRCUIT))
        self.outside_button.clicked.connect(lambda: self.set_schematic_page(SCHEMATIC_OUTSIDE))
        self.circuit_button.setVisible(False)
        self.outside_button.setVisible(False)
        layout.addWidget(self.circuit_button)
        layout.addWidget(self.outside_button)

    def _build_schematic_bottom(self) -> None:
        self._schematic_bottom = QWidget()
        self._schematic_bottom.setMaximumHeight(SCHEMATIC_BOTTOM_MAX_HEIGHT)
        layout = QGridLayout(self._schematic_bottom)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(4)
        activity = QWidget()
        activity_layout = QVBoxLayout(activity)
        activity_layout.setContentsMargins(0, 0, 0, 0)
        activity_layout.setSpacing(2)
        self._schematic_activity = activity
        layout.addWidget(activity, 0, 1)
        layout.setColumnStretch(0, 1)
        layout.setColumnStretch(1, 2)
        layout.setColumnStretch(2, 2)

    def _move_schematic_widgets(self, enabled: bool) -> None:
        if enabled:
            scroll = self._panels_area.widget(0).layout().itemAt(0).widget()
            scroll.widget().layout().removeWidget(self._input_group)
            self._panels_area.widget(0).layout().removeWidget(self._output_group)
            preview_column = self._panels_area.widget(1)
            preview_layout = preview_column.layout()
            for widget in (self._playback_row, self._position_row, self._preview_row):
                preview_layout.removeWidget(widget)
            self._schematic_bottom.layout().addWidget(self._input_group, 0, 0)
            activity_layout = self._schematic_activity.layout()
            activity_layout.addWidget(self._playback_row)
            activity_layout.addWidget(self._position_row)
            activity_layout.addWidget(self._preview_row)
            self._schematic_bottom.layout().addWidget(self._output_group, 0, 2)
        else:
            bottom_layout = self._schematic_bottom.layout()
            bottom_layout.removeWidget(self._input_group)
            bottom_layout.removeWidget(self._output_group)
            activity = bottom_layout.itemAtPosition(0, 1).widget()
            activity.layout().removeWidget(self._playback_row)
            activity.layout().removeWidget(self._position_row)
            activity.layout().removeWidget(self._preview_row)
            left = self._panels_area.widget(0)
            scroll = left.layout().itemAt(0).widget()
            scroll.widget().layout().insertWidget(0, self._input_group)
            left.layout().addWidget(self._output_group, 0)
            preview_layout = self._panels_area.widget(1).layout()
            preview_layout.insertWidget(1, self._playback_row)
            preview_layout.insertWidget(2, self._position_row)
            preview_layout.insertWidget(3, self._preview_row)

    def _grow_for_schematic(self) -> None:
        if self.isMaximized() or self.isFullScreen():
            return
        screen = self.screen()
        if screen is None:
            return
        available = screen.availableGeometry()
        self.resize(
            min(available.width(), max(self.width(), SCHEMATIC_MIN_WIDTH)),
            min(available.height(), max(self.height(), SCHEMATIC_MIN_HEIGHT)),
        )

    def _build_input_group(self) -> QGroupBox:
        box = QGroupBox("Input")
        grid = QGridLayout(box)
        self.input_edit = QLineEdit()
        self.input_edit.setObjectName("inputEdit")
        self.input_edit.setReadOnly(True)
        self.browse_input_button = QPushButton("Open...")
        self.browse_input_button.setObjectName("browseInputButton")
        self.browse_input_button.clicked.connect(self._browse_input)
        self.info_label = _wrapping_label("infoLabel", "No file chosen")
        grid.addWidget(self.input_edit, 0, 0)
        grid.addWidget(self.browse_input_button, 0, 1)
        grid.addWidget(self.info_label, 1, 0, 1, 2)
        return box

    def _build_look_group(self) -> QGroupBox:
        box = QGroupBox("Look")
        grid = QGridLayout(box)
        self.preset_combo = QComboBox()
        self.preset_combo.setObjectName("presetCombo")
        self.preset_combo.setToolTip(LOOK_HELP["preset"])
        self.seed_spin = QSpinBox()
        self.seed_spin.setObjectName("seedSpin")
        self.seed_spin.setRange(0, SEED_MAX)
        self.seed_spin.setValue(1)
        self.seed_spin.setToolTip(LOOK_HELP["seed"])
        self.seed_spin.valueChanged.connect(self._on_look_changed)
        self.random_seed_button = QPushButton("Random")
        self.random_seed_button.setObjectName("randomSeedButton")
        self.random_seed_button.clicked.connect(self._random_seed)
        self.reset_button = QPushButton("Reset to preset")
        self.reset_button.setObjectName("resetButton")
        self.reset_button.clicked.connect(self._reset_to_preset)

        self.generations_spin = QSpinBox()
        self.generations_spin.setObjectName("generationsSpin")
        self.generations_spin.setRange(1, MAX_GENERATIONS)
        self.generations_spin.valueChanged.connect(self._on_look_changed)
        generations_label = QLabel("Generations")
        for widget in (generations_label, self.generations_spin):
            widget.setToolTip(LOOK_HELP["generations"])
        self.signal_combo = QComboBox()
        self.signal_combo.setObjectName("signalCombo")
        self.signal_combo.setToolTip(LOOK_HELP["signal"])
        for text, value in zip(SIGNAL_LABELS, SIGNALS, strict=True):
            self.signal_combo.addItem(text, value)
        self.signal_combo.currentIndexChanged.connect(self._on_look_changed)
        self.audio_combo = QComboBox()
        self.audio_combo.setObjectName("audioCombo")
        self.audio_combo.setToolTip(LOOK_HELP["audio"])
        for text, value in zip(AUDIO_LABELS, AUDIO_MODES, strict=True):
            self.audio_combo.addItem(text, value)
        self.audio_combo.currentIndexChanged.connect(self._on_look_changed)

        self.standard_combo = QComboBox()
        self.standard_combo.setObjectName("field_standard")
        for text, value in zip(STANDARD_LABELS, STANDARDS, strict=True):
            self.standard_combo.addItem(text, value)
        standard_label = QLabel("Standard")
        for widget in (standard_label, self.standard_combo):
            widget.setToolTip(STANDARD_HELP)
        self.standard_combo.currentIndexChanged.connect(self._on_look_changed)
        self.camera_tube_combo = QComboBox()
        self.camera_tube_combo.setObjectName("field_camera_tube")
        for text, value in zip(CAMERA_TUBE_LABELS, CAMERA_TUBES, strict=True):
            self.camera_tube_combo.addItem(text, value)
        camera_tube_label = QLabel("Camera tube")
        for widget in (camera_tube_label, self.camera_tube_combo):
            widget.setToolTip(CAMERA_TUBE_HELP)
        self.camera_tube_combo.currentIndexChanged.connect(self._on_camera_tube_changed)
        self.iris_spin = QDoubleSpinBox()
        self.iris_spin.setObjectName("field_iris_stops")
        self.iris_spin.setRange(*IRIS_STOPS_RANGE)
        self.iris_spin.setSingleStep(IRIS_STEP)
        self.iris_spin.setDecimals(IRIS_DECIMALS)
        self.iris_spin.setSuffix(" stops")
        self.iris_spin.valueChanged.connect(self._on_look_changed)
        iris_label = QLabel("Iris")
        for widget in (iris_label, self.iris_spin):
            widget.setToolTip(IRIS_HELP)
        self._update_iris_enabled()

        self.save_look_button = QPushButton("Save look...")
        self.save_look_button.setObjectName("saveLookButton")
        self.save_look_button.clicked.connect(self._browse_save_look)
        self.load_look_button = QPushButton("Load look...")
        self.load_look_button.setObjectName("loadLookButton")
        self.load_look_button.clicked.connect(self._browse_load_look)
        look_files = QHBoxLayout()
        look_files.addWidget(self.save_look_button)
        look_files.addWidget(self.load_look_button)

        grid.addWidget(QLabel("Preset"), 0, 0)
        grid.addWidget(self.preset_combo, 0, 1, 1, 2)
        grid.addWidget(QLabel("Seed"), 1, 0)
        grid.addWidget(self.seed_spin, 1, 1)
        grid.addWidget(self.random_seed_button, 1, 2)
        grid.addWidget(generations_label, 2, 0)
        grid.addWidget(self.generations_spin, 2, 1)
        grid.addWidget(QLabel("Signal"), 3, 0)
        grid.addWidget(self.signal_combo, 3, 1, 1, 2)
        grid.addWidget(QLabel("Sound"), 4, 0)
        grid.addWidget(self.audio_combo, 4, 1, 1, 2)
        grid.addWidget(standard_label, 5, 0)
        grid.addWidget(self.standard_combo, 5, 1, 1, 2)
        grid.addWidget(camera_tube_label, 6, 0)
        grid.addWidget(self.camera_tube_combo, 6, 1, 1, 2)
        grid.addWidget(iris_label, 7, 0)
        grid.addWidget(self.iris_spin, 7, 1)
        grid.addWidget(self.reset_button, 8, 0, 1, 3)
        grid.addLayout(look_files, 9, 0, 1, 3)
        grid.setColumnStretch(1, 1)
        return box

    def _build_condition_groups(self) -> list[QGroupBox]:
        boxes: list[QGroupBox] = []
        for group in GROUPS:
            box = QGroupBox(group)
            grid = QGridLayout(box)
            grid.setColumnStretch(1, 1)
            row = 0
            for spec in CONDITION_FIELDS:
                if spec.group != group:
                    continue
                if spec.kind == "bool":
                    self._add_bool_field(spec, grid, row)
                elif spec.kind == "choice":
                    self._add_choice_field(spec, grid, row)
                else:
                    self._add_float_field(spec, grid, row)
                row += 1
            boxes.append(box)
        if self._field_labels:
            max_width = max(label.sizeHint().width() for label in self._field_labels)
            for label in self._field_labels:
                label.setMinimumWidth(max_width)
        return boxes

    def _add_bool_field(self, spec: FieldSpec, grid: QGridLayout, row: int) -> None:
        check = QCheckBox(spec.label)
        check.setObjectName(f"field_{spec.key}")
        check.setToolTip(spec.help)
        check.toggled.connect(self._on_look_changed)
        grid.addWidget(check, row, 0, 1, 3)
        self._checks[spec.key] = check

    def _add_choice_field(self, spec: FieldSpec, grid: QGridLayout, row: int) -> None:
        label = QLabel(spec.label)
        self._field_labels.append(label)
        combo = QComboBox()
        combo.setObjectName(f"field_{spec.key}")
        for choice in spec.choices:
            combo.addItem(choice.upper(), choice)
        for widget in (label, combo):
            widget.setToolTip(spec.help)
        combo.currentIndexChanged.connect(self._on_look_changed)
        grid.addWidget(label, row, 0)
        grid.addWidget(combo, row, 1, 1, 2)
        self._choice_boxes[spec.key] = combo

    def _add_float_field(self, spec: FieldSpec, grid: QGridLayout, row: int) -> None:
        label = QLabel(spec.label)
        self._field_labels.append(label)
        slider = QSlider(Qt.Orientation.Horizontal)
        slider.setObjectName(f"slider_{spec.key}")
        slider.setRange(0, max(1, round((spec.hi - spec.lo) / spec.step)))
        spin = QDoubleSpinBox()
        spin.setObjectName(f"field_{spec.key}")
        spin.setDecimals(_decimals(spec.step))
        spin.setRange(spec.lo, spec.hi)
        spin.setSingleStep(spec.step)
        if spec.unit:
            spin.setSuffix(f" {spec.unit}")
        for widget in (label, slider, spin):
            widget.setToolTip(spec.help)

        def spin_changed(value: float) -> None:
            slider.blockSignals(True)
            slider.setValue(round((value - spec.lo) / spec.step))
            slider.blockSignals(False)
            self._on_look_changed()

        def slider_moved(position: int) -> None:
            spin.setValue(spec.lo + position * spec.step)

        spin.valueChanged.connect(spin_changed)
        slider.valueChanged.connect(slider_moved)
        grid.addWidget(label, row, 0)
        grid.addWidget(slider, row, 1)
        grid.addWidget(spin, row, 2)
        self._spins[spec.key] = spin

    def _build_advanced_group(self) -> QGroupBox:
        box = QGroupBox("Advanced")
        layout = QVBoxLayout(box)
        self.advanced_edit = QPlainTextEdit()
        self.advanced_edit.setObjectName("advancedEdit")
        self.advanced_edit.setFont(QFontDatabase.systemFont(QFontDatabase.SystemFont.FixedFont))
        self.advanced_edit.setPlaceholderText("[tape]\ncreases_per_m = 0.5")
        self.advanced_edit.setMinimumHeight(90)
        self.advanced_edit.textChanged.connect(self._on_advanced_changed)
        self.advanced_status = _wrapping_label("advancedStatus")
        self.advanced_status.setStyleSheet(ERROR_STYLE)
        layout.addWidget(self.advanced_edit)
        layout.addWidget(self.advanced_status)
        return box

    def _build_output_group(self) -> QGroupBox:
        box = QGroupBox("Output")
        grid = QGridLayout(box)
        grid.setColumnStretch(1, 1)
        grid.setColumnStretch(3, 1)

        self.output_edit = QLineEdit()
        self.output_edit.setObjectName("outputEdit")
        self.browse_output_button = QPushButton("Save as...")
        self.browse_output_button.setObjectName("browseOutputButton")
        self.browse_output_button.clicked.connect(self._browse_output)
        file_row = QHBoxLayout()
        file_row.addWidget(self.output_edit, 1)
        file_row.addWidget(self.browse_output_button)
        grid.addWidget(QLabel("File"), 0, 0)
        grid.addLayout(file_row, 0, 1, 1, 3)

        self.start_spin = QDoubleSpinBox()
        self.start_spin.setObjectName("startSpin")
        self.duration_spin = QDoubleSpinBox()
        self.duration_spin.setObjectName("durationSpin")
        for spin in (self.start_spin, self.duration_spin):
            spin.setDecimals(TIME_DECIMALS)
            spin.setSingleStep(TIME_STEP_S)
            spin.setRange(0.0, UNKNOWN_DURATION_MAX_S)
            spin.setSuffix(" s")
        self.duration_spin.setSpecialValueText("to the end")
        grid.addWidget(QLabel("Start"), 1, 0)
        grid.addWidget(self.start_spin, 1, 1)
        grid.addWidget(QLabel("Length"), 1, 2)
        grid.addWidget(self.duration_spin, 1, 3)

        self.size_combo = QComboBox()
        self.size_combo.setObjectName("sizeCombo")
        for text, size in SIZE_CHOICES:
            self.size_combo.addItem(text, size)
        grid.addWidget(QLabel("Size"), 2, 0)
        grid.addWidget(self.size_combo, 2, 1)

        self.crf_spin = QSpinBox()
        self.crf_spin.setObjectName("crfSpin")
        self.crf_spin.setRange(0, CRF_MAX)
        self.crf_spin.setValue(DEFAULT_CRF)
        self.crf_spin.setToolTip("CRF: lower is better")
        grid.addWidget(QLabel("Quality"), 2, 2)
        grid.addWidget(self.crf_spin, 2, 3)

        self.render_button = QPushButton("Render")
        self.render_button.setObjectName("renderButton")
        self.render_button.clicked.connect(self._start_render)
        self.cancel_button = QPushButton("Cancel")
        self.cancel_button.setObjectName("cancelButton")
        self.cancel_button.clicked.connect(self._cancel)
        self.open_button = QPushButton("Open")
        self.open_button.setObjectName("openButton")
        self.open_button.clicked.connect(self._open_result)
        self.open_button.setVisible(False)
        buttons = QHBoxLayout()
        buttons.addWidget(self.render_button)
        buttons.addWidget(self.cancel_button)
        buttons.addWidget(self.open_button)
        grid.addLayout(buttons, 3, 0, 1, 4)

        self.progress_bar = QProgressBar()
        self.progress_bar.setObjectName("progressBar")
        self.progress_bar.setRange(0, PROGRESS_FULL)
        self.status_label = _wrapping_label("statusLabel")
        grid.addWidget(self.progress_bar, 4, 0, 1, 4)
        grid.addWidget(self.status_label, 5, 0, 1, 4)
        return box

    def _build_preview_column(self) -> QWidget:
        column = QWidget()
        layout = QVBoxLayout(column)
        self._preview_column = column
        self._preview_layout = layout
        self.preview = PreviewView()
        self.preview.setObjectName("previewView")
        layout.addWidget(self.preview, 1)

        self._preview_placeholder = QWidget()
        self._preview_placeholder.setObjectName("previewPlaceholder")
        placeholder_layout = QVBoxLayout(self._preview_placeholder)
        placeholder_layout.setContentsMargins(20, 20, 20, 20)
        placeholder_label = QLabel("Preview is detached into a floating window.")
        placeholder_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.reattach_button = QPushButton("Reattach preview")
        self.reattach_button.setObjectName("reattachButton")
        self.reattach_button.clicked.connect(self._dock_preview)
        placeholder_layout.addStretch(1)
        placeholder_layout.addWidget(placeholder_label)
        placeholder_layout.addWidget(self.reattach_button, 0, Qt.AlignmentFlag.AlignCenter)
        placeholder_layout.addStretch(1)
        self._preview_placeholder.setVisible(False)

        self.original_button = QPushButton("Original")
        self.original_button.setObjectName("originalButton")
        self.processed_button = QPushButton("Processed")
        self.processed_button.setObjectName("processedButton")
        self.split_button = QPushButton("Split")
        self.split_button.setObjectName("splitButton")
        modes = QButtonGroup(self)
        modes.setExclusive(True)
        for button in (self.original_button, self.processed_button, self.split_button):
            button.setCheckable(True)
            modes.addButton(button)
        self.original_button.setChecked(True)
        self.original_button.clicked.connect(lambda: self._set_view_mode(MODE_ORIGINAL))
        self.processed_button.clicked.connect(lambda: self._set_view_mode(MODE_PROCESSED))
        self.split_button.clicked.connect(lambda: self._set_view_mode(MODE_SPLIT))
        self.save_frame_button = QPushButton("Save frame...")
        self.save_frame_button.setObjectName("saveFrameButton")
        self.save_frame_button.setEnabled(False)
        self.save_frame_button.clicked.connect(self._browse_save_frame)
        self.preview.image_changed.connect(self._update_save_frame_button)
        self.popout_button = QPushButton("Pop out")
        self.popout_button.setObjectName("popoutButton")
        self.popout_button.clicked.connect(self._toggle_preview_popout)
        self.play_button = QPushButton("Play")
        self.play_button.setObjectName("playButton")
        self.play_button.clicked.connect(self._toggle_play)
        self.frame_slider = QSlider(Qt.Orientation.Horizontal)
        self.frame_slider.setObjectName("frameSlider")
        self.frame_slider.valueChanged.connect(self._scrub)
        self.frame_label = QLabel("")
        playback = QHBoxLayout()
        playback.addWidget(self.original_button)
        playback.addWidget(self.processed_button)
        playback.addWidget(self.split_button)
        playback.addWidget(self.save_frame_button)
        playback.addWidget(self.popout_button)
        playback.addWidget(self.play_button)
        playback.addWidget(self.frame_slider, 1)
        playback.addWidget(self.frame_label)
        self._playback_row = QWidget()
        self._playback_row.setLayout(playback)
        layout.addWidget(self._playback_row)

        self.position_slider = QSlider(Qt.Orientation.Horizontal)
        self.position_slider.setObjectName("positionSlider")
        self.position_slider.setRange(0, 0)
        self.position_slider.valueChanged.connect(self._on_position_changed)
        self.position_label = QLabel("")
        position = QHBoxLayout()
        position.addWidget(QLabel("Position"))
        position.addWidget(self.position_slider, 1)
        position.addWidget(self.position_label)
        self._position_row = QWidget()
        self._position_row.setLayout(position)
        layout.addWidget(self._position_row)

        self.length_spin = QDoubleSpinBox()
        self.length_spin.setObjectName("lengthSpin")
        self.length_spin.setRange(0.0, LENGTH_MAX_S)
        self.length_spin.setSingleStep(LENGTH_STEP_S)
        self.length_spin.setDecimals(1)
        self.length_spin.setSuffix(" s")
        self.length_spin.setSpecialValueText("single frame")
        self.length_spin.setValue(PREVIEW_DEFAULT_SECONDS)
        self.preview_button = QPushButton("Render preview")
        self.preview_button.setObjectName("previewButton")
        self.preview_button.clicked.connect(self._start_preview)
        self.stale_label = QLabel("Settings changed since this preview")
        self.stale_label.setObjectName("staleLabel")
        self.stale_label.setVisible(False)
        preview_row = QHBoxLayout()
        preview_row.addWidget(QLabel("Preview length"))
        preview_row.addWidget(self.length_spin)
        preview_row.addWidget(self.preview_button)
        preview_row.addWidget(self.stale_label, 1)
        self._preview_row = QWidget()
        self._preview_row.setLayout(preview_row)
        layout.addWidget(self._preview_row)
        return column

    def _fill_presets(self) -> None:
        presets = backend.list_presets(self._binary)
        for index, preset in enumerate(presets):
            self.preset_combo.addItem(preset.name)
            self.preset_combo.setItemData(index, preset.summary, Qt.ItemDataRole.ToolTipRole)
        names = [preset.name for preset in presets]
        field_help = {spec.key: spec.help for spec in CONDITION_FIELDS}
        for view in self._schematic_views:
            if "preset" in view.control_ids():
                view.set_choices("preset", names)
                view.set_help({**field_help, **LOOK_HELP})
            else:
                view.set_help(field_help)
        if DEFAULT_PRESET in names:
            self.preset_combo.setCurrentIndex(names.index(DEFAULT_PRESET))
        self._load_preset()
        self.preset_combo.currentIndexChanged.connect(self._on_preset_changed)

    # ------------------------------------------------------------ the look

    def _condition(self) -> dict[str, float | bool | str]:
        values: dict[str, float | bool | str] = {}
        for spec in CONDITION_FIELDS:
            if spec.kind == "bool":
                values[spec.key] = self._checks[spec.key].isChecked()
            elif spec.kind == "choice":
                values[spec.key] = self._choice_boxes[spec.key].currentData()
            else:
                values[spec.key] = self._spins[spec.key].value()
        return values

    def _apply_condition(self, values: dict[str, float | bool | str]) -> None:
        self._loading = True
        try:
            for spec in CONDITION_FIELDS:
                if spec.kind == "bool":
                    self._checks[spec.key].setChecked(bool(values.get(spec.key, False)))
                elif spec.kind == "choice":
                    combo = self._choice_boxes[spec.key]
                    value = values.get(spec.key)
                    if value is None and spec.choices:
                        value = spec.choices[0]
                    combo.setCurrentIndex(combo.findData(value))
                else:
                    self._spins[spec.key].setValue(float(values.get(spec.key, spec.lo)))
        finally:
            self._loading = False

    def _schematic_values(self) -> dict[str, object]:
        look = self.look()
        return {
            **look.condition,
            "preset": look.preset,
            "seed": look.seed,
            "generations": look.generations,
            "signal": look.signal,
            "audio": look.audio,
            "standard": look.standard,
            "camera_tube": look.camera_tube,
            "iris_stops": look.iris_stops,
        }

    def _sync_schematic(self) -> None:
        if self._sync_guard:
            return
        self._sync_guard = True
        try:
            values = self._schematic_values()
            for view in self._schematic_views:
                for control_id in view.control_ids():
                    view.set_value(control_id, values[control_id], emit=False)
        finally:
            self._sync_guard = False

    def _sync_screen_image(self) -> None:
        for view in self._schematic_views:
            view.set_screen_image(self.preview.image())

    def _set_schematic_baseline(self, condition: dict[str, float | bool | str]) -> None:
        self._schematic_baseline = {
            **condition,
            "seed": 1,
            "generations": 1,
            "signal": "svideo",
            "audio": "process",
            "standard": DEFAULT_STANDARD,
            "camera_tube": backend.DEFAULT_CAMERA_TUBE,
            "iris_stops": DEFAULT_IRIS_STOPS,
        }
        for view in self._schematic_views:
            view.set_baseline(self._schematic_baseline)

    def _load_preset(self) -> None:
        name = self.preset_combo.currentText()
        if not name:
            return
        try:
            values = backend.preset_condition(self._binary, name)
        except GuiError as err:
            self._set_status(str(err))
            return
        self._apply_condition(values)
        self._set_schematic_baseline(values)
        self._sync_schematic()

    def _on_preset_changed(self, _index: int) -> None:
        self._load_preset()
        self._set_stale(True)

    def _reset_to_preset(self) -> None:
        self._load_preset()
        self._set_stale(True)

    def _random_seed(self) -> None:
        self.seed_spin.setValue(random.randint(0, SEED_MAX))

    def _on_schematic_value_changed(self, control_id: str, value: object) -> None:
        if self._sync_guard:
            return
        self._sync_guard = True
        try:
            if control_id in self._checks:
                self._checks[control_id].setChecked(bool(value))
            elif control_id in self._spins:
                self._spins[control_id].setValue(float(value))
            elif control_id in self._choice_boxes:
                self._choice_boxes[control_id].setCurrentIndex(self._choice_boxes[control_id].findData(value))
            elif control_id == "preset":
                self.preset_combo.setCurrentText(str(value))
            elif control_id == "seed":
                self.seed_spin.setValue(int(value))
            elif control_id == "generations":
                self.generations_spin.setValue(int(value))
            elif control_id == "signal":
                self.signal_combo.setCurrentIndex(self.signal_combo.findData(value))
            elif control_id == "audio":
                self.audio_combo.setCurrentIndex(self.audio_combo.findData(value))
            elif control_id == "standard":
                self.standard_combo.setCurrentIndex(self.standard_combo.findData(value))
            elif control_id == "camera_tube":
                self.camera_tube_combo.setCurrentIndex(self.camera_tube_combo.findData(value))
            elif control_id == "iris_stops":
                self.iris_spin.setValue(float(value))
        finally:
            self._sync_guard = False
        self._sync_schematic()

    def _reset_schematic_value(self, control_id: str) -> None:
        if control_id == "preset":
            return
        if control_id not in self._schematic_baseline:
            return
        self._on_schematic_value_changed(control_id, self._schematic_baseline[control_id])
        self._sync_schematic()

    def _on_look_changed(self, *_args: object) -> None:
        if not self._loading:
            self._set_stale(True)
            if not self._sync_guard:
                self._sync_schematic()

    def _update_iris_enabled(self) -> None:
        # Without a tube the camera adds nothing, so the iris has no effect to set.
        self.iris_spin.setEnabled(self.camera_tube_combo.currentData() != backend.DEFAULT_CAMERA_TUBE)

    def _on_camera_tube_changed(self, *_args: object) -> None:
        self._update_iris_enabled()
        self._on_look_changed()

    def _on_advanced_changed(self) -> None:
        self._advanced_error = backend.validate_advanced(self.advanced_edit.toPlainText())
        self.advanced_status.setText(self._advanced_error or "")
        self._update_controls()
        self._on_look_changed()

    def _set_stale(self, stale: bool) -> None:
        self._stale = stale
        # Without a preview there is nothing the settings could have changed since.
        self.stale_label.setVisible(stale and self.processed_frames is not None)

    # ------------------------------------------------------------ look files

    def _browse_save_look(self) -> None:
        start = str(Path(self._info.path).parent) if self._info is not None else str(Path.home())
        path, _ = QFileDialog.getSaveFileName(self, "Save look", start, LOOK_FILTER)
        if path:
            self.save_look(path if Path(path).suffix else path + LOOK_SUFFIX)

    def _browse_load_look(self) -> None:
        start = str(Path(self._info.path).parent) if self._info is not None else str(Path.home())
        path, _ = QFileDialog.getOpenFileName(self, "Load look", start, LOOK_FILTER)
        if path:
            self.load_look(path)

    def save_look(self, path: str) -> None:
        try:
            text = backend.save_look(self.look())
            Path(path).write_text(text, encoding="utf-8")
        except GuiError as err:
            self._set_status(str(err))
        except OSError as err:
            self._set_status(f"Cannot write {path}: {err}")
        else:
            self._set_status(f"Look saved to {path}")

    def load_look(self, path: str) -> None:
        try:
            look = backend.load_look(Path(path).read_text(encoding="utf-8"))
            self._check_loadable(look)
        except GuiError as err:
            self._set_status(str(err))
            return
        except (OSError, UnicodeDecodeError) as err:
            self._set_status(f"Cannot read {path}: {err}")
            return
        # Choosing the preset loads its condition; the file's own values go on top of that.
        self.preset_combo.blockSignals(True)
        self.preset_combo.setCurrentIndex(self.preset_combo.findText(look.preset))
        self.preset_combo.blockSignals(False)
        self._load_preset()
        self._apply_condition({**self._condition(), **look.condition})
        self.seed_spin.setValue(look.seed)
        self.generations_spin.setValue(look.generations)
        self.signal_combo.setCurrentIndex(self.signal_combo.findData(look.signal))
        self.audio_combo.setCurrentIndex(self.audio_combo.findData(look.audio))
        self.standard_combo.setCurrentIndex(self.standard_combo.findData(look.standard))
        self.camera_tube_combo.setCurrentIndex(self.camera_tube_combo.findData(look.camera_tube))
        self.iris_spin.setValue(look.iris_stops)
        self.advanced_edit.setPlainText(look.advanced)
        self._set_stale(True)
        self._sync_schematic()
        self._set_status(f"Look loaded from {path}")

    def _check_loadable(self, look: Look) -> None:
        """Fail before anything is changed when the editors could not show the look."""
        if self.preset_combo.findText(look.preset) < 0:
            raise GuiError(f"Look file: unknown preset '{look.preset}'")
        for spec in CONDITION_FIELDS:
            if spec.key not in look.condition:
                continue
            value = look.condition[spec.key]
            if spec.kind == "bool":
                valid = isinstance(value, bool)
            elif spec.kind == "choice":
                valid = value in spec.choices
            else:
                valid = isinstance(value, (int, float)) and not isinstance(value, bool)
            if not valid:
                raise GuiError(f"Look file: '{spec.key}' has an unusable value")

    # ------------------------------------------------------------ save frame

    def _update_save_frame_button(self) -> None:
        self.save_frame_button.setEnabled(self.preview.image() is not None)

    def _browse_save_frame(self) -> None:
        if self._info is not None:
            source = Path(self._info.path)
            start = str(source.with_name(f"{source.stem}_frame{FRAME_SUFFIX}"))
        else:
            start = str(Path.home())
        path, _ = QFileDialog.getSaveFileName(self, "Save frame", start, FRAME_FILTER)
        if path:
            self.save_frame(path if Path(path).suffix else path + FRAME_SUFFIX)

    def save_frame(self, path: str) -> None:
        image = self.preview.image()
        if image is None or image.isNull():
            self._set_status("There is no frame to save.")
        elif image.save(path, FRAME_FORMAT):
            self._set_status(f"Frame saved to {path}")
        else:
            self._set_status(f"Cannot write {path}")

    # ------------------------------------------------------------ status line

    def _set_status(self, text: str, detail: str = "") -> None:
        self.status_label.setText(text)
        self.status_label.setToolTip(detail or text)

    def _update_controls(self) -> None:
        busy = self.busy
        ready = self._info is not None
        runnable = ready and self._advanced_error is None and not busy
        self.preview_button.setEnabled(runnable)
        self.render_button.setEnabled(runnable)
        self.cancel_button.setEnabled(busy)
        for widget in (
            self.browse_input_button,
            self.browse_output_button,
            self.output_edit,
            self.start_spin,
            self.duration_spin,
            self.size_combo,
            self.crf_spin,
            self.length_spin,
        ):
            widget.setEnabled(not busy)
        self.position_slider.setEnabled(ready and not busy)
        count = self._frame_count()
        self.original_button.setEnabled(ready)
        self.processed_button.setEnabled(self.processed_frames is not None)
        self.split_button.setEnabled(self.processed_frames is not None)
        self._update_save_frame_button()
        self.play_button.setEnabled(count > 1)
        self.frame_slider.setEnabled(count > 1)

    # ------------------------------------------------------------ input

    def _browse_input(self) -> None:
        start = str(Path(self._info.path).parent) if self._info is not None else str(Path.home())
        path, _ = QFileDialog.getOpenFileName(self, "Open video", start, VIDEO_FILTER)
        if path:
            self._open(path, interactive=True)

    def _open(self, path: str, interactive: bool) -> None:
        if self.busy:
            self._set_status("Wait for the running job to finish first.")
            return
        try:
            info = backend.probe(path, self._ffprobe)
        except GuiError as err:
            self._set_status(str(err))
            if interactive:
                QMessageBox.warning(self, WINDOW_TITLE, str(err))
            return
        self._info = info
        self._source_timer.stop()
        self.input_edit.setText(info.path)
        self.info_label.setText(_describe(info))
        self.position_slider.blockSignals(True)
        self.position_slider.setRange(0, round(info.duration * POSITION_STEPS_PER_S))
        self.position_slider.blockSignals(False)
        limit = info.duration or UNKNOWN_DURATION_MAX_S
        for spin in (self.start_spin, self.duration_spin):
            spin.setMaximum(limit)
            spin.setValue(0.0)
        self.set_output(str(_suggest_output(Path(info.path).absolute())))
        self.open_button.setVisible(False)
        self.progress_bar.setValue(0)
        self._set_status("")
        self._clear_frames()
        self._update_position_label()
        self._update_controls()
        self._request_source()

    def _position_s(self) -> float:
        return self.position_slider.value() / POSITION_STEPS_PER_S

    def _update_position_label(self) -> None:
        total = self._info.duration if self._info is not None else 0.0
        self.position_label.setText(f"{_clock(self._position_s())} / {_clock(total)}")

    def _on_position_changed(self, _value: int) -> None:
        self._update_position_label()
        self._clear_frames()
        self._source_timer.start()

    # ------------------------------------------------------------ source frame

    def _request_source(self) -> None:
        self._source_timer.stop()
        if self._source is not None:
            self._source.job.cancel()
            self._source = None
        info = self._info
        if info is None:
            return
        standard = self.look().standard
        plan = backend.preview_plan(info, self._position_s(), self.length_spin.value(), standard=standard)
        args = backend.decode_args(
            self._ffmpeg, info.path, plan.size, start=frame_time(plan.start_frame, standard), frames=1, standard=standard
        )
        try:
            job = backend.Job(args, capture_stdout=True).start()
        except GuiError as err:
            self._set_status(str(err))
            return
        self._source = _SourceRequest(job, plan.size)

    def _poll_source(self) -> None:
        request = self._source
        if request is None:
            return
        state = request.job.poll()
        if state.running:
            return
        self._source = None
        width, height = request.size
        need = width * height * RGB_BYTES
        raw = request.job.stdout[:need]
        if state.returncode == 0 and len(raw) == need:
            # A preview that was started meanwhile owns the picture.
            if self.original_frames is None and self.processed_frames is None:
                self.preview.set_image(QImage(raw, width, height, RGB_BYTES * width, QImage.Format.Format_RGB888).copy())
        elif not self.busy:
            self._set_status(_tail(state.log) if state.log.strip() else "There is no frame at this position.", state.log)

    # ------------------------------------------------------------ frames and playback

    def _clear_frames(self) -> None:
        self._set_playing(False)
        self.original_frames = None
        self.processed_frames = None
        self._frame_index = 0
        self._set_stale(False)
        self._sync_frame_slider()
        self.frame_label.setText("")
        self._set_view_mode(MODE_ORIGINAL)
        self._update_controls()

    def _frame_count(self) -> int:
        counts = [f.count for f in (self.original_frames, self.processed_frames) if f is not None]
        return min(counts) if counts else 0

    def _sync_frame_slider(self) -> None:
        last = max(0, self._frame_count() - 1)
        self.frame_slider.blockSignals(True)
        self.frame_slider.setRange(0, last)
        self.frame_slider.setValue(min(self._frame_index, last))
        self.frame_slider.blockSignals(False)

    def _set_view_mode(self, mode: str) -> None:
        if mode in (MODE_PROCESSED, MODE_SPLIT) and self.processed_frames is None:
            mode = MODE_ORIGINAL
        self._view_mode = mode
        self.original_button.setChecked(mode == MODE_ORIGINAL)
        self.processed_button.setChecked(mode == MODE_PROCESSED)
        self.split_button.setChecked(mode == MODE_SPLIT)
        self._show_frame(self._frame_index)

    @property
    def preview_detached(self) -> bool:
        return self._is_preview_detached

    def _toggle_preview_popout(self) -> None:
        if self._is_preview_detached:
            self._dock_preview()
        else:
            self._detach_preview()

    def _detach_preview(self) -> None:
        if self._is_preview_detached:
            return
        if self._detached_window is None:
            self._detached_window = DetachedPreviewWindow(self)
        self._preview_layout.removeWidget(self.preview)
        self._preview_layout.insertWidget(0, self._preview_placeholder, 1)
        self._preview_placeholder.setVisible(True)
        self._detached_window.set_preview_widget(self.preview)
        self.preview.setVisible(True)
        self._detached_window.show()
        self._detached_window.raise_()
        self._detached_window.activateWindow()
        self._is_preview_detached = True
        self.popout_button.setText("Dock")

    def _dock_preview(self) -> None:
        if not self._is_preview_detached:
            return
        if self._detached_window is not None:
            self._detached_window.layout().removeWidget(self.preview)
            self._detached_window.hide()
        self._preview_layout.removeWidget(self._preview_placeholder)
        self._preview_placeholder.setVisible(False)
        self._preview_layout.insertWidget(0, self.preview, 1)
        self.preview.setVisible(self._panels_area.isVisible())
        self._is_preview_detached = False
        self.popout_button.setText("Pop out")

    def show_about_dialog(self) -> None:
        dialog = AboutDialog(self)
        dialog.exec()

    def _toggle_view(self) -> None:
        if self.processed_frames is None:
            return
        self._set_view_mode(MODE_ORIGINAL if self._view_mode == MODE_PROCESSED else MODE_PROCESSED)

    def _show_frame(self, index: int) -> None:
        count = self._frame_count()
        if count == 0:
            return
        self._frame_index = max(0, min(index, count - 1))
        original, processed = self.original_frames, self.processed_frames
        if self._view_mode == MODE_SPLIT and original is not None and processed is not None:
            self.preview.set_split_images(
                _image_from(original, self._frame_index), _image_from(processed, self._frame_index)
            )
        else:
            frames = processed if self._view_mode == MODE_PROCESSED else original
            if frames is None:
                return
            self.preview.set_image(_image_from(frames, self._frame_index))
        self._sync_frame_slider()
        standard = self._frames_standard
        self.frame_label.setText(f"{_clock(frame_time(self._frame_index, standard))} / {_clock(frame_time(count, standard))}")

    def _set_playing(self, playing: bool) -> None:
        playing = playing and self._frame_count() > 1
        self._playing = playing
        self.play_button.setText("Pause" if playing else "Play")
        if playing:
            self._play_timer.setInterval(round(MS_PER_SECOND / fps_of(self._frames_standard)))
            self._play_timer.start()
        else:
            self._play_timer.stop()

    def _toggle_play(self) -> None:
        self._set_playing(not self._playing)

    def _advance_frame(self) -> None:
        count = self._frame_count()
        if count == 0:
            self._set_playing(False)
            return
        self._show_frame((self._frame_index + 1) % count)

    def _scrub(self, index: int) -> None:
        self._set_playing(False)
        self._show_frame(index)

    # ------------------------------------------------------------ running a job

    def _jobs(self) -> list[backend.Job]:
        return [job for job in (self._render_job, self._orig_job, self._decode_job) if job is not None]

    def _write_config(self, look: Look) -> Path:
        config = self._workdir / CONFIG_FILE
        config.write_text(backend.merge_config(look), encoding="utf-8")
        return config

    def _show_progress(self, state: JobState, expected: int, label: str) -> None:
        progress = state.progress
        if progress is None:
            return
        if expected > 0:
            percent = int(progress.frames * PROGRESS_FULL / expected)
            self.progress_bar.setValue(min(PROGRESS_RUNNING_CAP, percent))
        text = f"{label}: {progress.fps:.1f} fps"
        if progress.fps > 0 and expected > progress.frames:
            text += f", about {_remaining((expected - progress.frames) / progress.fps)} left"
        self._set_status(text)

    def _clear_task(self) -> None:
        self._task = None
        self._cancel_requested = False
        self._render_job = None
        self._orig_job = None
        self._decode_job = None
        self._plan = None
        self._preview_look = None
        self._partial = None
        self._render_out = None
        self._update_controls()

    def _end_task(self, message: str, detail: str = "") -> None:
        """Stop whatever is left of the task, drop its partial file and show `message`."""
        for job in self._jobs():
            job.cancel()
        if self._partial is not None:
            self._partial.unlink(missing_ok=True)
        self._clear_task()
        self._set_status(message, detail)

    def _cancel(self) -> None:
        if self._task is None:
            return
        self._cancel_requested = True
        for job in self._jobs():
            job.cancel()
        self._set_status("Cancelling...")

    def _poll(self) -> None:
        self._poll_source()
        try:
            if self._task == TASK_PREVIEW:
                self._poll_preview()
            elif self._task == TASK_RENDER:
                self._poll_render()
        except (GuiError, OSError) as err:
            self._end_task(str(err))

    # ------------------------------------------------------------ preview

    def _start_preview(self) -> None:
        info = self._info
        if info is None or self.busy or self._advanced_error is not None:
            return
        look = self.look()
        plan = backend.preview_plan(info, self._position_s(), self.length_spin.value(), standard=look.standard)
        preview_path = self._workdir / PREVIEW_FILE
        self._task = TASK_PREVIEW
        self._plan = plan
        self._preview_look = look
        self.progress_bar.setValue(0)
        self._set_status("Rendering preview...")
        self._update_controls()
        try:
            config = self._write_config(look)
            preview_path.unlink(missing_ok=True)
            request = RunRequest(
                input=info.path,
                output=str(preview_path),
                look=replace(look, audio=PREVIEW_AUDIO),
                start=plan.start,
                duration=plan.duration,
                size=plan.size,
                crf=PREVIEW_CRF,
            )
            self._render_job = backend.Job(backend.run_args(self._binary, request, config)).start()
            self._orig_job = backend.Job(
                backend.decode_args(
                    self._ffmpeg,
                    info.path,
                    plan.size,
                    start=frame_time(plan.start_frame, look.standard),
                    frames=plan.frames,
                    standard=look.standard,
                ),
                capture_stdout=True,
            ).start()
        except (GuiError, OSError) as err:
            self._end_task(str(err))

    def _preview_standard(self) -> str:
        """The standard of the preview being rendered: the look may be edited while it runs."""
        previewed = self._preview_look
        return previewed.standard if previewed is not None else DEFAULT_STANDARD

    def _poll_preview(self) -> None:
        plan = self._plan
        render_job, orig_job = self._render_job, self._orig_job
        if plan is None or render_job is None or orig_job is None:
            return
        if self._cancel_requested:
            if all(not job.poll().running for job in self._jobs()):
                self._end_task("Cancelled")
            return
        if self._decode_job is None:
            rendered, original = render_job.poll(), orig_job.poll()
            if rendered.running:
                self._show_progress(rendered, plan.lead_frames + plan.frames, "Rendering preview")
            elif rendered.returncode != 0:
                self._end_task(_tail(rendered.log), rendered.log)
                return
            if not original.running and original.returncode != 0:
                self._end_task(_tail(original.log), original.log)
                return
            if rendered.running or original.running:
                return
            self._set_status("Decoding the preview...")
            self._decode_job = backend.Job(
                backend.decode_args(
                    self._ffmpeg, self._workdir / PREVIEW_FILE, plan.size, standard=self._preview_standard()
                ),
                capture_stdout=True,
            ).start()
            return
        decoded = self._decode_job.poll()
        if decoded.running:
            return
        if decoded.returncode != 0:
            self._end_task(_tail(decoded.log), decoded.log)
            return
        self._finish_preview(plan, orig_job, self._decode_job)

    def _finish_preview(self, plan: PreviewPlan, orig_job: backend.Job, decode_job: backend.Job) -> None:
        width, height = plan.size
        shown = plan.frames * width * height * RGB_BYTES
        processed = Frames(width, height, decode_job.stdout).skip(plan.lead_frames)
        processed = Frames(width, height, processed.data[:shown])
        original = Frames(width, height, orig_job.stdout[:shown])
        if processed.count == 0 or original.count == 0:
            self._end_task("The preview has no frames.")
            return
        previewed = self._preview_look
        self._clear_task()
        self.original_frames = original
        self.processed_frames = processed
        self._frames_standard = previewed.standard if previewed is not None else DEFAULT_STANDARD
        self._frame_index = 0
        self._set_stale(previewed != self.look())
        self.progress_bar.setValue(PROGRESS_FULL)
        self._set_status("Preview ready.")
        self._sync_frame_slider()
        self._set_view_mode(MODE_PROCESSED)
        self._set_playing(True)
        self._update_controls()

    # ------------------------------------------------------------ render

    def _browse_output(self) -> None:
        current = self.output_edit.text().strip()
        if not current and self._info is not None:
            current = str(Path(self._info.path).parent)
        path, chosen = QFileDialog.getSaveFileName(
            self, "Save as", current, SAVE_FILTER, options=QFileDialog.Option.DontConfirmOverwrite
        )
        if not path:
            return
        if not Path(path).suffix:
            path += FILTER_SUFFIXES.get(chosen, DEFAULT_SUFFIX)
        self.set_output(path)

    def _open_result(self) -> None:
        if self._rendered_path is not None:
            QDesktopServices.openUrl(QUrl.fromLocalFile(str(self._rendered_path)))

    def _start_render(self) -> None:
        info = self._info
        if info is None or self.busy or self._advanced_error is not None:
            return
        text = self.output_edit.text().strip()
        if not text:
            self._set_status("Choose where to save the result.")
            return
        out = Path(text).expanduser()
        if not out.suffix:
            out = out.with_suffix(DEFAULT_SUFFIX)
            self.set_output(str(out))
        try:
            same = out.resolve() == Path(info.path).resolve()
        except OSError:
            same = False
        if same:
            self._set_status("The output must not be the input file.")
            return
        if out.is_dir() or not out.parent.is_dir():
            self._set_status(f"Cannot write to {out}: the folder does not exist or the name is a folder.")
            return
        if out.exists() and not self.confirm_overwrite(str(out)):
            return

        look = self.look()
        start = self.start_spin.value()
        length = self.duration_spin.value()
        duration = length if length > 0 else None
        partial = backend.partial_path(out)
        available = info.duration - start
        self._render_expected = round(fps_of(look.standard) * min(duration if duration is not None else available, available))
        self._task = TASK_RENDER
        self._partial = partial
        self._render_out = out
        self.open_button.setVisible(False)
        self.progress_bar.setValue(0)
        self._set_status("Rendering...")
        self._update_controls()
        # Qt may hand a stored tuple back as a list.
        chosen_size = self.size_combo.currentData()
        size = (int(chosen_size[0]), int(chosen_size[1])) if chosen_size else None
        try:
            config = self._write_config(look)
            partial.unlink(missing_ok=True)
            request = RunRequest(
                input=info.path,
                output=str(partial),
                look=look,
                start=start,
                duration=duration,
                size=size,
                crf=self.crf_spin.value(),
            )
            self._render_job = backend.Job(backend.run_args(self._binary, request, config)).start()
        except (GuiError, OSError) as err:
            self._end_task(str(err))

    def _poll_render(self) -> None:
        job = self._render_job
        if job is None:
            return
        state = job.poll()
        if state.running:
            if not self._cancel_requested:
                self._show_progress(state, self._render_expected, "Rendering")
            return
        if self._cancel_requested or state.cancelled:
            self._end_task("Cancelled")
        elif state.returncode == 0:
            self._complete_render()
        else:
            self._end_task(_tail(state.log), state.log)

    def _complete_render(self) -> None:
        partial, out = self._partial, self._render_out
        if partial is None or out is None:
            return
        try:
            size = partial.stat().st_size
            os.replace(partial, out)
        except OSError as err:
            self._end_task(f"Cannot write {out}: {err}")
            return
        self._clear_task()
        self._rendered_path = out
        self.progress_bar.setValue(PROGRESS_FULL)
        self._set_status(f"Done: {out} ({size / MEGABYTE:.1f} MB)")
        self.open_button.setVisible(True)

    # ------------------------------------------------------------ closing

    def _wait_for_exit(self, jobs: list[backend.Job]) -> None:
        """Let the event loop run until the killed processes are gone, for a short while at most."""
        if all(not job.poll().running for job in jobs):
            return
        loop = QEventLoop(self)
        ticker = QTimer(self)
        ticker.setInterval(CLOSE_POLL_MS)
        limit = QTimer(self)
        limit.setSingleShot(True)
        limit.setInterval(CLOSE_WAIT_MS)

        def check() -> None:
            if all(not job.poll().running for job in jobs):
                loop.quit()

        ticker.timeout.connect(check)
        limit.timeout.connect(loop.quit)
        ticker.start()
        limit.start()
        loop.exec()
        ticker.stop()
        limit.stop()

    def closeEvent(self, event: QCloseEvent) -> None:
        if not self._closed:
            self._closed = True
            if self._detached_window is not None:
                self._detached_window.close()
            for timer in (self._poll_timer, self._source_timer, self._play_timer):
                timer.stop()
            jobs = self._jobs()
            if self._source is not None:
                jobs.append(self._source.job)
            for job in jobs:
                job.cancel()
            self._wait_for_exit(jobs)
            if self._partial is not None:
                self._partial.unlink(missing_ok=True)
            if self._owns_workdir:
                shutil.rmtree(self._workdir, ignore_errors=True)
        super().closeEvent(event)
