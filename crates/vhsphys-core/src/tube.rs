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
        let _ = (beam_ratio, active_scan_s);
        todo!("operating points from the constants above; charge_scale_na fitted per channel")
    }
}

/// Photo current for a linear scene light (1.0 = the light that gives peak white), nA.
pub fn photo_current(ch: &TubeChannel, light: f64) -> f64 {
    let _ = (ch, light);
    todo!("peak_signal_na * max(light, 0)^GAMMA")
}

/// One frame of one element: add this frame's photo, dark and bias charge (stopping at
/// `saturation_na`), apply the ACT flyback recharge if fitted (no element above `beam_na`),
/// then scan as described in the module docs. Returns the signal read, nA.
pub fn frame_step(ch: &TubeChannel, q: &mut f64, photo_na: f64) -> f64 {
    let _ = (ch, q, photo_na);
    todo!()
}

/// Charge of an element after a long time in the dark (the state a camera starts in).
pub fn dark_equilibrium(ch: &TubeChannel) -> f64 {
    let _ = ch;
    todo!()
}

/// The data sheet's lag measurement on one channel at a frame period `frame_s`: signal and
/// beam set to `LAG_TEST_SIGNAL_NA` / `LAG_TEST_BEAM_NA` (keeping the channel's charge
/// scale, dark and bias currents), a uniform light on for 5 s then off (decay) and off for 10 s
/// then on (build-up). The change happens just after a scan, so scan k reads at
/// `k * frame_s` after it. Decay is the signal above the dark level as a percentage of
/// the signal before the change, interpolated log-linearly between scans to 60 and 200 ms;
/// build-up is the percentage of the final signal, interpolated linearly.
pub fn lag_measurement(ch: &TubeChannel, frame_s: f64) -> LagFigures {
    let _ = (ch, frame_s);
    todo!()
}
