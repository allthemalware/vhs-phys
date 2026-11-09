# Turning the core into a plugin

The core has no I/O and no global state; the CLI is only a thin ffmpeg wrapper
around `pipeline::Session`.

## API

```rust
let params = vhsphys_core::config::build(Some("worn"), toml_text, seed)?;
let mut s = Session::with_start(params, start_frame);   // absolute timeline position
s.push(&frame);                 // 754x480 RGB, frames in timeline order
while s.ready() > 0 { for f in s.process(8) { /* output frame */ } }
s.finish();                     // flush the tail
```

* Input frames are always `ACTIVE_W x ACTIVE_H` (754 x 480) RGB8. A host plugin
  resamples to and from this size (the CLI uses ffmpeg's Lanczos/bicubic).
* Latency is three frames: `ready()` becomes positive once frame `k+3` has been
  pushed (or after `finish`).
* `with_start` makes tape position, defects and timebase functions of the
  absolute frame index, so the same tape is produced wherever the host starts
  rendering. Tested in `tests/pipeline.rs::windowed_session_matches_a_full_run`.
* `Params` is `Clone + Serialize + Deserialize`: every field is a UI parameter
  candidate. A sensible plugin UI is the `Condition` struct (sliders for tape
  age, storage, head wear, deck age and temperature ...) plus an "advanced" panel
  that exposes raw physical parameters.

## Hosting model

Hosts such as After Effects, Premiere or OpenFX request frames at arbitrary times,
often in parallel. Two workable designs:

1. **Sequential cache (recommended).** Keep one `Session` per effect instance.
   When a requested frame is `next_out`, continue; when the host seeks, create a
   new `Session::with_start(params, k - 3)` and push the three frames before it
   (cost: three extra frames per seek). Parameter changes create a new session
   (calibration takes ~0.1 s).
2. **Stateless per frame.** Build a short-lived session for `k-3 .. k+3` for every
   request: simple and thread safe, about 6-7x the per-frame cost.

`ntsc-rs` (https://github.com/valadaptive/ntsc-rs) is a good reference for the
plugin crates (OpenFX, After Effects, Premiere) built around a Rust effect core.

## Performance notes

~13 fps at 8 cores for the full chain. Hot spots: Lorentzian pulse rendering
(~135 k reversals per field), the discriminator and the RF filters. Options if
more speed is needed: a lower RF rate (8 x fsc, 1820 samples/line), f32 filters
with SIMD, caching the per-field transitions across parameter changes that only
touch playback, GPU offload of the pulse rendering.

## Things a plugin should add

* Field-accurate output (59.94 fps) for interlaced hosts: decode fields instead
  of frames; the per-field structure already exists.
* A progress/abort path (the session is already batch based).
* Audio degradation (wow/flutter from the same timebase error, Hi-Fi carrier
  dropouts, linear track hiss).
