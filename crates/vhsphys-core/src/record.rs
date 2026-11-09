//! Record side of the deck: baseband video -> luma FM + colour-under -> record
//! current -> saturating tape magnetisation (two-state relay model).
//!
//! The record head field crosses the tape coercivity where the head current
//! crosses +/-`relay_threshold`; the tape keeps the last state it was driven to.
//! Sub-sample crossing times are retained so that transition timing is not
//! quantised to the 43 MS/s grid (a one-sample error would cap the carrier to
//! noise ratio near 15 dB). Because the sum of the FM carrier and the (much
//! smaller) colour-under signal is what crosses the threshold, the chroma is
//! recorded as a duty-cycle modulation of the FM carrier: the FM luma acts as
//! its bias, exactly as in the VHS standard.

use serde::{Deserialize, Serialize};
use crate::consts::*;
use crate::dsp::*;
use crate::video::Baseband;
use std::f64::consts::PI;
use std::sync::OnceLock;

/// One magnetisation reversal on tape.
#[derive(Clone, Copy, Debug)]
pub struct Transition {
    /// Position in RF samples from the start of the track window (fractional).
    pub t: f64,
    /// +1: rising reversal (state -> +1), -1: falling.
    pub s: i8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordParams {
    /// Input (record) luma bandwidth.
    pub luma_lpf_hz: f64,
    /// Emphasis time constant and boost ratio (standard: 1.3 us, 5x).
    pub emph_tau: f64,
    pub emph_ratio: f64,
    pub white_clip_ire: f32,
    pub dark_clip_ire: f32,
    /// Modulator carrier frequency error (fraction) and gain error.
    pub fm_freq_error: f64,
    pub fm_deviation_scale: f64,
    /// FM high-pass before the head (>17 dB down at 629 kHz).
    pub fm_hpf_hz: f64,
    /// Per-component chroma bandwidth (record band-pass, +/-0.5 MHz).
    pub chroma_lpf_hz: f64,
    /// Colour-under amplitude relative to the FM carrier for |C| = 100 IRE.
    pub chroma_level: f64,
    pub burst_boost: f32,
    /// Head current threshold (relative to FM amplitude 1) = tape coercivity.
    pub relay_threshold: f64,
    /// Asymmetry of the FM drive (DC offset of the record current): a failing
    /// modulator gives an asymmetrical, distorted FM carrier.
    pub drive_offset: f64,
}

impl Default for RecordParams {
    fn default() -> Self {
        Self {
            luma_lpf_hz: 4.2e6,
            emph_tau: PRE_EMPH_TAU,
            emph_ratio: PRE_EMPH_RATIO,
            white_clip_ire: WHITE_CLIP_IRE,
            dark_clip_ire: DARK_CLIP_IRE,
            fm_freq_error: 0.0,
            fm_deviation_scale: 1.0,
            fm_hpf_hz: 1.6e6,
            chroma_lpf_hz: 0.65e6,
            chroma_level: 0.30,
            burst_boost: 2.0,
            relay_threshold: 0.30,
            drive_offset: 0.0,
        }
    }
}

/// cos/sin of the colour-under carrier phase across one line (40 cycles).
fn carrier_table() -> &'static (Vec<f32>, Vec<f32>) {
    static T: OnceLock<(Vec<f32>, Vec<f32>)> = OnceLock::new();
    T.get_or_init(|| {
        let c = (0..RF_PER_LINE).map(|r| (2.0 * PI * 40.0 * r as f64 / RF_PER_LINE as f64).cos() as f32).collect();
        let s = (0..RF_PER_LINE).map(|r| (2.0 * PI * 40.0 * r as f64 / RF_PER_LINE as f64).sin() as f32).collect();
        (c, s)
    })
}

/// Phase rotation applied to the colour-under carrier on a line:
/// track parity 0 advances +90 deg per line, parity 1 retards 90 deg.
/// Returns the unit vector (cos, sin) of the rotation.
pub fn line_rotation(parity: usize, line: i64) -> (f32, f32) {
    let m = line.rem_euclid(4) as usize;
    let m = if parity == 0 { m } else { (4 - m) % 4 };
    [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)][m]
}

