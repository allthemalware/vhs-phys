//! Transport and drum mechanics: the time-base error `e(t)` of the tape signal.
//!
//! Every transition time on tape is displaced by `e(t)` (in RF samples). Line
//! jitter, brightness shifts (frequency scaling) and chroma phase errors then
//! emerge from the signal chain instead of being painted on.
//!
//! The error is a sum of independent physical processes. Each Gaussian
//! process is synthesised as a bank of random-phase sinusoids (a stationary
//! Gaussian process with the wanted spectrum), which makes `e(t)` a pure
//! function of the global sample index and the seed: no state, so tracks can
//! be rendered in parallel and in any order.
//!
//! * slow drift: capstan wow / servo hunting, 0.3 .. 8 Hz;
//! * drum eccentricity: lines at the drum rotation rate (29.97 Hz) and harmonics;
//! * tape-path resonance: tape spring between capstan and drum, a second-order
//!   resonance excited by friction and motor cogging;
//! * stick-slip ("scrape flutter"): tape vibrating over the stationary heads,
//!   100 Hz .. 3 kHz;
//! * line-to-line noise: head/tape contact micro-slip, independent per line;
//! * per-track terms: head-to-head offset, tension kick when the tape wraps
//!   onto the drum ("flagging" at the top), and a head-switch error at the end.

use serde::{Deserialize, Serialize};
use super::consts::*;
use super::head::{Controls, STRIDE};
use crate::rng::{hash3, Rng};
use std::f64::consts::PI;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MechParams {
    /// Slow drift, rms microseconds.
    pub slow_us: f64,
    /// Drum eccentricity, rms nanoseconds (fundamental; harmonics fall as 1/k^2).
    pub drum_ns: f64,
    /// Tape-path resonance: frequency (Hz), Q and rms (ns).
    pub tension_hz: f64,
    pub tension_q: f64,
    pub tension_ns: f64,
    /// Stick-slip band rms (ns).
    pub stick_ns: f64,
    /// Independent per-line jitter rms (ns).
    pub line_ns: f64,
    /// Constant offset of head 2 relative to head 1 (microseconds).
    pub head_step_us: f64,
    /// Tension kick at the start of every track: amplitude (us) and decay (lines).
    pub flag_us: f64,
    pub flag_lines: f64,
    /// Head-switch error at the end of the track: amplitude (us) and decay (lines).
    pub switch_us: f64,
    pub switch_lines: f64,
}

impl MechParams {
    /// A perfectly steady transport (for calibration and tests).
    pub fn none() -> Self {
        Self { slow_us: 0.0, drum_ns: 0.0, tension_ns: 0.0, stick_ns: 0.0, line_ns: 0.0, head_step_us: 0.0, flag_us: 0.0, switch_us: 0.0, ..Self::default() }
    }
}

impl Default for MechParams {
    fn default() -> Self {
        Self {
            slow_us: 0.15,
            drum_ns: 12.0,
            tension_hz: 45.0,
            tension_q: 3.0,
            tension_ns: 15.0,
            stick_ns: 6.0,
            line_ns: 3.0,
            head_step_us: 0.05,
            flag_us: 0.0,
            flag_lines: 6.0,
            switch_us: 0.3,
            switch_lines: 2.0,
        }
    }
}

struct Bank {
    f: Vec<f64>,
    a: Vec<f64>,
    ph: Vec<f64>,
}

impl Bank {
    /// `n` components, log spaced over [lo, hi] Hz, amplitudes shaped by `psd`
    /// (amplitude^2 proportional to `psd(f) * df`) and scaled to `rms`.
    fn new(seed: u64, stream: u64, n: usize, lo: f64, hi: f64, rms: f64, psd: impl Fn(f64) -> f64) -> Self {
        let mut rng = Rng::new(seed, stream, 0);
        let mut f = Vec::new();
        let mut p = Vec::new();
        for k in 0..n {
            let x = (k as f64 + rng.uniform()) / n as f64;
            let fk = lo * (hi / lo).powf(x);
            let df = fk * (hi / lo).ln() / n as f64;
            f.push(fk);
            p.push(psd(fk) * df);
        }
        let tot: f64 = p.iter().sum();
        let a: Vec<f64> = p.iter().map(|v| (2.0 * v / tot).sqrt() * rms).collect();
        let ph: Vec<f64> = (0..n).map(|_| rng.range(0.0, 2.0 * PI)).collect();
        Self { f, a, ph }
    }

