# PAL VHS: the 625-line engine

`vhsphys_core::pal` is the whole model compiled against `consts_pal.rs` (see "Two engines, one
source" in `ARCHITECTURE.md`). It records a 625-line, 50-field PAL picture onto a VHS tape and plays
it back into a PAL television. The 525-line engine (`vhsphys_core::ntsc`, also the crate root) is
unchanged by it: every 625-line rule is chosen by `consts::STANDARD` or follows from a constant, and
the 525-line output stays bit-identical (`tests/golden.rs`).

`config::build` in the `pal` engine accepts only `standard = "pal"`, always decodes with a PAL
television (`decode.tv = "pal"`), and refuses `tape_speed = "ep"` and `composite.enabled` (see "Not
modelled"). The 525-line engine refuses `standard = "pal"` (that is this engine) and a PAL television
(PAL-60, not modelled yet).

## Sources

The figures and their sources are in the doc comments of `consts_pal.rs` and in `RESEARCH.md`
section 1.1:

* raster, levels, line timing, vertical interval, swinging burst: ITU-R BT.470 / BT.1700;
* luma FM (3.8 MHz sync tip, 4.8 MHz white, 1 MHz per volt), emphasis time constant, the
  (40 + 1/8) fH colour-under and the per-head rotation (`PAL_ROTATION = [0, -1]`): vhs-decode
  `format_defs/vhs.py`;
* drum, tape speed (23.39 mm/s), 49 um track pitch, track angle, SP and LP only: IEC 60774-1 via the
  service literature, with the head speed and the moving-tape angle derived from them;
* the 2H deck comb and the tape advance of 1.5 H per field: derived (below and in `consts_pal.rs`).

## What is physically different from the 525-line engine

| | 525 lines (NTSC) | 625 lines (PAL) |
|---|---|---|
| drum speed, head-to-tape speed | 29.97 rev/s, 5.84 m/s | 25 rev/s, 4.87 m/s |
| luma FM, sync tip to white | 3.4 to 4.4 MHz | 3.8 to 4.8 MHz |
| recorded wavelength at peak white | 1.33 um | 1.01 um |
| track pitch SP / LP / EP | 58 / 28.9 / 19.3 um | 49 / 24.4 um / none |
| colour-under | 40 fH = 629.4 kHz, line locked | (40 + 1/8) fH = 626.95 kHz |
| chroma rotation per line | channel 1 +90 deg, channel 2 -90 deg | channel 1 none, channel 2 -90 deg |
| deck chroma comb | 1H | 2H |
| burst | 180 deg | +135 / -135 deg on alternate lines |
| television | NTSC: the phasor is U + jV | PAL: V un-switched, delay-line averaging |
| mains | 60 Hz | 50 Hz |

**Slower head, shorter wavelength.** The drum turns once per frame, so the head crosses the tape at
4.87 m/s instead of 5.84 m/s, while the FM carrier sits 0.4 MHz higher. The recorded wavelength at
peak white is 1.01 um against 1.33 um. The playback model works in head travel per RF sample
(`UM_PER_SAMPLE`, 0.114 um against 0.136 um), so the same head-to-tape spacing costs more: the
spacing loss (about 54.6 d / lambda dB) is about a third larger. A clean head's output is normalised by
the deck's alignment, but extra spacing (wear, a clog, a crease) costs a PAL deck more than an NTSC
one. With the `worn` preset the clogged head's RF is about 14 % of the pristine level in PAL against
19 % in NTSC, and its dropout compensator is active for about 15 % of the field against 6 %.

**Narrower tracks.** 49 um at SP, and the heads are as wide as the track. The head noise is media
dominated and scales with the width read (see `TAPE_SPEEDS.md`), but the reference of that scaling
is the format's own SP pitch, so a PAL SP head has the same carrier-to-noise ratio as an NTSC SP
head `[est]`. The tape advance per field (467.8 um) is again 1.5 H of head travel, so neighbouring
tracks are line aligned as in the 525-line format.

**Colour-under and crosstalk.** The carrier is 40 + 1/8 cycles per line. The per-sample carrier table
keeps the whole 40 cycles, so it is the same on every line, and `record::line_rotation` adds the
carrier's extra eighth of a turn per line to the head's own rotation:
`2 pi (1/8 + q/4) n` for line `n`, with `q` = 0 for channel 1 and -1 for channel 2. Every term is
a whole number of eighths, so the phase is counted in eighths modulo 8 and stays exact for any line
number. Relative to the wanted track, the crosstalk from a neighbouring track (the other head's
rotation) turns -90 deg per line. The V switch flips the wanted chroma every line, so the 1H comb
of the 525-line deck would cancel V instead of the crosstalk. The PAL deck adds line n and line
n - 2 of the field: the crosstalk is 180 deg apart there and the switch state is the same.

**Line switch.** The camera's baseband is the unswitched (U, V); its burst sits at +135 deg (-U + V).
The recorder (`record::record_track`) inverts V, burst included, on every odd global line
(`record::v_switched`) before the record band-pass, as a PAL coder does before the signal reaches
the deck. A frame has an odd number of lines, so the sequence runs on from one frame into the next.

**Deck and television are kept apart**, because where a phase error enters decides whether PAL
cancels it:

* the deck converts the colour-under back with its APC locked to the played-back burst. It compares
  the burst with the angle that line should have (+135 deg unswitched, -135 deg switched) and turns
  the chroma back by the smoothed error. Its output is still V-switched, as a PAL deck's 4.43 MHz output is;
* the television's colour reference is `decode.tv_phase_deg` off (every line alike). With
  `delay_line = true` (the default) it decodes with `standard::pal_delay_line`, the average of the
  line and the line before it in the same field. A phase error then costs saturation (by cos phi)
  and keeps the hue. With `delay_line = false` (simple PAL, `standard::pal_simple`) the hue goes
  wrong in opposite directions on alternate lines (Hanover bars). The first line of each field has
  no line before it and is decoded as simple PAL.

**Raster.** 625 lines with the 625-line vertical interval (5 equalising, 5 broad, 5 equalising half
lines, line 1 opening with the broad pulses), 576 visible rows, 740 active samples, no setup. The
last visible line of the second field (PAL line 623) is a half line: its second half carries the
equalising pulses, so the right part of the frame's last row (from about mid line) is not recorded.

**Mains.** The condition's default mains frequency is 50 Hz (`MAINS_HZ_DEFAULT`). The presets give
an off-nominal mains frequency as an offset from the default (`ground-loop`: 0.6 Hz low), so the
525-line figures are unchanged.

## Estimates `[est]`

Carried over from the 525-line format without a 625-line source:

* the emphasis ratio (5) and the white and dark clip levels (as fractions of the sync-to-white span);
* the deck's circuit figures: playback equaliser peaks, head resonance, FM and luma filters,
  dropout compensator, AGC, chroma filters, APC and ACC loop speeds;
* the head switch lead (6.5 H before the vertical interval, as at 525 lines);
* the `Condition` laws and presets (head wear, clog, tape ageing). They were set against the 525-line
  picture. Because of the extra spacing loss above, `worn` and `bad` are harsher in PAL. `bad` loses
  most of the clogged head's field to the dropout compensator.

## Not modelled

* **Bruch blanking**: the burst suppression on four lines of the vertical interval that keeps the
  swinging-burst phase the same at the start of every field. Here every line outside the vertical
  interval carries burst, and the deck's APC knows each line's switch state from the line count.
* **PAL LP rules**: SMPTE ST 32's slow-speed luminance rules (the LP sub-emphasis, the fH/2 carrier
  interleave and the undoubled LP burst) are 525-line rules, and no 625-line source has been found
  (`ST32_SLOW_SPEED_RULES = false`). PAL LP is SP at half the tape speed: narrower tracks, more noise
  and crosstalk, the doubled burst, no sub-emphasis.
* **EP**: not part of the 625-line format; `config::build` refuses it.
* **Composite PAL**: the composite stage (`composite.rs`) is an NTSC encoder and decoder; with a PAL
  television `config::build` refuses `composite.enabled`.
* **PAL-60** (a 525-line tape into a PAL television): a later step; the 525-line engine refuses it.
* The 4.43 MHz subcarrier itself: as in the 525-line engine, the chroma is a complex baseband
  envelope from the camera to the television.