/// Source of baseband video: concatenated frames addressed by baseband sample
/// index. Out-of-range regions are blank (sync, no picture).
pub trait VideoSource: Sync {
    fn fill(&self, bb_start: i64, y: &mut [f32], cu: &mut [f32], cv: &mut [f32]);
}

/// A fixed list of encoded frames, blank outside.
pub struct FrameStore {
    pub frames: Vec<Baseband>,
    blank: Baseband,
}

impl FrameStore {
    pub fn new(frames: Vec<Baseband>) -> Self {
        Self { frames, blank: Baseband::blank() }
    }
}

impl VideoSource for FrameStore {
    fn fill(&self, bb_start: i64, y: &mut [f32], cu: &mut [f32], cv: &mut [f32]) {
        let n = y.len();
        let mut i = 0;
        while i < n {
            let g = bb_start + i as i64;
            let fr = g.div_euclid(FRAME_BB as i64);
            let off = g.rem_euclid(FRAME_BB as i64) as usize;
            let take = (FRAME_BB - off).min(n - i);
            let src = if fr >= 0 && (fr as usize) < self.frames.len() { &self.frames[fr as usize] } else { &self.blank };
            y[i..i + take].copy_from_slice(&src.y[off..off + take]);
            cu[i..i + take].copy_from_slice(&src.cu[off..off + take]);
            cv[i..i + take].copy_from_slice(&src.cv[off..off + take]);
            i += take;
        }
    }
}

/// Luma FM modulation of an RF-rate luma signal (IRE) -> carrier in [-1, 1].
pub fn fm_modulate(y_rf: &[f32], p: &RecordParams, out: &mut Vec<f32>) {
    out.clear();
    out.reserve(y_rf.len());
    let mut phase = 0.0f64;
    let k = 2.0 * PI / FS_RF;
    let f0 = F_SYNC_TIP * (1.0 + p.fm_freq_error);
    for &y in y_rf {
        let f = f0 + (y as f64 + 40.0) * HZ_PER_IRE * p.fm_deviation_scale;
        phase += k * f;
        if phase > PI {
            phase -= 2.0 * PI;
        }
        out.push(phase.sin() as f32);
    }
}

