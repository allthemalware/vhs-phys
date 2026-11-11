# VHS tape/deck physics: research notes

These notes record what the simulator models, why, and where each number comes
from. Confidence tags: **[std]** published standard, **[meas]** measured/tuned
against real captures by another project, **[svc]** service literature,
**[derived]** derived here from first principles, **[est]** engineering estimate
(tunable, no hard source).

## 1. Format parameters (NTSC VHS, SP mode)

| Quantity | Value | Source |
|---|---|---|
| Drum diameter / rotation | 62.00 mm, 1800 rpm (29.97 rev/s) | SMPTE ST 32-1998 [std] |
| Head-to-tape (writing) speed | 5.80 m/s nominal (5.837 m/s from pi*D*29.97) | ST 32 [std], [derived] |
| Tape speed | 33.35 mm/s (SP), 16.67 (LP), 11.12 (EP) | ST 32 [std] |
| Track pitch / width | 58 um (SP), 29 um (LP), 19 um (EP) | ST 32 [std] |
| Track angle | 5 deg 58' 9.9'' (moving), 5 deg 56' 7.4'' (stationary) | ST 32 [std] |
| Azimuth | +6 deg / -6 deg (adjacent tracks differ by 12 deg) | ST 32 [std] |
| Video area / track centre | 10.07 mm wide, centred 6.2 mm from the reference edge | ST 32 [std] |
| Track length | 10.07 mm / sin(5.97 deg) = 96.8 mm (= 16.7 ms at 5.8 m/s) | [derived] |
| Head switch point | 5-8 lines before the leading edge of vsync (6.5 H typical) | ST 32 [std]; Beeching [svc] |
| Tape tension at drum entry | 0.30-0.45 N | ST 32 [std] |
| Luma FM carrier | sync tip 3.4 MHz, white (100 IRE) 4.4 MHz, 1.0 MHz per 140 IRE | ST 32 [std], Sencore TT189 [svc] |
| (PAL VHS) | sync tip 3.8 MHz, white 4.8 MHz | JVC manual via search; vhs-decode [meas] |
| S-VHS | 5.4-7.0 MHz, deviation 1.6 MHz | Sencore [svc] |
| Luma pre-emphasis | first-order shelf, T = C*Rb = 1.3 us, Rb/Ra = 4 (gain 1+4 = 5 = 14 dB) | ST 32 [std]; vhs-decode [meas] |
| Clip levels (after emphasis) | white 160% of sync-tip-to-white span (155-200% allowed), dark -40% (-50..-30%) | ST 32 [std] |
| Sub-preemphasis (LP, EP only) | level-dependent HF boost before the main emphasis, undone after the main de-emphasis; response of a sine at 50 kHz..3 MHz for inputs of 0/-10/-14/-20 dB of the sync-to-white span, e.g. LP at 1 MHz 0.7/2.2/3.8/5.0 dB, EP peaks near 1 MHz (2.0..7.0 dB) and falls by 3 MHz; SP has none | ST 32 4.3.1.1.1, table 4 [std]; network form (high-pass, gain, diode limiter) [est], figures fitted to the table |
| LP/EP carrier interleave and burst | channel-1 head's FM carrier fH/2 above channel 2's at LP and EP; burst not doubled at LP (doubled at SP and, by 4.3.3, EP) | ST 32 4.3.1.2, 4.3.1.3 [std] |
| => FM extremes | 3.0 MHz (dark clip) .. 5.0 MHz (white clip) | [derived] |
| FM high-pass before recording | >17 dB down at 629 kHz, >10 dB down at 1 MHz | ST 32 fig. 10 [std] |
| Colour-under carrier | 40 x fH = 629.371 kHz (NTSC); 40 fH + 1.953 kHz = 626.953 kHz (PAL) | ST 32 [std]; vhs-decode [meas] |
| Chroma phase rotation | +90 deg per line on track 1, -90 deg per line on track 2 | ST 32 [std] |
| Chroma record | AM, with the FM luma acting as bias; burst amplitude doubled (+6 dB) | ST 32 [std] |
| Chroma level | played-back chroma 7-10 dB below chroma saturation | ST 32 [std] |
| Chroma input BPF | 3.58 MHz centre, -3 dB at 3.08 and 4.08 MHz | ST 32 [std] |
| Playback luma LPF | ~3.3 MHz corner (deck service data) | vhs-decode comment on a JVC manual [meas] |
| FM audio carriers | 1.3 / 1.7 MHz (NTSC Hi-Fi) | vhs-decode [meas] |
| Typical RF bandwidth | 6-7 MHz wide for PAL VHS | vhs-decode wiki [meas] |

