//! Physics of the outside world: what a phone, a radio transmitter, the mains, a dimmer and a
//! magnet do to the signals of a VHS deck and its TV. The full derivations, the coupling
//! estimates and their provenance are in `docs/INTERFERENCE.md`.
//!
//! The structure is always the same: a *source* has a field at the equipment (closed-form
//! physics: free-space fields, dipole fields, mains waveforms), a *coupling* turns the field into
//! a voltage at a node of the circuit (loop pickup, square-law rectification, induction), and
//! the voltage is added at that node, where the existing signal chain (FM discriminator, sync
//! slicer, clamp, colour decoder, audio filters) reacts to it as it would in a real set.
//!
//! Names, signatures and the semantics written here are fixed; `tests/interference_physics.rs` and
//! `tests/interference_signal.rs` pin the behaviour.

use crate::consts::FS_RF;
use crate::interference_params::*;
use crate::rng::hash3;
use std::f64::consts::PI;

const TAU: f64 = 2.0 * PI;

pub const MU0: f64 = 4.0e-7 * PI;
pub const C_LIGHT: f64 = 299_792_458.0;
pub const E_CHARGE: f64 = 1.602_176_634e-19;
pub const M_ELECTRON: f64 = 9.109_383_701_5e-31;

/// GSM TDMA frame (120/26 ms = 4.615 ms, 216.7 Hz), slot (15/26 ms = 577 us) and the raised-cosine
/// ramp of a burst (the power/time template of the standard allows 28 us).
pub const GSM_FRAME_S: f64 = 120.0e-3 / 26.0;
pub const GSM_SLOT_S: f64 = 15.0e-3 / 26.0;
pub const GSM_RAMP_S: f64 = 28.0e-6;
/// LTE subframe, the probability that one carries an uplink transmission, and its ramp.
pub const LTE_SUBFRAME_S: f64 = 1.0e-3;
pub const LTE_ON_PROBABILITY: f64 = 0.3;
pub const LTE_RAMP_S: f64 = 10.0e-6;
/// 1 V peak to peak of composite video is 140 IRE.
pub const VOLTS_PER_IRE: f64 = 1.0 / 140.0;
/// The mains waveform is a fundamental with a 3rd and 5th harmonic (a rectifier-loaded line).
pub const MAINS_H3: f64 = 0.2;
pub const MAINS_H5: f64 = 0.1;

/// Hash streams of this module (see `rng::hash3`): programme phases, LTE scheduling, the carrier phase.
const STREAM_PROGRAM: u64 = 81;
const STREAM_LTE: u64 = 82;
const STREAM_CARRIER: u64 = 83;

/// The synthetic programme: partial `k` sits at `PROGRAM_BASE_HZ * PROGRAM_RATIO^k` (a spread over the speech band)
/// and its amplitude falls as `f^PROGRAM_SLOPE` (a speech-like spectral tilt).
const PROGRAM_PARTIALS: usize = 10;
const PROGRAM_BASE_HZ: f64 = 180.0;
const PROGRAM_RATIO: f64 = 1.35;
const PROGRAM_SLOPE: f64 = -0.5;

/// The head amplifier and the sampler cannot carry a radio signal above this share of the RF sample rate
/// (the anti-alias margin below Nyquist).
const RF_PICKUP_LIMIT: f64 = 0.45;

/// A dimmer's ring is gone (e^-10) after this many time constants; later events are not summed.
const DIMMER_WINDOW_TAUS: f64 = 10.0;

/// One partial of the programme: frequency (Hz), amplitude, phase (rad).
type Partial = (f64, f64, f64);

/// A deterministic number in `[0, 1)` from three integers: the top 53 bits of `hash3`, so it is exact in an `f64`.
pub fn hash_unit(a: u64, b: u64, c: u64) -> f64 {
    (hash3(a, b, c) >> 11) as f64 / (1u64 << 53) as f64
}

/// The sine-shaped rise from 0 to 1 over `ramp_s` of a switched carrier (a raised cosine keeps the spectrum of the
/// burst narrow); 1 once `x_s >= ramp_s`.
fn raised_cosine(x_s: f64, ramp_s: f64) -> f64 {
    if x_s >= ramp_s {
        1.0
    } else {
        0.5 * (1.0 - (PI * x_s / ramp_s).cos())
    }
}

