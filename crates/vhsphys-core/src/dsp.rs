//! Small DSP toolbox: biquads designed by bilinear transform, Butterworth
//! cascades, first-order pole/zero sections (emphasis networks), cubic
//! interpolation and 3x resampling between the baseband and RF grids.
//!
//! Filter state and coefficients are `f64` because several corners sit at a
//! tiny fraction of the 43 MS/s RF rate (e.g. 122 kHz de-emphasis).

use std::f64::consts::PI;

#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    pub fn identity() -> Self {
        Self::from_coeffs(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
    }

    pub fn from_coeffs(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, z1: 0.0, z2: 0.0 }
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    /// Settle the state so that a constant input `x` gives a constant output.
    pub fn settle(&mut self, x: f64) {
        let dc = (self.b0 + self.b1 + self.b2) / (1.0 + self.a1 + self.a2);
        let y = dc * x;
        self.z2 = self.b2 * x - self.a2 * y;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
    }

    #[inline(always)]
    pub fn tick(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for v in buf.iter_mut() {
            *v = self.tick(*v as f64) as f32;
        }
    }

    // ---- designs (RBJ cookbook, bilinear transform) ----
    pub fn lowpass(fc: f64, q: f64, fs: f64) -> Self {
        let w = 2.0 * PI * fc / fs;
        let (s, c) = w.sin_cos();
        let al = s / (2.0 * q);
        Self::from_coeffs((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + al, -2.0 * c, 1.0 - al)
    }

    pub fn highpass(fc: f64, q: f64, fs: f64) -> Self {
        let w = 2.0 * PI * fc / fs;
        let (s, c) = w.sin_cos();
        let al = s / (2.0 * q);
        Self::from_coeffs((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + al, -2.0 * c, 1.0 - al)
    }

    pub fn notch(fc: f64, q: f64, fs: f64) -> Self {
        let w = 2.0 * PI * fc / fs;
        let (s, c) = w.sin_cos();
        let al = s / (2.0 * q);
        Self::from_coeffs(1.0, -2.0 * c, 1.0, 1.0 + al, -2.0 * c, 1.0 - al)
    }

    /// Peaking EQ: `gain_db` at `fc`, unity far away.
    pub fn peaking(fc: f64, q: f64, gain_db: f64, fs: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w = 2.0 * PI * fc / fs;
        let (s, c) = w.sin_cos();
        let al = s / (2.0 * q);
        Self::from_coeffs(1.0 + al * a, -2.0 * c, 1.0 - al * a, 1.0 + al / a, -2.0 * c, 1.0 - al / a)
    }

    /// First-order section from the analog prototype `(1 + s/wz) / (1 + s/wp)`
    /// (DC gain 1), bilinear with prewarp at the geometric-mean frequency.
    /// `wz < wp` is a pre-emphasis shelf, `wz > wp` a de-emphasis shelf.
    pub fn first_order_pz(wz: f64, wp: f64, fs: f64) -> Self {
        let wm = (wz * wp).sqrt();
        let k = wm / (wm / (2.0 * fs)).tan(); // prewarped 2*fs
        // H(z) = (1 + s/wz)/(1 + s/wp), s = k (1-z^-1)/(1+z^-1)
        let (az, ap) = (k / wz, k / wp);
        Self::from_coeffs(1.0 + az, 1.0 - az, 0.0, 1.0 + ap, 1.0 - ap, 0.0)
    }

    pub fn one_pole_lp(fc: f64, fs: f64) -> Self {
        let k = (PI * fc / fs).tan();
        Self::from_coeffs(k, k, 0.0, 1.0 + k, k - 1.0, 0.0)
    }

    pub fn one_pole_hp(fc: f64, fs: f64) -> Self {
        let k = (PI * fc / fs).tan();
        Self::from_coeffs(1.0, -1.0, 0.0, 1.0 + k, k - 1.0, 0.0)
    }

    /// Complex frequency response at `f`.
    pub fn response(&self, f: f64, fs: f64) -> (f64, f64) {
        let w = 2.0 * PI * f / fs;
        let (c1, s1) = (w.cos(), -w.sin());
        let (c2, s2) = ((2.0 * w).cos(), -(2.0 * w).sin());
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = self.b1 * s1 + self.b2 * s2;
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = self.a1 * s1 + self.a2 * s2;
        let den = dr * dr + di * di;
        ((nr * dr + ni * di) / den, (ni * dr - nr * di) / den)
    }

    pub fn gain_db(&self, f: f64, fs: f64) -> f64 {
        let (r, i) = self.response(f, fs);
        10.0 * (r * r + i * i).log10()
    }
}

/// Cascade of biquads.
#[derive(Clone, Debug)]
pub struct Cascade(pub Vec<Biquad>);

impl Cascade {
    pub fn new(v: Vec<Biquad>) -> Self {
        Self(v)
    }

    #[inline(always)]
    pub fn tick(&mut self, x: f64) -> f64 {
        let mut y = x;
        for b in self.0.iter_mut() {
            y = b.tick(y);
        }
        y
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for v in buf.iter_mut() {
            *v = self.tick(*v as f64) as f32;
        }
    }

    pub fn reset(&mut self) {
        for b in self.0.iter_mut() {
            b.reset();
        }
    }

    pub fn settle(&mut self, x: f64) {
        let mut v = x;
        for b in self.0.iter_mut() {
            b.settle(v);
            let dc = (b.b0 + b.b1 + b.b2) / (1.0 + b.a1 + b.a2);
            v *= dc;
        }
    }

    pub fn gain_db(&self, f: f64, fs: f64) -> f64 {
        self.0.iter().map(|b| b.gain_db(f, fs)).sum()
    }
}

/// Butterworth low-pass of even `order` (cascade of 2nd-order sections).
pub fn butter_lp(order: usize, fc: f64, fs: f64) -> Cascade {
    Cascade::new(butter_qs(order).into_iter().map(|q| Biquad::lowpass(fc, q, fs)).collect())
}

pub fn butter_hp(order: usize, fc: f64, fs: f64) -> Cascade {
    Cascade::new(butter_qs(order).into_iter().map(|q| Biquad::highpass(fc, q, fs)).collect())
}

fn butter_qs(order: usize) -> Vec<f64> {
    assert!(order >= 2 && order % 2 == 0, "even order >= 2");
    (0..order / 2)
        .map(|k| 1.0 / (2.0 * (PI * (2 * k + 1) as f64 / (2.0 * order as f64)).cos()))
        .collect()
}

// -------------------------------------------------------------- interpolation

/// Catmull-Rom cubic through `p1`..`p2` at fraction `t` in [0, 1).
#[inline(always)]
pub fn cubic(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let a = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
    let b = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let c = -0.5 * p0 + 0.5 * p2;
    ((a * t + b) * t + c) * t + p1
}

/// Sample `x` at fractional index `pos` with cubic interpolation (edges clamp).
#[inline]
pub fn sample_cubic(x: &[f32], pos: f64) -> f32 {
    let n = x.len() as i64;
    let i = pos.floor();
    let t = (pos - i) as f32;
    let i = i as i64;
    let g = |k: i64| x[k.clamp(0, n - 1) as usize];
    cubic(g(i - 1), g(i), g(i + 1), g(i + 2), t)
}

/// Upsample by 3 with cubic interpolation. `out[3k + r]` is `x` at `k + r/3`.
/// `prev` / `next` supply neighbouring samples for the first/last interval
/// (pass the edge value when unknown).
pub fn upsample3(x: &[f32], out: &mut Vec<f32>) {
    out.clear();
    out.reserve(x.len() * 3);
    let n = x.len();
    for k in 0..n {
        let g = |j: i64| x[j.clamp(0, n as i64 - 1) as usize];
        let (p0, p1, p2, p3) = (g(k as i64 - 1), x[k], g(k as i64 + 1), g(k as i64 + 2));
        out.push(p1);
        out.push(cubic(p0, p1, p2, p3, 1.0 / 3.0));
        out.push(cubic(p0, p1, p2, p3, 2.0 / 3.0));
    }
}

/// Fractional-delay taps (4-point Lagrange) for delay `frac` in [0, 1) applied
/// as `y[n] = sum taps[j] * x[n - 1 + ... ]`; see `lagrange_delay`.
#[inline]
pub fn lagrange3(d: f64) -> [f64; 4] {
    // Delay d in samples (use d in [1, 2] for best accuracy): taps for x[n], x[n-1], x[n-2], x[n-3].
    let mut h = [0.0; 4];
    for (k, hk) in h.iter_mut().enumerate() {
        let mut v = 1.0;
        for m in 0..4 {
            if m != k {
                v *= (d - m as f64) / (k as f64 - m as f64);
            }
        }
        *hk = v;
    }
    h
}

/// Moving average via prefix sums; `out[i]` = mean of `x[i-w+1..=i]` (edges use
/// the available samples).
pub fn boxcar(x: &[f32], w: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; x.len()];
    let mut acc = 0.0f64;
    for i in 0..x.len() {
        acc += x[i] as f64;
        if i >= w {
            acc -= x[i - w] as f64;
        }
        out[i] = (acc / (i + 1).min(w) as f64) as f32;
    }
    out
}

