//! VHS audio: the linear track read by the stationary audio/control head.
//!
//! The sound fails for the same physical reasons as the picture, because it comes from
//! the same simulated tape and deck: the capstan's speed error (`Mech::wow`/`flutter`)
//! is wow and flutter, the supply ripple that makes hum bars in the picture is hum in
//! the sound, head wear and the tape speed set the wavelength-limited bandwidth, tape
//! age and quality set the hiss, and defects lift the tape off the head (spacing loss
//! costs the high frequencies first, `54.6 d / lambda` dB). See
//! `docs/AUDIO.md` for the model and its parameters.
//!
//! Chain, per channel unless noted:
//! record (level control, tape saturation) -> variable delay (one capstan for all channels)
//! -> spacing loss from head wear and tape defects (and level loss from voids) -> head and
//! playback equaliser (high-pass, low-pass at a corner proportional to the tape speed),
//! with hiss entering before the equaliser -> hum (one supply for all channels) -> clip.
//!
//! Every time-dependent quantity is a pure function of the absolute sample index, so a
//! processor can start anywhere in the programme and any split of the input gives the
//! same output.

use crate::dsp::{lagrange3, Biquad};
use super::head::HeadParams;
use super::interference::{audio_add, audio_gain};
use crate::interference_params::InterferenceParams;
use super::mech::Mech;
use super::pipeline::Params;
use crate::rng::Rng;
use super::tape::{Kind, Tape};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// Frames processed per internal step (bounds the scratch memory of one call).
const CHUNK: usize = 2048;
/// Noise is generated in blocks of this many samples, each seeded from its block index.
const NOISE_BLOCK: usize = 1024;
/// Rng streams of the audio path (the other modules use 11..15, 31..33, 51).
const STREAM_HISS: u64 = 71;
/// Half length of the spacing-loss FIR (taps `2 * FIR_HALF + 1`); it also delays the signal by this much.
const FIR_HALF: usize = 24;
/// The linear audio track occupies this band at the tape edge (um across the tape).
const BAND_UM: f64 = 1000.0;
/// Each of the two stereo tracks is this wide, at the two ends of the band (um).
const STEREO_TRACK_UM: f64 = 350.0;
/// Spacing-loss filter bank: first non-zero node and the ratio between nodes (in units of
/// the dimensionless loss rate `alpha`, see `SpacingLoss`), up to `ALPHA_MAX`.
const ALPHA_FIRST: f64 = 0.04;
const ALPHA_RATIO: f64 = 1.6;
const ALPHA_MAX: f64 = 7.0;
/// Q of the Butterworth sections.
const Q_BUTTER: f64 = std::f64::consts::FRAC_1_SQRT_2;
/// Picture CNR (dB, `heads[*].cnr_db`) and media AM noise at which the hiss level parameters apply.
const CNR_REF_DB: f64 = 40.0;
const MEDIA_REF: f64 = 0.02;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioParams {
    /// When false the processor returns its input unchanged.
    pub enabled: bool,

    // ---- record deck ----
    /// Automatic level control: above this envelope (full scale = 1) the gain falls so that
    /// `ratio` dB of input give 1 dB of output; time constants in ms. [est]
    pub alc_threshold: f64,
    pub alc_ratio: f64,
    pub alc_attack_ms: f64,
    pub alc_release_ms: f64,
    /// Tape saturation `tanh(drive x) / drive`: drive 1 saturates smoothly near full scale. [est]
    pub sat_drive: f64,
    /// Bias asymmetry per unit of the record deck's `drive_offset` (even harmonics from a
    /// drifting bias). [est]
    pub bias_coupling: f64,

    // ---- head and tape response ----
    /// Corner of the playback response at SP, Hz (about 10 kHz in the format sheets). The
    /// corner is proportional to the tape speed, so it is 5 kHz at LP and 3.5 kHz at EP.
    pub corner_sp_hz: f64,
    /// Low-frequency roll-off of the head, Hz. [est]
    pub low_cut_hz: f64,
    /// Audio head gap and the nominal head-to-tape spacing the equaliser is aligned for, um. [est]
    pub gap_um: f64,
    pub nominal_spacing_um: f64,

    // ---- hiss ----
    /// Rms of the white hiss at the equaliser input at SP (full scale = 1) from the head
    /// amplifier when the picture's `cnr_db` is 40 dB, and from the tape when `media_am_noise`
    /// is 0.02. The two add in power. Tuned so that a good SP tape measures about 42 dB SNR.
    pub hiss_head: f64,
    pub hiss_tape: f64,

    // ---- transport ----
    /// Delay of the audio per second of `Mech::wow` and of `Mech::flutter`. The video's
    /// timebase error is what is left after the drum servo has followed the capstan; the
    /// audio head sees the capstan directly, so the audio error is a few tens of times larger.
    /// Constants: the wear of the deck already enters through the `Mech` parameters that
    /// `condition.rs` derives for the picture. Real VHS linear audio is specified around
    /// 0.1 to 0.2 % wow and flutter. [est]
    pub wow_gain: f64,
    pub flutter_gain: f64,
    /// Largest delay deviation allowed, ms (it sets the latency). [est]
    pub max_delay_ms: f64,
    /// The transport model is evaluated every this many samples and interpolated. [est]
    pub transport_step: u32,

    // ---- hum ----
    /// Hum amplitude (full scale = 1) of the second harmonic of the mains per IRE of peak-to-peak
    /// hum in the picture (`deck.supply.hum_ire`). [est]
    pub hum_per_ire: f64,
    /// Amplitude of the mains fundamental relative to the second harmonic (the transformer's
    /// stray field, not the rectified ripple). [est]
    pub hum_fundamental: f64,
}

