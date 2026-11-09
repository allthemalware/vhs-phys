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
//! Frozen contract: names, signatures and the semantics written here. Bodies are written by the
//! card that implements them; `tests/interference_physics.rs` and `tests/interference_signal.rs`
//! fix the behaviour.

use crate::interference_params::*;
use std::f64::consts::PI;

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

// ------------------------------------------------------------------------- time and distance

/// How far a moving source still is from its final distance, as a factor of that distance:
/// 1 when `approach_s <= 0` or once `t_s >= approach_s`, else `1 + 3 (1 - t/approach_s)` (four times
/// as far at t = 0, closing in linearly).
pub fn approach_scale(approach_s: f64, t_s: f64) -> f64 {
    todo!("approach_scale")
}

// ------------------------------------------------------------------------------ the phone

/// Peak field of a transmitter at distance `distance_m`, V/m: `sqrt(30 P G) / d` times the
/// broadside near-field factor of a short dipole, `sqrt(1 - 1/(kd)^2 + 1/(kd)^4)` with `k = 2 pi f / c`.
pub fn phone_field_vpm(power_w: f64, gain: f64, distance_m: f64, freq_hz: f64) -> f64 {
    todo!("phone_field_vpm")
}

/// The field inside the case: `e * 10^(-shielding_db / 20)`.
pub fn shielded_vpm(e_vpm: f64, shielding_db: f64) -> f64 {
    todo!("shielded_vpm")
}

/// Power envelope (0..1) of the phone's transmissions at time `t_s`. `Off` is 0.
///
/// * `Gsm`: periodic with `GSM_FRAME_S`; the burst occupies `[0, GSM_SLOT_S)` of each frame with
///   raised-cosine ramps of `GSM_RAMP_S` inside both ends of the slot.
/// * `Gprs`: the same with two adjacent slots, `[0, 2 GSM_SLOT_S)`.
/// * `Lte`: subframe `s = floor(t / LTE_SUBFRAME_S)` is on when a hash of `(seed, s)` is below
///   `LTE_ON_PROBABILITY`; ramps of `LTE_RAMP_S` where a subframe borders an off one.
pub fn phone_envelope(mode: PhoneMode, t_s: f64, seed: u64) -> f64 {
    todo!("phone_envelope")
}

/// Mean of `phone_envelope` over a long time (what AC coupling removes): GSM
/// `(GSM_SLOT_S - GSM_RAMP_S) / GSM_FRAME_S`, GPRS `(2 GSM_SLOT_S - GSM_RAMP_S) / GSM_FRAME_S`,
/// LTE `LTE_ON_PROBABILITY (1 - LTE_RAMP_S / LTE_SUBFRAME_S)`, Off 0.
pub fn phone_mean_envelope(mode: PhoneMode) -> f64 {
    todo!("phone_mean_envelope")
}

// ------------------------------------------------------------------------------ the radio

/// Partials `(frequency_hz, amplitude, phase_rad)` of the synthetic programme (a stand-in for
/// speech or music, stationary and deterministic in `seed`): 10 partials at `180 * 1.35^k` Hz,
/// amplitudes proportional to `(f/180)^-0.5` scaled so that they sum to 1, phases from a hash of
/// `(seed, k)`.
pub fn program_partials(seed: u64) -> Vec<(f64, f64, f64)> {
    todo!("program_partials")
}

/// `sum amp_k sin(2 pi f_k t + phase_k)`: between -1 and 1, zero mean.
pub fn program(t_s: f64, seed: u64) -> f64 {
    todo!("program")
}

/// Envelope of the received carrier: `Am` = `1 + depth * program`, `Cw` = 1, others 0.
pub fn radio_envelope(m: RadioMod, depth: f64, t_s: f64, seed: u64) -> f64 {
    todo!("radio_envelope")
}

/// Amplitude of the carrier induced in the head leads relative to the nominal head signal:
/// `E_eff * (2 pi f / c) * loop_area * |H(f)| / head_signal_v`, with `E_eff` the shielded field and
/// `|H| = 1 / sqrt(1 + (f / preamp_lpf_hz)^4)`.
pub fn radio_carrier_ratio(r: &RadioParams, shielding_db: f64, c: &Coupling) -> f64 {
    todo!("radio_carrier_ratio")
}

// -------------------------------------------------------------------------------- the mains

/// The mains as a unit-rms waveform: `(sin th + MAINS_H3 sin 3 th + MAINS_H5 sin 5 th) / sqrt((1 + H3^2 + H5^2) / 2)`
/// with `th = 2 pi f t + phase`.
pub fn mains_waveform(t_s: f64, freq_hz: f64, phase: f64) -> f64 {
    todo!("mains_waveform")
}