### 1.1 625-line PAL VHS (the `pal` engine, `consts_pal.rs`)

| Parameter | Value | Source |
|---|---|---|
| Raster | 625 lines, 50 fields/s, fH = 15 625 Hz; 0.7 V picture, 0.3 V sync, no setup | ITU-R BT.470 / BT.1700 [std] |
| Line timing | sync 4.7 us, burst 5.6 us after sync for 10 cycles of 4.433 MHz, active 10.5..62.5 us (52 us) | BT.470 [std] |
| Vertical interval | 5 equalising, 5 broad, 5 equalising half lines; PAL line 1 opens with the broad pulses | BT.470 [std] |
| Burst | +/-135 deg (swinging with the V switch), 0.3 V p-p | BT.470 [std] |
| Drum, head speed | 62 mm at 25 rev/s = 4.87 m/s | [derived] |
| Tape speed, track pitch | SP 23.39 mm/s, 49 um; LP half; no EP in the format | IEC 60774-1 via service literature [std] |
| Track angle | 5 deg 57' 50.3" moving (5 deg 56' 7.4" stationary) | [derived] from the stationary angle, matches the published figure |
| Track alignment | tape advance per field 467.8 um = 1.5 H of head travel (467.5 um) | [derived] |
| Luma FM | sync tip 3.8 MHz, white 4.8 MHz (1 MHz per volt) | vhs-decode `format_defs/vhs.py` [meas] |
| Emphasis | 1.3 us as NTSC; ratio and clip levels as NTSC | vhs-decode [meas]; [est] |
| Colour-under | (40 + 1/8) fH = 626.953 kHz | vhs-decode ("40H + 1953") [meas] |
| Chroma rotation | channel 1 none, channel 2 -90 deg per line (`PAL_ROTATION = [0, -1]`) | vhs-decode [meas] |
| Deck chroma comb | 2H (crosstalk turns -90 deg per line relative to the wanted signal; the V switch flips every line) | [derived] |

### 1.2 Camera pickup tubes (`tube.rs`)

| Parameter | Value | Source |
|---|---|---|
| Lag definitions | decay: residual % of the signal 60 and 200 ms after the light is cut (after 5 s of light); build-up: % of the final signal 60 and 200 ms after (after 10 s of dark); at 50 Hz | Philips T10 1985, General section H [data] |
| Lag, XQ1020 R/G/B, low key, Is/Ib = 20/300 nA | build-up 60 ms 85/95/70 %; decay 60 ms 12/10/15 %, 200 ms 4/3.5/5 % | Philips T10 [data] |
| Lag, XQ1410 (internal light bias 3/2/3.5 nA) | build-up 98/98/95 %; decay 60 ms 8/7/11 %, 200 ms 3/2/3.5 % | Philips T10 [data] |
| Lag, XQ1520 (ACT gun) | build-up 85/95/70 %; decay 13/9/15 %, 3.5/3/5.5 % | Philips T10 [data] |
| Operating point | peak signal 300 nA (G), 150 nA (R, B); beam set to twice what peak white needs | Philips T10 [data] |
| Transfer gamma, dark current, target voltage | 0.95 +/- 0.05; <= 1..3 nA; 45 V | Philips T10 [data] |
| Target | PbO 10-18 um thick, scanned area 12.8 x 17.1 mm; permittivity ~25 [est] => 3.2 nF | Philips T10 [data]; [est] |
| Lag mechanisms | PbO photoconductive lag negligible; discharge lag set by layer capacitance and beam resistance, worst at low key, reduced by light bias | Philips T10, Plumbicon notes 1.4 [data] |
| Comet tails, ACT | highlights beyond the beam current lose stabilisation and take several scans to recover; the ACT gun recharges them during line flyback (about 100 uA defocused beam) | Philips T10, notes 1.6 [data] |
| CRT phosphor afterglow | P22 decays in microseconds (blue, green) to about a millisecond (red), far below one 20 ms field: not a source of visible trails | [lit] |

