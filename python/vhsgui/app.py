"""The vhs-phys window: pick a video, choose a look, preview it and render it to a file.

All process handling and arithmetic lives in `vhsgui.backend`; this module only holds
widgets and the glue that polls background jobs from a timer.
"""
from __future__ import annotations

import os
import random
import shutil
import tempfile
from dataclasses import dataclass
from pathlib import Path

from PySide6.QtCore import QEventLoop, QRect, Qt, QTimer, QUrl
from PySide6.QtGui import (
    QCloseEvent,
    QColor,
    QDesktopServices,
    QFontDatabase,
    QImage,
    QKeySequence,
    QPainter,
    QPaintEvent,
    QShortcut,
)
from PySide6.QtWidgets import (
    QButtonGroup,
    QCheckBox,
    QComboBox,
    QDoubleSpinBox,
    QFileDialog,
    QFormLayout,
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
    QVBoxLayout,
    QWidget,
)

from vhsgui import backend
from vhsgui.model import (
    CONDITION_FIELDS,
    FPS,
    GROUPS,
    PREVIEW_DEFAULT_SECONDS,
    FieldSpec,
    Frames,
    GuiError,
    JobState,
    Look,
    MediaInfo,
    PreviewPlan,
    RunRequest,
    frame_time,
)

WINDOW_TITLE = "vhs-phys"
WINDOW_SIZE = (1180, 760)  # default size of the window
WINDOW_MIN_SIZE = (900, 600)
LEFT_WIDTH = 440  # initial width of the controls column
LEFT_MIN_WIDTH = 340  # the controls scroll below this width instead of squeezing further
PREVIEW_MIN_SIZE = (320, 200)
PREVIEW_BACKGROUND = "#141414"
ERROR_STYLE = "color: #c0392b;"

POLL_MS = 100  # how often running jobs are looked at
DEBOUNCE_MS = 150  # wait after the last move of the position slider before decoding a frame
PLAY_INTERVAL_MS = round(1000 / FPS)  # preview playback runs at the core's frame rate
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