/// Envelope of one transmission of length `len_s` seen `u_s` after its start, ramped at both ends.
fn burst_envelope(u_s: f64, len_s: f64) -> f64 {
    if !(0.0..len_s).contains(&u_s) {
        return 0.0;
    }
    raised_cosine(u_s, GSM_RAMP_S) * raised_cosine(len_s - u_s, GSM_RAMP_S)
}

// ------------------------------------------------------------------------- time and distance

/// How far a moving source still is from its final distance, as a factor of that distance:
/// 1 when `approach_s <= 0` or once `t_s >= approach_s`, else `1 + 3 (1 - t/approach_s)` (four times
/// as far at t = 0, closing in linearly).
pub fn approach_scale(approach_s: f64, t_s: f64) -> f64 {
    if approach_s <= 0.0 || t_s >= approach_s {
        1.0
    } else {
        1.0 + 3.0 * (1.0 - t_s / approach_s)
    }
}

// ------------------------------------------------------------------------------ the phone

/// Peak field of a transmitter at distance `distance_m`, V/m: `sqrt(30 P G) / d` times the
/// broadside near-field factor of a short dipole, `sqrt(1 - 1/(kd)^2 + 1/(kd)^4)` with `k = 2 pi f / c`.
pub fn phone_field_vpm(power_w: f64, gain: f64, distance_m: f64, freq_hz: f64) -> f64 {
    let kd = 2.0 * PI * freq_hz / C_LIGHT * distance_m;
    let kd2 = kd * kd;
    let near_field = (1.0 - 1.0 / kd2 + 1.0 / (kd2 * kd2)).sqrt();
    (30.0 * power_w * gain).sqrt() / distance_m * near_field
}

/// The field inside the case: `e * 10^(-shielding_db / 20)`.
pub fn shielded_vpm(e_vpm: f64, shielding_db: f64) -> f64 {
    e_vpm * 10f64.powf(-shielding_db / 20.0)
}

/// Power envelope (0..1) of the phone's transmissions at time `t_s`. `Off` is 0.
///
/// * `Gsm`: periodic with `GSM_FRAME_S`; the burst occupies `[0, GSM_SLOT_S)` of each frame with
///   raised-cosine ramps of `GSM_RAMP_S` inside both ends of the slot.
/// * `Gprs`: the same with two adjacent slots, `[0, 2 GSM_SLOT_S)`.
/// * `Lte`: subframe `s = floor(t / LTE_SUBFRAME_S)` is on when a hash of `(seed, s)` is below
///   `LTE_ON_PROBABILITY`; ramps of `LTE_RAMP_S` where a subframe borders an off one.
pub fn phone_envelope(mode: PhoneMode, t_s: f64, seed: u64) -> f64 {
    match mode {
        PhoneMode::Off => 0.0,
        PhoneMode::Gsm => burst_envelope(t_s.rem_euclid(GSM_FRAME_S), GSM_SLOT_S),
        PhoneMode::Gprs => burst_envelope(t_s.rem_euclid(GSM_FRAME_S), 2.0 * GSM_SLOT_S),
        PhoneMode::Lte => lte_envelope(t_s, seed),
    }
}

/// Mean of `phone_envelope` over a long time (what AC coupling removes): GSM
/// `(GSM_SLOT_S - GSM_RAMP_S) / GSM_FRAME_S`, GPRS `(2 GSM_SLOT_S - GSM_RAMP_S) / GSM_FRAME_S`,
/// LTE `LTE_ON_PROBABILITY (1 - LTE_RAMP_S / LTE_SUBFRAME_S)`, Off 0.
pub fn phone_mean_envelope(mode: PhoneMode) -> f64 {
    match mode {
        PhoneMode::Off => 0.0,
        PhoneMode::Gsm => (GSM_SLOT_S - GSM_RAMP_S) / GSM_FRAME_S,
        PhoneMode::Gprs => (2.0 * GSM_SLOT_S - GSM_RAMP_S) / GSM_FRAME_S,
        PhoneMode::Lte => LTE_ON_PROBABILITY * (1.0 - LTE_RAMP_S / LTE_SUBFRAME_S),
    }
}