impl Default for AudioParams {
    fn default() -> Self {
        Self {
            enabled: true,
            alc_threshold: 0.5,
            alc_ratio: 4.0,
            alc_attack_ms: 10.0,
            alc_release_ms: 500.0,
            sat_drive: 1.0,
            bias_coupling: 1.0,
            corner_sp_hz: 10_500.0,
            low_cut_hz: 80.0,
            gap_um: 1.0,
            nominal_spacing_um: 0.36,
            hiss_head: 3.0e-4,
            hiss_tape: 3.0e-4,
            wow_gain: 65.0,
            flutter_gain: 4.0,
            max_delay_ms: 1.5,
            transport_step: 16,
            hum_per_ire: 1.3e-4,
            hum_fundamental: 0.4,
        }
    }
}

// ------------------------------------------------------------------ pieces

/// Automatic level control: a peak envelope follower (fast attack, slow release) drives a
/// compressor with a hard threshold, so signals below it pass untouched and the gain does
/// not depend on the history.
struct Alc {
    env: f64,
    thr: f64,
    ratio: f64,
    att: f64,
    rel: f64,
}

impl Alc {
    fn new(a: &AudioParams, rate: f64) -> Self {
        let coef = |ms: f64| (-1.0 / (ms.max(1e-3) * 1e-3 * rate)).exp();
        Self { env: 0.0, thr: a.alc_threshold.max(1e-3), ratio: a.alc_ratio.max(1.0), att: coef(a.alc_attack_ms), rel: coef(a.alc_release_ms) }
    }

    #[inline]
    fn tick(&mut self, x: f64) -> f64 {
        let a = x.abs();
        let c = if a > self.env { self.att } else { self.rel };
        self.env = c * self.env + (1.0 - c) * a;
        if self.env > self.thr {
            x * (self.thr + (self.env - self.thr) / self.ratio) / self.env
        } else {
            x
        }
    }
}

/// Tape saturation with a bias offset `b`; the DC the offset creates is not recorded.
#[inline]
fn saturate(x: f64, drive: f64, b: f64) -> f64 {
    ((drive * (x + b)).tanh() - (drive * b).tanh()) / drive
}

/// Sample of `buf` that is `delay` samples (>= 1) older than index `n`, by 4-point Lagrange
/// interpolation. Reads `buf[n + 1 - floor(delay) - 3 ..= n + 1 - floor(delay)]`.
#[inline]
fn read_delayed(buf: &[f32], n: usize, delay: f64) -> f32 {
    let whole = delay.floor();
    let h = lagrange3(1.0 + (delay - whole));
    let m = n + 1 - whole as usize;
    (h[0] * buf[m] as f64 + h[1] * buf[m - 1] as f64 + h[2] * buf[m - 2] as f64 + h[3] * buf[m - 3] as f64) as f32
}

/// White Gaussian noise that is a pure function of (seed, stream, absolute sample index):
/// block `b` is drawn from `Rng::new(seed, stream, b)`.
struct NoiseGen {
    seed: u64,
    stream: u64,
    block: i64,
    buf: Vec<f32>,
}

impl NoiseGen {
    fn new(seed: u64, stream: u64) -> Self {
        Self { seed, stream, block: i64::MIN, buf: vec![0.0; NOISE_BLOCK] }
    }

    /// Unit-variance noise for samples `k0 .. k0 + out.len()`.
    fn fill(&mut self, k0: i64, out: &mut [f32]) {
        for (i, o) in out.iter_mut().enumerate() {
            let k = k0 + i as i64;
            let b = k.div_euclid(NOISE_BLOCK as i64);
            if b != self.block {
                let mut rng = Rng::new(self.seed, self.stream, b);
                for v in self.buf.iter_mut() {
                    *v = rng.gauss();
                }
                self.block = b;
            }
            *o = self.buf[k.rem_euclid(NOISE_BLOCK as i64) as usize];
        }
    }
}

/// Dimensionless loss rate of a spacing `d_um`: the loss at angular frequency `w` (rad per
/// sample) is `exp(-alpha |w|)`, which is the spacing loss `exp(-2 pi f d / v)`.
fn alpha_of(d_um: f64, rate: f64, speed_m_s: f64) -> f64 {
    rate * d_um * 1e-6 / speed_m_s
}

