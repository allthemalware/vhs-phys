//! Camera pickup tube: the photoconductive target of a three-tube Plumbicon camera, the
//! source of the lag and the comet tails of 1970s and 1980s studio and outside-broadcast
//! video (the "trails" behind lamps and highlights).
//!
//! Each element of the target is a small capacitor. Light discharges it through the
//! photoconductor between scans (here: adds charge, counted positive); the scanning beam
//! recharges it to the cathode potential, and the charge it delivers is the signal. Two
//! physical limits of the beam make the picture remember the past:
//!
//! * **Discharge (capacitive) lag.** Near the cathode potential only the faster electrons
//!   of the beam can land (their energies are spread thermally, kT/e of a few tenths of a
//!   volt), so the landing current falls exponentially as the element approaches it:
//!   `i = I_b exp(u / V_T)` for `u <= 0`, `i = I_b` above. An element left slightly
//!   charged after a scan gives up the rest over the next scans. In charge units per dwell
//!   (`q = C u / t_dwell`, nA) the scan of one element is, exactly:
//!     1. above the cathode potential the full beam lands: `r1 = min(q, I_b)`, `q -= r1`;
//!     2. with the beam charge left, `I_b - r1`, below it: `exp(-q'/Q_T) = exp(-q/Q_T) + (I_b - r1)/Q_T`
//!   where `Q_T = C V_T / t_dwell` (`charge_scale_na`). The signal is the charge removed.
//!   Light bias adds a steady charge that keeps elements off the slow part of the curve.
//! * **Comet tails.** The beam is set to twice what peak white needs (data sheet). A
//!   highlight beyond that leaves charge the scan cannot remove; it accumulates (up to the
//!   point where the element reaches the signal-plate voltage and photoconduction stops,
//!   `saturation_na`) and drains at the beam current per frame after the highlight has
//!   gone. An anti-comet-tail (ACT) gun recharges, during line flyback, every element above
//!   the level the read beam can stabilise, so nothing is left over.
//!
//! The only fitted figure is `charge_scale_na` per channel, fitted to the decay lag at 60 and
//! 200 ms of the data sheet; everything else is the data sheet's operating point.
//! Source: Philips Data Handbook T10, Plumbicon camera tubes, 1985 (XQ1020, XQ1410, XQ1520),
//! lag at Is/Ib = 20/300 nA, low key, 50 Hz. See docs/CAMERA.md.

use serde::{Deserialize, Serialize};

/// Gamma of the light transfer characteristic (all three types: 0.95 +/- 0.05).
pub const GAMMA: f64 = 0.95;
/// The camera's gamma corrector follows the BT.709 transfer: linear (slope 4.5) below
/// 0.018, then `1.099 L^0.45 - 0.099`. The input video is taken through its inverse to get
/// the scene light and the tube's signal through it again, so a tube with gamma 1 and no
/// lag returns the input.
pub fn oetf(l: f64) -> f64 {
    let l = l.clamp(0.0, 1.0);
    if l < 0.018 { 4.5 * l } else { 1.099 * l.powf(0.45) - 0.099 }
}

/// Inverse of `oetf`.
pub fn oetf_inverse(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v < 0.081 { v / 4.5 } else { ((v + 0.099) / 1.099).powf(1.0 / 0.45) }
}
/// Peak-white signal current per channel `[R, G, B]`, nA (30 mm tubes).
pub const PEAK_SIGNAL_NA: [f64; 3] = [150.0, 300.0, 150.0];
/// Beam current as a multiple of what peak white needs, as the data sheet sets it.
pub const BEAM_RATIO_DEFAULT: f64 = 2.0;
/// Dark current, nA. The data sheets give maxima of 1 to 3 nA. [est]
pub const DARK_NA: f64 = 1.0;
/// Internal light bias of the XQ1410, nA per channel `[R, G, B]` (data sheet setting).
pub const LIGHT_BIAS_XQ1410_NA: [f64; 3] = [3.0, 2.0, 3.5];
/// Signal-plate (target) voltage, V.
pub const TARGET_VOLTAGE_V: f64 = 45.0;
/// Target capacitance, F: PbO layer of 10 to 18 um (data sheet; 15 used), relative
/// permittivity about 25 [est], scanned area 12.8 x 17.1 mm (data sheet).
pub const TARGET_CAPACITANCE_F: f64 = 3.2e-9;
/// The lag measurement condition of the data sheets: signal and beam current, nA.
pub const LAG_TEST_SIGNAL_NA: f64 = 20.0;
pub const LAG_TEST_BEAM_NA: f64 = 300.0;

