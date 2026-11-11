//! Physical model of VHS record and playback.
//!
//! Signal flow (see `docs/ARCHITECTURE.md`):
//! frame -> NTSC raster -> record electronics -> record head/tape (relay model)
//! -> tape with defects and head/tape spacing -> playback head (Lorentzian
//! pulses, azimuth crosstalk) -> mechanics (timebase) -> deck electronics
//! (equaliser, dropout compensator, limiter, FM discriminator, emphasis) ->
//! sync/colour decoding -> frame.

pub mod audio;
pub mod camera;
pub mod composite;
pub mod condition;
pub mod config;
pub mod consts;
pub mod crt;
pub mod decode;
pub mod dsp;
pub mod emphasis;
pub mod electronics;
pub mod format;
pub mod head;
pub mod interference;
pub mod interference_params;
pub mod mech;
pub mod pipeline;
pub mod tape;
pub mod tracking;
pub mod record;
pub mod rng;
pub mod video;