/// Symmetric FIR whose response is `exp(-alpha |w|)` for `|w| <= pi`, windowed and normalised
/// to unit gain at DC. (The impulse response of that spectrum is a Lorentzian, the same
/// kernel as the picture's head pulse.)
fn spacing_taps(alpha: f64, half: usize) -> Vec<f32> {
    let n_taps = 2 * half + 1;
    if alpha < 1e-9 {
        let mut h = vec![0.0f32; n_taps];
        h[half] = 1.0;
        return h;
    }
    let mut h: Vec<f64> = (0..n_taps)
        .map(|j| {
            let n = j as f64 - half as f64;
            let sign = if (j as i64 - half as i64) % 2 == 0 { 1.0 } else { -1.0 };
            let lorentz = alpha * (1.0 - sign * (-alpha * PI).exp()) / (alpha * alpha + n * n) / PI;
            let window = 0.5 * (1.0 + (PI * n / (half as f64 + 1.0)).cos());
            lorentz * window
        })
        .collect();
    let sum: f64 = h.iter().sum();
    h.iter_mut().for_each(|v| *v /= sum);
    h.into_iter().map(|v| v as f32).collect()
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let tail: f32 = ca.remainder().iter().zip(cb.remainder()).map(|(x, y)| x * y).sum();
    for (x, y) in ca.zip(cb) {
        for k in 0..8 {
            acc[k] += x[k] * y[k];
        }
    }
    acc.iter().sum::<f32>() + tail
}

/// Time-varying spacing loss as a small bank of fixed filters, crossfaded by the extra
/// loss rate. Node 0 is the permanent loss of the worn head (a constant spacing), the others
/// add geometrically spaced extra spacing; a clean tape only ever uses node 0.
struct SpacingLoss {
    half: usize,
    /// Extra `alpha` of each node.
    nodes: Vec<f64>,
    taps: Vec<Vec<f32>>,
}

impl SpacingLoss {
    fn new(alpha_static: f64, half: usize) -> Self {
        let mut nodes = vec![0.0];
        let mut a = ALPHA_FIRST;
        while a < ALPHA_MAX * ALPHA_RATIO.sqrt() {
            nodes.push(a);
            a *= ALPHA_RATIO;
        }
        let taps = nodes.iter().map(|e| spacing_taps(alpha_static + e, half)).collect();
        Self { half, nodes, taps }
    }

    /// `x` holds `2 * half` past samples followed by the new ones; `out` gets one value per
    /// new sample, delayed by `half`. `dirt` is `(extra loss rate, covered fraction)` per
    /// output sample, or `None` when nothing touches this stretch. The head reads the whole
    /// track width, so the amplitude is the uncovered fraction at the permanent loss plus the
    /// covered fraction at the extra loss, in the amplitude domain.
    fn apply(&self, x: &[f32], dirt: Option<(&[f32], &[f32])>, out: &mut [f32]) {
        let len = 2 * self.half + 1;
        for (i, o) in out.iter_mut().enumerate() {
            let win = &x[i..i + len];
            let clean = dot(&self.taps[0], win);
            let (e, cover) = dirt.map_or((0.0, 0.0), |(e, c)| (e[i] as f64, c[i]));
            *o = if e <= 0.0 || cover <= 0.0 {
                clean
            } else {
                let j = match self.nodes.iter().position(|n| *n > e) {
                    None => self.nodes.len() - 1,
                    Some(j) => j - 1,
                };
                let lossy = if j + 1 >= self.nodes.len() {
                    dot(&self.taps[j], win)
                } else {
                    let w = ((e - self.nodes[j]) / (self.nodes[j + 1] - self.nodes[j])) as f32;
                    (1.0 - w) * dot(&self.taps[j], win) + w * dot(&self.taps[j + 1], win)
                };
                clean + cover * (lossy - clean)
            };
        }
    }
}

/// Supply hum: the mains fundamental and the full-wave ripple at twice the mains frequency
/// with its harmonics (more of them for peaky ripple), as sinusoids of absolute time.
struct Hum {
    rate: f64,
    /// (frequency Hz, amplitude, phase rad)
    lines: Vec<(f64, f64, f64)>,
}

impl Hum {
    fn new(p: &Params, rate: f64) -> Self {
        let (s, a) = (&p.deck.supply, &p.audio);
        let mut lines = vec![];
        if s.mains_hz > 0.0 && s.hum_ire > 0.0 {
            let amp = s.hum_ire * a.hum_per_ire;
            let peaky = s.ripple_peakiness.clamp(0.0, 1.0);
            for (k, (harmonic, rel)) in [(1.0, a.hum_fundamental), (2.0, 1.0), (4.0, 0.5 * peaky), (6.0, 0.25 * peaky)].into_iter().enumerate() {
                let f = harmonic * s.mains_hz;
                if f < rate / 2.0 && rel > 0.0 {
                    lines.push((f, amp * rel, harmonic * s.phase + 0.9 * k as f64));
                }
            }
        }
        Self { rate, lines }
    }

