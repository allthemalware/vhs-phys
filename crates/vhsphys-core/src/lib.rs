//! Physical model of VHS record and playback.
//!
//! Signal flow (see `docs/ARCHITECTURE.md`):
//! frame -> NTSC raster -> record electronics -> record head/tape (relay model)
//! -> tape with defects and head/tape spacing -> playback head (Lorentzian
//! pulses, azimuth crosstalk) -> mechanics (timebase) -> deck electronics
//! (equaliser, dropout compensator, limiter, FM discriminator, emphasis) ->
//! sync/colour decoding -> frame.

pub mod condition;
pub mod config;
pub mod consts;
pub mod decode;
pub mod dsp;
pub mod electronics;
pub mod head;
pub mod mech;
pub mod pipeline;
pub mod tape;
pub mod tracking;
pub mod record;
pub mod rng;
pub mod video;