/// Whether the scheduler gave subframe `s` to the phone.
fn lte_subframe_on(seed: u64, s: i64) -> bool {
    hash_unit(seed, s as u64, STREAM_LTE) < LTE_ON_PROBABILITY
}

/// A subframe is ramped only at an edge that borders a silent one: two sent subframes in a row are one continuous burst.
fn lte_envelope(t_s: f64, seed: u64) -> f64 {
    let s = (t_s / LTE_SUBFRAME_S).floor();
    let index = s as i64;
    if !lte_subframe_on(seed, index) {
        return 0.0;
    }
    let u = t_s - s * LTE_SUBFRAME_S;
    let rise = if u < LTE_RAMP_S && !lte_subframe_on(seed, index - 1) { raised_cosine(u, LTE_RAMP_S) } else { 1.0 };
    let left = LTE_SUBFRAME_S - u;
    let fall = if left < LTE_RAMP_S && !lte_subframe_on(seed, index + 1) { raised_cosine(left, LTE_RAMP_S) } else { 1.0 };
    rise * fall
}

// ------------------------------------------------------------------------------ the radio

/// Partials `(frequency_hz, amplitude, phase_rad)` of the synthetic programme (a stand-in for
/// speech or music, stationary and deterministic in `seed`): 10 partials at `180 * 1.35^k` Hz,
/// amplitudes proportional to `(f/180)^-0.5` scaled so that they sum to 1, phases from a hash of
/// `(seed, k)`.
pub fn program_partials(seed: u64) -> Vec<(f64, f64, f64)> {
    partials(seed).to_vec()
}

/// `sum amp_k sin(2 pi f_k t + phase_k)`: between -1 and 1, zero mean.
pub fn program(t_s: f64, seed: u64) -> f64 {
    sum_sines(&partials(seed), t_s)
}

/// Envelope of the received carrier: `Am` = `1 + depth * program`, `Cw` = 1, others 0.
pub fn radio_envelope(m: RadioMod, depth: f64, t_s: f64, seed: u64) -> f64 {
    match m {
        RadioMod::Am => 1.0 + depth * program(t_s, seed),
        RadioMod::Cw => 1.0,
        RadioMod::Off | RadioMod::Ssb => 0.0,
    }
}

/// Amplitude of the carrier induced in the head leads relative to the nominal head signal:
/// `E_eff * (2 pi f / c) * loop_area * |H(f)| / head_signal_v`, with `E_eff` the shielded field and
/// `|H| = 1 / sqrt(1 + (f / preamp_lpf_hz)^4)`.
pub fn radio_carrier_ratio(r: &RadioParams, shielding_db: f64, c: &Coupling) -> f64 {
    let e_eff = shielded_vpm(r.field_vpm, shielding_db);
    let preamp = 1.0 / (1.0 + (r.freq_hz / c.preamp_lpf_hz).powi(4)).sqrt();
    e_eff * (TAU * r.freq_hz / C_LIGHT) * c.rf_loop_area_m2 * preamp / c.head_signal_v
}

/// The partials of the programme. The amplitudes are a geometric series, normalised to sum to 1 so the programme never exceeds 1.
fn partials(seed: u64) -> [Partial; PROGRAM_PARTIALS] {
    let step = PROGRAM_RATIO.powf(PROGRAM_SLOPE);
    let mut raw = [0.0; PROGRAM_PARTIALS];
    let mut amp = 1.0;
    for a in raw.iter_mut() {
        *a = amp;
        amp *= step;
    }
    let total: f64 = raw.iter().sum();
    let mut out = [(0.0, 0.0, 0.0); PROGRAM_PARTIALS];
    for (k, (slot, a)) in out.iter_mut().zip(raw).enumerate() {
        let phase = TAU * hash_unit(seed, k as u64, STREAM_PROGRAM);
        *slot = (PROGRAM_BASE_HZ * PROGRAM_RATIO.powi(k as i32), a / total, phase);
    }
    out
}

fn sum_sines(parts: &[Partial], t_s: f64) -> f64 {
    parts.iter().map(|&(f, a, phi)| a * (TAU * f * t_s + phi).sin()).sum()
}

// -------------------------------------------------------------------------------- the mains