    /// Hum for samples `k0 .. k0 + out.len()` (rotating phasors re-anchored on every call).
    fn fill(&self, k0: i64, out: &mut [f64]) {
        out.iter_mut().for_each(|v| *v = 0.0);
        for &(f, amp, ph) in &self.lines {
            let th0 = 2.0 * PI * (f * k0 as f64 / self.rate).fract() + ph;
            let (mut s, mut c) = th0.sin_cos();
            let (sd, cd) = (2.0 * PI * f / self.rate).sin_cos();
            for o in out.iter_mut() {
                *o += amp * s;
                let ns = s * cd + c * sd;
                c = c * cd - s * sd;
                s = ns;
            }
        }
    }
}

/// Lateral band of the tape (um) read by channel `c` of `channels`: the whole band for mono,
/// two separate tracks at the ends of the band for stereo.
fn track_band(c: usize, channels: usize) -> (f64, f64) {
    if channels == 1 {
        (0.0, BAND_UM)
    } else if c % 2 == 0 {
        (0.0, STEREO_TRACK_UM)
    } else {
        (BAND_UM - STEREO_TRACK_UM, BAND_UM)
    }
}

// ---------------------------------------------------------------- processor

/// Per-channel state.
struct Chan {
    alc: Alc,
    /// Recorded signal before the variable delay (history for the interpolator).
    hist_rec: Vec<f32>,
    /// Signal before the spacing-loss FIR.
    hist_z: Vec<f32>,
    hp: Biquad,
    lp: Biquad,
    shelf: Biquad,
    noise: NoiseGen,
}

/// The audio path of the deck and tape described by a `Params`.
pub struct AudioProcessor {
    a: AudioParams,
    rate: f64,
    channels: usize,
    mech: Mech,
    tape: Tape,
    loss: SpacingLoss,
    hum: Hum,
    /// Fields and signals from outside the circuit that reach the audio path.
    itf: InterferenceParams,
    chan: Vec<Chan>,
    /// Tape speed (m/s) and the tape position of programme time zero (um).
    speed: f64,
    pos_um: f64,
    /// Fixed part of the transport delay (samples) and its largest deviation (s).
    base_delay: usize,
    max_dev: f64,
    /// White hiss rms at the equaliser input (this speed).
    sigma: f64,
    bias: f64,
    /// Absolute index expected by the next call (None: fresh).
    next: Option<i64>,
}

impl AudioProcessor {
    /// `channels` interleaved channels at `rate` Hz.
    pub fn new(p: &Params, rate: f64, channels: usize) -> Self {
        let a = p.audio.clone();
        let channels = channels.max(1);
        let v = p.speed.tape_speed_m_s();
        let ratio = p.speed.ratio();
        let corner = a.corner_sp_hz * ratio;

        // Permanent extra spacing: worn head tips and a film on the head (the picture's
        // `h0_um` above its new value and `extra_spacing_um`, averaged over the two heads)
        // plus the mean roughness of the oxide.
        let h_new = HeadParams::default().h0_um;
        let wear_um = p.heads.iter().map(|h| (h.h0_um - h_new).max(0.0) + h.extra_spacing_um).sum::<f64>() / p.heads.len() as f64;
        let spacing_um = wear_um + 0.8 * p.tape.rough_um;
        let loss = SpacingLoss::new(alpha_of(spacing_um, rate, v), FIR_HALF);

        // The playback equaliser boosts the nominal spacing and gap loss up to the corner;
        // that boost is what raises the hiss entering before it.
        let x = (PI * a.gap_um * 1e-6 * corner / v).min(2.5);
        let boost_db = 54.6 * a.nominal_spacing_um * 1e-6 * corner / v - 20.0 * (x.sin() / x).log10();
        let g = 10f64.powf(boost_db / 20.0).max(1.0);
        let mid = corner / 2.0;
        let shelf = Biquad::first_order_pz(2.0 * PI * mid / g.sqrt(), 2.0 * PI * mid * g.sqrt(), rate);

        // Hiss level from the picture's noise parameters.
        let n = p.heads.len() as f64;
        let cnr = p.heads.iter().map(|h| h.cnr_db).sum::<f64>() / n;
        let media = p.heads.iter().map(|h| h.media_am_noise).sum::<f64>() / n;
        let s_head = a.hiss_head * 10f64.powf((CNR_REF_DB - cnr) / 20.0);
        let s_tape = a.hiss_tape * media / MEDIA_REF;
        // The head's output voltage is proportional to the tape speed, the hiss is not.
        let sigma = (s_head * s_head + s_tape * s_tape).sqrt() / ratio;

        let max_dev = a.max_delay_ms.max(0.0) * 1e-3;
        let chan = (0..channels)
            .map(|c| Chan {
                alc: Alc::new(&a, rate),
                hist_rec: vec![],
                hist_z: vec![],
                hp: Biquad::highpass(a.low_cut_hz, Q_BUTTER, rate),
                lp: Biquad::lowpass(corner, Q_BUTTER, rate),
                shelf,
                noise: NoiseGen::new(p.seed, STREAM_HISS + c as u64),
            })
            .collect();
        let mut s = Self {
            rate,
            channels,
            mech: Mech::new(p.mech.clone(), p.seed),
            tape: Tape::new(p.tape.clone(), p.seed),
            loss,
            hum: Hum::new(p, rate),
            itf: p.interference.clone(),
            chan,
            speed: v,
            pos_um: p.tape.position_m * 1e6,
            base_delay: (max_dev * rate).ceil() as usize + 2,
            max_dev,
            sigma,
            bias: a.bias_coupling * p.record.drive_offset,
            next: None,
            a,
        };
        s.reset();
        s
    }