/// Zero-phase (symmetric) FIR low-pass by Kaiser-windowed sinc.
/// `transition` is the width between the pass-band edge and stop-band edge.
pub fn kaiser_lowpass(fc: f64, transition: f64, atten_db: f64, fs: f64) -> Vec<f32> {
    let beta = if atten_db > 50.0 { 0.1102 * (atten_db - 8.7) } else if atten_db >= 21.0 { 0.5842 * (atten_db - 21.0).powf(0.4) + 0.07886 * (atten_db - 21.0) } else { 0.0 };
    let dw = 2.0 * PI * transition / fs;
    let mut n = ((atten_db - 7.95) / (2.285 * dw)).ceil() as usize | 1; // odd
    n = n.max(5);
    let m = (n - 1) as f64 / 2.0;
    let wc = 2.0 * PI * (fc + transition / 2.0) / fs; // centre of the transition
    let i0 = |x: f64| {
        let (mut sum, mut term, mut k) = (1.0, 1.0, 1.0);
        while term > 1e-12 * sum {
            term *= (x / (2.0 * k)).powi(2);
            sum += term;
            k += 1.0;
        }
        sum
    };
    let mut h: Vec<f64> = (0..n)
        .map(|i| {
            let x = i as f64 - m;
            let sinc = if x == 0.0 { wc / PI } else { (wc * x).sin() / (PI * x) };
            let r = x / m;
            sinc * i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0(beta)
        })
        .collect();
    let sum: f64 = h.iter().sum();
    for v in h.iter_mut() {
        *v /= sum;
    }
    h.into_iter().map(|v| v as f32).collect()
}

