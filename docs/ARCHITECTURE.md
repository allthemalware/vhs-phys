# Architecture

## Timing grid

Everything lives on a line-locked grid so that line, field and carrier phases
are exact integers.

| Domain | Rate | Samples per line | Used for |
|---|---|---|---|
| RF | 12 x fsc = 42.95 MS/s | 2730 | FM, tape, head, deck RF, discriminator |
| Baseband | 4 x fsc = 14.32 MS/s | 910 | video luma/chroma, display decode |

A field is 262.5 lines = 716 625 RF samples, a frame 1 433 250. Field `n` is
exactly one helical track; its playback head takes over 6.5 lines before the
field's vertical sync (`consts::switch_time`). Global sample indices (i64) are
the time axis for everything, so any process that is a function of global time
(timebase error, tape position) is reproducible and order independent.

## Modules (`crates/vhsphys-core/src`)

| Module | Responsibility |
|---|---|
| `consts` | timing and format constants with their sources |
| `dsp` | biquads, Butterworth, first-order emphasis sections, Kaiser FIR, cubic resampling |
| `rng` | seeded, order-independent random numbers (hash of seed/stream/index) |
| `video` | NTSC raster including vsync structure; RGB <-> YUV; encode/decode helpers |
| `record` | record deck chain and the saturating tape relay; produces `Transition`s |
| `tape` | defect map in tape coordinates, spacing roughness, creases, edge profile |
| `tracking` | lateral head position and the weights of the tracks under the head |
| `mech` | timebase error: drift, drum, tape-path resonance, stick-slip, per-line, flagging |
| `head` | Lorentzian pulse rendering, azimuth smoothing, head amplifier noise, resonance |
| `electronics` | playback chain: equaliser, AGC, dropout compensator, discriminator, de-emphasis |
| `decode` | sync separator, AFC, clamp, colour demodulation, comb |
| `condition` | ageing laws: years/temperature/humidity -> component values -> parameters |
| `config` | TOML merging, `--set` overrides |
| `pipeline` | streaming session, calibration, per-field scheduling and caching |

## Data flow per field

```
Session::process(batch)
  1 record_track(n)      parallel over tracks   video window -> Vec<Transition>   (+-2 neighbours kept)
  2 render_rf(n)         parallel over tracks   own track + overlapped neighbours -> RF buffer
                          controls (stride 16): timebase warp, extra spacing, oxide scale, lateral weights
  3 segment(n)           parallel over fields   splice heads n-1|n|n+1 at the switch points,
                          run the deck electronics (state warmed up on 2 lines either side)
  4 decode_frame(k)      parallel over frames   sync/AFC/colour decode of fields 2k, 2k+1
```

Frame `k` needs fields `2k..2k+2` (the last visible lines of a field are read by
the next head) and therefore video frames `k-2 .. k+2`: the output lags the input
by three frames. Intermediate results are cached per field and evicted as the
window moves, so memory is bounded (a few tens of MB) for any clip length.

## Key numerical decisions

* **Sub-sample transition times.** The record relay interpolates the exact
  crossing of the head current with the coercive threshold. Quantising reversal
  times to the 23 ns grid would add ~0.16 rad of phase noise at 3.9 MHz (CNR
  limited to ~15 dB). Readback pulses are evaluated at fractional positions.
* **Exact limiter + discriminator.** Clipping a sampled waveform has the same
  quantisation problem and aliases the limiter harmonics (3.9 MHz x 11 ~ fs).
  `electronics::discriminate` treats the limiter as a comparator with
  interpolated switching times and integrates the product of the limited wave
  with its delayed copy exactly (4x oversampled area integration, triangle
  anti-alias). For an ideal square wave the DC output is `1 - 4 f tau`.
* **Lorentzian pulses.** One reversal reads back as a unit-area Lorentzian with
  half-width `h = sqrt((g/2)^2 + (d+a)(d+a+delta))`; spectrum `exp(-k h)`. Spacing
  changes add to `h`, so time-varying spacing is exact per pulse and Wallace's
  law is a consequence, not an input.
* **Stateless processes.** Timebase error, tape defects and tracking are pure
  functions of (seed, global time or track index). Parallel scheduling cannot
  change a result.
* **Alignment by calibration.** `pipeline::calibrate` plays a clean black frame
  through the nominal deck and measures the RF level (sets the playback gain),
  the demodulated burst size (ACC reference) and the chroma path delay (the
  deck's luma delay line); worn parts then deviate from an aligned deck, as in
  life.

## Geometry (tape.rs, tracking.rs)

Tape coordinates `(u, v)` in micrometres; track `n` starts at
`u_n = position + n * 556.4 um` and runs at 5.97 deg, 96.8 mm long. Lateral
position maps linearly to picture line. Adjacent tracks are offset by exactly
1.5 H along the track (tape advance per field = 1.5 H of head travel), so they
are line aligned after the 0.5 H interlace offset; neighbour tracks are
rendered with a shift of `-+1.5 H`. The head covers `width / pitch` of one or two
tracks depending on the lateral offset; the opposite-azimuth track is read
through the 12-degree sinc filter (~ -28 dB at 4 MHz, ~ -14 dB at 629 kHz).

## Graphical interface

The desktop front end lives in `python/vhsgui`:
* `model.py`: shared types, constants, and the condition field table.
* `backend.py`: non-Qt operations including tool discovery, probing, config text generation, command lines, and background jobs.
* `app.py`: the main application window and preview player.
* `__main__.py`: CLI entry point and screenshot support.

To preview an excerpt, the backend runs the CLI with `--start` beginning a few frames early (`LEAD_FRAMES` = 3). The first frames of a fresh render lack the neighbouring frames the model looks at, so they are dropped. Because the tape is a function of the absolute frame index, the remaining frames match a render of the whole file (about 38-41 dB PSNR against it, codec noise only; a one-frame offset gives about 17 dB).

## Extending

* A new degradation: add parameters to the relevant `*Params`, apply them where
  the physical effect enters (usually a `Controls` array in `render_rf`, or a
  stage in `electronics::process_segment`), and map a `Condition` field to them
  in `condition.rs`. Serde derives expose them to TOML automatically.
* PAL: constants in `consts`, a per-line V-switch in `video`/`record`/`decode`,
  colour-under 626.953 kHz, FM 3.8-4.8 MHz, 625-line vsync pattern.
* S-VHS: FM 5.4-7.0 MHz, 1.6 MHz deviation, extra sub-emphasis, a higher RF
  sample rate.