    /// Samples by which the output lags the input: the fixed transport delay that leaves room
    /// for the wow and flutter excursions plus the delay of the spacing-loss filter. The first
    /// `latency()` output samples hold the empty tape; drop them to line the sound up with the
    /// picture. The other filters add under 0.3 ms of group delay (at EP).
    pub fn latency(&self) -> usize {
        if self.a.enabled {
            self.base_delay + FIR_HALF
        } else {
            0
        }
    }

    fn reset(&mut self) {
        let hist_rec = self.base_delay + (self.max_dev * self.rate).ceil() as usize + 4;
        for c in self.chan.iter_mut() {
            c.alc.env = 0.0;
            c.hist_rec = vec![0.0; hist_rec];
            c.hist_z = vec![0.0; 2 * FIR_HALF];
            c.hp.reset();
            c.lp.reset();
            c.shelf.reset();
        }
    }

    /// Process `interleaved` (frames x channels), whose first frame is absolute sample
    /// `start` of the programme (at the construction rate). Everything stochastic is a
    /// pure function of (seed, absolute sample index), so a processor started at any
    /// `start` reproduces the same tape. Calling `process` again continues the stream.
    /// Returns the same number of samples, delayed by `latency()`.
    pub fn process(&mut self, interleaved: &[f32], start: i64) -> Vec<f32> {
        if !self.a.enabled {
            return interleaved.to_vec();
        }
        let ch = self.channels;
        let frames = interleaved.len() / ch;
        let mut out = vec![0.0f32; interleaved.len()];
        if self.next != Some(start) {
            self.reset();
        }
        let mut done = 0;
        while done < frames {
            let len = (frames - done).min(CHUNK);
            self.chunk(&interleaved[done * ch..(done + len) * ch], start + done as i64, &mut out[done * ch..(done + len) * ch]);
            done += len;
        }
        self.next = Some(start + frames as i64);
        out
    }

    /// Delay of the transport in samples for samples `n0 .. n0 + len`: the base delay plus
    /// `wow_gain * wow + flutter_gain * flutter`, evaluated on a grid anchored at absolute
    /// sample zero and interpolated linearly.
    fn transport_delay(&self, n0: i64, len: usize) -> Vec<f64> {
        if self.a.wow_gain == 0.0 && self.a.flutter_gain == 0.0 {
            return vec![self.base_delay as f64; len];
        }
        let step = self.a.transport_step.max(1) as i64;
        let (g0, g1) = (n0.div_euclid(step), (n0 + len as i64 - 1).div_euclid(step) + 1);
        let grid: Vec<f64> = (g0..=g1)
            .map(|g| {
                let t = (g * step) as f64 / self.rate;
                self.a.wow_gain * self.mech.wow(t) + self.a.flutter_gain * self.mech.flutter(t)
            })
            .collect();
        (0..len as i64)
            .map(|i| {
                let k = n0 + i;
                let (g, f) = (k.div_euclid(step), k.rem_euclid(step) as f64 / step as f64);
                let j = (g - g0) as usize;
                let d = (grid[j] * (1.0 - f) + grid[j + 1] * f).clamp(-self.max_dev, self.max_dev);
                self.base_delay as f64 + d * self.rate
            })
            .collect()
    }