/// The mains as a unit-rms waveform: `(sin th + MAINS_H3 sin 3 th + MAINS_H5 sin 5 th) / sqrt((1 + H3^2 + H5^2) / 2)`
/// with `th = 2 pi f t + phase`.
pub fn mains_waveform(t_s: f64, freq_hz: f64, phase: f64) -> f64 {
    let th = TAU * freq_hz * t_s + phase;
    (th.sin() + MAINS_H3 * (3.0 * th).sin() + MAINS_H5 * (5.0 * th).sin()) / mains_rms_norm()
}

/// Rms of the unnormalised line; dividing by it gives the waveform unit rms.
fn mains_rms_norm() -> f64 {
    ((1.0 + MAINS_H3 * MAINS_H3 + MAINS_H5 * MAINS_H5) / 2.0).sqrt()
}

/// Time derivative of `mains_waveform`, per second.
pub fn mains_waveform_derivative(t_s: f64, freq_hz: f64, phase: f64) -> f64 {
    let th = TAU * freq_hz * t_s + phase;
    TAU * freq_hz * (th.cos() + 3.0 * MAINS_H3 * (3.0 * th).cos() + 5.0 * MAINS_H5 * (5.0 * th).cos()) / mains_rms_norm()
}

/// The ringing that a phase-cut dimmer leaves in a circuit, unit amplitude at a firing angle of 90 degrees.
///
/// Events: zero crossings of the mains are at `t = (k pi - phase) / (2 pi f)`; the dimmer fires `dimmer_angle_deg`
/// after each, at `t_k = (k + angle/180) / (2 f) - phase / (2 pi f)`. Each event contributes, for `tau = t - t_k >= 0`,
/// `(-1)^k sin(angle) exp(-tau / tau_s) sin(2 pi ring_hz tau)`; events older than `10 tau_s` are ignored. Zero when the dimmer is off.
pub fn dimmer_ring(t_s: f64, mains: &MainsParams, ring_hz: f64, tau_s: f64) -> f64 {
    let f = mains.freq_hz;
    if mains.dimmer_angle_deg <= 0.0 || f <= 0.0 {
        return 0.0;
    }
    // Event `k` fires at (k + angle/180) / (2 f) - phase / (2 pi f); invert that to find which events lie in the window.
    let fraction = mains.dimmer_angle_deg / 180.0;
    let shift_s = mains.phase / (TAU * f);
    let event_s = |k: i64| (k as f64 + fraction) / (2.0 * f) - shift_s;
    let index_at = |t: f64| (t + shift_s) * 2.0 * f - fraction;
    let window_s = DIMMER_WINDOW_TAUS * tau_s;
    // the step height is the mains voltage at the firing angle
    let step = mains.dimmer_angle_deg.to_radians().sin();
    let first = index_at(t_s - window_s).ceil() as i64;
    let last = index_at(t_s).floor() as i64;
    let mut sum = 0.0;
    for k in first..=last {
        let age = t_s - event_s(k);
        if (0.0..=window_s).contains(&age) {
            // successive half cycles are opposite in polarity
            let sign = if k.rem_euclid(2) == 0 { 1.0 } else { -1.0 };
            sum += sign * step * (-age / tau_s).exp() * (TAU * ring_hz * age).sin();
        }
    }
    sum
}

// ------------------------------------------------------------------------------- magnetism

/// Field in tesla at `r` (metres from the dipole) of a point dipole of moment `moment_am2` along the unit vector `axis`:
/// `mu0 / (4 pi) * (3 (m . rhat) rhat - m) / r^3`.
pub fn dipole_field(moment_am2: f64, axis: [f64; 3], r: [f64; 3]) -> [f64; 3] {
    let r2 = r[0] * r[0] + r[1] * r[1] + r[2] * r[2];
    if r2 == 0.0 {
        return [0.0; 3];
    }
    let dist = r2.sqrt();
    let m_dot_rhat = moment_am2 * (axis[0] * r[0] + axis[1] * r[1] + axis[2] * r[2]) / dist;
    let scale = MU0 / (4.0 * PI) / (dist * dist * dist);
    let mut b = [0.0; 3];
    for i in 0..3 {
        b[i] = scale * (3.0 * m_dot_rhat * r[i] / dist - moment_am2 * axis[i]);
    }
    b
}

