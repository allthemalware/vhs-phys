//! VHS audio: the linear track read by the stationary audio/control head.
//!
//! The sound fails for the same physical reasons as the picture, because it comes from
//! the same simulated tape and deck: the capstan's speed error (`Mech::wow`/`flutter`)
//! is wow and flutter, the supply ripple that makes hum bars in the picture is hum in
//! the sound, head wear and the tape speed set the wavelength-limited bandwidth, tape
//! age and quality set the hiss, and defects lift the tape off the head (spacing loss
//! costs the high frequencies first, `54.6 d / lambda` dB). See
//! `docs/AUDIO.md` for the model and its parameters.

use crate::pipeline::Params;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioParams {
    /// When false the processor returns its input unchanged.
    pub enabled: bool,
}

impl Default for AudioParams {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// The audio path of the deck and tape described by a `Params`.
pub struct AudioProcessor {
    _state: (),
}

impl AudioProcessor {
    /// `channels` interleaved channels at `rate` Hz.
    pub fn new(p: &Params, rate: f64, channels: usize) -> Self {
        let _ = (p, rate, channels);
        unimplemented!("audio processor")
    }

    /// Process `interleaved` (frames x channels), whose first frame is absolute sample
    /// `start` of the programme (at the construction rate). Everything stochastic is a
    /// pure function of (seed, absolute sample index), so a processor started at any
    /// `start` reproduces the same tape. Calling `process` again continues the stream.
    /// Returns the same number of samples.
    pub fn process(&mut self, interleaved: &[f32], start: i64) -> Vec<f32> {
        let _ = (interleaved, start);
        unimplemented!("audio processor")
    }
}
