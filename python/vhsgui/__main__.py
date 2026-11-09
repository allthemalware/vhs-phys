"""Command line entry point: python -m vhsgui [--binary PATH] [--screenshot PNG] [input]."""
from __future__ import annotations

import argparse
import shutil
import sys
from collections.abc import Sequence

EXIT_FAILURE = 1
EXIT_MISSING_TOOL = 2
SCREENSHOT_TIMEOUT_MS = 30_000  # longest wait for the first source frame
SCREENSHOT_POLL_MS = 50

MISSING_BINARY = "vhsphys not found. Build it with: cargo build --release"
MISSING_FFMPEG = "ffmpeg not found. Install it with your package manager, for example: apt install ffmpeg"
MISSING_FFPROBE = "ffprobe not found. It comes with ffmpeg: install it with your package manager"


def _parse(argv: Sequence[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(prog="python -m vhsgui", description="Desktop front end for vhs-phys.")
    parser.add_argument("input", nargs="?", help="video file to open")
    parser.add_argument("--binary", metavar="PATH", help="the vhsphys executable (default: search for it)")
    parser.add_argument("--screenshot", metavar="PNG", help="save a picture of the window and exit")
    return parser.parse_args(argv)


def _run_window(args: argparse.Namespace, binary: str, ffmpeg: str, ffprobe: str) -> int:
    # Qt is loaded only now, so `--help` and the tool checks work without a display.
    from PySide6.QtCore import QEventLoop, QTimer
    from PySide6.QtWidgets import QApplication

    from vhsgui.app import MainWindow
    from vhsgui.model import GuiError

    app = QApplication(sys.argv[:1])
    app.setStyle("Fusion")
    try:
        window = MainWindow(binary, ffmpeg=ffmpeg, ffprobe=ffprobe)
    except GuiError as err:
        print(str(err), file=sys.stderr)
        return EXIT_FAILURE
    if args.input:
        window.open_input(args.input)
    window.show()

    if not args.screenshot:
        return app.exec()

    if args.input and window.media is None:
        print(window.status_label.text(), file=sys.stderr)
        window.close()
        return EXIT_FAILURE
    loop = QEventLoop()
    ticker = QTimer()
    ticker.setInterval(SCREENSHOT_POLL_MS)
    limit = QTimer()
    limit.setSingleShot(True)
    limit.setInterval(SCREENSHOT_TIMEOUT_MS)

    def check() -> None:
        if window.preview.image() is not None or not window.source_pending:
            loop.quit()

    ticker.timeout.connect(check)
    limit.timeout.connect(loop.quit)
    ticker.start()
    limit.start()
    loop.exec()
    ticker.stop()
    limit.stop()
    app.processEvents()
    saved = window.grab().save(args.screenshot)
    window.close()
    if not saved:
        print(f"Cannot save {args.screenshot}", file=sys.stderr)
        return EXIT_FAILURE
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    args = _parse(argv)
    from vhsgui import backend
    from vhsgui.model import GuiError

    try:
        binary = backend.find_binary(args.binary)
    except GuiError as err:
        print(str(err), file=sys.stderr)
        return EXIT_MISSING_TOOL
    if binary is None:
        print(MISSING_BINARY, file=sys.stderr)
        return EXIT_MISSING_TOOL
    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None:
        print(MISSING_FFMPEG, file=sys.stderr)
        return EXIT_MISSING_TOOL
    ffprobe = shutil.which("ffprobe")
    if ffprobe is None:
        print(MISSING_FFPROBE, file=sys.stderr)
        return EXIT_MISSING_TOOL
    return _run_window(args, str(binary), ffmpeg, ffprobe)


if __name__ == "__main__":
    sys.exit(main())