/// Relativistic momentum of an electron accelerated through `anode_kv`, kg m/s:
/// `sqrt((eV)^2 + 2 eV m c^2) / c`.
pub fn electron_momentum(anode_kv: f64) -> f64 {
    let ev = E_CHARGE * anode_kv * 1.0e3;
    let rest_energy = M_ELECTRON * C_LIGHT * C_LIGHT;
    (ev * ev + 2.0 * ev * rest_energy).sqrt() / C_LIGHT
}

/// Complementary error function, to about 1e-15 relative: a power series where it converges quickly, a continued fraction
/// in the tail. (std has none, and the tape's switching-field distribution needs the far tail to stay positive.)
fn erfc(x: f64) -> f64 {
    const SERIES_LIMIT: f64 = 3.0;
    const FRACTION_TERMS: u32 = 60;
    let two_over_sqrt_pi = 2.0 / PI.sqrt();
    if x < 0.0 {
        return 2.0 - erfc(-x);
    }
    if x < SERIES_LIMIT {
        // erf(x) = 2/sqrt(pi) e^-x^2 sum_n 2^n x^(2n+1) / (2n+1)!!, all terms positive so nothing cancels
        let (mut term, mut sum) = (x, x);
        let mut n = 0u32;
        while term > 1.0e-17 * sum && n < 200 {
            n += 1;
            term *= 2.0 * x * x / (2 * n + 1) as f64;
            sum += term;
        }
        return 1.0 - two_over_sqrt_pi * (-x * x).exp() * sum;
    }
    // erfc(x) = e^-x^2 / sqrt(pi) / (x + (1/2) / (x + (2/2) / (x + (3/2) / ...)))
    let mut tail = x;
    for n in (1..=FRACTION_TERMS).rev() {
        tail = x + 0.5 * n as f64 / tail;
    }
    (-x * x).exp() / PI.sqrt() / tail
}

/// Fraction of the particles of a tape that a field `h_a_per_m` switches: the normal distribution function
/// of `(h - hc) / (sfd * hc)` (a switching-field distribution of relative width `sfd`).
pub fn erase_fraction(h_a_per_m: f64, hc_a_per_m: f64, sfd: f64) -> f64 {
    let z = (h_a_per_m - hc_a_per_m) / (sfd * hc_a_per_m);
    0.5 * erfc(-z / std::f64::consts::SQRT_2)
}

/// Incremental permeability of a ferrite core relative to its unbiased value when an external flux density biases it:
/// `sech^2(mu_i * coupling * b_ext / b_sat)` (from `B = B_sat tanh(mu_i mu0 H / B_sat)`); even in `b_ext`.
pub fn core_gain(b_ext_t: f64, mu_i: f64, b_sat_t: f64, coupling: f64) -> f64 {
    let c = (mu_i * coupling * b_ext_t / b_sat_t).cosh();
    1.0 / (c * c)
}

/// Output of the playback head relative to normal while a magnet is near it, at time `t_s`: the core gain at the on-axis
/// field of the magnet times the fraction of the tape that the field has not erased. The magnet is `distance_m * approach_scale`
/// away; the field is `mu0 / (4 pi) * 2 m / d^3` on its axis and `H = B / mu0`. 1 without a magnet.
pub fn head_signal_gain(p: &InterferenceParams, t_s: f64) -> f64 {
    if !p.head_magnet_on() {
        return 1.0;
    }
    let h = &p.head_magnet;
    let d = h.distance_m * approach_scale(p.approach_s, t_s);
    let b = dipole_field(h.kind.moment_am2(), [0.0, 0.0, 1.0], [0.0, 0.0, d])[2];
    core_gain(b, h.core_mu_i, h.core_b_sat_t, h.core_coupling) * (1.0 - erase_fraction(b / MU0, h.tape_hc_a_per_m, h.tape_sfd))
}

// ------------------------------------------------------------------------------- the nodes

