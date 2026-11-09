# Composite video

By default the simulator hands the deck's luma and chroma straight to the display, as an
S-Video connection would. With the composite stage on, the picture instead goes through a
composite cable: luma and chroma are summed on the NTSC subcarrier and a television's Y/C
separator splits them again. Its artefacts sit on top of the tape's own.

## Signal model

The baseband grid runs at 4 x fsc, so the subcarrier phase `phi` advances exactly 90
degrees per sample. A line is 227.5 cycles and a frame 119 437.5 cycles: the phase flips
from line to line and from frame to frame, and the pattern repeats every two frames (the
stage uses the absolute frame index, so a render started with `--start` matches). The
composite is `Y + cu sin(phi) + cv cos(phi)` with the burst on `-cu` (180 degrees), the
standard NTSC form. Only the active picture is re-encoded; sync, blanking and burst are
untouched, and the TV's subcarrier lock is taken as perfect. Chroma is demodulated with
that subcarrier and low-passed to 1.3 MHz on the I axis and 0.6 MHz on Q.

## Decoders

* `notch` (default): luma is the composite with a notch at fsc (Q about 1.7), chroma the
  matching band-pass. Fine luma detail near 3.6 MHz turns into colour (rainbows on
  stripes and zone plates) and is lost from the luma; colour edges along a line leave dots
  in the luma that crawl as the phase flips from frame to frame.
* `comb1h`: chroma is half the difference of the line and the line before it, luma half
  their sum. Fine horizontal luma detail stays luma and clean; colour that changes from one
  line to the next leaves a row of hanging dots in the luma, and luma loses some vertical
  detail. Rainbows remain only where luma detail changes from line to line (diagonals).
* `comb2h`: the same over three lines (`(2 c[n] - c[n-1] - c[n+1]) / 4`); dots straddle
  the edge on two lines, luma is softened 1-2-1 vertically.

A comb falls back to the notch where it lacks a neighbouring picture line of the same
field: `comb1h` on the first line of each field, `comb2h` on the first and the last.

## Using it

    vhsphys run in.mp4 out.mp4 --preset worn --set composite.enabled=true --set composite.decoder=comb1h

or, in a TOML config, a `[composite]` table with `enabled = true` and `decoder = "notch"`
(`"comb1h"`, `"comb2h"`). It is off unless asked for. On a worn tape the
effect is modest: VHS luma has little energy left at 3.6 MHz, so expect hanging dots on
horizontal colour edges and faint crawl rather than strong rainbows. Clean sources with
fine detail (text, stripes, test patterns) show the classic composite look.
