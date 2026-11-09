//! Common playback electronics after the head switch: equaliser, dropout
//! compensator, limiter, delay-line FM discriminator, de-emphasis, luma
//! low-pass/aperture corrector, colour-under extraction.
//!
//! Every stage is an explicit circuit model so that component drift (R, C,
//! delay-line detuning) and supply faults change the picture the way they do in
//! a real deck. Fixed "alignment" constants (e.g. the discriminator's assumed
//! delay) come from the deck's nominal design; faults perturb the *actual*
//! values only.

use serde::{Deserialize, Serialize};
use crate::consts::*;
use crate::dsp::*;

/// Nominal discriminator delay assumed by the downstream scaling (samples).
pub const DEMOD_DELAY_NOMINAL: f64 = 3.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DeckParams {
    /// Playback equaliser peaks: (centre Hz, Q, gain dB).
    pub eq_peaks: Vec<(f64, f64, f64)>,
    /// Gain applied after the equaliser (alignment: nominal carrier -> 1.0).
    pub fm_gain: f64,
    /// Slow RF level control ahead of the compensator and limiter: absorbs head
    /// wear and tape output differences (time constant `agc_ms`, gain range
    /// 0.25 .. `agc_max_gain`) so that only *fast* dips trip the compensator.
    pub agc_enabled: bool,
    pub agc_ms: f64,
    pub agc_target: f64,
    pub agc_max_gain: f64,
    pub doc_enabled: bool,
    /// Dropout compensator trip level and release level, fraction of the AGC
    /// reference level (the release level gives the hysteresis).
    pub doc_trip: f64,
    pub doc_release: f64,
    /// Gain per recirculation of the line delay (glass ~0.85, CCD 1.0).
    pub doc_recirculation: f64,
    /// Limiter comparator hysteresis as a fraction of the nominal carrier
    /// amplitude (finite limiter gain / input-referred offset).
    pub limiter_threshold: f64,
    /// FM-path high-pass ahead of the limiter: keeps the colour-under (and the
    /// FM audio band) out of the discriminator.
    pub fm_hpf_hz: f64,
    /// Actual discriminator delay (samples); nominal is `DEMOD_DELAY_NOMINAL`.
    pub demod_delay: f64,
    /// Discriminator output low-pass at RF rate (first stage against the
    /// 2 x carrier product) ...
    pub demod_lpf_hz: f64,
    pub demod_lpf_order: usize,
    /// ... and the linear-phase video low-pass after decimation (the deck's
    /// ~3 MHz luminance bandwidth).
    pub luma_lpf_hz: f64,
    pub luma_lpf_transition_hz: f64,
    /// Scale factors on the de-emphasis network's time constant and ratio.
    pub deemph_tau_scale: f64,
    pub deemph_ratio_scale: f64,
    /// Aperture corrector ("sharpness"): centre, Q, gain dB.
    pub sharp_freq: f64,
    pub sharp_q: f64,
    pub sharp_db: f64,
    /// Colour-under extraction low-pass.
    pub chroma_lpf_hz: f64,
    /// Additive disturbances (IRE) injected after the discriminator, keyed by
    /// global RF sample index: supplies, hum, switching ripple.
    pub supply: SupplyParams,
    /// Coupling capacitor in the luma path: low-frequency cutoff (Hz). Aged
    /// electrolytics (lower C, higher ESR) raise it and cause tilt/streaking.
    pub coupling_hpf_hz: f64,
    /// DC offset error of the discriminator (IRE), e.g. a detuned tank.
    pub dc_error_ire: f64,
}

/// Power-supply related disturbances reaching the video path.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SupplyParams {
    /// Peak-to-peak hum at the discriminator output, IRE.
    pub hum_ire: f64,
    /// Mains frequency (Hz) and ripple harmonic content (0 = sine at 2 x mains
    /// only, 1 = strongly peaked charging pulses).
    pub mains_hz: f64,
    pub ripple_peakiness: f64,
    /// Switch-mode supply: frequency (Hz) and amplitude (IRE p-p) of the
    /// switching ripple leaking into the video path.
    pub smps_hz: f64,
    pub smps_ire: f64,
    /// Random phase for the supply waveforms (derived from the seed).
    pub phase: f64,
}