/// Add to the head signal (after the head amplifier, before the deck electronics) what is picked up there. `g0` is the global RF
/// sample index of `buf[0]` (time `g / FS_RF`) and `a_nom` the nominal carrier amplitude in the units of `buf`.
///
/// * radio below `0.45 FS_RF`: `a_nom * radio_carrier_ratio * env(t) * cos(2 pi f t + phi0)`, `env` = `radio_envelope`
///   (AM, CW); for `Ssb` the upper sideband of the programme, `sum amp_k cos(2 pi (f + f_k) t + phi_k)` times the same amplitude
///   (no carrier); `phi0` from a hash of the seed. Nothing at or above `0.45 FS_RF`.
/// * dimmer: `a_nom * dimmer_rf_ratio * dimmer_ring(t, mains, dimmer_rf_hz, dimmer_rf_tau_s)`.
///
/// A function of the absolute index only: calling it on consecutive pieces equals one call on the whole.
pub fn rf_pickup_add(p: &InterferenceParams, g0: i64, buf: &mut [f32], a_nom: f64) {
    if p.radio_on() && p.radio.freq_hz < RF_PICKUP_LIMIT * FS_RF {
        add_radio_carrier(p, g0, buf, a_nom);
    }
    if p.dimmer_on() {
        let c = &p.coupling;
        let scale = a_nom * c.dimmer_rf_ratio;
        for (i, v) in buf.iter_mut().enumerate() {
            let t = rf_time(g0 + i as i64);
            *v += (scale * dimmer_ring(t, &p.mains, c.dimmer_rf_hz, c.dimmer_rf_tau_s)) as f32;
        }
    }
}

/// What the outside world adds to the picture **after the TV's clamp** (the video stages and the cathode drive, which
/// no clamp follows), in IRE, at global RF sample `g` (time `g / FS_RF`):
///
/// * phone: `video_ire_per_vpm2 * E_eff^2 * (phone_envelope - phone_mean_envelope)` with `E_eff` the shielded field at the distance
///   `distance_m * approach_scale(approach_s, t)`;
/// * AM radio (square-law detection of the envelope): `video_ire_per_vpm2 * E_eff^2 * (radio_envelope^2 - 1)`;
/// * ground loop, what the clamp leaves: `ground_loop_mv * 1e-3 * ground_to_video * clamp_residual / VOLTS_PER_IRE * mains_waveform(t, mains.freq_hz, mains.phase)`;
/// * dimmer: `dimmer_video_ire * dimmer_ring(t, mains, dimmer_ring_hz, dimmer_ring_tau_s)`.
pub fn video_add_ire(p: &InterferenceParams, g: i64) -> f32 {
    let t = rf_time(g);
    let c = &p.coupling;
    let mut ire = c.video_ire_per_vpm2 * (phone_pulse_vpm2(p, t) + am_rectified_vpm2(p, t));
    if p.ground_on() {
        ire += ground_hum_ire(p) * c.clamp_residual * mains_waveform(t, p.mains.freq_hz, p.mains.phase);
    }
    if p.dimmer_on() {
        ire += c.dimmer_video_ire * dimmer_ring(t, &p.mains, c.dimmer_ring_hz, c.dimmer_ring_tau_s);
    }
    ire as f32
}

/// Fractional gain modulation of the video stages at global RF sample `g`: the ground loop moves the bias of the amplifiers, so
/// whites move while black stays: `ground_gain_per_ire * ground_loop_mv * 1e-3 * ground_to_video / VOLTS_PER_IRE * mains_waveform`.
/// The picture is `(y - clamp) * (1 + video_gain) + video_add_ire`.
pub fn video_gain(p: &InterferenceParams, g: i64) -> f32 {
    if !p.ground_on() {
        return 0.0;
    }
    let hum = mains_waveform(rf_time(g), p.mains.freq_hz, p.mains.phase);
    (p.coupling.ground_gain_per_ire * ground_hum_ire(p) * hum) as f32
}

