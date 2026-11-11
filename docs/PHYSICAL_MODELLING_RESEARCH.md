# Comparison beyond ntsc-rs, and a roadmap

`docs/COMPARISON.md` covers ntsc-rs. This page covers two more projects that model
part of the same ground - `vhs-decode` (reverse-decoding real RF captures) and the
Chow Tape Model (audio tape hysteresis) - and lists physics not yet in `vhs-phys`,
each with the circuit or field theory behind it and roughly where it would go.

## 1. vhs-decode and Chow Tape Model

| | `vhs-decode` | Chow Tape Model |
|---|---|---|
| Problem | decode real RF captured from a VCR's heads (an archival tool) | magnetic tape saturation and hysteresis, as an audio plugin |
| Domain | captured RF, 20-40 MS/s | continuous-time audio, VST3/AU/LV2/CLAP |
| Medium model | none (works from the real signal) | Jiles-Atherton hysteresis ODE: `dM/dH = (Man - M) / (k sgn(dH/dt) - alpha(Man - M)) + c dMan/dH` |
| Standard coverage | NTSC, PAL, SECAM, S-VHS, Video8, Hi8, LaserDisc | format agnostic |
| Ahead of `vhs-phys` on | demodulation profiles fitted to specific ICs (Sony CXA1201, Panasonic AN3248, JVC VC5036); PLL colour-under recovery that tracks large phase errors; every format above | minor hysteresis loops and magnetic memory as continuous state, not the relay's hard threshold |

`vhs-decode` is the reference for tuning against real tapes (see "Known limits" in
the README); `vhs-phys` generates the signal from a model instead of decoding a
capture, so the two are complementary rather than competing. The relay model
(`record.rs`) trades the hysteresis loop's fine structure for a form that holds up
in the video RF domain at 42.95 MS/s; see 2.2 below for the effect of moving toward
Chow's formulation.

## 2. Roadmap

Physics not yet implemented in `vhs-phys`, ordered roughly by how much of the picture
each would change. For broader project goals (tooling, plugins, GPU architecture, format support),
see `docs/ROADMAP.md`.

### 2.1 Non-linear noise reduction (HQ decks, SP only)

Not in SMPTE ST 32: a manufacturer feature on better SP decks, separate from the
LP/EP sub-preemphasis of table 4 (`emphasis.rs`, implemented). A high-pass feeds a
back-to-back diode limiter whose output is added to the signal: small high
frequencies get the full boost, a sharp edge forward-biases the diodes and is
boosted less, so the FM carrier is not over-deviated. Playback runs the inverse.
Patent sources only (US4597021A, US3944853), no standard figures to fit against;
building it means choosing a corner and a limit level by ear or against a real HQ
deck's measured response, then reusing the closed-form inverse already built for
the LP/EP stage (`emphasis.rs`).

### 2.2 Magnetic recording beyond the saturating relay

The relay (`record.rs`) switches instantaneously at the coercive threshold, which
is what lets chroma ride the FM bias as a duty cycle. A Preisach/switching-field
distribution would broaden the transition instead: `M(H) = Ms * tanh((H +/- Hc) /
dHc)`, `dHc` the switching-field spread (about 0.2-0.3 Hc for the tape's oxide).
That gives transition-width broadening as a function of recording current and
oxide, and so high-frequency recording loss and self-demagnetisation, as a
consequence of the model rather than a separate loss term - at the cost of solving
a differential equation per transition instead of a comparison.

### 2.3 Head pole-tip contour ("head bumps")

The Karlqvist field (`head.rs`) assumes an infinite pole face; a real pole tip is
50-100 um across, and long-wavelength flux couples into its edges, rippling the
response below 1 MHz. A multiplicative factor `1 + sum_m alpha_m cos(2 pi f L_m /
v) exp(-beta_m f)` on the readback sensitivity would add this, affecting the
colour-under carrier (629 kHz) and the audio low end.

### 2.4 Closed-loop chroma PLL

Chroma is demodulated against a fixed table with phase corrected per line
(`decode.rs`), not a closed loop. Real decks heterodyne colour-under to baseband
through a VCO locked to burst by an analog phase detector and a lag-lead filter.
Modelling that loop (not just its steady state) would let a sudden phase jump -
tape stretch, head switching - show up as genuine unlocking and hue smear rather
than a smoothed correction.

### 2.5 True 59.94 field-sequential input

`pipeline.rs` treats the source as progressive and duplicates it across both
fields of a frame; each helical pass is really an independent field. Taking
59.94 fps (progressive or already-interlaced) input and mapping each field to its
own pass would remove that duplication and let interlace comb filtering of a
moving picture behave as it does on a real deck. Candidate; see the handover.

### 2.6 Hi-Fi depth-multiplex audio

Hi-Fi records two FM carriers (1.3 / 1.7 MHz) with azimuth heads buried under the
video track before it is recorded. Not modelled: only the linear audio track
(`audio.rs`) exists. Adding it means a second pair of rotating heads in `record.rs`
and `head.rs` and an FM audio path in `audio.rs`; the degradation it would add -
carrier dropping below the limiter on tracking error, giving 59.94 Hz switching
buzz and a fallback mute to the linear track - is itself a reason to model it,
since it is a real and fairly distinctive failure mode.