## 2. Playback signal path (what the deck does)

From Beeching, *Domestic Videocassette Recorders* (ch. 3, 10) [svc], the Backman
dropout study [svc], and vhs-decode:

1. Video head -> **rotary transformer** (one coupling per head, so the heads do
   not interact) -> **Q network** that peaks head+transformer to ~5 MHz -> low
   noise FET **preamp per head**. Q networks must be balanced between heads; an
   imbalance produces field-rate "flashing" at particular spatial frequencies.
2. **Head-switch** flip-flop driven from the drum. Both heads are in contact for a
   few lines (wrap angle > 180 deg) so the data is duplicated and one head is
   switched off and the other on, 6-8 lines before vsync (3-4 line tolerance).
3. Colour-under taken through a low-pass (removes FM); FM taken through an
   **equaliser** that lifts the upper sideband.
4. **Dropout compensator**: level detector + one-line (64 us) delay, switched in
   when the RF level falls below a threshold with hysteresis. Glass delay lines
   fade after ~7-10 recirculations; CCD cyclic compensators repeat indefinitely
   (deck chroma is killed during a dropout). Over-sensitive compensators create
   extra short glitches.
5. Heavy **limiter** (removes head-bounce AM, hides wear) -> FM demodulator
   (balanced modulator / delay-line detector) -> low-pass doubling as
   **de-emphasis** -> aperture corrector ("crispener") -> chroma added -> output.
6. **Colour**: replay colour-under -> ACC from the (doubled) burst -> APC loop
   locks the 4.21 MHz conversion oscillator to burst -> up-convert to 3.58 MHz ->
   **1 H comb filter** that cancels adjacent-track crosstalk (relies on the
   +/-90 deg rotation).

## 3. Magnetic physics used by the model

### 3.1 Playback (reciprocity) [derived; DAFx-2019 Chowdhury eqs. 1-14 confirm]

Head voltage is `V = -N dPhi/dt`. By reciprocity the readback of a magnetisation
pattern M(x) is its x-derivative convolved with the head sensitivity function.
For a thin layer, with k = 2 pi f / v:

```
|H(k)| = k * exp(-k d) * (1 - exp(-k delta)) / (k delta) * sinc(k g / 2)
         spacing        thickness                      gap
```

- Spacing loss = `54.6 d / lambda` dB (= 20 log10(e) * 2 pi d / lambda) [std, confirmed by search].
- Gap loss = sinc(pi g / lambda), infinite null at g = n lambda.
- Thickness loss uses the *recorded* depth, which at short wavelengths is far
  below the layer thickness (layer ~2.5-3.6 um [patents], recorded depth ~ lambda/2 pi).
- Karlqvist head field: `Hx = (H0/pi)[atan((g/2+x)/y) + atan((g/2-x)/y)]`.

### 3.2 Readback pulse of one transition [derived, checked numerically in `python/vhslab`]

An arctangent transition `M = (2 Mr/pi) atan(x/a)` read through the Karlqvist
head is a Lorentzian: `e(x) = (1/pi) h / (h^2 + x^2)` with
`PW50 = 2h = sqrt(g^2 + 4 (d+a)(d+a+delta))`. Properties used:

- unit **area** (independent of spacing); peak = 1/(pi h), so extra spacing
  broadens and lowers the pulse;
- spectrum `exp(-k h)`: this *is* Wallace's law with effective spacing h;
- Lorentzians compose: spacing change `dd` adds to the half-width, so
  `h(t) = h0 + dd(t)` is exact for the spacing term.

Because FM is recorded near saturation, the tape is almost a **two-state
medium**; the playback RF is then a train of alternating-sign Lorentzians at the
transition times. That avoids a stiff hysteresis ODE at 43 MS/s.

### 3.3 Record process: relay (Preisach) model and why chroma works [derived]

A saturating record head switches the local magnetisation when the head field
crosses the coercive field, i.e. a relay with thresholds +/-h acting on the
record current. For `i = A cos(phi) + c(t)` the rising crossing moves earlier by
`(c/A)/sin(phi)` and the falling crossing later by the same amount: the
**duty cycle** of the two-state waveform is modulated by c(t). The low-frequency
magnetisation is therefore proportional to the chroma, which is how an AM
chroma signal is recorded "with the FM luma acting as bias" (ST 32 7.5.1.2.2).
The same mechanism produces the observed intermodulation (`fy - 2fc` beat,
"herringbone" when the chroma record level is wrong). Sub-sample crossing times
are kept so transition-timing quantisation does not add jitter noise (at 43 MS/s
a 1-sample quantisation would limit CNR to ~15 dB).

