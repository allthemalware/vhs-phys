//! The sub-preemphasis of the LP and EP modes (ST 32 clause 4.3.1.1.1; table 4 is in `format.rs`):
//! a level-dependent boost of the high frequencies of the luminance, applied before the main
//! emphasis on record and undone after the main de-emphasis on playback. Small detail gets the
//! full boost; a large transition gets no more than the limiter can give, so the clip levels
//! are not reached by edges. The playback stage puts the same network in a feedback loop and is
//! its exact inverse, which is how the high-frequency noise of the slower tape is taken out
//! while edges keep their size.
//!
//! The network: a high-pass (and, where the format's response falls again above 1 MHz, a
//! low-pass after it) feeding a gain and a diode limiter; the limiter drives a coupling
//! high-pass, whose output is added to the signal:
//!
//! ```text
//! y = x + H2(L(gain * H1(x)))      H1 = high-pass [then low-pass], H2 = coupling high-pass
//! ```
//!
//! The coupling network is what lets a full-amplitude signal's boost grow with frequency, as
//! table 4's 0 dB row does: the limiter clips a large low-frequency sine to a near square wave,
//! and the coupling high-pass passes only its edges, which sit near the sine's zero crossings
//! and so leave its crests, and its peak-to-peak size, almost alone.
//!
//! All filters are first-order sections discretised with the bilinear transform, their corners
//! prewarped, so the response at the chain's rate matches the analogue network's at the corner
//! and stays within a few hundredths of a dB of it up to 3 MHz.
//!
//! The limiter is the rational soft limiter `L(u) = clip * u / (clip + |u|)`: odd, monotone,
//! with unit slope at zero, continuous slope everywhere, and approaching `clip` as the diode
//! pair's output does. With the coupling high-pass it meets table 4 at both speeds. Its virtue
//! over `clip * tanh(u / clip)`, or a limiter with a sharper knee (which would leave more
//! margin at LP), is the playback: the loop is a quadratic in the limiter's input, so the
//! inverse is exact in closed form and costs one square root a sample, where an iterative
//! solve inside the recursion would cost several times the rest of the playback chain.

use crate::format::TapeSpeed;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// LP figures, fitted to the LP half of table 4 (measured as the format prescribes). The first
/// high-pass only keeps the picture's level out of the limiter; the coupling high-pass gives
/// the rise of the response with frequency, after the limiter, so a full-amplitude signal's
/// boost still grows with frequency as the table's 0 dB row does.
const LP_FIGURES: SubEmphasisParams =
    SubEmphasisParams { hp_hz: 51.3e3, lp_hz: 0.0, gain: 2.188, clip_ire: 17.25, post_hp_hz: 790.0e3 };
/// EP figures, fitted to the EP half of table 4; the low-pass gives the fall of the EP response
/// above 2 MHz, so the network is a band-pass centred near 1.7 MHz, and it needs no coupling
/// high-pass to meet the table.
const EP_FIGURES: SubEmphasisParams =
    SubEmphasisParams { hp_hz: 1.05e6, lp_hz: 2.62e6, gain: 3.28, clip_ire: 17.1, post_hp_hz: 0.0 };

/// The figures of the network.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SubEmphasisParams {
    /// Corner of the high-pass, Hz.
    pub hp_hz: f64,
    /// Corner of the low-pass that follows it, Hz; 0 for none.
    pub lp_hz: f64,
    /// Gain between the filters and the limiter; 0 or less switches the network off.
    pub gain: f64,
    /// Level at which the limiter saturates, IRE; 0 or less switches the network off.
    pub clip_ire: f64,
    /// Corner of the coupling high-pass between the limiter and the sum, Hz; 0 for none.
    pub post_hp_hz: f64,
}

impl Default for SubEmphasisParams {
    fn default() -> Self {
        Self { hp_hz: 1.0e6, lp_hz: 0.0, gain: 1.0, clip_ire: 10.0, post_hp_hz: 0.0 }
    }
}

impl SubEmphasisParams {
    /// The figures that meet table 4 of the format for this speed; none at SP.
    pub fn for_speed(speed: TapeSpeed) -> Option<Self> {
        match speed {
            TapeSpeed::Sp => None,
            TapeSpeed::Lp => Some(LP_FIGURES),
            TapeSpeed::Ep => Some(EP_FIGURES),
        }
    }
}

/// Coefficients `(b0, a1)` of a bilinear first-order high-pass, `H = b0 (1 - z^-1) / (1 + a1 z^-1)`.
fn high_pass(fc: f64, fs: f64) -> (f64, f64) {
    let k = (PI * fc / fs).tan();
    (1.0 / (1.0 + k), (k - 1.0) / (k + 1.0))
}