/// Run the record chain for one track and return the magnetisation reversals.
///
/// `g0` is the global RF index of the first window sample (multiple of 3);
/// `n_rf` the window length (multiple of 3); `parity` selects head/track 1 or 2.
pub fn record_track(src: &dyn VideoSource, p: &RecordParams, parity: usize, g0: i64, n_rf: usize) -> Vec<Transition> {
    assert!(g0 % OS as i64 == 0 && n_rf % OS == 0);
    const PAD: usize = 2 * BB_PER_LINE; // filter warm-up lines each side
    let n_bb = n_rf / OS;
    let bb0 = g0 / OS as i64 - PAD as i64;
    let tot = n_bb + 2 * PAD;
    let mut y = vec![0.0f32; tot];
    let mut cu = vec![0.0f32; tot];
    let mut cv = vec![0.0f32; tot];
    src.fill(bb0, &mut y, &mut cu, &mut cv);

    // ---- chroma: burst boost, record band-limiting, then upsample ----
    for i in 0..tot {
        let col = (bb0 + i as i64).rem_euclid(BB_PER_LINE as i64) as usize;
        if (BURST_START..BURST_START + BURST_LEN).contains(&col) {
            cu[i] *= p.burst_boost;
            cv[i] *= p.burst_boost;
        }
    }
    for c in [&mut cu, &mut cv] {
        let mut f = butter_lp(2, p.chroma_lpf_hz, FS_BB);
        f.settle(0.0);
        f.process(c);
    }
    let (mut cu_rf, mut cv_rf, mut y_rf) = (Vec::new(), Vec::new(), Vec::new());
    upsample3(&cu, &mut cu_rf);
    upsample3(&cv, &mut cv_rf);

    // ---- luma: bandwidth, emphasis, clip ----
    let mut lpf = butter_lp(2, p.luma_lpf_hz, FS_BB);
    lpf.settle(0.0);
    lpf.process(&mut y);
    upsample3(&y, &mut y_rf);
    let tau = p.emph_tau;
    let mut emph = Biquad::first_order_pz(1.0 / tau, p.emph_ratio / tau, FS_RF);
    emph.settle(0.0);
    emph.process(&mut y_rf);
    for v in y_rf.iter_mut() {
        *v = v.clamp(p.dark_clip_ire, p.white_clip_ire);
    }

    // ---- FM carrier, record high-pass ----
    let mut fm = Vec::new();
    fm_modulate(&y_rf, p, &mut fm);
    let mut hpf = butter_hp(2, p.fm_hpf_hz, FS_RF);
    hpf.process(&mut fm);

    // ---- colour-under + sum + relay ----
    let (tc, ts) = carrier_table();
    let a = (p.chroma_level / 100.0) as f32;
    let off = PAD * OS;
    let thr = p.relay_threshold as f32;
    let mut out: Vec<Transition> = Vec::with_capacity(n_rf / 5);
    let mut state: i8 = if fm[off] > 0.0 { 1 } else { -1 };
    let mut prev_i = 0.0f32;
    for n in 0..n_rf {
        let gi = g0 + n as i64;
        let line = gi.div_euclid(RF_PER_LINE as i64);
        let r = gi.rem_euclid(RF_PER_LINE as i64) as usize;
        let (rc, rs) = line_rotation(parity, line);
        // carrier phasor e^{j(theta + rho)}
        let (cr, ci) = (tc[r] * rc - ts[r] * rs, tc[r] * rs + ts[r] * rc);
        let u = a * (cu_rf[off + n] * cr - cv_rf[off + n] * ci);
        let i_rec = fm[off + n] + u + p.drive_offset as f32;
        if n == 0 {
            prev_i = i_rec;
            continue;
        }
        if state < 0 && i_rec > thr {
            let t = (n - 1) as f64 + ((thr - prev_i) / (i_rec - prev_i)) as f64;
            out.push(Transition { t, s: 1 });
            state = 1;
        } else if state > 0 && i_rec < -thr {
            let t = (n - 1) as f64 + ((-thr - prev_i) / (i_rec - prev_i)) as f64;
            out.push(Transition { t, s: -1 });
            state = -1;
        }
        prev_i = i_rec;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constant-luma source (sync-free) for carrier-frequency checks.
    struct Flat(f32);
    impl VideoSource for Flat {
        fn fill(&self, _s: i64, y: &mut [f32], cu: &mut [f32], cv: &mut [f32]) {
            y.fill(self.0);
            cu.fill(0.0);
            cv.fill(0.0);
        }
    }

    fn mean_freq(tr: &[Transition]) -> f64 {
        let n = tr.len();
        let span = tr[n - 1].t - tr[0].t;
        (n as f64 - 1.0) / 2.0 / span * FS_RF
    }

    #[test]
    fn carrier_frequency_follows_luma_level() {
        let p = RecordParams { luma_lpf_hz: 20e6, emph_ratio: 1.0, ..Default::default() };
        for (ire, want) in [(-40.0f32, 3.4e6), (0.0, 3.4e6 + 40.0 * HZ_PER_IRE), (100.0, 4.4e6)] {
            // emphasis ratio 1 (flat): the clip must not engage for these levels
            let tr = record_track(&Flat(ire), &p, 0, 0, 30_000);
            let f = mean_freq(&tr);
            assert!((f - want).abs() < 0.01e6, "ire {ire}: {f} vs {want}");
        }
    }

    #[test]
    fn transitions_alternate_and_are_sub_sample() {
        let p = RecordParams { emph_ratio: 1.0, ..Default::default() };
        let tr = record_track(&Flat(50.0), &p, 1, 0, 30_000);
        for w in tr.windows(2) {
            assert_eq!(w[0].s, -w[1].s);
            assert!(w[1].t > w[0].t + 3.0);
        }
        let frac: Vec<f64> = tr.iter().map(|t| t.t.fract()).collect();
        let distinct = frac.iter().filter(|f| **f > 0.05 && **f < 0.95).count();
        assert!(distinct > tr.len() / 2, "positions should not be on the sample grid");
    }

    #[test]
    fn clip_bounds_carrier_frequency() {
        // a huge luma step is clipped to the 3.0 .. 5.0 MHz deviation limits
        let p = RecordParams { luma_lpf_hz: 20e6, emph_ratio: 1.0, ..Default::default() };
        let hi = mean_freq(&record_track(&Flat(500.0), &p, 0, 0, 30_000));
        let lo = mean_freq(&record_track(&Flat(-500.0), &p, 0, 0, 30_000));
        assert!((hi - 5.0e6).abs() < 0.02e6, "{hi}");
        assert!((lo - 3.0e6).abs() < 0.02e6, "{lo}");
    }

    #[test]
    fn rotation_alternates_by_head() {
        assert_eq!(line_rotation(0, 1), (0.0, 1.0));
        assert_eq!(line_rotation(1, 1), (0.0, -1.0));
        assert_eq!(line_rotation(0, 4), (1.0, 0.0));
        assert_eq!(line_rotation(1, 2), (-1.0, 0.0));
    }

    /// Constant-chroma source for the duty-cycle test.
    struct FlatChroma(f32, f32, f32);
    impl VideoSource for FlatChroma {
        fn fill(&self, _s: i64, y: &mut [f32], cu: &mut [f32], cv: &mut [f32]) {
            y.fill(self.0);
            cu.fill(self.1);
            cv.fill(self.2);
        }
    }

    /// The chroma is recorded as a duty-cycle modulation of the FM carrier. With
    /// threshold h and amplitude A the low-frequency magnetisation is
    /// dM/dc = 2 / (pi A sqrt(1 - (h/A)^2)) per unit of chroma current c.
    #[test]
    fn chroma_is_recorded_as_duty_cycle_of_the_fm_carrier() {
        let p = RecordParams { luma_lpf_hz: 20e6, emph_ratio: 1.0, burst_boost: 1.0, chroma_level: 0.30, ..Default::default() };
        let n = 20 * RF_PER_LINE;
        let tr = record_track(&FlatChroma(20.0, 100.0, 0.0), &p, 0, 0, n);
        // render the two-state magnetisation at RF resolution (box-averaged)
        let mut m = vec![0.0f64; n];
        let mut state = -tr[0].s as f64;
        let mut last = 0.0f64;
        for t in &tr {
            let i = t.t.floor() as usize;
            if i >= n { break; }
            // fill whole samples up to i with the old state, then split sample i
            for k in last as usize..i { m[k] = state; }
            let frac = t.t - i as f64;
            m[i] = state * frac + (t.s as f64) * (1.0 - frac);
            state = t.s as f64;
            last = (i + 1) as f64;
        }
        // amplitude of the 40-cycles-per-line component, averaged over lines
        let mut amps = vec![];
        for line in 4..16 {
            let seg = &m[line * RF_PER_LINE..(line + 1) * RF_PER_LINE];
            let (mut re, mut im) = (0.0, 0.0);
            for (r, v) in seg.iter().enumerate() {
                let a = 2.0 * PI * 40.0 * r as f64 / RF_PER_LINE as f64;
                re += v * a.cos();
                im += v * a.sin();
            }
            amps.push(2.0 * (re * re + im * im).sqrt() / RF_PER_LINE as f64);
        }
        let amp = amps.iter().sum::<f64>() / amps.len() as f64;
        let (a, h, c) = (1.0f64, 0.30f64, 0.30f64);
        // exact duty cycle for this c, evaluated at the peak of the chroma carrier
        let exact = ((-(h + c) / a).acos() + ((h - c) / a).acos()) / PI - 1.0;
        let small = 2.0 / (PI * a * (1.0 - (h / a).powi(2)).sqrt()) * c;
        assert!((amp - small).abs() < 0.03, "measured {amp}, small-signal {small}, exact {exact}");
    }
}