Jiles-Atherton parameters for gamma-Fe2O3 (Ms 3.5e5 A/m, k 27 kA/m, a 22 kA/m,
c 0.17, alpha 1.6e-3) are from the DAFx paper; the relay model is the
saturation limit of that model and is what the prototype uses.

### 3.4 Azimuth loss and adjacent-track crosstalk [derived]

Reading a track whose azimuth differs by theta: loss = sinc(pi w tan(theta)/lambda)
with w the track width. With w = 58 um, theta = 12 deg:

| signal | lambda | loss |
|---|---|---|
| FM luma 4 MHz | 1.46 um | ~ -28 dB |
| colour-under 629 kHz | 9.3 um | ~ -14 dB (hence the phase-rotation trick) |
| LP mode (w = 29 um), 629 kHz | 9.3 um | ~ -7.6 dB |

In the time domain this is a boxcar of width `w tan(theta) / v` (~2.1 us).

**Track alignment:** tape advance per field = 33.35 mm/s / 59.94 = 556.4 um, and
1.5 H of head travel = 1.5 * 63.556 us * 5.837 m/s = 556.5 um. Adjacent tracks
are therefore offset by exactly 1.5 H along the track, which together with the
0.5 H interlace offset leaves them **line aligned**; 40 colour-under cycles per
line make a 1.5 H offset an integer number of cycles. This is why the +/-90 deg
rotation turns crosstalk into a sign that alternates every line and a 1 H comb
cancels it.

### 3.5 Geometry => picture position [derived]

The head sweeps the tape diagonally; **lateral tape position maps linearly to
picture line**. Consequences the model relies on:

- a longitudinal crease/scratch/edge defect appears as a streak at a *fixed
  picture height* on every field;
- low back-tension or a dirty entry guide raises spacing at the drum entry and
  causes noise at the *top* of the picture (Beeching fault table);
- a point defect crossing successive tracks slides by ~1.5 H per field.

Foil-bearing theory (Eshel & Elrod) gives the air film over the drum as
`h ~ 0.643 R (6 eta U / T)^(2/3)` ~ 15 um at VHS tension; heads protrude through
it. Treated here as a phenomenological tension-to-spacing map [est].

## 4. Degradation catalogue

### 4.1 Tape

| Mechanism | Physical cause | Model |
|---|---|---|
| Dropouts | shed oxide, embedded debris, head-tip collisions; length distribution rises as length falls down to 1-2 us, none < 0.5 us (RF cannot decay faster); depth < 10 dB is hidden by the limiter | Poisson defect map in tape coordinates (u, v), power-law size, void vs debris |
| "Bathtub" | more dropouts at both tape ends (hub pressure) | density multiplier vs tape position |
| Re-run cleaning | repeated playback removes loose debris, up to 10x fewer | `passes` parameter scales debris density |
| Creases / scratches | lost contact immediately before and after a ridge; fixed picture height | longitudinal line defects with spacing halo |
| Edge damage / curl | spacing grows near tape edges => noise at top/bottom | spacing profile along the track |
| Sticky shed / binder hydrolysis | coating sheds, head clogs | raises debris and head-clog rate |
| Magnetic loss | demagnetisation, thermal decay (small for Co-gamma-Fe2O3) | amplitude/HF scale [est] |
| Media noise | finite particle count | additive + AM noise ahead of the preamp |

Dropout depth reference: artificial -16 dB, 30 us dropouts used for compensator
alignment; count definitions vary (14 dB/15 us BASF, 20 dB JVC, 18 dB patents);
spec sheets cap at < 30 per minute [Backman; patents].

Tape: Co-modified gamma-Fe2O3, Hc ~ 50-60 kA/m (650-750 Oe) [patents/Birmingham
notes], layer 2.5-3.6 um [patents].

### 4.2 Heads and drum

