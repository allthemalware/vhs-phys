# SP, LP and EP

`Params.speed` (`condition.tape_speed`) selects the recording speed. The extra noise, the
crosstalk and the tighter tracking tolerance of the slow speeds are not added as effects:
they follow from narrower tracks and a narrower head.

## Geometry (from `format.rs`)

| speed | tape speed | advance / field | track angle | track pitch | adjacent track offset |
|-------|-----------|-----------------|-------------|-------------|-----------------------|
| SP    | 33.35 mm/s | 556.4 um       | 5.9694 deg  | 58.0 um     | 4095 RF samples (1.5 H) |
| LP    | 16.68 mm/s | 278.2 um       | 5.9523 deg  | 28.9 um     | 2047.5 (0.75 H)       |
| EP    | 11.12 mm/s | 185.5 um       | 5.9467 deg  | 19.3 um     | 1365 (0.5 H)          |

The drum and head speed do not change; only the tape moves slower, so the track is a
little shallower and the head path advances less across the tape between two fields.

## What depends on the speed

1. `tape.rs`: `Tape` knows the speed (`TapeParams.speed`, copied from `Params.speed` in
   the pipeline, not a user setting). Track angle and advance per field come from it, so a
   point defect slides 1.5 / 0.75 / 0.5 lines per field and a longer stretch of tape
   passes under the head in the same time.
2. `tracking.rs`: `lateral_weights` takes the speed and uses its pitch. A 8 um offset is
   14 % of an SP track, 28 % of an LP track and 41 % of an EP track.
3. `pipeline.rs`: the neighbouring tracks are rendered `adjacent_track_offset_rf()` away.
4. `condition.rs`: both heads are as wide as the track pitch (`width_um`). [est] The LP/EP
   head pair of a four-head deck is assumed matched to the track. `HeadParams::default()`
   stays 58 um.
5. `head.rs`, noise: the head noise is media dominated. Its power is proportional to the
   track width read, the signal power to its square, so
   `CNR = cnr_db + 10 log10(width / 58 um)`: -3.0 dB at LP, -4.8 dB at EP (data sheets
   quote about 3 and 5 dB). The media AM noise (an amplitude) scales with
   `sqrt(58 / width)`. At 58 um both factors are exactly 1, so SP is bit-identical.
6. Azimuth crosstalk: a neighbouring track is attenuated by the azimuth loss of the width
   the head overlaps it with, `weight * head width`, `sinc(pi w tan(12 deg) / lambda)`.
   The call already used the overlap (`hp.width_um * wmax`); with the narrower head it
   is now the right figure at every speed. A given tracking error overlaps the neighbour by
   the same number of um at any speed (so the same azimuth loss per um), but that overlap
   is a larger fraction of the narrow head, so the neighbour weighs more in the sum and
   more crosstalk gets through. The loss of a fixed fraction of a track shrinks with the
   width, which is why the chroma of LP and EP degrades faster.

## Format details at LP and EP

SMPTE ST 32 sets two extra record rules for the slow speeds (`format.rs`, wired in `condition.rs` and `record.rs`):

- **Carrier interleave (clause 4.3.1.2):** at LP and EP (`TapeSpeed::carrier_interleave_hz`), the FM carrier recorded by the channel-1 video head sits `fH / 2` higher than the channel-2 head's (`RecordParams.carrier_interleave_hz`). This makes adjacent-track crosstalk beat at odd multiples of `fH / 2`, so it alternates in polarity from line to line and the display's 1 H comb cancels it.
- **Colour burst amplitude (clause 4.3.1.3, with 4.3.3 for EP):** the colour burst is doubled before recording at SP (clause 3.9.2.1.3) and, through clause 4.3.3, at EP (`record.burst_boost = 2.0`), whereas clause 4.3.1.3 states that the burst amplitude is not doubled in the LP mode (`TapeSpeed::burst_doubled`, `record.burst_boost = 1.0`). The undoubled burst gives the LP decoder half the burst amplitude to lock on, so colour lock on a worn LP tape is weaker.

## Sub-emphasis at LP and EP (`emphasis.rs`)

ST 32 clause 4.3.1.1.1: at LP and EP the luminance is sub-preemphasised before the main
preemphasis, with the level-dependent response of table 4 (`format.rs`, `SUB_EMPHASIS_*`).
Small detail is boosted by up to 7 dB at a few MHz, a full-amplitude signal by about 1.5 dB.
The playback undoes it after the main de-emphasis, and with it takes out the high-frequency
noise the slower, narrower track adds, while a large edge, which the record stage hardly
boosted, keeps its size.

**Network.** A first-order high-pass at `hp_hz` (at EP a first-order low-pass at `lp_hz` after
it), a gain, a limiter standing in for the back-to-back diode pair, and the coupling high-pass
at `post_hp_hz` that the diodes drive, whose output is added to the signal:

    y = x + H2(L(gain * H1(x)))      L(u) = clip * u / (clip + |u|)

`H1` is the high-pass [then low-pass], `H2` the coupling high-pass (none when `post_hp_hz` is
0). All filters use the bilinear transform with prewarped corners. The limiter is odd,
monotone, of unit slope at zero and saturates at `clip_ire`. The coupling high-pass is what
lets a full-amplitude signal's boost grow with frequency, as table 4's 0 dB row does: a large
low-frequency sine is clipped to a near square wave, and the coupling network passes only its
edges, which sit near the sine's zero crossings and leave its crests, and so its peak to peak,
almost alone. A gain or clip level of 0 or less switches both stages off.