    /// Extra loss rate (`alpha`), covered fraction of the track and level factor along the
    /// track of channel `c` for samples `k0 .. k0 + len`, from the defects of the tape under
    /// the head. `None` when nothing touches this stretch. A defect is a disc of radius `r`
    /// on the tape; at each instant its lateral chord overlaps the track's band by some
    /// fraction of the band width (as `Tape::paint` does with the head width). Debris lifts
    /// the tape by `strength` um with the envelope `(1 - x^2)^2` along the footprint, but only
    /// over that covered fraction; a void removes the covered fraction of the level. Where
    /// several defects overlap, the lift is the coverage-weighted mean.
    fn defect_fields(&self, c: usize, k0: i64, len: usize) -> Option<(Vec<f32>, Vec<f32>, Vec<f32>)> {
        if self.tape.p.defects_per_mm2 <= 0.0 {
            return None;
        }
        let (v0, v1) = track_band(c, self.channels);
        let (centre, half) = ((v0 + v1) / 2.0, (v1 - v0) / 2.0);
        let r_max = self.tape.p.r_max_um;
        let um_per_sample = self.speed * 1e6 / self.rate;
        let u_at = |k: i64| self.pos_um + k as f64 * um_per_sample;
        let defects = self.tape.defects_in_band(u_at(k0) - r_max, u_at(k0 + len as i64) + r_max, v0 - r_max, v1 + r_max);
        // (covered fraction, coverage-weighted lift in um, level factor)
        let mut fields: Option<(Vec<f64>, Vec<f64>, Vec<f32>)> = None;
        for d in defects {
            let rho = d.v - centre;
            if rho.abs() >= d.r + half {
                continue;
            }
            let ka = (((d.u - d.r - self.pos_um) / um_per_sample).ceil() as i64).max(k0);
            let kb = (((d.u + d.r - self.pos_um) / um_per_sample).floor() as i64).min(k0 + len as i64 - 1);
            for k in ka..=kb {
                let (cover, lift, mag) = fields.get_or_insert_with(|| (vec![0.0; len], vec![0.0; len], vec![1.0; len]));
                let i = (k - k0) as usize;
                let ds = u_at(k) - d.u;
                let hc = (d.r * d.r - ds * ds).max(0.0).sqrt();
                let frac = ((rho + hc).min(half) - (rho - hc).max(-half)).max(0.0) / (2.0 * half);
                match d.kind {
                    Kind::Debris => {
                        let x = ds / d.r;
                        cover[i] += frac;
                        lift[i] += frac * d.strength * (1.0 - x * x).powi(2);
                    }
                    Kind::Void => mag[i] *= (1.0 - d.strength * frac.min(1.0)) as f32,
                }
            }
        }
        fields.map(|(cover, lift, mag)| {
            let alpha = lift.iter().zip(&cover).map(|(l, c)| if *c > 0.0 { alpha_of(l / c, self.rate, self.speed) as f32 } else { 0.0 }).collect();
            (alpha, cover.iter().map(|c| c.min(1.0) as f32).collect(), mag)
        })
    }