| Mechanism | Model |
|---|---|
| Head wear (lower output, higher spacing) | per-head spacing offset + gain |
| Head clog (one field far noisier) | per-head large spacing offset, slowly varying |
| Q-network imbalance (field-rate flashing) | per-head peak frequency/gain mismatch |
| Rotary transformer dirt/wear | per-head gain ripple at drum rate |
| Drum bearing wear/runout | timebase component at 30 Hz and harmonics |

### 4.3 Mechanics and timebase

Sources [svc: repairfaq, Beeching]: drum rotation error, capstan flutter,
tape-tension variation (back tension 28-45 g/cm, take-up 80-220 g/cm; low values
shift colour vs luma "periodically over minutes"), stick-slip "scrape flutter"
above 100 Hz, head-switch step. Modelled as a sum of (a) servo-limited drum
error, (b) a second-order tape-spring resonance driven by noise and stick-slip
bursts, (c) capstan wow, and (d) a per-head offset step at the head switch with
exponential recovery. A time-base error `e(t)` shifts each transition time, so
line jitter, DC (brightness) shifts from frequency scaling, and chroma phase
errors all emerge.

### 4.4 Electronics

| Component ageing | Mechanism | Effect modelled |
|---|---|---|
| Aluminium electrolytics | electrolyte loss: C down (fail at ~-20%), ESR up 5-20x; life halves per +10 C | PSU ripple, coupling-cap tilt |
| PSU ripple | `Vr ~ I/(2 f C) + I*ESR`; ripple at 2x mains (100/120 Hz) beats with the field rate => slowly rolling hum bars (0.12 Hz at 60 Hz mains vs 59.94 Hz fields) | additive + FM-pushing hum |
| SMPS ripple | switching 30-100 kHz leaks into video as fine herringbone/diagonal lines | narrowband carrier ripple |
| Carbon-film / carbon-comp resistors | humidity drift (usually upward), excess 1/f noise (carbon comp worst) | emphasis/EQ corner drift, noise |
| Presets/trimmers | contact noise, heat drift | FM level, clip level, centre-frequency drift |
| Demod tank detuning | L/C drift | S-curve centre shift (brightness), nonlinearity |
| Emphasis mismatch between decks | R/C drift on record vs play deck | overshoot / softness |
| Low supply rail | clipping headroom drops | modulator overload, negative picture |

Fault-to-symptom entries from Beeching ch. 10 [svc] used as cross-checks:
asymmetrical FM carrier -> white smearing + odd black lines; failed
demodulator IC -> streaky picture with black speckle; wrong chroma record level
-> herringbone on colour; excessive drum-motor ripple (>300 mV) -> lateral
wobble; low back tension / dirty entry guide -> severe noise at the top;
tape-tension variation -> periodic colour/luma misregistration.

## 5. FM physics the model gets "for free"

Because the chain is simulated as an actual FM signal (modulate -> channel ->
limit -> discriminate), the following are not painted on: triangular (f^2) noise
spectrum, FM threshold "sparkle" clicks when CNR falls below ~10-12 dB,
bandwidth-limited edge ringing, sideband-truncation streaking, and the
differential phase/gain from carrier-dependent head response.

## 6. Not modelled in this prototype (next steps)

SECAM and S-VHS, composite PAL, the PAL LP record rules, Bruch burst blanking, LP/EP track geometry, linear audio and Hi-Fi audio, the
luma noise canceller of HQ decks (SP, playback only) and S-VHS sub-emphasis, tape
print-through, a full Jiles-Atherton record model, detailed head contour effect,
and an interlaced 59.94 fps output mode.

## 7. Sources

