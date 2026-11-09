//! Head-to-track alignment. The head is `width` wide, tracks are `pitch` wide
//! and contiguous (no guard band; adjacent tracks differ by 12 degrees of
//! azimuth). The fraction of the head over each track decides how much of that
//! track's signal is read, so a tracking error produces a noise band that
//! sweeps across the picture (the head crosses onto an opposite-azimuth track),
//! ghosts from tracks of the same azimuth two fields away, and chroma
//! crosstalk that the +/-90 degree rotation normally cancels.

use serde::{Deserialize, Serialize};
use crate::consts::*;
use crate::head::STRIDE;
use crate::rng::Rng;

const STREAM_TRACKING: u64 = 41;
/// Neighbouring tracks that can be read: n-2 .. n+2.
pub const NEIGHBOURS: [i64; 5] = [-2, -1, 0, 1, 2];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TrackingParams {
    /// Mean lateral offset of the head from the track centre (um).
    pub offset_um: f64,
    /// Change of the offset from the start to the end of the track (um):
    /// tape-speed / track-angle mismatch or stretched tape.
    pub slope_um: f64,
    /// Peak of a parabolic bow along the track (um): track bend.
    pub bow_um: f64,
    /// Random per-track offset, rms (um): servo noise, tape edge wander.
    pub wander_um: f64,
}

impl Default for TrackingParams {
    fn default() -> Self {
        Self { offset_um: 0.0, slope_um: 0.0, bow_um: 0.0, wander_um: 0.0 }
    }
}

/// Lateral weight (fraction of the head width over the track) of track `n + j`
/// at every control stride of the window, for j in `NEIGHBOURS`.
pub fn lateral_weights(p: &TrackingParams, head_width_um: f64, seed: u64, n: i64, len_rf: usize, g0: i64) -> [Vec<f32>; 5] {
    let m = len_rf / STRIDE + 3;
    let mut rng = Rng::new(seed, STREAM_TRACKING, n);
    let wander = p.wander_um * rng.gauss() as f64;
    let g_in = switch_time(n);
    let w = head_width_um;
    let pitch = TRACK_PITCH_UM;
    let mut out: [Vec<f32>; 5] = std::array::from_fn(|_| vec![0.0f32; m]);
    for i in 0..m {
        let g = g0 + (i * STRIDE) as i64;
        let s = ((g - g_in) as f64 / FIELD_RF as f64).clamp(0.0, 1.0);
        let y = p.offset_um + wander + p.slope_um * (s - 0.5) + 4.0 * p.bow_um * s * (1.0 - s);
        let (h0, h1) = (y - w / 2.0, y + w / 2.0);
        for (k, j) in NEIGHBOURS.iter().enumerate() {
            let (t0, t1) = (*j as f64 * pitch - pitch / 2.0, *j as f64 * pitch + pitch / 2.0);
            let ov = (h1.min(t1) - h0.max(t0)).max(0.0);
            out[k][i] = (ov / w) as f32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_tracking_reads_only_the_own_track() {
        let w = lateral_weights(&TrackingParams::default(), 58.0, 1, 4, FIELD_RF, 0);
        assert!(w[2].iter().all(|v| (*v - 1.0).abs() < 1e-6));
        assert!(w[1].iter().chain(w[3].iter()).all(|v| *v == 0.0));
    }

    #[test]
    fn half_pitch_offset_splits_the_head_between_two_tracks() {
        let p = TrackingParams { offset_um: 29.0, ..Default::default() };
        let w = lateral_weights(&p, 58.0, 1, 4, FIELD_RF, 0);
        assert!((w[2][100] - 0.5).abs() < 1e-5 && (w[3][100] - 0.5).abs() < 1e-5);
        // weights always sum to one while the head is over recorded tape
        for i in [10usize, 1000, 9000] {
            let s: f32 = (0..5).map(|k| w[k][i]).sum();
            assert!((s - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn slope_sweeps_the_noise_band_across_the_picture() {
        // a 58 um ramp over the track: starts on track n, ends on track n+1
        // offset 29 um with a 58 um slope: head centre goes from 0 to +58 um (track n -> n+1)
        let p = TrackingParams { offset_um: 29.0, slope_um: 58.0, ..Default::default() };
        let g_in = switch_time(4);
        let w = lateral_weights(&p, 58.0, 1, 4, FIELD_RF, g_in);
        let at = |f: f64| (f * FIELD_RF as f64) as usize / STRIDE;
        assert!(w[2][at(0.05)] > 0.9);
        assert!(w[3][at(0.5)] > 0.4 && w[3][at(0.5)] < 0.6);
        assert!(w[3][at(0.95)] > 0.9);
    }
}