/// Lag figures in percent: build-up at 60 ms (percent of the final signal), decay at 60 and
/// 200 ms (percent of the initial signal), at 50 Hz.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LagFigures {
    pub build_60: f64,
    pub decay_60: f64,
    pub decay_200: f64,
}

const fn lag(build_60: f64, decay_60: f64, decay_200: f64) -> LagFigures {
    LagFigures { build_60, decay_60, decay_200 }
}

/// Data-sheet lag `[R, G, B]`, low key, without light bias: XQ1020 (standard gun).
pub const XQ1020_LAG: [LagFigures; 3] = [lag(85.0, 12.0, 4.0), lag(95.0, 10.0, 3.5), lag(70.0, 15.0, 5.0)];
/// XQ1410 (the XQ1020 target and gun with internal light bias), with its light bias.
pub const XQ1410_LAG: [LagFigures; 3] = [lag(98.0, 8.0, 3.0), lag(98.0, 7.0, 2.0), lag(95.0, 11.0, 3.5)];
/// XQ1520 (anti-comet-tail gun), without light bias.
pub const XQ1520_LAG: [LagFigures; 3] = [lag(85.0, 13.0, 3.5), lag(95.0, 9.0, 3.0), lag(70.0, 15.0, 5.5)];

/// Which camera tube the picture was shot through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TubeKind {
    /// No camera stage: the input is the video signal itself.
    #[default]
    Off,
    /// Plumbicon, standard gun, no light bias (XQ1020): lag and comet tails.
    Plumbicon,
    /// Plumbicon with internal light bias (XQ1410): less lag, the same comet tails.
    PlumbiconBias,
    /// Plumbicon with an anti-comet-tail gun (XQ1520): lag, no comet tails.
    PlumbiconAct,
}

/// Charge scale `Q_T` per channel `[R, G, B]`, nA: fitted, one value per channel, to the decay lag
/// at 60 and 200 ms of the XQ1020 data (`XQ1020_LAG`) by `lag_measurement` at 0.04 s, least
/// squares on the logarithm of the two figures. The XQ1410 is the same target and gun, so it
/// uses these too (its lag is then a prediction, not a fit). See docs/CAMERA.md.
pub const CHARGE_SCALE_XQ1020_NA: [f64; 3] = [6.32, 5.64, 7.55];
/// The same fit to the XQ1520 (anti-comet-tail gun) data, `XQ1520_LAG`.
pub const CHARGE_SCALE_XQ1520_NA: [f64; 3] = [6.12, 5.12, 7.90];

/// One tube of the camera (one colour channel) at its operating point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TubeChannel {
    /// Signal current at peak white, nA.
    pub peak_signal_na: f64,
    /// Beam current, nA.
    pub beam_na: f64,
    /// `Q_T = C kT/e / t_dwell` in signal-current units, nA (fitted to the decay lag).
    pub charge_scale_na: f64,
    pub dark_na: f64,
    pub bias_na: f64,
    /// Charge at which an element reaches the signal-plate voltage, nA:
    /// `TARGET_CAPACITANCE_F * TARGET_VOLTAGE_V / active_scan_s`.
    pub saturation_na: f64,
    /// Anti-comet-tail gun.
    pub act: bool,
}

impl TubeKind {
    /// The three channels `[R, G, B]`, or `None` for `Off`. `beam_ratio` scales the beam
    /// current to that multiple of each channel's peak-white current; `active_scan_s` is the
    /// time per frame the beam spends on the picture (it sets `saturation_na`).
    pub fn channels(self, beam_ratio: f64, active_scan_s: f64) -> Option<[TubeChannel; 3]> {
        let charge_scale = match self {
            TubeKind::Off => return None,
            TubeKind::Plumbicon | TubeKind::PlumbiconBias => CHARGE_SCALE_XQ1020_NA,
            TubeKind::PlumbiconAct => CHARGE_SCALE_XQ1520_NA,
        };
        // C V / t in nA: the charge that takes an element from the cathode to the plate voltage
        let saturation_na = TARGET_CAPACITANCE_F * TARGET_VOLTAGE_V / active_scan_s * 1e9;
        Some(std::array::from_fn(|k| TubeChannel {
            peak_signal_na: PEAK_SIGNAL_NA[k],
            beam_na: beam_ratio * PEAK_SIGNAL_NA[k],
            charge_scale_na: charge_scale[k],
            dark_na: DARK_NA,
            bias_na: if self == TubeKind::PlumbiconBias { LIGHT_BIAS_XQ1410_NA[k] } else { 0.0 },
            saturation_na,
            act: self == TubeKind::PlumbiconAct,
        }))
    }
}