**Playback.** The same network in a feedback loop, `x + H2(L(gain * H1(x))) = y`, solved sample
by sample: each filter's output is split into the current sample's share and the past's, and
the loop becomes a quadratic in the limiter's input (the coupling network's direct term scales
the limiter's share), so one square root per sample solves it exactly. Record then play returns
the input to the rounding of the f32 signal (largest error 1.1e-6 IRE at LP and 7.6e-6 IRE at
EP on a test line of steps, mid and fine detail and noise). The rational limiter is chosen for
this: tanh, or a limiter with a sharper knee (which fits LP with more margin), has no closed
form, and an iterative solve inside the recursion cost several times the rest of the playback
chain.

**Where.** Record (`record.rs`): luma low-pass, sub-preemphasis, main emphasis, clip, FM.
Playback (`electronics.rs`): discriminator, main de-emphasis, sub-deemphasis, the deck's
coupling capacitor. `Condition::to_params` sets `record.sub_emphasis` and `deck.sub_deemphasis` to
`SubEmphasisParams::for_speed(tape_speed)`: the same figures on both sides, none at SP (so SP
is bit-identical), and no ageing drift yet.

**Figures** (`SubEmphasisParams::for_speed`), fitted to table 4 measured as the format
measures it (peak to peak of a sine, 0 dB at 10 kHz, levels 0 / -10 / -14 / -20 dB of the
140 IRE sync-to-white span, at 4 x the RF rate), maximising the smallest margin to a
tolerance (a multi-start search over all five figures):

| speed | `hp_hz`  | `lp_hz`  | `gain` | `clip_ire` | `post_hp_hz` | worst margin to table 4 |
|-------|----------|----------|--------|------------|--------------|-------------------------|
| LP    | 51.3 kHz | none     | 2.188  | 17.25      | 790 kHz      | +0.074 dB (-10 dB at 1 MHz: 2.63 against 2.2 +/- 0.5) |
| EP    | 1.05 MHz | 2.62 MHz | 3.28   | 17.1       | none         | +0.22 dB (-20 dB at 500 kHz: 4.72 against 6.0 +/- 1.5) |

Both meet every point of their table. LP's margin is thin: four points are within 0.09 dB of
a tolerance (-10 dB at 1 MHz, -14 dB at 500 kHz and 3 MHz, -20 dB at 50 kHz). At LP the first
high-pass only keeps the picture's level out of the limiter, and the coupling network shapes
the response; without it no figures of the network meet LP's table (best -0.27 dB, the 0 dB row
at 500 kHz). EP meets its table with the band-pass alone, centred near 1.7 MHz. At the chain's
own rate the responses move by at most 0.03 dB.

**Noise.** On a flat grey field (`pristine`, seed 3, frame 3), the luma noise with the stages is
0.54 of that without at LP (1.67 against 3.10 levels) and 0.47 at EP (1.67 against 3.58).

## Standard numbers and estimates

Standard: tape speeds, track pitches 58 / 28.9 / 19.3 um, the track angle at SP, the
12 deg relative azimuth. Derived: LP and EP angle and offsets (velocity vector, see
`format.rs`). [est]: heads matched to the track at every speed, the media-noise-dominated
scaling law (its 3 and 5 dB match typical data sheets), the media AM factor.

## Measured (`tests/common` luma_noise, flat grey 110, frame 3, seed 3)

RMS luma noise in levels; in brackets the mean over seeds 3, 4 and 5. Measured as
`tests/tape_speed.rs` (`a_slower_tape_is_noisier_until_the_format_steps_in`) measures it: six flat grey-110 frames through
`Session::run(config::build(Some(preset), "[condition]\ntape_speed = \"lp\"", seed))` and
`luma_noise` of frame 3, for each preset, speed and seed. "without" clears
`record.sub_emphasis` and `deck.sub_deemphasis` from the same parameters.

| preset | SP          | LP           | EP            | LP without   | EP without    |
|--------|-------------|--------------|---------------|--------------|---------------|
| good   | 2.98 (3.02) | 2.07 (2.40)  | 2.11 (2.24)   | 3.84 (4.16)  | 4.57 (4.70)   |
| worn   | 8.16 (8.78) | 5.80 (8.31)  | 7.47 (15.45)  | 11.55 (13.25)| 16.33 (20.86) |

With its sub-emphasis a slower tape is now quieter than SP on a flat field: the playback
stage takes out about half of the noise in the luma band, more than the 3 and 5 dB of carrier
to noise the narrower tracks lose. A worn LP seed (13.0) and a worn EP seed (30.3) with heavy
dropouts pull the worn means up.

Measured before the sub-emphasis (and before the burst and interleave rules): mean bar colour error of `pristine` (levels, 7 bars, 3 channels):
SP 3.8, LP 4.8, EP 5.2 (largest single error 12.5 / 14.4 / 13.7). An 8 um tracking error
on `good` costs 12.5 (SP), 18.9 (LP) and 22.4 (EP) levels of mean absolute error.
