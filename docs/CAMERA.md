# Camera pickup tubes

By default the video that goes into the recorder is taken as it is. With a camera tube on,
it is first treated as what a three-tube Plumbicon camera made of the scene: every picture
element has a charge on the tube target that outlasts the frame, so moving highlights leave
lag and, when they are bright enough, comet tails. The model is in `tube.rs`; the stage that
runs it over a frame is `camera.rs`.

## Chain

The input frame is the camera's gamma-corrected output. Per element and colour channel:

1. `v = rgb / 255`; scene light `L = oetf_inverse(v) * 2^iris_stops` (BT.709 transfer, so
   `L = 1` is the light that gives peak white at an iris of 0).
2. The tube adds the charge `peak * L^0.95` (the light transfer exponent of the data sheet),
   plus the dark and bias charge, and the beam scans (below). The signal is the charge removed.
3. `lin = (signal - dark_signal) / peak`, where `dark_signal` is the signal of an element
   that has sat in the dark (the camera's black clamp), and `out = round(255 * oetf(lin))`.

The stage holds one charge per element and channel and starts every element at dark
equilibrium. It is built only when a tube is selected, so with `tube = "off"` (the default)
the picture is untouched and the golden renders do not change. The same code runs at 525/60
and 625/50: the time the beam spends on the picture per frame is computed from the engine's
raster constants.

Because the tube's exponent is 0.95 and not 1, a still scene is not returned exactly even
with the lag at rest: mid greys come out two or three levels brighter. Peak white, black
and the saturated primaries are unchanged.

## Physics

Each element of the target is a small capacitor. Light discharges it through the
photoconductor between scans (in the model: adds charge, counted positive), and the scanning
beam recharges it to the cathode potential. The charge the beam delivers is the signal. Two
limits of the beam make the picture remember the past.

**Discharge (capacitive) lag.** Near the cathode potential only the faster electrons of the
beam can land, because their energies are spread thermally. The landing current falls
exponentially as the element approaches the cathode potential: `i = I_b exp(u / V_T)` for
`u <= 0`, and `i = I_b` above. An element left slightly charged after a scan gives up the
rest over the next scans. In charge units per dwell (`q = C u / t_dwell`, in nA) one scan of
one element is exact:

1. above the cathode potential the whole beam lands: `r1 = min(q, I_b)`, `q -= r1`;
2. with the beam charge `I_b - r1` left, below it: `exp(-q'/Q_T) = exp(-q/Q_T) + (I_b - r1)/Q_T`.

`Q_T = C V_T / t_dwell` is the charge scale (`charge_scale_na`) and the signal is the charge
removed. A scan always leaves the element a little below the cathode potential, which is why
a tube at rest holds a negative charge (about -30 to -45 nA here, "dark equilibrium"), and why
the first frames of a new light are weak: that charge has to be worked off first. A light bias
adds a steady charge that keeps the elements off the slow part of the curve, which is how the
XQ1410 cuts the lag.

**Comet tails.** The beam is set to twice what peak white needs. A highlight beyond that
(the sun on chrome, a lamp, an overexposed face) leaves charge the scan cannot remove. It
accumulates until the element reaches the signal-plate voltage and photoconduction stops
(`saturation_na = C * 45 V / t_active`, about 4800 nA), and after the highlight has gone it
drains at the beam current per frame, reading above peak white (clipped white) until it is
gone. The tail is as many frames long as the beam current divides the stored charge. A
longer or brighter highlight stores more; a stronger beam (`beam_ratio`) shortens it.

**Anti-comet-tail (ACT) gun.** It recharges, during the line flyback, every element above the
level the read beam can stabilise, so nothing is left over: the stored charge is cut to the
beam current before the scan and the highlight never outlasts its frame. Only the ordinary
lag remains.

## Data

All from the Philips Data Handbook T10, Plumbicon camera tubes, 1985 (types XQ1020, XQ1410,
XQ1520, 30 mm tubes), except where marked as an estimate.

| Quantity | Value |
|---|---|
| Peak signal current | G 300 nA, R and B 150 nA |
| Beam current | twice what peak white needs (G 600, R and B 300 nA) |
| Light transfer exponent | 0.95 +/- 0.05, all three types |
| Dark current | at most 1 to 3 nA (1 nA used) [est] |
| Target (signal-plate) voltage | 45 V |
| Target capacitance | PbO layer 10 to 18 um, 12.8 x 17.1 mm scanned area, relative permittivity about 25 [est]: 3.2 nF |
| Light bias of the XQ1410 | R 3, G 2, B 3.5 nA |
| Lag, XQ1020 (standard gun) | build-up 60 ms: 85 / 95 / 70 %; decay 60 ms: 12 / 10 / 15 %; decay 200 ms: 4 / 3.5 / 5 % (R / G / B) |
| Lag, XQ1410 (XQ1020 with light bias) | build-up 98 / 98 / 95 %; decay 60 ms 8 / 7 / 11 %; decay 200 ms 3 / 2 / 3.5 % |
| Lag, XQ1520 (ACT gun) | build-up 85 / 95 / 70 %; decay 60 ms 13 / 9 / 15 %; decay 200 ms 3.5 / 3 / 5.5 % |

The lag figures are the data sheet's low-key measurement (signal 20 nA, beam 300 nA, 50 Hz):
decay is the residual signal above the dark level, as a percentage of the signal, 60 and
200 ms after the light is cut off (after 5 s of light); build-up is the percentage of the
final signal 60 ms after the light is switched on (after 10 s of dark). `tube::lag_measurement`
reproduces that measurement on the model, with the scans at multiples of the frame period
and the figure at 60 or 200 ms interpolated between them.

The PbO photoconductive lag is negligible (data sheet, Plumbicon notes 1.4), so the lag the
sheet shows is the beam's: it is worst at low key and reduced by light bias. The ACT gun's
flyback recharge uses a defocused beam of about 100 uA (notes 1.6); the model needs only that
it takes every element above the stabilised level back to it.

## Fitted charge scales

Only one figure is fitted: `Q_T` per channel. It is the least-squares fit, on the logarithm
of the two figures, of the decay lag at 60 and 200 ms of the tube's own data by
`lag_measurement` at 0.04 s: the XQ1020 data for the Plumbicon and the Plumbicon with light
bias (the same target and gun), the XQ1520 data for the ACT tube. Everything else is the data
sheet's operating point.

| Channel | `Q_T` XQ1020 (nA) | `Q_T` XQ1520 (nA) |
|---|---|---|
| R | 6.32 | 6.12 |
| G | 5.64 | 5.12 |
| B | 7.55 | 7.90 |

As a voltage, `V_T = Q_T t_active / C` is 0.05 to 0.07 V, the order of kT/e for a hot cathode
(0.086 V at 1000 K).

How the model meets the data (percent; the data sheet's figure in brackets; the model's
deviation from it for the decay):

| Tube, channel | Decay 60 ms | Decay 200 ms | Build-up 60 ms |
|---|---|---|---|
| XQ1020 R | 13.9 (12), +16 % | 3.58 (4), -11 % | 70.3 (85) |
| XQ1020 G | 12.3 (10), +23 % | 3.02 (3.5), -14 % | 74.8 (95) |
| XQ1020 B | 16.8 (15), +12 % | 4.60 (5), -8 % | 62.3 (70) |
| XQ1520 R | 13.4 (13), +3 % | 3.41 (3.5), -2 % | 71.6 (85) |
| XQ1520 G | 11.0 (9), +22 % | 2.59 (3), -14 % | 78.1 (95) |
| XQ1520 B | 17.6 (15), +17 % | 4.89 (5.5), -11 % | 60.0 (70) |
| XQ1410 R (not fitted) | 8.2 (8), +3 % | 0.63 (3), -79 % | 88.4 (98) |
| XQ1410 G (not fitted) | 8.3 (7), +19 % | 0.85 (2), -57 % | 87.8 (98) |
| XQ1410 B (not fitted) | 10.0 (11), -9 % | 0.87 (3.5), -75 % | 85.7 (95) |

The two decay figures of each tube are met within 23 % at 60 ms and 14 % at 200 ms. The
XQ1410 is not fitted: it is the XQ1020 target and gun with its bias current added, and the
model has to predict that the bias cuts the lag. It does, and at 60 ms by about the data
sheet's amount.

## Known limits

* The build-up figure comes out lower than the data sheet's (62 to 78 % against 70 to 95 %):
  the model's picture builds up more slowly than the tube's. The one charge scale per channel
  is fitted to the decay, which is what shows on moving highlights, and the build-up is left
  as it comes out.
* The slow tail with light bias is too short: at 200 ms the model leaves under 1 % where the
  XQ1410 data shows 2 to 3.5 %. That tail is the PbO photoconductive lag, which is left out
  because the data sheet calls it negligible for the plain tube; with bias it is what is left.
* Blooming by beam bending (a highlight spreading sideways) and burn-in are not modelled. The
  charge stays on its own element.
* The charge scales are fitted at 625/50 and used unchanged at 525/60. The model is per frame,
  so at 60 Hz the same lag takes less time: at 59.94 Hz the XQ1020 decay at 60 ms is 5 to 7 %.
* Only the three types of the data sheet. Saticon, Newvicon and Vidicon tubes (a photoconductor
  with a lag of its own) are not here.

## Why tails end magenta

The three tubes share one kind of target: the same capacitance and the same signal-plate
voltage, so the same charge at saturation. The red and blue tubes carry half the peak current
of the green with half the beam on it. At saturation they therefore store twice the overload
per unit of beam, and a white highlight's tail lasts about twice as many frames in red and in
blue (15 frames against 7 here for a saturating highlight at 4 times peak white). Their
fitted charge scales are also larger, so their lag is the longer after a briefer highlight.
Green runs out first and the tail ends in red plus blue: magenta.

A white square, 32 pixels wide, moving 30 pixels a frame across black, two stops
overexposed: behind its trailing edge the camera output stays full white for one frame
(30 pixels), then steps down by one frame (30 pixels) at a time through
R 56 G 35 B 61, then 25 / 11 / 29, 14 / 6 / 17, 10 / 4 / 12, 7 / 3 / 9: a purple tail above
16 levels for about 120 pixels and still a few levels 250 pixels back. The ACT tube has no
white frame: the first frame behind the square is already the lag only (R 97 G 65 B 108),
then 28 / 12 / 35 and falling the same way.

## Why there is no phosphor stage

An earlier version also smeared the decoded picture with a CRT phosphor afterglow of 40 % per
frame. The P22 phosphors of a colour tube decay in microseconds (blue and green) to about a
millisecond (red), far below the 20 ms of a field, so a display does not leave visible trails
and the stage had nothing to model. The trails on old recordings are the camera's, and they
are on the tape.

## Using it

    vhsphys run in.mp4 out.mp4 --preset worn --camera-tube plumbicon --iris-stops 2

`--camera-tube` takes `off`, `plumbicon` (XQ1020), `plumbicon-bias` (XQ1410) and
`plumbicon-act` (XQ1520). `--iris-stops N` is the exposure in stops: 0 is the light that
gives peak white, 2 overexposes by a factor of four, so that bright parts of the picture are
beyond what the beam can take off and leave tails. In a config file:

    [camera]
    tube = "plumbicon"
    iris_stops = 2.0
    beam_ratio = 2.0

The tube's memory starts at dark equilibrium when a session starts. A render that starts
part-way into a clip (`--start`) with a tube on therefore needs a few frames of pre-roll
before its first output frame to match a render of the whole clip; the lag of a long tail is
a few tens of frames.