/// Photo current for a linear scene light (1.0 = the light that gives peak white), nA.
pub fn photo_current(ch: &TubeChannel, light: f64) -> f64 {
    ch.peak_signal_na * light.max(0.0).powf(GAMMA)
}

/// One frame of one element: add this frame's photo, dark and bias charge (stopping at
/// `saturation_na`), apply the ACT flyback recharge if fitted (no element above `beam_na`),
/// then scan as described in the module docs. Returns the signal read, nA.
pub fn frame_step(ch: &TubeChannel, q: &mut f64, photo_na: f64) -> f64 {
    let mut before = (*q + photo_na.max(0.0) + ch.dark_na + ch.bias_na).min(ch.saturation_na);
    if ch.act {
        // the flyback beam takes everything above what the read beam can stabilise; that
        // charge never reaches the signal
        before = before.min(ch.beam_na);
    }
    *q = scan(ch.beam_na, ch.charge_scale_na, before);
    before - *q
}

/// The scan of one element holding charge `q`: the charge it holds afterwards.
fn scan(beam: f64, scale: f64, q: f64) -> f64 {
    if beam <= 0.0 {
        return q;
    }
    // above the cathode potential the whole beam lands, up to the charge there is
    let landed = q.clamp(0.0, beam);
    let q = q - landed;
    let left = beam - landed;
    if left <= 0.0 {
        return q;
    }
    // below it, exp(-q'/Q_T) = exp(-q/Q_T) + left/Q_T
    let x = -q / scale;
    if x < 700.0 {
        return -scale * (x.exp() + left / scale).ln();
    }
    // an element left far below the cathode potential would overflow the exponential: the
    // same sum as a log-sum-exp
    let y = (left / scale).ln();
    let m = x.max(y);
    -scale * (m + ((x - m).exp() + (y - m).exp()).ln())
}

/// Charge of an element after a long time in the dark (the state a camera starts in).
pub fn dark_equilibrium(ch: &TubeChannel) -> f64 {
    // The fixed point of "add the dark and bias charge, scan". `a` is what a frame adds and,
    // in equilibrium, what the scan reads back.
    let a = ch.dark_na + ch.bias_na;
    let (beam, scale) = (ch.beam_na, ch.charge_scale_na);
    if a <= 0.0 {
        // nothing is added, so the beam keeps walking the charge down and there is no
        // equilibrium: one scan of an empty element is the state to start from
        return scan(beam, scale, 0.0);
    }
    if beam <= a {
        // the beam cannot hold off the dark current: the element fills up to the plate voltage
        return scan(beam, scale, ch.saturation_na);
    }
    // below the cathode potential before the scan (q_b <= 0): with q_a = q_b - a,
    // exp(-q_b/Q_T) (exp(a/Q_T) - 1) = beam/Q_T
    let growth = (a / scale).exp_m1();
    if beam / scale >= growth {
        let q_before = -scale * (beam / scale / growth).ln();
        return q_before - a;
    }
    // otherwise the element is above it and the beam lands in full before the rest charges it
    // below: solve q_b = a - Q_T ln(1 + (beam - q_b)/Q_T) for q_b in (0, a) by bisection
    let (mut lo, mut hi) = (0.0, a);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if mid - a + scale * (1.0 + (beam - mid) / scale).ln() < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi) - a
}