/// Add to the audio path (full scale = 1) over `out.len()` samples starting at sample `start` of a signal at `rate`:
/// `pre_eq` is what enters at the head and is shaped by the playback equaliser, `post_eq` what enters after it. Adds, never overwrites.
///
/// * `pre_eq`: phone `audio_fs_per_vpm2 * E_eff^2 * (env - mean)`; AM radio `audio_fs_per_vpm2 * E_eff^2 * (env^2 - 1)`;
///   stray field `head_emf_v_per_ut_hz / (2 pi) * stray_field_ut * mains_waveform_derivative / audio_head_ref_v`;
/// * `post_eq`: ground loop `ground_loop_mv * 1e-3 * ground_to_audio * mains_waveform`; dimmer
///   `dimmer_audio_fs * dimmer_ring(t, mains, dimmer_audio_hz, dimmer_audio_tau_s)`.
pub fn audio_add(p: &InterferenceParams, start: i64, rate: f64, pre_eq: &mut [f64], post_eq: &mut [f64]) {
    let c = &p.coupling;
    let m = &p.mains;
    for (i, (pre, post)) in pre_eq.iter_mut().zip(post_eq.iter_mut()).enumerate() {
        let t = (start + i as i64) as f64 / rate;
        *pre += c.audio_fs_per_vpm2 * (phone_pulse_vpm2(p, t) + am_rectified_vpm2(p, t));
        if p.stray_on() {
            *pre += c.head_emf_v_per_ut_hz / TAU * m.stray_field_ut * mains_waveform_derivative(t, m.freq_hz, m.phase) / c.audio_head_ref_v;
        }
        if p.ground_on() {
            *post += m.ground_loop_mv * 1.0e-3 * c.ground_to_audio * mains_waveform(t, m.freq_hz, m.phase);
        }
        if p.dimmer_on() {
            *post += c.dimmer_audio_fs * dimmer_ring(t, m, c.dimmer_audio_hz, c.dimmer_audio_tau_s);
        }
    }
}

/// Gain of the audio signal from the tape while a magnet is near the heads: `head_signal_gain`.
pub fn audio_gain(p: &InterferenceParams, t_s: f64) -> f64 {
    head_signal_gain(p, t_s)
}

// ---------------------------------------------------------------------------------- helpers

/// Time of a global RF sample, seconds. Always from the absolute index, so no phase accumulates and any split of a buffer agrees.
fn rf_time(g: i64) -> f64 {
    g as f64 / FS_RF
}

/// Square of the field inside the case, (V/m)^2, times the part of the phone's envelope that AC coupling does not remove.
fn phone_pulse_vpm2(p: &InterferenceParams, t_s: f64) -> f64 {
    if !p.phone_on() {
        return 0.0;
    }
    let ph = &p.phone;
    let d = ph.distance_m * approach_scale(p.approach_s, t_s);
    let e = shielded_vpm(phone_field_vpm(ph.peak_power_w, ph.antenna_gain, d, ph.freq_hz), p.shielding_db);
    e * e * (phone_envelope(ph.mode, t_s, p.seed) - phone_mean_envelope(ph.mode))
}

/// What a square-law stage makes of an AM carrier: the field squared times `env^2 - 1` (the carrier's own rectified
/// level is a constant that AC coupling removes).
fn am_rectified_vpm2(p: &InterferenceParams, t_s: f64) -> f64 {
    if !p.radio_on() || p.radio.modulation != RadioMod::Am {
        return 0.0;
    }
    let e = shielded_vpm(p.radio.field_vpm, p.shielding_db);
    let env = radio_envelope(RadioMod::Am, p.radio.depth, t_s, p.seed);
    e * e * (env * env - 1.0)
}

/// The ground-loop voltage across the video input, in IRE rms.
fn ground_hum_ire(p: &InterferenceParams) -> f64 {
    p.mains.ground_loop_mv * 1.0e-3 * p.coupling.ground_to_video / VOLTS_PER_IRE
}

/// Add the radio carrier (or, for single sideband, the voice sidebands) induced in the head leads.
fn add_radio_carrier(p: &InterferenceParams, g0: i64, buf: &mut [f32], a_nom: f64) {
    let r = &p.radio;
    let amp = a_nom * radio_carrier_ratio(r, p.shielding_db, &p.coupling);
    let phi0 = TAU * hash_unit(p.seed, STREAM_CARRIER, 0);
    let parts = partials(p.seed);
    for (i, v) in buf.iter_mut().enumerate() {
        let t = rf_time(g0 + i as i64);
        let carrier = (TAU * r.freq_hz * t + phi0).cos();
        let s = match r.modulation {
            RadioMod::Am => (1.0 + r.depth * sum_sines(&parts, t)) * carrier,
            RadioMod::Cw => carrier,
            // upper sideband: each partial is shifted up by the carrier and the carrier itself is suppressed
            RadioMod::Ssb => parts.iter().map(|&(f, a, phi)| a * (TAU * (r.freq_hz + f) * t + phi).cos()).sum(),
            RadioMod::Off => 0.0,
        };
        *v += (amp * s) as f32;
    }
}
