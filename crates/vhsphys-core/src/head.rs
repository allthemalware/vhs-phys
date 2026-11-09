//! Playback head: turns the magnetisation reversals on tape into the voltage
//! at the video head terminals.
//!
//! By reciprocity the readback of a reversal is a pulse of unit area whose
//! half-width is `h = sqrt((g/2)^2 + (d+a)(d+a+delta))` (see
//! `docs/RESEARCH.md` 3.2). Its spectrum is `exp(-k h)`, i.e. Wallace's spacing
//! law with an effective spacing `h`, and Lorentzian widths add, so a change of
//! head-to-tape spacing simply adds to `h`. The playback signal is the sum of
//! these alternating-sign pulses, each placed at its (time-warped) position,
//! with its own local spacing and magnetisation scale.

use serde::{Deserialize, Serialize};
use crate::consts::*;
use crate::dsp::*;
use crate::record::Transition;
use crate::rng::Rng;
use std::f32::consts::PI as PI32;

/// Controls (timebase, spacing, magnetisation, lateral weight) are stored every
/// `STRIDE` RF samples and linearly interpolated.
pub const STRIDE: usize = 16;

#[derive(Clone)]
pub struct Controls {
    /// Timebase displacement of the tape signal, RF samples (positive = later).
    pub warp: Vec<f32>,
    /// Extra head-to-tape spacing in micrometres.
    pub dd: Vec<f32>,
    /// Magnetisation scale (1 = intact oxide, 0 = void).
    pub mag: Vec<f32>,
}

impl Controls {
    pub fn new(n_rf: usize) -> Self {
        let n = n_rf / STRIDE + 3;
        Self { warp: vec![0.0; n], dd: vec![0.0; n], mag: vec![1.0; n] }
    }

    #[inline(always)]
    pub fn at(v: &[f32], t: f64) -> f32 {
        let x = (t / STRIDE as f64).max(0.0);
        let i = (x as usize).min(v.len() - 2);
        let f = (x - i as f64) as f32;
        v[i] + (v[i + 1] - v[i]) * f
    }
}

/// Render the pulses of `tr` into `out` (window of `out.len()` RF samples).
///
/// * `shift`: time offset added to every transition before rendering (used to
///   align neighbouring tracks and to centre the azimuth smoothing).
/// * `lateral`: optional per-stride weight of this track under the head.
/// * `h0`: nominal pulse half-width in RF samples.
pub fn render_pulses(
    tr: &[Transition],
    shift: f64,
    ctl: &Controls,
    lateral: Option<&[f32]>,
    h0: f64,
    out: &mut [f32],
) {
    let n = out.len() as i64;
    for t in tr {
        let tt = t.t + shift;
        if tt < -60.0 || tt > (n + 60) as f64 {
            continue;
        }
        let mag = Controls::at(&ctl.mag, tt);
        let w = lateral.map_or(1.0, |l| Controls::at(l, tt));
        let amp = 2.0 * t.s as f32 * mag * w;
        if amp == 0.0 {
            continue;
        }
        let tc = tt + Controls::at(&ctl.warp, tt) as f64;
        let h = (h0 + Controls::at(&ctl.dd, tt) as f64 / UM_PER_SAMPLE) as f32;
        let r = ((8.0 * h) as i64).clamp(16, 56);
        let i0 = tc.floor() as i64 - r;
        let c = amp * h / PI32;
        let h2 = h * h;
        let lo = i0.max(0);
        let hi = (i0 + 2 * r).min(n - 1);
        let mut i = lo;
        while i <= hi {
            let x = i as f32 - tc as f32;
            out[i as usize] += c / (h2 + x * x);
            i += 1;
        }
    }
}

/// Head front-end parameters (one per head).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HeadParams {
    /// Nominal pulse half-width, micrometres (gap, transition width, recorded
    /// depth and nominal spacing combined).
    pub h0_um: f64,
    /// Output gain (head wear / efficiency), 1 = nominal.
    pub gain: f64,
    /// Carrier-to-noise ratio in 6 MHz at nominal spacing, dB.
    pub cnr_db: f64,
    /// Head resonance (head + rotary transformer + Q network): frequency, Q,
    /// peak gain in dB.
    pub q_freq_hz: f64,
    pub q_q: f64,
    pub q_gain_db: f64,
    /// Azimuth error of this head versus the track it reads, degrees.
    pub azimuth_error_deg: f64,
    /// Playback head width, micrometres.
    pub width_um: f64,
    /// Amplitude modulation noise (media noise) rms, fraction of the carrier.
    pub media_am_noise: f64,
    /// Constant extra head-to-tape spacing (um): worn head tips, a film of
    /// debris on the drum, a clogged head.
    pub extra_spacing_um: f64,
}

impl Default for HeadParams {
    fn default() -> Self {
        Self {
            h0_um: 0.36,
            gain: 1.0,
            cnr_db: 36.0,
            q_freq_hz: 4.8e6,
            q_q: 1.1,
            q_gain_db: 7.0,
            azimuth_error_deg: 0.0,
            width_um: TRACK_PITCH_UM,
            media_am_noise: 0.02,
            extra_spacing_um: 0.0,
        }
    }
}

impl HeadParams {
    pub fn h0_samples(&self) -> f64 {
        self.h0_um / UM_PER_SAMPLE
    }
}

