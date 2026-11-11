"""Command line entry point: python -m vhsgui [--schematic | --panels] [--page {circuit,outside}] [--binary PATH] [--screenshot PNG] [input]."""
from __future__ import annotations

import argparse
import shutil
import sys
from collections.abc import Sequence

EXIT_FAILURE = 1
EXIT_MISSING_TOOL = 2
SCREENSHOT_TIMEOUT_MS = 30_000  # longest wait for the first source frame
SCREENSHOT_POLL_MS = 50
SCREENSHOT_SIZE = (1280, 900)  # screenshots use a stable canvas regardless of the desktop

MISSING_BINARY = "vhsphys not found. Build it with: cargo build --release"
MISSING_FFMPEG = "ffmpeg not found. Install it with your package manager, for example: apt install ffmpeg"
MISSING_FFPROBE = "ffprobe not found. It comes with ffmpeg: install it with your package manager"


HELP_DESCRIPTION = """\
Desktop front end for vhs-phys. The window opens on the phosphor schematic
(the signal-chain page); the Panels / Schematic buttons switch views at any time."""
HELP_EPILOG = """\
examples:
  python -m vhsgui clip.mp4                 open a video on the schematic
  python -m vhsgui --page outside clip.mp4  start on the interference page
  python -m vhsgui --panels clip.mp4        start with the plain settings panels
  QT_QPA_PLATFORM=offscreen python -m vhsgui --screenshot out.png clip.mp4
"""


def _parse(argv: Sequence[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="python -m vhsgui",
        description=HELP_DESCRIPTION,
        epilog=HELP_EPILOG,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("input", nargs="?", help="video file to open")
    parser.add_argument(
        "--binary",
        metavar="PATH",
        help="the vhsphys executable (default: $VHSPHYS_BIN, then target/release, target/debug, then the PATH)",
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--schematic", action="store_true", help="start with the phosphor schematic (the default)")
    mode.add_argument("--panels", action="store_true", help="start with the settings panels instead of the schematic")
    parser.add_argument(
        "--page",
        choices=("circuit", "outside"),
        help="schematic page to start on: circuit (signal chain) or outside (interference); default: circuit",
    )
    parser.add_argument("--screenshot", metavar="PNG", help="save a picture of the window and exit (works with QT_QPA_PLATFORM=offscreen)")
    args = parser.parse_args(argv)
    if args.page is not None and args.panels:
        parser.error("--page cannot be used with --panels")
    args.schematic = not args.panels
    if args.page is None:
        args.page = "circuit"
    return args


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
    if args.schematic:
        window.set_schematic_page(args.page)
        window.set_schematic_mode(True)
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
    window.resize(*SCREENSHOT_SIZE)
    app.processEvents()
    if args.schematic:
        shown = window.schematic if window.schematic_page == "circuit" else window.outside
        shown.skip_boot()
    picture = window.grab()
    saved = picture.save(args.screenshot)
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