impl Default for DeckParams {
    fn default() -> Self {
        Self {
            eq_peaks: vec![(5.4e6, 0.9, 6.0), (7.2e6, 1.2, 3.0)],
            fm_gain: 1.0,
            agc_enabled: true,
            agc_ms: 6.0,
            agc_target: 0.6,
            agc_max_gain: 6.0,
            doc_enabled: true,
            doc_trip: 0.30,
            doc_release: 0.45,
            doc_recirculation: 1.0,
            limiter_threshold: 0.03,
            fm_hpf_hz: 1.8e6,
            demod_delay: DEMOD_DELAY_NOMINAL,
            demod_lpf_hz: 4.5e6,
            demod_lpf_order: 6,
            luma_lpf_hz: 3.0e6,
            luma_lpf_transition_hz: 2.0e6,
            deemph_tau_scale: 1.0,
            deemph_ratio_scale: 1.0,
            sharp_freq: 2.2e6,
            sharp_q: 0.7,
            sharp_db: 3.0,
            chroma_lpf_hz: 1.2e6,
            supply: SupplyParams::default(),
            coupling_hpf_hz: 15.0,
            dc_error_ire: 0.0,
        }
    }
}

/// What a segment of playback produced (all at the baseband rate, 4 x fsc).
pub struct SegmentOut {
    pub luma: Vec<f32>,
    pub cunder: Vec<f32>,
    pub dropout: Vec<u8>,
}

/// Supply disturbance at global RF sample `g`: `(offset_ire, gain_modulation)`.
///
/// The supply rail feeds the video amplifiers, so ripple both shifts the level
/// (which the TV's back-porch clamp removes again) and modulates the gain of the
/// stages (which it cannot: whites move while black stays put). The gain term is
/// what makes hum bars visible. Hum is the 2 x mains ripple (a full-wave
/// rectified waveform, sharper as the filter capacitor's ESR grows); a switch
/// mode supply adds a fixed-frequency ripple that beats with the line rate.
pub fn supply_disturbance(s: &SupplyParams, g: i64) -> (f32, f32) {
    let t = g as f64 / FS_RF;
    let (mut add, mut gain) = (0.0, 0.0);
    if s.hum_ire != 0.0 {
        let w = 2.0 * std::f64::consts::PI * 2.0 * s.mains_hz * t + s.phase;
        let saw = (w.sin() + 0.35 * s.ripple_peakiness * (2.0 * w).sin() + 0.2 * s.ripple_peakiness * (3.0 * w).sin())
            / (1.0 + 0.35 * s.ripple_peakiness);
        add += 0.5 * s.hum_ire * saw;
        gain += 0.5 * s.hum_ire / 100.0 * 0.8 * saw;
    }
    if s.smps_ire != 0.0 {
        let w = 2.0 * std::f64::consts::PI * s.smps_hz * t + 1.7 * s.phase;
        add += 0.5 * s.smps_ire * w.sin();
    }
    (add as f32, gain as f32)
}