    fn eval(&self, t: f64) -> f64 {
        let mut s = 0.0;
        for i in 0..self.f.len() {
            s += self.a[i] * (2.0 * PI * self.f[i] * t + self.ph[i]).cos();
        }
        s
    }
}

pub struct Mech {
    p: MechParams,
    seed: u64,
    slow: Bank,
    tension: Bank,
    stick: Bank,
    drum_phase: [f64; 3],
}

impl Mech {
    pub fn new(p: MechParams, seed: u64) -> Self {
        let slow = Bank::new(seed, 11, 40, 0.3, 8.0, p.slow_us * 1e-6, |f| 1.0 / f);
        let (f0, q) = (p.tension_hz, p.tension_q);
        // second-order resonance power spectrum |H|^2 of a white-driven mass-spring
        let tension = Bank::new(seed, 12, 48, f0 * 0.3, f0 * 3.0, p.tension_ns * 1e-9, move |f| {
            let x = f / f0;
            1.0 / ((1.0 - x * x).powi(2) + (x / q).powi(2))
        });
        let stick = Bank::new(seed, 13, 48, 100.0, 3000.0, p.stick_ns * 1e-9, |_| 1.0);
        let mut r = Rng::new(seed, 14, 0);
        let drum_phase = [r.range(0.0, 2.0 * PI), r.range(0.0, 2.0 * PI), r.range(0.0, 2.0 * PI)];
        Self { p, seed, slow, tension, stick, drum_phase }
    }

    /// Continuous time-base error in seconds at global time `t` (s).
    fn base(&self, t: f64) -> f64 {
        let mut e = self.slow.eval(t) + self.tension.eval(t) + self.stick.eval(t);
        let w = 2.0 * PI * FRAME_RATE;
        for k in 0..3 {
            let amp = self.p.drum_ns * 1e-9 / ((k + 1) * (k + 1)) as f64 * 1.3;
            e += amp * (w * (k + 1) as f64 * t + self.drum_phase[k]).cos();
        }
        e
    }

    /// Capstan wow: the slow drift of the transport, seconds at time `t`. The audio track is
    /// read by a stationary head, so this is the speed error the sound shares with the picture.
    pub fn wow(&self, t: f64) -> f64 {
        self.slow.eval(t)
    }

    /// Faster transport error: the tape-path resonance and stick-slip scrape, seconds.
    pub fn flutter(&self, t: f64) -> f64 {
        self.tension.eval(t) + self.stick.eval(t)
    }

    fn line_node(&self, l: i64) -> f64 {
        let h = hash3(self.seed, 15, l as u64);
        // sum of 4 uniforms, scaled to unit variance: close enough to Gaussian
        let mut s = 0.0;
        let mut x = h;
        for _ in 0..4 {
            x = crate::rng::mix(x);
            s += (x >> 11) as f64 / (1u64 << 53) as f64 - 0.5;
        }
        s * (12.0f64 / 4.0).sqrt()
    }

    /// Independent line-to-line jitter (seconds) at global RF sample `g`. One
    /// random value per line, placed at mid-line and joined linearly: the tape
    /// cannot jump, so the displacement must change smoothly (a step would be a
    /// phase jump that the FM discriminator turns into a spike at the sync edge).
    fn line_noise(&self, g: i64) -> f64 {
        let x = g as f64 / RF_PER_LINE as f64 - 0.5;
        let l = x.floor() as i64;
        let f = x - l as f64;
        (self.line_node(l) * (1.0 - f) + self.line_node(l + 1) * f) * self.p.line_ns * 1e-9
    }

