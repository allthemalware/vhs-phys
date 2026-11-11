# Architecture

## Timing grid

Everything lives on a line-locked grid so that line, field and carrier phases
are exact integers. The figures below are those of the 525-line engine; the 625-line
engine uses the same 2730 and 910 samples per line at its own line rate (see
"Two engines, one source").

| Domain | Rate | Samples per line | Used for |
|---|---|---|---|
| RF | 12 x fsc = 42.95 MS/s | 2730 | FM, tape, head, deck RF, discriminator |
| Baseband | 4 x fsc = 14.32 MS/s | 910 | video luma/chroma, display decode |

A field is 262.5 lines = 716 625 RF samples, a frame 1 433 250. Field `n` is
exactly one helical track; its playback head takes over 6.5 lines before the
field's vertical sync (`consts::switch_time`). Global sample indices (i64) are
the time axis for everything, so any process that is a function of global time
(timebase error, tape position) is reproducible and order independent.

## Two engines, one source

The raster (525 or 625 lines, the levels, the vertical interval, the colour-under carrier,
the track geometry) is a set of compile-time constants, so the model is written once and
compiled twice instead of carrying the standard around at run time. `consts.rs` holds the 525-line
set and `consts_pal.rs` the 625-line one; they define the same names with the same types.
`ntsc.rs` and `pal.rs` each declare every engine module from the same source files
(`engine!` in `lib.rs`, `#[path]` per module) against their own constants, giving
`vhsphys_core::ntsc::{consts, video, record, pipeline, ...}` and `vhsphys_core::pal::{...}`.
The crate root re-exports the `ntsc` engine, so `vhsphys_core::pipeline::Session` is the 525-line
`Session` and every existing path is unchanged. The 625-line engine costs nothing at run time: the
525-line path does not know it exists.

Inside an engine file, another engine module is `super::x` (so each engine sees its own
constants); a shared module is `crate::x`.

Shared, compiled once and common to both engines (nothing in them depends on the raster):

* `standard`: the television standards (`TvStandard`) and the PAL delay-line decoding rules
* `tube`: the camera pickup tube model
* `frame`: the RGB `Frame` (`video` re-exports it), so a frame made by one engine is a frame of the other
* `dsp`: filters and resamplers
* `rng`: the seeded random numbers
* `interference_params`: the outside world as parameters

Everything else in the table below is an engine module and exists once per engine. Unit tests
inside engine files that assert 525-line figures return early in the other engine.

## Modules (`crates/vhsphys-core/src`)

| Module | Responsibility |
|---|---|
| `consts` | timing and format constants with their sources (`consts_pal` for the 625-line engine) |
| `frame` | the RGB frame, shared by both engines |
| `standard` | television standards and PAL delay-line decoding (shared) |
| `tube` | camera pickup tube model (shared) |
| `dsp` | biquads, Butterworth, first-order emphasis sections, Kaiser FIR, cubic resampling |
| `rng` | seeded, order-independent random numbers (hash of seed/stream/index) |
| `video` | the raster of the engine's standard including vsync structure; RGB <-> YUV; encode/decode helpers |
| `record` | record deck chain (with the PAL V switch) and the saturating tape relay; produces `Transition`s |
| `emphasis` | the LP/EP sub-preemphasis (a level-dependent high-frequency boost, table 4 of ST 32) and its exact inverse on playback |
| `tape` | defect map in tape coordinates, spacing roughness, creases, edge profile |
| `tracking` | lateral head position and the weights of the tracks under the head |
| `mech` | timebase error: drift, drum, tape-path resonance, stick-slip, per-line, flagging |
| `head` | Lorentzian pulse rendering, azimuth smoothing, head amplifier noise, resonance |
| `electronics` | playback chain: equaliser, AGC, dropout compensator, discriminator, de-emphasis |
| `decode` | sync separator, AFC, clamp, colour demodulation, the deck's comb (1H, 2H at 625 lines), the television's colour decoding (NTSC, or PAL with or without the delay line) |
| `condition` | ageing laws: years/temperature/humidity -> component values -> parameters |
| `config` | TOML merging, `--set` overrides, refusing what the engine cannot play (another standard, EP at 625 lines) |
| `pipeline` | streaming session, calibration, per-field scheduling and caching |
| `format` | tape speed geometry: SP/LP/EP track pitch, angle, advance |
| `composite` | Y/C sum and separation, notch and combs |
| `audio` | the linear audio path (`audio.rs`): transport delay, loss, hiss, hum, dropouts |
| `interference_params` | the outside world as parameters: phone, radio, mains, magnets, couplings (`[est]`) |
| `interference` | fields of the outside sources and what each adds at its circuit node, as functions of absolute time |
| `crt` | the picture tube: Lorentz force on the beam integrated along its path, landing shift, brightness, stripe purity |

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