/// Process one playback segment. `rf` is the head-switched RF (after head
/// amplifiers); `g0` its global RF index. The output covers the same span.
pub fn process_segment(rf: &[f32], p: &DeckParams, g0: i64) -> SegmentOut {
    let n = rf.len();
    let mut x: Vec<f32> = rf.to_vec();

    // ---- playback equaliser and level ----
    for &(f, q, g) in &p.eq_peaks {
        let mut b = Biquad::peaking(f, q, g, FS_RF);
        b.process(&mut x);
    }
    let gain = p.fm_gain as f32;
    for v in x.iter_mut() {
        *v *= gain;
    }

    // ---- slow RF level control (feed-forward AGC on the mean rectified level) ----
    let reference = if p.agc_enabled { p.agc_target as f32 } else { 1.0 };
    if p.agc_enabled {
        let alpha = (1.0 - (-1.0 / (p.agc_ms * 1e-3 * FS_RF)).exp()) as f32;
        let k = n.min(4000);
        let mut avg = x[..k].iter().map(|v| v.abs()).sum::<f32>() / k as f32;
        let (gmin, gmax) = (0.25f32, p.agc_max_gain as f32);
        for v in x.iter_mut() {
            let a = v.abs();
            let g = (reference / (avg * std::f32::consts::FRAC_PI_2).max(1e-6)).clamp(gmin, gmax);
            avg += alpha * (a - avg);
            *v *= g;
        }
    }

    // ---- dropout compensator: envelope detect, hysteresis, 1 H recirculation ----
    let mut doc = vec![0u8; n];
    if p.doc_enabled {
        let decay = (-1.0 / (0.35e-6 * FS_RF)).exp() as f32;
        let mut env = reference;
        let mut active = false;
        let mut hold = 0usize;
        let (trip, rel) = (p.doc_trip as f32 * reference, p.doc_release as f32 * reference);
        let rec = p.doc_recirculation as f32;
        for i in 0..n {
            env = (x[i].abs()).max(env * decay);
            if !active && env < trip {
                active = true;
            } else if active && env > rel {
                if hold == 0 {
                    hold = 6;
                }
                hold -= 1;
                if hold == 0 {
                    active = false;
                }
            } else if active {
                hold = 0;
            }
            if active && i >= RF_PER_LINE {
                x[i] = x[i - RF_PER_LINE] * rec;
                doc[i] = 1;
            }
        }
    }

    // ---- colour-under branch: tapped before the limiter ----
    let nb = n / OS;
    let mut cu: Vec<f32> = (0..nb).map(|k| (x[OS * k] + x[OS * k + 1] + x[OS * k + 2]) / 3.0).collect();
    let mut clpf = butter_lp(4, p.chroma_lpf_hz, FS_BB);
    clpf.process(&mut cu);
    // chroma is killed while the compensator is active
    let mut dropout = vec![0u8; nb];
    for k in 0..nb {
        if doc[OS * k] | doc[OS * k + 1] | doc[OS * k + 2] != 0 {
            dropout[k] = 1;
        }
    }
    // spread the kill a little (filter memory)
    for k in 0..nb {
        if dropout[k] == 1 {
            cu[k] = 0.0;
        }
    }

    // ---- limiter + delay-line discriminator ----
    let mut fmx = x.clone();
    butter_hp(4, p.fm_hpf_hz, FS_RF).process(&mut fmx);
    let mut prod = discriminate(&fmx, p.limiter_threshold as f32, p.demod_delay);
    let mut lpf = butter_lp(p.demod_lpf_order, p.demod_lpf_hz, FS_RF);
    // the discriminator idles at the sync-tip... settle on the 0 IRE level
    lpf.settle(r_of_freq(F_SYNC_TIP + 40.0 * HZ_PER_IRE) as f64);
    lpf.process(&mut prod);

    // product -> frequency -> IRE using the *nominal* design delay
    let tau = DEMOD_DELAY_NOMINAL / FS_RF;
    let mut y = vec![0.0f32; n];
    for i in 0..n {
        let f = (1.0 - prod[i] as f64) / (4.0 * tau);
        y[i] = ((f - F_SYNC_TIP) / HZ_PER_IRE - 40.0) as f32;
    }

    // ---- de-emphasis (inverse of the record emphasis, with component drift) ----
    let t = PRE_EMPH_TAU * p.deemph_tau_scale;
    let ratio = PRE_EMPH_RATIO * p.deemph_ratio_scale;
    let mut de = Biquad::first_order_pz(ratio / t, 1.0 / t, FS_RF);
    de.settle(y[0] as f64);
    de.process(&mut y);

    // ---- coupling capacitor (high-pass), luma low-pass, aperture corrector ----
    let mut cpl = Biquad::one_pole_hp(p.coupling_hpf_hz, FS_RF);
    let mid = y[..y.len().min(RF_PER_LINE)].iter().sum::<f32>() / y.len().min(RF_PER_LINE) as f32;
    cpl.settle(mid as f64);
    // DC is restored by the TV clamp, so the high-pass only needs to shape tilt:
    for v in y.iter_mut() {
        *v = (cpl.tick(*v as f64) as f32) + mid;
    }
    // decimate to baseband, then the video low-pass (zero-phase FIR) and the
    // aperture corrector
    let mut lb: Vec<f32> = (0..nb).map(|k| y[OS * k + 1]).collect();
    let fir = kaiser_lowpass(p.luma_lpf_hz, p.luma_lpf_transition_hz, 55.0, FS_BB);
    lb = fir_centred(&lb, &fir);
    if p.sharp_db != 0.0 {
        let mut sh = Biquad::peaking(p.sharp_freq, p.sharp_q, p.sharp_db, FS_BB);
        sh.settle(mid as f64);
        sh.process(&mut lb);
    }

    // ---- supply disturbances and detuning ----
    let have_supply = p.supply.hum_ire != 0.0 || p.supply.smps_ire != 0.0;
    let mut luma = Vec::with_capacity(nb);
    for k in 0..nb {
        let mut v = lb[k];
        if have_supply {
            let (add, gm) = supply_disturbance(&p.supply, g0 + (OS * k + 1) as i64);
            v = v * (1.0 + gm) + add;
        }
        luma.push(v + p.dc_error_ire as f32);
    }
    SegmentOut { luma, cunder: cu, dropout }
}