/// Time derivative of `mains_waveform`, per second.
pub fn mains_waveform_derivative(t_s: f64, freq_hz: f64, phase: f64) -> f64 {
    todo!("mains_waveform_derivative")
}

/// The ringing that a phase-cut dimmer leaves in a circuit, unit amplitude at a firing angle of 90 degrees.
///
/// Events: zero crossings of the mains are at `t = (k pi - phase) / (2 pi f)`; the dimmer fires `dimmer_angle_deg`
/// after each, at `t_k = (k + angle/180) / (2 f) - phase / (2 pi f)`. Each event contributes, for `tau = t - t_k >= 0`,
/// `(-1)^k sin(angle) exp(-tau / tau_s) sin(2 pi ring_hz tau)`; events older than `10 tau_s` are ignored. Zero when the dimmer is off.
pub fn dimmer_ring(t_s: f64, mains: &MainsParams, ring_hz: f64, tau_s: f64) -> f64 {
    todo!("dimmer_ring")
}

// ------------------------------------------------------------------------------- magnetism

/// Field in tesla at `r` (metres from the dipole) of a point dipole of moment `moment_am2` along the unit vector `axis`:
/// `mu0 / (4 pi) * (3 (m . rhat) rhat - m) / r^3`.
pub fn dipole_field(moment_am2: f64, axis: [f64; 3], r: [f64; 3]) -> [f64; 3] {
    todo!("dipole_field")
}

/// Relativistic momentum of an electron accelerated through `anode_kv`, kg m/s:
/// `sqrt((eV)^2 + 2 eV m c^2) / c`.
pub fn electron_momentum(anode_kv: f64) -> f64 {
    todo!("electron_momentum")
}

/// Fraction of the particles of a tape that a field `h_a_per_m` switches: the normal distribution function
/// of `(h - hc) / (sfd * hc)` (a switching-field distribution of relative width `sfd`).
pub fn erase_fraction(h_a_per_m: f64, hc_a_per_m: f64, sfd: f64) -> f64 {
    todo!("erase_fraction")
}

/// Incremental permeability of a ferrite core relative to its unbiased value when an external flux density biases it:
/// `sech^2(mu_i * coupling * b_ext / b_sat)` (from `B = B_sat tanh(mu_i mu0 H / B_sat)`); even in `b_ext`.
pub fn core_gain(b_ext_t: f64, mu_i: f64, b_sat_t: f64, coupling: f64) -> f64 {
    todo!("core_gain")
}

/// Output of the playback head relative to normal while a magnet is near it, at time `t_s`: the core gain at the on-axis
/// field of the magnet times the fraction of the tape that the field has not erased. The magnet is `distance_m * approach_scale`
/// away; the field is `mu0 / (4 pi) * 2 m / d^3` on its axis and `H = B / mu0`. 1 without a magnet.
pub fn head_signal_gain(p: &InterferenceParams, t_s: f64) -> f64 {
    todo!("head_signal_gain")
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
    todo!("rf_pickup_add")
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
    todo!("video_add_ire")
}

/// Fractional gain modulation of the video stages at global RF sample `g`: the ground loop moves the bias of the amplifiers, so
/// whites move while black stays: `ground_gain_per_ire * ground_loop_mv * 1e-3 * ground_to_video / VOLTS_PER_IRE * mains_waveform`.
/// The picture is `(y - clamp) * (1 + video_gain) + video_add_ire`.
pub fn video_gain(p: &InterferenceParams, g: i64) -> f32 {
    todo!("video_gain")
}

/// Add to the audio path (full scale = 1) over `out.len()` samples starting at sample `start` of a signal at `rate`:
/// `pre_eq` is what enters at the head and is shaped by the playback equaliser, `post_eq` what enters after it. Adds, never overwrites.
///
/// * `pre_eq`: phone `audio_fs_per_vpm2 * E_eff^2 * (env - mean)`; AM radio `audio_fs_per_vpm2 * E_eff^2 * (env^2 - 1)`;
///   stray field `head_emf_v_per_ut_hz / (2 pi) * stray_field_ut * mains_waveform_derivative / audio_head_ref_v`;
/// * `post_eq`: ground loop `ground_loop_mv * 1e-3 * ground_to_audio * mains_waveform`; dimmer
///   `dimmer_audio_fs * dimmer_ring(t, mains, dimmer_audio_hz, dimmer_audio_tau_s)`.
pub fn audio_add(p: &InterferenceParams, start: i64, rate: f64, pre_eq: &mut [f64], post_eq: &mut [f64]) {
    todo!("audio_add")
}

/// Gain of the audio signal from the tape while a magnet is near the heads: `head_signal_gain`.
pub fn audio_gain(p: &InterferenceParams, t_s: f64) -> f64 {
    todo!("audio_gain")
}
