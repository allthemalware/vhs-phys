//! The sub-preemphasis of the LP and EP modes (ST 32 clause 4.3.1.1.1; table 4 is in `format.rs`):
//! a level-dependent boost of the high frequencies of the luminance, applied before the main
//! emphasis on record and undone after the main de-emphasis on playback. The network is a
//! high-pass (and, where the format's response falls again above 1 MHz, a low-pass after it)
//! feeding a gain and a diode limiter, whose output is added to the signal. Small detail gets
//! the full boost; a large transition gets no more than the limiter can give, so the clip
//! levels are not reached by edges. The playback stage puts the same network in a feedback
//! loop and is its exact inverse, which is how the high-frequency noise of the slower tape is
//! taken out while edges keep their size.
//!
//! Placeholder: `for_speed` has no figures yet and both stages pass the signal through.

use crate::format::TapeSpeed;
use serde::{Deserialize, Serialize};

/// The figures of the network.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SubEmphasisParams {
    /// Corner of the high-pass, Hz.
    pub hp_hz: f64,
    /// Corner of the low-pass that follows it, Hz; 0 for none.
    pub lp_hz: f64,
    /// Gain between the filters and the limiter.
    pub gain: f64,
    /// Level at which the limiter saturates, IRE.
    pub clip_ire: f64,
}

impl Default for SubEmphasisParams {
    fn default() -> Self {
        Self { hp_hz: 1.0e6, lp_hz: 0.0, gain: 1.0, clip_ire: 10.0 }
    }
}

impl SubEmphasisParams {
    /// The figures that meet table 4 of the format for this speed; none at SP.
    pub fn for_speed(_speed: TapeSpeed) -> Option<Self> {
        None
    }
}

/// The record stage.
pub struct SubPreemphasis;

impl SubPreemphasis {
    pub fn new(_p: &SubEmphasisParams, _fs: f64) -> Self {
        Self
    }

    /// Set the state as if the input had been `x` forever.
    pub fn settle(&mut self, _x: f64) {}

    pub fn process(&mut self, _buf: &mut [f32]) {}
}

/// The playback stage: the inverse of [`SubPreemphasis`] with the same figures.
pub struct SubDeemphasis;

impl SubDeemphasis {
    pub fn new(_p: &SubEmphasisParams, _fs: f64) -> Self {
        Self
    }

    /// Set the state as if the input had been `x` forever.
    pub fn settle(&mut self, _x: f64) {}

    pub fn process(&mut self, _buf: &mut [f32]) {}
}