The luma passes these stages in order (the LP/EP pair only when `RecordParams.sub_emphasis` and
`DeckParams.sub_deemphasis` are set, which `Condition::to_params` does at LP and EP):

```
record     luma low-pass (baseband) -> x3 to RF -> sub-preemphasis (LP/EP) -> main emphasis -> white/dark clip -> FM
playback   discriminator -> main de-emphasis -> sub-deemphasis (LP/EP) -> coupling capacitor -> x1/3 to baseband
           -> video low-pass -> aperture corrector -> supply disturbances
```

The two emphasis pairs nest: the playback undoes the record stages in the reverse order, so the sub-deemphasis
sees what the sub-preemphasis produced (less what the clip and the FM path's bandwidth took) plus the tape's noise.

The outside world enters at the circuit nodes where it physically arrives (all functions of the global sample index,
so a split buffer or a render from any start gives the same result; with every source off nothing is called):

```
render_rf    tape signal x head_signal_gain (magnet at the drum)   -> head_front_end (noise)
             + rf_pickup_add (radio carrier in the head leads, dimmer burst)
decode       after the clamp: (y - clamp) (1 + video_gain) + video_add_ire (ground loop, rectified phone and AM, dimmer ring)
             after decode_ideal: crt::apply (magnet at the tube, stray mains field)
audio        pre-equaliser: audio_add pre_eq (rectified phone and AM, stray-field induction), tape gain (magnet)
             post-equaliser: audio_add post_eq (ground loop, dimmer click)
```

Calibration aligns the deck with every source off (on the bench), so a source is seen by the AGC and the slicers as it
would be in use, not absorbed into the alignment.

Frame `k` needs fields `2k..2k+2` (the last visible lines of a field are read by
the next head) and therefore video frames `k-2 .. k+2`: the output lags the input
by three frames. Intermediate results are cached per field and evicted as the
window moves, so memory is bounded (a few tens of MB) for any clip length.
`Chain` runs one `Session` per generation, and the decode may be followed by
`composite::roundtrip`.

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
LP and EP change the pitch and the advance (see docs/TAPE_SPEEDS.md).

## Graphical interface

The desktop front end lives in `python/vhsgui`:
* `model.py`: shared types, constants, and the condition field table.
* `backend.py`: non-Qt operations including tool discovery, probing, config text generation, command lines, and background jobs.
* `app.py`: the main application window and preview player.
* `__main__.py`: CLI entry point and screenshot support.
* `schematic/`: the phosphor schematic, a QPainter view drawn from data (`layout.json` and `glyphs.json` per page: the circuit, and the
  outside world in `schematic/outside/`). The format is in `docs/design/FORMAT.md`; the window mirrors every editor of a page and the
  panels through one path, so the views cannot disagree.

To preview an excerpt, the backend runs the CLI with `--start` beginning a few frames early (`LEAD_FRAMES` = 3). The first frames of a fresh render lack the neighbouring frames the model looks at, so they are dropped. Because the tape is a function of the absolute frame index, the remaining frames match a render of the whole file (about 38-41 dB PSNR against it, codec noise only; a one-frame offset gives about 17 dB).

## Extending

* A new degradation: add parameters to the relevant `*Params`, apply them where
  the physical effect enters (usually a `Controls` array in `render_rf`, or a
  stage in `electronics::process_segment`), and map a `Condition` field to them
  in `condition.rs`. Serde derives expose them to TOML automatically.
* PAL: the `pal` engine records and plays 625-line PAL (`PAL.md`). Standard-specific behaviour is
  chosen by `consts::STANDARD` or follows from a constant: the burst angle (`video`), the V switch
  and the eighth-of-a-turn carrier rotation (`record::v_switched`, `record::line_rotation`), the
  deck's burst reference and 2H comb and the television (`decode`), the slow-speed rules
  (`ST32_SLOW_SPEED_RULES`), EP (`HAS_EP`) and the mains (`MAINS_HZ_DEFAULT`). PAL-60 and
  composite PAL are still to come.
* S-VHS: FM 5.4-7.0 MHz, 1.6 MHz deviation, extra sub-emphasis, a higher RF
  sample rate.