VIDEO_FILTER = "Video (*.mp4 *.mov *.mkv *.avi *.webm *.m4v *.ts);;All files (*)"
SAVE_FILTER = "MP4 (*.mp4);;Matroska (*.mkv);;QuickTime (*.mov)"
DEFAULT_SUFFIX = ".mp4"
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
TASK_PREVIEW = "preview"
TASK_RENDER = "render"


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
    """Paints one image, scaled to fit, on a dark background."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._image: QImage | None = None
        self.setMinimumSize(*PREVIEW_MIN_SIZE)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)

    def set_image(self, image: QImage | None) -> None:
        self._image = image
        self.update()

    def image(self) -> QImage | None:
        return self._image

    def paintEvent(self, event: QPaintEvent) -> None:
        painter = QPainter(self)
        painter.fillRect(self.rect(), QColor(PREVIEW_BACKGROUND))
        image = self._image
        if image is not None and not image.isNull():
            fitted = image.size().scaled(self.size(), Qt.AspectRatioMode.KeepAspectRatio)
            target = QRect(0, 0, fitted.width(), fitted.height())
            target.moveCenter(self.rect().center())
            painter.setRenderHint(QPainter.RenderHint.SmoothPixmapTransform)
            painter.drawImage(target, image)
        painter.end()


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
        self.original_frames: Frames | None = None
        self.processed_frames: Frames | None = None
        self._frame_index = 0
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
        self._field_labels: list[QLabel] = []

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
        self._play_timer = QTimer(self)
        self._play_timer.setInterval(PLAY_INTERVAL_MS)
        self._play_timer.timeout.connect(self._advance_frame)

        QShortcut(QKeySequence(Qt.Key.Key_Space), self).activated.connect(self._toggle_play)
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

    def look(self) -> Look:
        return Look(
            preset=self.preset_combo.currentText(),
            seed=self.seed_spin.value(),
            condition=self._condition(),
            advanced=self.advanced_edit.toPlainText(),
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
        controls = QWidget()
        column = QVBoxLayout(controls)
        column.addWidget(self._build_input_group())
        column.addWidget(self._build_look_group())
        for group in self._build_condition_groups():
            column.addWidget(group)
        column.addWidget(self._build_advanced_group())
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
        left_layout.addWidget(self._build_output_group(), 0)

        splitter = QSplitter(Qt.Orientation.Horizontal)
        splitter.setChildrenCollapsible(False)
        splitter.addWidget(left)
        splitter.addWidget(self._build_preview_column())
        splitter.setStretchFactor(0, 0)
        splitter.setStretchFactor(1, 1)
        splitter.setSizes([LEFT_WIDTH, WINDOW_SIZE[0] - LEFT_WIDTH])
        self.setCentralWidget(splitter)

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
        self.seed_spin = QSpinBox()
        self.seed_spin.setObjectName("seedSpin")
        self.seed_spin.setRange(0, SEED_MAX)
        self.seed_spin.setValue(1)
        self.seed_spin.valueChanged.connect(self._on_look_changed)
        self.random_seed_button = QPushButton("Random")
        self.random_seed_button.setObjectName("randomSeedButton")
        self.random_seed_button.clicked.connect(self._random_seed)
        self.reset_button = QPushButton("Reset to preset")
        self.reset_button.setObjectName("resetButton")
        self.reset_button.clicked.connect(self._reset_to_preset)
        grid.addWidget(QLabel("Preset"), 0, 0)
        grid.addWidget(self.preset_combo, 0, 1, 1, 2)
        grid.addWidget(QLabel("Seed"), 1, 0)
        grid.addWidget(self.seed_spin, 1, 1)
        grid.addWidget(self.random_seed_button, 1, 2)
        grid.addWidget(self.reset_button, 2, 0, 1, 3)
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
        self.preview = PreviewView()
        self.preview.setObjectName("previewView")
        layout.addWidget(self.preview, 1)

        self.original_button = QPushButton("Original")
        self.original_button.setObjectName("originalButton")
        self.processed_button = QPushButton("Processed")
        self.processed_button.setObjectName("processedButton")
        modes = QButtonGroup(self)
        modes.setExclusive(True)
        for button in (self.original_button, self.processed_button):
            button.setCheckable(True)
            modes.addButton(button)
        self.original_button.setChecked(True)
        self.original_button.clicked.connect(lambda: self._set_view_mode(MODE_ORIGINAL))
        self.processed_button.clicked.connect(lambda: self._set_view_mode(MODE_PROCESSED))
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
        playback.addWidget(self.play_button)
        playback.addWidget(self.frame_slider, 1)
        playback.addWidget(self.frame_label)
        layout.addLayout(playback)

        self.position_slider = QSlider(Qt.Orientation.Horizontal)
        self.position_slider.setObjectName("positionSlider")
        self.position_slider.setRange(0, 0)
        self.position_slider.valueChanged.connect(self._on_position_changed)
        self.position_label = QLabel("")
        position = QHBoxLayout()
        position.addWidget(QLabel("Position"))
        position.addWidget(self.position_slider, 1)
        position.addWidget(self.position_label)
        layout.addLayout(position)

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
        layout.addLayout(preview_row)
        return column

    def _fill_presets(self) -> None:
        presets = backend.list_presets(self._binary)
        for index, preset in enumerate(presets):
            self.preset_combo.addItem(preset.name)
            self.preset_combo.setItemData(index, preset.summary, Qt.ItemDataRole.ToolTipRole)
        names = [preset.name for preset in presets]
        if DEFAULT_PRESET in names:
            self.preset_combo.setCurrentIndex(names.index(DEFAULT_PRESET))
        self._load_preset()
        self.preset_combo.currentIndexChanged.connect(self._on_preset_changed)

    # ------------------------------------------------------------ the look

    def _condition(self) -> dict[str, float | bool]:
        values: dict[str, float | bool] = {}
        for spec in CONDITION_FIELDS:
            if spec.kind == "bool":
                values[spec.key] = self._checks[spec.key].isChecked()
            else:
                values[spec.key] = self._spins[spec.key].value()
        return values

    def _apply_condition(self, values: dict[str, float | bool]) -> None:
        self._loading = True
        try:
            for spec in CONDITION_FIELDS:
                if spec.kind == "bool":
                    self._checks[spec.key].setChecked(bool(values.get(spec.key, False)))
                else:
                    self._spins[spec.key].setValue(float(values.get(spec.key, spec.lo)))
        finally:
            self._loading = False

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

    def _on_preset_changed(self, _index: int) -> None:
        self._load_preset()
        self._set_stale(True)

    def _reset_to_preset(self) -> None:
        self._load_preset()
        self._set_stale(True)

    def _random_seed(self) -> None:
        self.seed_spin.setValue(random.randint(0, SEED_MAX))

    def _on_look_changed(self, *_args: object) -> None:
        if not self._loading:
            self._set_stale(True)

    def _on_advanced_changed(self) -> None:
        self._advanced_error = backend.validate_advanced(self.advanced_edit.toPlainText())
        self.advanced_status.setText(self._advanced_error or "")
        self._update_controls()
        self._on_look_changed()

    def _set_stale(self, stale: bool) -> None:
        self._stale = stale
        # Without a preview there is nothing the settings could have changed since.
        self.stale_label.setVisible(stale and self.processed_frames is not None)

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
        plan = backend.preview_plan(info, self._position_s(), self.length_spin.value())
        args = backend.decode_args(
            self._ffmpeg, info.path, plan.size, start=frame_time(plan.start_frame), frames=1
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
        if mode == MODE_PROCESSED and self.processed_frames is None:
            mode = MODE_ORIGINAL
        self._view_mode = mode
        self.original_button.setChecked(mode == MODE_ORIGINAL)
        self.processed_button.setChecked(mode == MODE_PROCESSED)
        self._show_frame(self._frame_index)

    def _toggle_view(self) -> None:
        if self.processed_frames is None:
            return
        self._set_view_mode(MODE_ORIGINAL if self._view_mode == MODE_PROCESSED else MODE_PROCESSED)

    def _show_frame(self, index: int) -> None:
        frames = self.processed_frames if self._view_mode == MODE_PROCESSED else self.original_frames
        count = self._frame_count()
        if frames is None or count == 0:
            return
        self._frame_index = max(0, min(index, count - 1))
        self.preview.set_image(_image_from(frames, self._frame_index))
        self._sync_frame_slider()
        self.frame_label.setText(f"{_clock(frame_time(self._frame_index))} / {_clock(frame_time(count))}")

    def _set_playing(self, playing: bool) -> None:
        playing = playing and self._frame_count() > 1
        self._playing = playing
        self.play_button.setText("Pause" if playing else "Play")
        if playing:
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
        plan = backend.preview_plan(info, self._position_s(), self.length_spin.value())
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
                look=look,
                start=plan.start,
                duration=plan.duration,
                size=plan.size,
                crf=PREVIEW_CRF,
            )
            self._render_job = backend.Job(backend.run_args(self._binary, request, config)).start()
            self._orig_job = backend.Job(
                backend.decode_args(
                    self._ffmpeg, info.path, plan.size, start=frame_time(plan.start_frame), frames=plan.frames
                ),
                capture_stdout=True,
            ).start()
        except (GuiError, OSError) as err:
            self._end_task(str(err))

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
                backend.decode_args(self._ffmpeg, self._workdir / PREVIEW_FILE, plan.size), capture_stdout=True
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
        self._render_expected = round(FPS * min(duration if duration is not None else available, available))
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