/// Hard limiter followed by a delay-line product detector, evaluated exactly
/// in continuous time.
///
/// The limiter is a Schmitt comparator (hysteresis `th`) whose switching times
/// are interpolated to sub-sample accuracy; the product of the limited wave
/// with itself delayed by `delay` samples is piecewise constant, so it is
/// integrated over each sample interval (area sampling). This avoids the
/// edge-timing quantisation (about 6.7 ns at 43 MS/s, ~0.16 rad of phase noise
/// at 3.9 MHz) that clipping the sampled waveform would add, and has no aliased
/// limiter harmonics. For an ideal square wave the output is `1 - 4 f tau`.
pub fn discriminate(x: &[f32], th: f32, delay: f64) -> Vec<f32> {
    let n = x.len();
    let mut edges: Vec<f64> = Vec::with_capacity(n / 4);
    let mut state: i8 = if x[0] >= 0.0 { 1 } else { -1 };
    for i in 1..n {
        let (a, b) = (x[i - 1], x[i]);
        if state < 0 && b > th {
            edges.push((i - 1) as f64 + ((th - a) / (b - a)) as f64);
            state = 1;
        } else if state > 0 && b < -th {
            edges.push((i - 1) as f64 + ((-th - a) / (b - a)) as f64);
            state = -1;
        }
    }
    // merge the edges of the wave and of its delayed copy (both sorted) and
    // integrate the piecewise-constant product on a 4x finer grid
    const SUB: usize = 4;
    let nb = n * SUB;
    let mut fine = vec![0.0f32; nb];
    let (mut i, mut j) = (0usize, 0usize);
    let mut sign = 1.0f32; // steady-state assumption before the first edge
    let mut t_prev = 0.0f64;
    let mut acc = 0.0f64;
    let mut cur = 0usize;
    let w = 1.0 / SUB as f64;
    let next_edge = |i: usize, j: usize| -> Option<(f64, bool)> {
        let a = edges.get(i).copied();
        let b = edges.get(j).map(|e| e + delay);
        match (a, b) {
            (Some(a), Some(b)) => Some(if a <= b { (a, true) } else { (b, false) }),
            (Some(a), None) => Some((a, true)),
            (None, Some(b)) => Some((b, false)),
            (None, None) => None,
        }
    };
    loop {
        let (te, first) = next_edge(i, j).unwrap_or((n as f64, true));
        let te = te.min(n as f64);
        let mut t = t_prev;
        while t < te {
            let end = ((cur + 1) as f64 * w).min(te);
            acc += sign as f64 * (end - t);
            t = end;
            if t >= (cur + 1) as f64 * w {
                if cur < nb {
                    fine[cur] = (acc / w) as f32;
                }
                acc = 0.0;
                cur += 1;
            }
        }
        t_prev = te;
        if te >= n as f64 {
            break;
        }
        sign = -sign;
        if first { i += 1 } else { j += 1 }
    }
    // anti-alias with a two-sample-wide triangle (sinc^2 response, nulls at
    // multiples of fs), then decimate by SUB
    let tri = [1.0f32, 2.0, 3.0, 4.0, 3.0, 2.0, 1.0];
    let mut out = vec![0.0f32; n];
    for (k, o) in out.iter_mut().enumerate() {
        let centre = k * SUB + SUB / 2; // box of the original sample is centred here
        let mut v = 0.0;
        for (m, &c) in tri.iter().enumerate() {
            let idx = (centre + m) as i64 - 3;
            if idx >= 0 && (idx as usize) < nb {
                v += c * fine[idx as usize];
            }
        }
        *o = v / 16.0;
    }
    out
}