/// The data sheet's lag measurement on one channel at a frame period `frame_s`: signal and
/// beam set to `LAG_TEST_SIGNAL_NA` / `LAG_TEST_BEAM_NA` (keeping the channel's charge
/// scale, dark and bias currents), a uniform light on for 5 s then off (decay) and off for 10 s
/// then on (build-up). The change happens just after a scan, so scan k reads at
/// `k * frame_s` after it. Decay is the signal above the dark level as a percentage of
/// the signal before the change, interpolated log-linearly between scans to 60 and 200 ms;
/// build-up is the percentage of the final signal, interpolated linearly.
pub fn lag_measurement(ch: &TubeChannel, frame_s: f64) -> LagFigures {
    let t = TubeChannel { peak_signal_na: LAG_TEST_SIGNAL_NA, beam_na: LAG_TEST_BEAM_NA, ..*ch };
    let frames = |seconds: f64| (seconds / frame_s).ceil() as usize;
    // readings are kept for the scans that bracket 200 ms (and 60 ms, which comes first)
    let kept = frames(0.2) + 2;

    let mut q = dark_equilibrium(&t);
    let dark = frame_step(&t, &mut q, 0.0);

    // light on for 5 s, then off
    for _ in 0..frames(5.0) {
        frame_step(&t, &mut q, LAG_TEST_SIGNAL_NA);
    }
    let lit = frame_step(&t, &mut q, LAG_TEST_SIGNAL_NA);
    let span = lit - dark;
    // reading k is `k` frames after the change; reading 0 is the signal before it
    let mut decay = vec![lit];
    let mut dark_scans = 0;
    for _ in 1..kept {
        decay.push(frame_step(&t, &mut q, 0.0));
        dark_scans += 1;
    }
    // ... and off for 10 s in all, then on
    for _ in dark_scans..frames(10.0) {
        frame_step(&t, &mut q, 0.0);
    }
    let mut build = vec![dark];
    for _ in 1..kept {
        build.push(frame_step(&t, &mut q, LAG_TEST_SIGNAL_NA));
    }

    let decay_pct: Vec<f64> = decay.iter().map(|s| (100.0 * (s - dark) / span).max(1e-12)).collect();
    let build_pct: Vec<f64> = build.iter().map(|s| 100.0 * (s - dark) / span).collect();
    LagFigures {
        build_60: at_time(&build_pct, 0.06, frame_s, false),
        decay_60: at_time(&decay_pct, 0.06, frame_s, true),
        decay_200: at_time(&decay_pct, 0.2, frame_s, true),
    }
}

/// `readings[k]` is the reading `k` frames after a change; the value at time `t` between two
/// readings, interpolated linearly or (for a decay) linearly in the logarithm.
fn at_time(readings: &[f64], t: f64, frame_s: f64, log: bool) -> f64 {
    let x = t / frame_s;
    let k = (x.floor() as usize).min(readings.len() - 2);
    let f = x - k as f64;
    let (a, b) = (readings[k], readings[k + 1]);
    if log { (a.ln() * (1.0 - f) + b.ln() * f).exp() } else { a * (1.0 - f) + b * f }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIVE_50: f64 = 0.04 * (52.0 / 64.0) * (575.0 / 625.0);

    #[test]
    fn dark_equilibrium_is_a_fixed_point_of_the_frame_step() {
        for kind in [TubeKind::Plumbicon, TubeKind::PlumbiconBias, TubeKind::PlumbiconAct] {
            for ch in kind.channels(BEAM_RATIO_DEFAULT, ACTIVE_50).unwrap() {
                let q0 = dark_equilibrium(&ch);
                let mut q = q0;
                let s = frame_step(&ch, &mut q, 0.0);
                assert!((q - q0).abs() < 1e-9 && (s - ch.dark_na - ch.bias_na).abs() < 1e-9, "{kind:?}: {q0} -> {q}, read {s}");
            }
        }
    }

    #[test]
    fn a_weak_beam_cannot_hold_off_the_dark_current() {
        let ch = TubeKind::Plumbicon.channels(0.001, ACTIVE_50).unwrap()[1];
        let q0 = dark_equilibrium(&ch);
        let mut q = q0;
        frame_step(&ch, &mut q, 0.0);
        assert!((q - q0).abs() < 1e-9, "{q0} -> {q}");
        assert!(q0 > 0.9 * ch.saturation_na);
    }

    #[test]
    fn an_element_far_below_the_cathode_potential_still_scans() {
        let ch = TubeKind::Plumbicon.channels(BEAM_RATIO_DEFAULT, ACTIVE_50).unwrap()[1];
        // exp(-q/Q_T) is out of range here; the beam takes the element to about -Q_T ln(beam/Q_T)
        let after = scan(ch.beam_na, ch.charge_scale_na, -10_000.0);
        assert!(after.is_finite() && after <= -10_000.0 && after > -10_001.0, "{after}");
    }

    #[test]
    fn the_scan_removes_all_the_charge_the_beam_can_reach() {
        // a charge below the beam is taken off entirely, and the rest of the beam only pushes
        // the element a few Q_T below the cathode potential
        let after = scan(600.0, 5.64, 100.0);
        assert!(after < 0.0 && after > -40.0, "{after}");
        // a charge above the beam loses exactly the beam
        assert_eq!(scan(600.0, 5.64, 1000.0), 400.0);
    }
}