    fn chunk(&mut self, x: &[f32], n0: i64, out: &mut [f32]) {
        let ch = self.channels;
        let len = out.len() / ch;
        let delays = self.transport_delay(n0, len);
        let mut hum = vec![0.0f64; len];
        self.hum.fill(n0, &mut hum);
        // The outside world, once per block and the same for every channel: what enters at the
        // head and is shaped by the equaliser like the hiss, what enters after the filters like
        // the hum, and how much a magnet at the heads weakens the signal from the tape (that
        // changes on the time scale of the approach, seconds: one value per block).
        let mut pre_eq = vec![0.0f64; len];
        let mut post_eq = vec![0.0f64; len];
        let mut tape_gain = 1.0f64;
        if self.itf.audio_on() {
            audio_add(&self.itf, n0, self.rate, &mut pre_eq, &mut post_eq);
            tape_gain = audio_gain(&self.itf, (n0 + (len / 2) as i64) as f64 / self.rate);
        }
        let mut chans = std::mem::take(&mut self.chan);
        let (drive, bias) = (self.a.sat_drive.max(1e-3), self.bias);
        let mut rec = vec![0.0f32; len];
        let mut z = vec![0.0f32; len];
        let mut w = vec![0.0f32; len];
        let mut hiss = vec![0.0f32; len];
        for (c, st) in chans.iter_mut().enumerate() {
            // 1. record: level control and tape saturation
            for i in 0..len {
                let v = x[i * ch + c];
                let v = if v.is_finite() { v as f64 } else { 0.0 };
                rec[i] = saturate(st.alc.tick(v), drive, bias) as f32;
            }
            // 2. the capstan's speed error: a variable delay, shared by all channels
            let h = st.hist_rec.len();
            let mut buf = Vec::with_capacity(h + len);
            buf.extend_from_slice(&st.hist_rec);
            buf.extend_from_slice(&rec);
            for i in 0..len {
                z[i] = read_delayed(&buf, h + i, delays[i]);
            }
            st.hist_rec.copy_from_slice(&buf[len..]);
            // 3. spacing loss and dropouts, centred on the sample `FIR_HALF` back
            let fields = self.defect_fields(c, n0 - FIR_HALF as i64, len);
            let mut buf = Vec::with_capacity(2 * FIR_HALF + len);
            buf.extend_from_slice(&st.hist_z);
            buf.extend_from_slice(&z);
            self.loss.apply(&buf, fields.as_ref().map(|f| (&f.0[..], &f.1[..])), &mut w);
            if let Some((_, _, mag)) = &fields {
                w.iter_mut().zip(mag).for_each(|(a, m)| *a *= m);
            }
            let keep = buf.len() - 2 * FIR_HALF;
            st.hist_z.copy_from_slice(&buf[keep..]);
            // 4. head response and playback equaliser; the hiss and the pick-up at the head enter
            //    before the equaliser, the hum and what enters after the filters follow it
            st.noise.fill(n0, &mut hiss);
            for i in 0..len {
                let s = st.hp.tick(w[i] as f64 * tape_gain) + st.shelf.tick(hiss[i] as f64 * self.sigma + pre_eq[i]);
                let y = st.lp.tick(s) + hum[i] + post_eq[i];
                out[i * ch + c] = y.clamp(-1.0, 1.0) as f32;
            }
        }
        self.chan = chans;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::condition::Condition;

    fn params(preset: &str, seed: u64) -> Params {
        Condition::preset(preset).unwrap().to_params(seed)
    }

    fn db(r: f64) -> f64 {
        20.0 * r.max(1e-12).log10()
    }

    /// Gain (dB) of an FIR at frequency `f` for sample rate `fs`.
    fn fir_gain_db(h: &[f32], f: f64, fs: f64) -> f64 {
        let w = 2.0 * PI * f / fs;
        let (mut re, mut im) = (0.0, 0.0);
        for (n, v) in h.iter().enumerate() {
            re += *v as f64 * (w * n as f64).cos();
            im -= *v as f64 * (w * n as f64).sin();
        }
        db((re * re + im * im).sqrt())
    }

    #[test]
    fn the_delay_line_delays_a_tone_by_the_requested_fraction() {
        let fs = 48_000.0;
        let f = 1000.0;
        let x: Vec<f32> = (0..400).map(|n| (2.0 * PI * f * n as f64 / fs).sin() as f32).collect();
        for delay in [4.0, 4.25, 4.5, 7.9] {
            for n in 100..200 {
                let want = (2.0 * PI * f * (n as f64 - delay) / fs).sin();
                let got = read_delayed(&x, n, delay) as f64;
                assert!((want - got).abs() < 2e-3, "delay {delay} at {n}: {got} vs {want}");
            }
        }
        // a whole-sample delay is exact
        assert_eq!(read_delayed(&x, 150, 5.0), x[145]);
    }

    #[test]
    fn the_spacing_filter_follows_the_exponential_loss() {
        let fs = 48_000.0;
        for alpha in [0.1, 0.4, 1.0, 2.0] {
            let h = spacing_taps(alpha, FIR_HALF);
            assert_eq!(h.len(), 2 * FIR_HALF + 1);
            assert!((h.iter().sum::<f32>() - 1.0).abs() < 1e-5, "unit gain at DC");
            assert!(h.iter().zip(h.iter().rev()).all(|(a, b)| (a - b).abs() < 1e-7), "symmetric: no phase distortion");
            for f in [2000.0, 5000.0, 9000.0] {
                let want = -alpha * 2.0 * PI * f / fs * 20.0 / std::f64::consts::LN_10;
                let got = fir_gain_db(&h, f, fs) - fir_gain_db(&h, 0.0, fs);
                // the filter is a short approximation of a Lorentzian (its tail is cut, which
                // matters for the deepest losses): stay within 1.5 dB or a tenth of the loss
                assert!((got - want).abs() < 1.5f64.max(want.abs() * 0.1), "alpha {alpha} at {f} Hz: {got:.2} vs {want:.2} dB");
            }
        }
        assert_eq!(spacing_taps(0.0, 4), vec![0., 0., 0., 0., 1., 0., 0., 0., 0.]);
    }

    #[test]
    fn the_loss_bank_is_exact_without_dirt_and_a_blend_with_it() {
        let alpha0 = 0.05;
        let bank = SpacingLoss::new(alpha0, FIR_HALF);
        let x: Vec<f32> = (0..300).map(|n| ((n * 37 % 11) as f32 - 5.0) * 0.1).collect();
        let len = x.len() - 2 * FIR_HALF;
        let (mut clean, mut zero) = (vec![0.0; len], vec![0.0; len]);
        bank.apply(&x, None, &mut clean);
        bank.apply(&x, Some((&vec![0.0; len], &vec![1.0; len])), &mut zero);
        assert_eq!(clean, zero, "no extra loss means the static filter, bit for bit");
        let direct = spacing_taps(alpha0, FIR_HALF);
        assert_eq!(clean[10], dot(&direct, &x[10..10 + 2 * FIR_HALF + 1]));
        // more loss, less high-frequency energy
        let hf: Vec<f32> = (0..300).map(|n| if n % 2 == 0 { 1.0 } else { -1.0 }).collect();
        let (mut a, mut b) = (vec![0.0; len], vec![0.0; len]);
        bank.apply(&hf, None, &mut a);
        bank.apply(&hf, Some((&vec![0.5; len], &vec![1.0; len])), &mut b);
        assert!(b[50].abs() < 0.3 * a[50].abs().max(1e-9) || b[50].abs() < 0.02, "{} vs {}", b[50], a[50]);
        // the blend between two nodes lies between them
        let mut m = vec![0.0; len];
        bank.apply(&hf, Some((&vec![0.09; len], &vec![1.0; len])), &mut m);
        let (mut n1, mut n2) = (vec![0.0; len], vec![0.0; len]);
        bank.apply(&hf, Some((&vec![0.064; len], &vec![1.0; len])), &mut n1);
        bank.apply(&hf, Some((&vec![0.1024; len], &vec![1.0; len])), &mut n2);
        assert!(m[50].abs() <= n1[50].abs() + 1e-6 && m[50].abs() >= n2[50].abs() - 1e-6);
        // half the track covered: the mean of the clean and the lossy amplitude
        let mut half = vec![0.0; len];
        bank.apply(&hf, Some((&vec![0.5; len], &vec![0.5; len])), &mut half);
        assert!((half[50] - 0.5 * (a[50] + b[50])).abs() < 1e-6);
    }

    #[test]
    fn the_noise_depends_on_absolute_position_only() {
        let mut a = NoiseGen::new(3, STREAM_HISS);
        let mut whole = vec![0.0; 5000];
        a.fill(-1500, &mut whole);
        let mut b = NoiseGen::new(3, STREAM_HISS);
        let mut parts = vec![0.0; 5000];
        b.fill(-1500, &mut parts[..777]);
        b.fill(-1500 + 777, &mut parts[777..]);
        assert_eq!(whole, parts, "a split of the request does not change the noise");
        let mut c = NoiseGen::new(3, STREAM_HISS);
        let mut late = vec![0.0; 1000];
        c.fill(2000, &mut late);
        assert_eq!(&whole[3500..4500], &late[..], "nor does where the generator started");
        let mean = whole.iter().sum::<f32>() / 5000.0;
        let var = whole.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / 5000.0;
        assert!(mean.abs() < 0.06 && (var - 1.0).abs() < 0.08, "unit Gaussian: {mean} {var}");
        let mut other = NoiseGen::new(3, STREAM_HISS + 1);
        let mut o = vec![0.0; 5000];
        other.fill(-1500, &mut o);
        let corr = whole.iter().zip(&o).map(|(x, y)| x * y).sum::<f32>() / 5000.0;
        assert!(corr.abs() < 0.06, "channels have their own hiss: {corr}");
    }

    #[test]
    fn the_level_control_passes_small_signals_untouched() {
        let a = AudioParams::default();
        let mut alc = Alc::new(&a, 48_000.0);
        for n in 0..1000 {
            let x = 0.4 * (n as f64 * 0.1).sin();
            assert_eq!(alc.tick(x), x);
        }
        let mut loud = 0.0f64;
        for n in 0..48_000 {
            let y = alc.tick(1.5 * (n as f64 * 0.05).sin()).abs();
            if n > 40_000 {
                loud = loud.max(y);
            }
        }
        assert!(loud < 1.0, "a loud signal is compressed: {loud}");
    }

    #[test]
    fn hum_follows_the_supply_and_is_continuous_across_calls() {
        let mut p = params("dry-caps", 1);
        p.deck.supply.mains_hz = 60.0;
        let hum = Hum::new(&p, 48_000.0);
        let mut whole = vec![0.0; 4000];
        hum.fill(10_000, &mut whole);
        let mut a = vec![0.0; 1500];
        let mut b = vec![0.0; 2500];
        hum.fill(10_000, &mut a);
        hum.fill(11_500, &mut b);
        for (x, y) in whole.iter().zip(a.iter().chain(b.iter())) {
            assert!((x - y).abs() < 1e-9);
        }
        let peak = whole.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(peak > 0.5 * p.deck.supply.hum_ire * p.audio.hum_per_ire && peak < 3.0 * p.deck.supply.hum_ire * p.audio.hum_per_ire);
        p.deck.supply.hum_ire = 0.0;
        assert!(Hum::new(&p, 48_000.0).lines.is_empty());
    }

    #[test]
    fn stereo_tracks_see_different_defects_and_a_clean_tape_sees_none() {
        let mut p = params("worn", 3);
        p.tape.defects_per_mm2 = 20.0;
        let ap = AudioProcessor::new(&p, 48_000.0, 2);
        let (l, r) = (ap.defect_fields(0, 0, 96_000), ap.defect_fields(1, 0, 96_000));
        let (l, r) = (l.expect("defects on the left track").0, r.expect("defects on the right track").0);
        assert_ne!(l, r);
        assert!(l.iter().all(|a| *a >= 0.0));
        p.tape.defects_per_mm2 = 0.0;
        assert!(AudioProcessor::new(&p, 48_000.0, 2).defect_fields(0, 0, 96_000).is_none());
    }

    #[test]
    fn the_latency_is_the_delay_of_an_impulse_on_a_steady_deck() {
        let mut p = params("pristine", 1);
        p.audio.wow_gain = 0.0;
        p.audio.flutter_gain = 0.0;
        p.tape.defects_per_mm2 = 0.0;
        p.audio.hiss_head = 0.0;
        p.audio.hiss_tape = 0.0;
        p.deck.supply.hum_ire = 0.0;
        let mut ap = AudioProcessor::new(&p, 48_000.0, 1);
        let mut x = vec![0.0f32; 2000];
        x[100] = 0.2;
        let y = ap.process(&x, 0);
        let peak = y.iter().enumerate().fold((0, 0.0f32), |m, (i, v)| if v.abs() > m.1 { (i, v.abs()) } else { m });
        let lat = ap.latency();
        // the equaliser adds a little group delay on top
        assert!(peak.0 >= 100 + lat && peak.0 < 100 + lat + 40, "peak at {} for latency {lat}", peak.0);
    }
}