/// The filters and the limiter shared by both stages.
///
/// The filters before the limiter are one two-state system (the high-pass's memory and the
/// low-pass's, `z`): their output is `dx * x + cz . z`, the next state `a z + bx * x`. Writing
/// the limiter's input as `direct * x + past()` separates what the current sample contributes
/// from what the past has fixed, which is what lets the playback stage solve its loop sample by
/// sample. The coupling high-pass after the limiter has the output `e0 * l + q` for a limiter
/// output `l` and the next state `alpha * l + beta * q`; without one, `e0 = 1` and the state
/// stays zero.
struct Network {
    /// A non-positive gain or clip level: the stages pass the signal through.
    off: bool,
    /// Whether a low-pass follows the first high-pass; without one the second state stays zero.
    has_lp: bool,
    z: [f64; 2],
    a: [[f64; 2]; 2],
    bx: [f64; 2],
    cz: [f64; 2],
    gain: f64,
    clip: f64,
    /// Gain from the current input to the limiter's input.
    direct: f64,
    /// The first high-pass's direct coefficient, for settling.
    hb0: f64,
    q: f64,
    e0: f64,
    alpha: f64,
    beta: f64,
}

impl Network {
    /// The high-pass `h = hb0 x + hs`, `hs' = -hb0 x - ha1 h` and the low-pass after it
    /// `o = lb0 h + ls`, `ls' = lb0 h - la1 o` (transposed direct form) are merged into one
    /// two-state system; the coupling high-pass is `p = e0 l + q`, `q' = -e0 l - ea1 p`.
    fn new(p: &SubEmphasisParams, fs: f64) -> Self {
        let (hb0, ha1) = high_pass(p.hp_hz, fs);
        let hs_row = ([-ha1, 0.0], -hb0 * (1.0 + ha1));
        let (a, bx, cz, dx) = if p.lp_hz > 0.0 {
            let k = (PI * p.lp_hz / fs).tan();
            let (lb0, la1) = (k / (1.0 + k), (k - 1.0) / (k + 1.0));
            let g = lb0 * (1.0 - la1);
            ([hs_row.0, [g, -la1]], [hs_row.1, g * hb0], [lb0, 1.0], lb0 * hb0)
        } else {
            ([hs_row.0, [0.0, 0.0]], [hs_row.1, 0.0], [1.0, 0.0], hb0)
        };
        let (e0, alpha, beta) = if p.post_hp_hz > 0.0 {
            let (e0, ea1) = high_pass(p.post_hp_hz, fs);
            (e0, -e0 * (1.0 + ea1), -ea1)
        } else {
            (1.0, 0.0, 0.0)
        };
        // `!(x > 0)` also catches a NaN from a hand-written figure
        let off = !(p.gain > 0.0 && p.clip_ire > 0.0);
        Self { off, has_lp: p.lp_hz > 0.0, z: [0.0; 2], a, bx, cz, gain: p.gain, clip: p.clip_ire, direct: p.gain * dx, hb0, q: 0.0, e0, alpha, beta }
    }

    /// A constant input has no high frequencies: the first high-pass holds the input's level in
    /// its state, and every filter output, the limiter's and the coupling network's are zero.
    fn settle(&mut self, x: f64) {
        self.z = [-self.hb0 * x, 0.0];
        self.q = 0.0;
    }

    /// The limiter's input less the current sample's share of it.
    #[inline]
    fn past(&self) -> f64 {
        self.gain * (self.cz[0] * self.z[0] + self.cz[1] * self.z[1])
    }
}

/// The record stage.
pub struct SubPreemphasis {
    net: Network,
}

impl SubPreemphasis {
    pub fn new(p: &SubEmphasisParams, fs: f64) -> Self {
        Self { net: Network::new(p, fs) }
    }

    /// Set the state as if the input had been `x` forever.
    pub fn settle(&mut self, x: f64) {
        self.net.settle(x);
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        match (self.net.off, self.net.has_lp) {
            (true, _) => {}
            (false, true) => self.run::<true>(buf),
            (false, false) => self.run::<false>(buf),
        }
    }

    /// The loop, compiled once with the low-pass and once without (where its state is zero).
    fn run<const LP: bool>(&mut self, buf: &mut [f32]) {
        let net = &mut self.net;
        let (a, bx, clip, d) = (net.a, net.bx, net.clip, net.direct);
        let gc = [net.gain * net.cz[0], net.gain * net.cz[1]];
        let (e0, alpha, beta) = (net.e0, net.alpha, net.beta);
        let (mut z, mut q) = (net.z, net.q);
        for v in buf.iter_mut() {
            let x = *v as f64;
            let u = d * x + if LP { gc[0] * z[0] + gc[1] * z[1] } else { gc[0] * z[0] };
            // the diode limiter: `u` for small `u`, approaching `clip` in size
            let l = clip * u / (clip + u.abs());
            z = if LP {
                [a[0][0] * z[0] + a[0][1] * z[1] + bx[0] * x, a[1][0] * z[0] + a[1][1] * z[1] + bx[1] * x]
            } else {
                [a[0][0] * z[0] + bx[0] * x, 0.0]
            };
            let boost = e0 * l + q;
            q = alpha * l + beta * q;
            *v = (x + boost) as f32;
        }
        net.z = z;
        net.q = q;
    }
}

