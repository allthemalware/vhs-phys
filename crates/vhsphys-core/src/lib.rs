//! Physical model of VHS record and playback.
//!
//! Signal flow (see `docs/ARCHITECTURE.md`):
//! frame -> raster -> record electronics -> record head/tape (relay model)
//! -> tape with defects and head/tape spacing -> playback head (Lorentzian
//! pulses, azimuth crosstalk) -> mechanics (timebase) -> deck electronics
//! (equaliser, dropout compensator, limiter, FM discriminator, emphasis) ->
//! sync/colour decoding -> frame.
//!
//! The raster (525 or 625 lines) is a set of compile-time constants, so the engine is written
//! once and compiled twice: `ntsc` against `consts.rs`, `pal` against `consts_pal.rs`. The
//! crate root re-exports the 525-line engine, so `vhsphys_core::pipeline::Session` is
//! `vhsphys_core::ntsc::pipeline::Session`. What needs no raster constant is compiled once
//! and shared by both engines: `dsp`, `frame`, `interference_params`, `rng`, `standard`
//! and `tube`.

// Shared by both engines: nothing in them depends on the raster.
pub mod automation;
pub mod dsp;
pub mod frame;
pub mod interference_params;
pub mod rng;
pub mod standard;
pub mod tube;

/// Declares every engine module, each from the one source file, inside the module that calls
/// it. The argument is the constants file the engine is built against. Engine files reach each
/// other as `super::x`, so each copy of the engine sees its own constants; the shared modules
/// above are reached as `crate::x`.
macro_rules! engine {
    ($consts:literal) => {
        #[path = $consts]
        pub mod consts;
        #[path = "audio.rs"]
        pub mod audio;
        #[path = "camera.rs"]
        pub mod camera;
        #[path = "composite.rs"]
        pub mod composite;
        #[path = "condition.rs"]
        pub mod condition;
        #[path = "config.rs"]
        pub mod config;
        #[path = "crt.rs"]
        pub mod crt;
        #[path = "decode.rs"]
        pub mod decode;
        #[path = "electronics.rs"]
        pub mod electronics;
        #[path = "emphasis.rs"]
        pub mod emphasis;
        #[path = "format.rs"]
        pub mod format;
        #[path = "head.rs"]
        pub mod head;
        #[path = "interference.rs"]
        pub mod interference;
        #[path = "mech.rs"]
        pub mod mech;
        #[path = "pipeline.rs"]
        pub mod pipeline;
        #[path = "record.rs"]
        pub mod record;
        #[path = "tape.rs"]
        pub mod tape;
        #[path = "tracking.rs"]
        pub mod tracking;
        #[path = "video.rs"]
        pub mod video;
    };
}

/// The 525-line engine (NTSC raster, `consts.rs`).
pub mod ntsc;
/// The 625-line engine (PAL raster, `consts_pal.rs`).
pub mod pal;

// The root is the 525-line engine, so every existing path keeps working.
pub use ntsc::*;