    /// Fill `ctl.warp` for the track window starting at global RF index `g0` of
    /// track `n` (whose head-in point is `switch_time(n)`).
    pub fn fill_warp(&self, n: i64, g0: i64, ctl: &mut Controls) {
        let g_in = switch_time(n);
        let g_out = switch_time(n + 1);
        let step = if n.rem_euclid(2) == 1 { self.p.head_step_us * 1e-6 } else { 0.0 };
        let (flag, tf) = (self.p.flag_us * 1e-6, self.p.flag_lines * RF_PER_LINE as f64);
        let (sw, ts) = (self.p.switch_us * 1e-6, self.p.switch_lines * RF_PER_LINE as f64);
        // coarse evaluation of the smooth processes, then linear interpolation
        const COARSE: usize = 256;
        let m = ctl.warp.len();
        let n_coarse = m * STRIDE / COARSE + 3;
        let mut coarse = Vec::with_capacity(n_coarse);
        for c in 0..n_coarse {
            let g = g0 + (c * COARSE) as i64;
            coarse.push(self.base(g as f64 / FS_RF));
        }
        for i in 0..m {
            let g = g0 + (i * STRIDE) as i64;
            let x = (i * STRIDE) as f64 / COARSE as f64;
            let c = x as usize;
            let f = x - c as f64;
            let mut e = coarse[c] * (1.0 - f) + coarse[c + 1] * f;
            e += self.line_noise(g);
            e += step;
            if g >= g_in - TRACK_MARGIN_RF as i64 {
                e += flag * (-((g - g_in).max(0) as f64) / tf).exp();
            }
            e += sw * (-((g_out - g).max(0) as f64) / ts).exp() * if g < g_out + TRACK_MARGIN_RF as i64 { 1.0 } else { 0.0 };
            ctl.warp[i] = (e * FS_RF) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms_of(f: impl Fn(f64) -> f64, secs: f64) -> f64 {
        let n = 20000;
        let v: Vec<f64> = (0..n).map(|k| f(k as f64 / n as f64 * secs)).collect();
        let m = v.iter().sum::<f64>() / n as f64;
        (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / n as f64).sqrt()
    }

    #[test]
    fn components_have_requested_rms() {
        let p = MechParams { slow_us: 0.0, drum_ns: 0.0, tension_ns: 0.0, stick_ns: 50.0, ..Default::default() };
        let m = Mech::new(p, 3);
        let r = rms_of(|t| m.base(t), 2.0);
        assert!((r - 50e-9).abs() < 20e-9, "{r}");
        let p = MechParams { slow_us: 1.0, drum_ns: 0.0, tension_ns: 0.0, stick_ns: 0.0, ..Default::default() };
        let m = Mech::new(p, 3);
        let r = rms_of(|t| m.base(t), 60.0);
        assert!((r - 1e-6).abs() < 0.5e-6, "{r}");
    }

    #[test]
    fn deterministic_and_seed_dependent() {
        let a = Mech::new(MechParams::default(), 1);
        let b = Mech::new(MechParams::default(), 1);
        let c = Mech::new(MechParams::default(), 2);
        assert_eq!(a.base(0.123), b.base(0.123));
        assert_ne!(a.base(0.123), c.base(0.123));
    }

    #[test]
    fn head_step_and_flag_enter_the_warp() {
        let p = MechParams { slow_us: 0.0, drum_ns: 0.0, tension_ns: 0.0, stick_ns: 0.0, line_ns: 0.0, head_step_us: 0.4, flag_us: 2.0, switch_us: 0.0, ..Default::default() };
        let m = Mech::new(p, 1);
        let len = FIELD_RF + 2 * TRACK_MARGIN_RF;
        let mut c = Controls::new(len);
        m.fill_warp(1, switch_time(1) - TRACK_MARGIN_RF as i64, &mut c);
        // odd track: step of 0.4 us everywhere, plus a 2 us kick decaying after head-in
        let at_in = c.warp[TRACK_MARGIN_RF / STRIDE] as f64 / FS_RF * 1e6;
        let late = c.warp[(TRACK_MARGIN_RF + FIELD_RF / 2) / STRIDE] as f64 / FS_RF * 1e6;
        assert!((at_in - 2.4).abs() < 0.05, "{at_in}");
        assert!((late - 0.4).abs() < 0.02, "{late}");
    }
}