/// The playback stage: the inverse of [`SubPreemphasis`] with the same figures.
pub struct SubDeemphasis {
    net: Network,
    /// The state update with the input written through the limiter's input `u`
    /// (`x = (u - past) / direct`): `z' = at z + bt * u`.
    at: [[f64; 2]; 2],
    bt: [f64; 2],
    /// `m . z + kappa * u` is the next sample's `past()`.
    m: [f64; 2],
    kappa: f64,
}

impl SubDeemphasis {
    pub fn new(p: &SubEmphasisParams, fs: f64) -> Self {
        let net = Network::new(p, fs);
        let inv_d = if net.off { 0.0 } else { 1.0 / net.direct };
        let gc = [net.gain * net.cz[0], net.gain * net.cz[1]];
        let mut at = net.a;
        for (i, row) in at.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v -= net.bx[i] * gc[j] * inv_d;
            }
        }
        let bt = [net.bx[0] * inv_d, net.bx[1] * inv_d];
        let m = [gc[0] * at[0][0] + gc[1] * at[1][0], gc[0] * at[0][1] + gc[1] * at[1][1]];
        let kappa = gc[0] * bt[0] + gc[1] * bt[1];
        Self { net, at, bt, m, kappa }
    }

    /// Set the state as if the input had been `x` forever.
    pub fn settle(&mut self, x: f64) {
        self.net.settle(x);
    }

    /// Solves `x + e0 L(u) + q = y` with `u = direct * x + past` for each sample (`e0 L + q` is
    /// the coupling network's output). In terms of the limiter's input this is
    /// `u + D L(u) = w` with `D = direct * e0` and `w = direct * (y - q) + past`; the left side is
    /// odd and increasing, so `u` has the sign of `w`, and for `w >= 0` it is the positive root
    /// of `u^2 + 2 b u - clip w = 0`, `b = (clip (1 + D) - w) / 2`: `u = sqrt(b^2 + clip w) - b`.
    /// Then `L(u) = (w - u) / D` and `x = y - q - (w - u) / direct`.
    ///
    /// The next sample's `w` is a sum of terms known before the root and one term in `u`, which
    /// is added last, so that only two operations stand between one sample's root and the next
    /// sample's `w`: the recursion, not the arithmetic, sets the speed of this stage.
    pub fn process(&mut self, buf: &mut [f32]) {
        match (self.net.off, self.net.has_lp) {
            (true, _) => {}
            (false, true) => self.run::<true>(buf),
            (false, false) => self.run::<false>(buf),
        }
    }

    /// The loop, compiled once with the low-pass and once without (where its state is zero).
    fn run<const LP: bool>(&mut self, buf: &mut [f32]) {
        let net = &mut self.net;
        let (d, clip, e0, alpha, beta) = (net.direct, net.clip, net.e0, net.alpha, net.beta);
        let big_d = d * e0;
        let (inv_d, inv_big_d) = (1.0 / d, 1.0 / big_d);
        let half_c1 = 0.5 * clip * (1.0 + big_d);
        // `w' = d y' + m . z - k_w w - d beta q + (kappa + k_w) u`, through `q' = alpha L + beta q`
        let k_w = d * alpha * inv_big_d;
        let (half_d, half_k) = (0.5 * d, 0.5 * (self.kappa + k_w));
        let (half_m, half_kw, half_dbeta) = ([0.5 * self.m[0], 0.5 * self.m[1]], 0.5 * k_w, 0.5 * d * beta);
        let (at, bt) = (self.at, self.bt);
        let (mut z, mut q) = (net.z, net.q);
        let mut half_rest = 0.5 * (net.past() - d * q);
        let mut u = 0.0f64;
        for v in buf.iter_mut() {
            let y = *v as f64;
            let half_w = (half_d * y + half_rest) + half_k * u;
            let a = half_w.abs();
            let b = half_c1 - a;
            u = ((b * b + 2.0 * clip * a).sqrt() - b).copysign(half_w);
            let w = 2.0 * half_w;
            *v = (y - q - (w - u) * inv_d) as f32;
            let mz = if LP { half_m[0] * z[0] + half_m[1] * z[1] } else { half_m[0] * z[0] };
            half_rest = mz - half_kw * w - half_dbeta * q;
            q = alpha * (w - u) * inv_big_d + beta * q;
            z = if LP {
                [at[0][0] * z[0] + at[0][1] * z[1] + bt[0] * u, at[1][0] * z[0] + at[1][1] * z[1] + bt[1] * u]
            } else {
                [at[0][0] * z[0] + bt[0] * u, 0.0]
            };
        }
        net.z = z;
        net.q = q;
    }
}
