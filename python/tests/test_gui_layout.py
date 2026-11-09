# CONTRACT
"""Layout acceptance: the controls that produce a file must always be on screen.

The settings column scrolls (there are twenty controls). Choosing the output and
pressing Render must not need a scroll: those widgets live outside the scroll area.
"""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest

pytest.importorskip("PySide6")
from PySide6.QtCore import QPoint, QRect  # noqa: E402
from PySide6.QtWidgets import QApplication, QScrollArea, QWidget  # noqa: E402

DEFAULT_SIZE = (1180, 760)
ALWAYS_VISIBLE = ["outputEdit", "browseOutputButton", "renderButton", "cancelButton", "progressBar"]
ALSO_PINNED = ALWAYS_VISIBLE + ["statusLabel", "startSpin", "durationSpin", "sizeCombo", "crfSpin"]


@pytest.fixture(scope="session")
def app():
    return QApplication.instance() or QApplication([])


@pytest.fixture
def window(app, binary, ffmpeg, ffprobe, tmp_path):
    from vhsgui.app import MainWindow

    win = MainWindow(binary=binary, workdir=tmp_path / "work")
    win.resize(*DEFAULT_SIZE)
    win.show()
    yield win
    win.close()
    app.processEvents()


def _inside_scroll_area(widget: QWidget) -> bool:
    parent = widget.parentWidget()
    while parent is not None:
        if isinstance(parent, QScrollArea):
            return True
        parent = parent.parentWidget()
    return False


def test_the_output_controls_are_not_inside_the_scrolling_settings(app, window):
    for name in ALSO_PINNED:
        widget = window.findChild(QWidget, name)
        assert widget is not None, name
        assert not _inside_scroll_area(widget), f"{name} scrolls away with the settings"


def test_the_output_controls_are_on_screen_at_the_default_size(app, window, clip):
    window.open_input(str(clip))
    for _ in range(30):
        app.processEvents()
    area = window.rect()
    for name in ALWAYS_VISIBLE:
        widget = window.findChild(QWidget, name)
        top_left = widget.mapTo(window, QPoint(0, 0))
        assert widget.isVisible(), name
        assert area.contains(QRect(top_left, widget.size())), f"{name} is off screen at {top_left}"