- SMPTE ST 32-1998 (VHS, type H), https://pub.smpte.org/doc/st32/19980923-pub/st0032-1998_stable2004.pdf
- vhs-decode (format parameters, tuned on RF captures), https://github.com/oyvindln/vhs-decode
- Sencore Tech Tips TT189 / TT135 / TT198, https://xdevs.com/doc/Sencore/Sencore_Tech_Tips/
- S. Beeching, *Domestic Videocassette Recorders*, https://worldradiohistory.com/BOOKSHELF-ARH/Servicing/Domestic-Videocassette-Recorders-Steve-Beeching.pdf
- L. Backman, *Video Signal Dropouts in Analog VCRs*, https://www.digiommel.fi/images/Video%20Signal%20Dropouts%20in%20Analog%20VCRs.pdf
- J. Chowdhury, *Real-time Physical Modelling for Analog Tape Machines*, DAFx-2019, https://dafx.de/paper-archive/2019/DAFx2019_paper_3.pdf
- J. Wheeler, *Videotape Preservation Handbook*, https://amianet.org/wp-content/uploads/Resources-Guide-Video-Handbook-Wheeler-2002.pdf
- Capacitor lifetime (10 C rule), Chemi-Con FAQ, https://www.chemi-con.co.jp/en/faq/detail.php?id=29BDBH5
- RepairFAQ VCR notes, https://www.repairfaq.org/sam/vcrfaq.htm
- ntsc-rs (signal-level VHS/NTSC effect, for comparison), https://github.com/valadaptive/ntsc-rs
- vhs-decode PAL VHS parameters, https://github.com/oyvindln/vhs-decode/blob/vhs_decode/vhsdecode/format_defs/vhs.py
- ITU-R BT.470 / BT.1700, conventional analogue television systems (625-line timing and levels)
- Philips Data Handbook T10, Plumbicon camera tubes and accessories, 1985, https://frank.pocnet.net/other/Philips/elcoma/AdK/Philips_ElectronTubes_T10_PlumbiconCameraTubes_more_1985_FP_AdK.pdf
- BBC Engineering Training information sheet, *The Plumbicon camera tube*, https://www.bbceng.info/additions/2020/ETD_info_sheets/Video/Cameras%20&%20Monitors/ETD_Info_Sheet_XXX_The%20Plumbicon%20Camera%20Tube.pdf

## 8. Findings from building and validating the model

Things the physics forced that a filter-chain approach would never have shown.

1. **The deck needs an RF level control.** With head wear the carrier falls 6-16 dB
   below alignment. Without a slow AGC ahead of the dropout compensator, the
   compensator (trip ~ -10 dB) fires continuously and the picture is replaced by
   recirculated garbage. Real decks limit and level the RF (Beeching: limiting
   "to eliminate amplitude variations ... head to tape bounce and head wear").
2. **Limiting a sampled signal is not limiting.** Clipping the 43 MS/s waveform
   snaps the edges to the sample grid (6.7 ns of jitter, ~0.16 rad at 3.9 MHz)
   and aliases harmonics (11 x 3.9 MHz ~ fs). The discriminator is therefore
   evaluated in continuous time from interpolated crossings.
3. **The FM path must be high-passed before the limiter.** Otherwise the 629 kHz
   colour-under phase-modulates the carrier (AM-PM conversion in the limiter)
   and appears as luma ripple at 0.5-1.4 MHz.
4. **The chroma path lags luma by ~25 samples (1.7 us)** through its narrow
   filters; decks carry a luminance delay to compensate, and the burst gate of a
   decoder must look where the burst really is. Measured at alignment.
5. **Timebase error must be smooth.** A step in the displacement (even 22 ns) is a
   phase jump; the discriminator turns it into a spike at the sync edge. Per-line
   jitter is interpolated between lines.
6. **Sync detection must be tip referenced.** The fall from a bright picture into
   the front porch is steeper than the sync pulse; separators slice relative to
   the sync tip (the one level the picture cannot go below). A bad separator
   reproduces "flagging" at the top of the field by itself.
7. **Hum bars need gain modulation.** Additive hum is removed by the TV's back
   porch clamp. Visible bars come from the supply modulating the gain of the video
   stages, which moves whites while black stays clamped.
8. **Chroma is recorded as FM duty cycle**, with low-frequency gain
   `dM/dc = 2 / (pi A sqrt(1 - (h/A)^2))`; checked numerically in
   `record::tests::chroma_is_recorded_as_duty_cycle_of_the_fm_carrier`.
9. **Pristine response.** With the default parameters the end-to-end luma response
   is -3.5 dB at 2 MHz and -18 dB at 3 MHz (the "240 line" limiting resolution
   quoted for VHS by Sencore), and chroma is -7 dB at 0.6 MHz. These come out of
   the chain (record high-pass cutting the lower FM sideband, head loss on the
   upper sideband, de-emphasis, filters); they are not set directly.

Open questions worth testing against real captures (vhs-decode RF captures would
allow it): the true record/playback emphasis non-linearity, head resonance and
equaliser shapes of specific decks, real dropout length statistics per tape type,
and the timebase error spectrum of real transports.
