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

## Standard numbers and estimates

Standard: tape speeds, track pitches 58 / 28.9 / 19.3 um, the track angle at SP, the
12 deg relative azimuth. Derived: LP and EP angle and offsets (velocity vector, see
`format.rs`). [est]: heads matched to the track at every speed, the media-noise-dominated
scaling law (its 3 and 5 dB match typical data sheets), the media AM factor.

## Measured (`tests/common` luma_noise, flat grey 110, frame 3, seed 3)

RMS luma noise in levels; in brackets the mean over seeds 3, 4 and 5.

| preset | SP          | LP           | EP            |
|--------|-------------|--------------|---------------|
| good   | 2.98 (3.02) | 3.85 (4.03)  | 4.63 (4.74)   |
| worn   | 8.16 (8.78) | 11.66 (14.13)| 16.63 (25.17) |

The worn EP mean is pulled up by one seed (45.3) with a heavy dropout; the single-seed
figures are 13.6 to 16.6. Mean bar colour error of `pristine` (levels, 7 bars, 3 channels):
SP 3.8, LP 4.8, EP 5.2 (largest single error 12.5 / 14.4 / 13.7). An 8 um tracking error
on `good` costs 12.5 (SP), 18.9 (LP) and 22.4 (EP) levels of mean absolute error.