/// Fundamental amplitude of the readback of a saturated square wave of
/// frequency `f` (state +/-1, pulses of area 2): `(4/pi) * w * exp(-w*h)`.
pub fn nominal_fm_amplitude(f: f64, h_samples: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * f / FS_RF;
    4.0 / std::f64::consts::PI * w * (-w * h_samples).exp()
}

/// Add head-amplifier noise (white, set by the nominal CNR) and media AM noise,
/// then apply the head resonance. `rng` must be unique to this track.
pub fn head_front_end(buf: &mut [f32], hp: &HeadParams, rng: &mut Rng) {
    let a_nom = nominal_fm_amplitude(3.9e6, hp.h0_samples());
    // white noise over fs/2 whose 6 MHz share gives the requested CNR
    let p_band = a_nom * a_nom / 2.0 / 10f64.powf(hp.cnr_db / 10.0);
    let sigma = (p_band * (FS_RF / 2.0) / 6.0e6).sqrt() as f32;
    // slowly varying multiplicative (media) noise: one-pole filtered Gaussian
    let mut am = Biquad::one_pole_lp(0.5e6, FS_RF);
    let am_gain = (hp.media_am_noise * (FS_RF / 2.0 / 0.5e6).sqrt() * 0.8) as f32;
    let mut q = Biquad::peaking(hp.q_freq_hz, hp.q_q, hp.q_gain_db, FS_RF);
    let g = hp.gain as f32;
    for v in buf.iter_mut() {
        let m = 1.0 + am.tick((rng.gauss() * am_gain) as f64) as f32;
        let x = *v * m * g + rng.gauss() * sigma;
        *v = q.tick(x as f64) as f32;
    }
}

/// Boxcar smoothing that models the azimuth loss `sinc(pi w tan(theta)/lambda)`
/// of reading a track with a different azimuth: averaging over the lateral
/// phase difference across the (overlapped) head width.
pub fn azimuth_smooth(buf: &mut [f32], width_um: f64, theta_deg: f64) {
    let w = (width_um * theta_deg.to_radians().tan() / UM_PER_SAMPLE).round() as usize;
    if w >= 2 {
        let sm = boxcar(buf, w);
        // causal boxcar delays by (w-1)/2: re-centre by shifting back
        let d = (w - 1) / 2;
        for i in 0..buf.len() {
            buf[i] = if i + d < sm.len() { sm[i + d] } else { 0.0 };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An alternating pulse train at frequency f has fundamental amplitude
    /// (4/pi) w exp(-w h) per the Lorentzian spectrum: this is Wallace's law.
    fn train(f: f64, h_um: f64, dd_um: f64, n: usize) -> Vec<f32> {
        let per = FS_RF / f / 2.0;
        let tr: Vec<Transition> = (0..(n as f64 / per) as usize - 4)
            .map(|k| Transition { t: 30.0 + k as f64 * per, s: if k % 2 == 0 { 1 } else { -1 } })
            .collect();
        let mut ctl = Controls::new(n);
        ctl.dd.iter_mut().for_each(|v| *v = dd_um as f32);
        let mut out = vec![0.0f32; n];
        render_pulses(&tr, 0.0, &ctl, None, h_um / UM_PER_SAMPLE, &mut out);
        out
    }

    fn fundamental(x: &[f32], f: f64) -> f64 {
        let n = x.len() / 4;
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in x[n..3 * n].iter().enumerate() {
            let a = 2.0 * std::f64::consts::PI * f * (i + n) as f64 / FS_RF;
            re += *v as f64 * a.cos();
            im += *v as f64 * a.sin();
        }
        2.0 * (re * re + im * im).sqrt() / (2 * n) as f64
    }

    #[test]
    fn pulse_train_amplitude_follows_lorentzian_spectrum() {
        let n = 60_000;
        for f in [1.0e6, 3.4e6, 4.4e6] {
            let x = train(f, 0.42, 0.0, n);
            let got = fundamental(&x, f);
            let want = nominal_fm_amplitude(f, 0.42 / UM_PER_SAMPLE);
            assert!((got / want - 1.0).abs() < 0.12, "f={f}: {got} vs {want}");
        }
    }

    #[test]
    fn extra_spacing_follows_wallace_54_6_db_per_wavelength() {
        let n = 60_000;
        let f = 4.0e6;
        let lambda_um = HEAD_SPEED / f * 1e6;
        let a0 = fundamental(&train(f, 0.42, 0.0, n), f);
        let dd = 0.3;
        let a1 = fundamental(&train(f, 0.42, dd, n), f);
        let loss_db = 20.0 * (a0 / a1).log10();
        let want = 54.575 * dd / lambda_um;
        assert!((loss_db - want).abs() < 0.8, "{loss_db} dB vs Wallace {want} dB");
    }

    #[test]
    fn azimuth_boxcar_matches_sinc_law() {
        // 12 deg across a 58 um head: loss at 4 MHz about -28 dB (envelope), at 629 kHz ~ -14 dB
        let n = 40_000;
        for (f, want_db, tol) in [(0.629e6, -13.8, 2.5), (4.0e6, -28.0, 6.0)] {
            let mut x: Vec<f32> = (0..n).map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / FS_RF).sin() as f32).collect();
            let a0 = fundamental(&x, f);
            azimuth_smooth(&mut x, 58.0, 12.0);
            let a1 = fundamental(&x, f);
            let db = 20.0 * (a1 / a0).log10();
            assert!((db - want_db).abs() < tol, "f={f}: {db} dB vs {want_db}");
        }
    }
}