/// Apply a symmetric FIR with zero delay (centred); edges clamp.
pub fn fir_centred(x: &[f32], h: &[f32]) -> Vec<f32> {
    let n = x.len();
    let half = h.len() / 2;
    let mut y = vec![0.0f32; n];
    for i in 0..n {
        let mut acc = 0.0f32;
        for (k, &c) in h.iter().enumerate() {
            let j = (i + k).saturating_sub(half).min(n - 1);
            acc += c * x[j];
        }
        y[i] = acc;
    }
    y
}

pub fn db(x: f64) -> f64 {
    20.0 * x.max(1e-30).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f64 = 42_954_545.0;

    #[test]
    fn butterworth_is_3db_at_corner() {
        let f = butter_lp(4, 3.3e6, FS);
        assert!((f.gain_db(3.3e6, FS) + 3.01).abs() < 0.05);
        assert!(f.gain_db(3.0 * 3.3e6, FS) < -35.0); // 4th order: -80 log10(3) = -38 dB
    }

    #[test]
    fn pre_emphasis_shelf_gains() {
        // zero 1/tau, pole 5/tau => +14 dB at HF, 0 dB at DC.
        let tau = 1.3e-6;
        let pre = Biquad::first_order_pz(1.0 / tau, 5.0 / tau, FS);
        assert!(pre.gain_db(1.0e3, FS).abs() < 0.1);
        assert!((pre.gain_db(10.0e6, FS) - 13.98).abs() < 0.2);
        // de-emphasis is its exact inverse
        let de = Biquad::first_order_pz(5.0 / tau, 1.0 / tau, FS);
        for f in [1e5, 3e5, 1e6, 4e6] {
            assert!((pre.gain_db(f, FS) + de.gain_db(f, FS)).abs() < 0.01);
        }
        // geometric-mean (mid) frequency matches vhs-decode's deemph_mid
        let mid = 5.0f64.sqrt() / (2.0 * PI * tau);
        assert!((mid - 273_755.8).abs() < 5.0);
        assert!((pre.gain_db(mid, FS) - 6.99).abs() < 0.1);
    }

    #[test]
    fn upsample3_reproduces_samples_and_smooth_signals() {
        let x: Vec<f32> = (0..64).map(|k| (k as f32 * 0.2).sin()).collect();
        let mut y = Vec::new();
        upsample3(&x, &mut y);
        for k in 3..60 {
            assert_eq!(y[3 * k], x[k]);
            let truth = ((k as f32 + 1.0 / 3.0) * 0.2).sin();
            assert!((y[3 * k + 1] - truth).abs() < 2e-3);
        }
    }

    #[test]
    fn settle_gives_constant_output() {
        let mut b = Biquad::lowpass(1e6, 0.7, FS);
        b.settle(5.0);
        let y = b.tick(5.0);
        assert!((y - 5.0).abs() < 1e-9);
    }

    #[test]
    fn kaiser_fir_has_requested_response() {
        let fs = FS_BB_TEST;
        let h = kaiser_lowpass(3.0e6, 2.0e6, 55.0, fs);
        let resp = |f: f64| {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            let m = (h.len() / 2) as f64;
            for (k, &c) in h.iter().enumerate() {
                let a = 2.0 * PI * f / fs * (k as f64 - m);
                re += c as f64 * a.cos();
                im += c as f64 * a.sin();
            }
            (re * re + im * im).sqrt()
        };
        assert!((resp(0.0) - 1.0).abs() < 1e-6);
        assert!(db(resp(2.8e6)) > -0.5, "{}", db(resp(2.8e6)));
        // -6 dB at the centre of the transition band (fc + transition/2)
        assert!((db(resp(4.0e6)) + 6.0).abs() < 1.0, "{}", db(resp(4.0e6)));
        assert!(db(resp(5.2e6)) < -52.0, "{}", db(resp(5.2e6)));
        assert!(h.len() < 41, "{}", h.len());
    }
    const FS_BB_TEST: f64 = 14_318_181.8;

    #[test]
    fn boxcar_is_running_mean() {
        let x = vec![1.0f32; 10];
        assert_eq!(boxcar(&x, 4), vec![1.0f32; 10]);
        let y = boxcar(&[0.0, 4.0, 0.0, 0.0, 0.0], 2);
        assert_eq!(y, vec![0.0, 2.0, 2.0, 0.0, 0.0]);
    }
}