/// Idle value of the product detector (hard-limited) for carrier frequency `f`.
fn r_of_freq(f: f64) -> f32 {
    let tau = DEMOD_DELAY_NOMINAL / FS_RF;
    (1.0 - 4.0 * f * tau) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// A pure FM carrier at a known frequency must demodulate to the matching
    /// IRE level through limiter + delay-line discriminator.
    #[test]
    fn discriminator_maps_frequency_to_ire() {
        for ire in [-40.0, 0.0, 50.0, 100.0] {
            let f = F_SYNC_TIP + (ire + 40.0) * HZ_PER_IRE;
            let n = 20_000;
            let rf: Vec<f32> = (0..n).map(|i| (0.5 * (2.0 * PI * f * i as f64 / FS_RF).sin()) as f32).collect();
            let p = DeckParams { eq_peaks: vec![], sharp_db: 0.0, doc_enabled: false, fm_gain: 2.0, ..Default::default() };
            let out = process_segment(&rf, &p, 0);
            let mid = &out.luma[n / OS / 2..n / OS / 2 + 200];
            let m = mid.iter().sum::<f32>() / mid.len() as f32;
            assert!((m as f64 - ire).abs() < 2.5, "ire {ire}: got {m}");
        }
    }

    /// Exact discriminator: for an ideal square wave the DC output is 1 - 4 f tau
    /// at every frequency, with no timing quantisation spurs.
    #[test]
    fn discriminator_is_exact_and_spur_free() {
        for f in [3.4e6, 3.9e6, 4.4e6] {
            let n = 40_000;
            let x: Vec<f32> = (0..n).map(|i| (2.0 * PI * f * i as f64 / FS_RF).sin() as f32).collect();
            let d = discriminate(&x, 0.03, 3.0);
            let mid = &d[2000..n - 2000];
            let mean = mid.iter().sum::<f32>() / mid.len() as f32;
            let want = 1.0 - 4.0 * f * 3.0 / FS_RF;
            // LPF the product so only the in-band content remains, then check ripple
            let mut lp = butter_lp(8, 2.5e6, FS_RF); // -70 dB at 2 x carrier
            let mut y = d.clone();
            lp.process(&mut y);
            let r = &y[4000..n - 2000];
            let rip = r.iter().cloned().fold(f32::MIN, f32::max) - r.iter().cloned().fold(f32::MAX, f32::min);
            assert!((mean as f64 - want).abs() < 2e-3, "f={f}: {mean} vs {want}");
            assert!(rip < 4e-3, "f={f}: ripple {rip}"); // < 2 IRE p-p, dominated by one -61 dB alias spur
        }
    }

    /// With a stable carrier lost for a while, the compensator must switch in
    /// the previous line (so the output stays near the true level).
    #[test]
    fn dropout_compensator_repairs_a_dropout() {
        let f = 3.9e6;
        let n = 6 * RF_PER_LINE;
        let mut rf: Vec<f32> = (0..n).map(|i| (0.5 * (2.0 * PI * f * i as f64 / FS_RF).sin()) as f32).collect();
        let (a, b) = (3 * RF_PER_LINE + 500, 3 * RF_PER_LINE + 1700);
        for v in &mut rf[a..b] {
            *v *= 0.01;
        }
        let base = DeckParams { eq_peaks: vec![], sharp_db: 0.0, fm_gain: 2.0, ..Default::default() };
        let with = process_segment(&rf, &DeckParams { doc_enabled: true, ..base.clone() }, 0);
        let without = process_segment(&rf, &DeckParams { doc_enabled: false, ..base }, 0);
        let dev = |o: &SegmentOut| {
            let r = &o.luma[(a + 200) / OS..(b - 200) / OS];
            r.iter().map(|v| (v - 30.0).abs()).sum::<f32>() / r.len() as f32
        };
        // 3.9 MHz carrier = 30 IRE
        assert!(dev(&with) < 3.0, "with DOC the dropout region stays near 30 IRE: {}", dev(&with));
        assert!(dev(&without) > 3.0 * dev(&with), "doc {} vs none {}", dev(&with), dev(&without));
        assert!(with.dropout[(a + 50) / OS] == 1);
    }
}

