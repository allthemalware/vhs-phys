//! Condition -> physical parameters.
//!
//! A `Condition` describes *how worn things are* in terms a person can reason
//! about (years, temperature, humidity, hours). The laws below turn those into
//! the component values and physical parameters the simulator uses: capacitor
//! capacitance and ESR, resistor drift, supply ripple and regulator headroom,
//! head wear, oxide defects, transport wear. Where a law is an engineering
//! estimate rather than a published figure it is marked `[est]`; the numbers
//! are meant to be tuned against real tapes and decks.

use crate::format::TapeSpeed;
use crate::interference::hash_unit;
use crate::interference_params::{CrtMagnet, CrtParams, HeadMagnet, InterferenceParams, MagnetKind, MainsParams, PhoneMode, PhoneParams, RadioMod, RadioParams};
use crate::pipeline::Params;
use crate::rng::Rng;
use serde::{Deserialize, Serialize};

const STREAM_COND: u64 = 51;
/// Hash stream of the mains phase, kept apart from the wear draws so adding it moves none of them.
const STREAM_MAINS_PHASE: u64 = 52;

const CM_PER_M: f64 = 100.0;
const HZ_PER_MHZ: f64 = 1.0e6;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Condition {
    // ---- tape ----
    /// Years since the recording was made.
    pub tape_age_years: f64,
    /// 0 = cool dry shelf, 1 = hot humid attic.
    pub tape_storage: f64,
    /// 0 = cheap tape, 1 = premium (smoother, better calendered coating).
    pub tape_quality: f64,
    /// Times the tape has already been played (loose debris is removed).
    pub tape_passes: f64,
    /// 0..1 stretched / edge damaged / creased.
    pub tape_stretch: f64,
    /// Position on the tape of the clip, metres from the start.
    pub tape_position_m: f64,
    /// Recording speed: sp, lp or ep (narrower tracks, more noise and crosstalk, less audio bandwidth).
    pub tape_speed: TapeSpeed,
    // ---- heads and drum (playback deck) ----
    pub head_wear: f64,
    /// 0..1 debris build-up on one head.
    pub head_clog: f64,
    pub drum_wear: f64,
    // ---- playback deck electronics ----
    pub deck_age_years: f64,
    /// Temperature around the electrolytic capacitors (deg C).
    pub deck_temp_c: f64,
    /// 0..1 storage humidity (resistor drift).
    pub deck_humidity: f64,
    /// Switch-mode supply (else linear transformer supply).
    pub psu_smps: bool,
    pub mains_hz: f64,
    /// Fractional mains sag (0.1 = 10% low).
    pub mains_sag: f64,
    /// Tracking control / servo offset in micrometres.
    pub tracking_error_um: f64,
    /// Change of the lateral head offset from the start to the end of each
    /// track (um): tape speed / track angle mismatch. Sweeps a noise band
    /// across the picture.
    pub tracking_slope_um: f64,
    /// -1 .. 1: negative = low back tension (noise at the top), positive = too tight.
    pub tension_error: f64,
    // ---- recording deck (the one that made the tape) ----
    pub rec_deck_age_years: f64,
    pub rec_deck_temp_c: f64,
    // ---- outside the circuit (all off by default; see `interference.rs`) ----
    /// A mobile phone transmitting near the equipment.
    pub phone_mode: PhoneMode,
    /// Distance of the phone, cm.
    pub phone_distance_cm: f64,
    /// A radio transmitter whose field reaches the equipment.
    pub radio_mod: RadioMod,
    /// Its carrier frequency, MHz (0.5 .. 19).
    pub radio_freq_mhz: f64,
    /// Its field strength at the equipment, V/m.
    pub radio_field_vpm: f64,
    /// Voltage between the grounds of the deck and the TV, mV rms at the mains frequency.
    pub ground_loop_mv: f64,
    /// Stray 50/60 Hz magnetic field at the audio head and the picture tube, uT rms.
    pub stray_field_ut: f64,
    /// Phase-cut dimmer on the same circuit: firing angle in degrees after each zero crossing (0 = none).
    pub dimmer_angle_deg: f64,
    /// A magnet near the picture tube, its place along the tube from the gun (cm), its distance from the axis (cm) and the angle around it.
    pub crt_magnet: MagnetKind,
    pub crt_magnet_z_cm: f64,
    pub crt_magnet_cm: f64,
    pub crt_magnet_angle_deg: f64,
    /// A magnet near the head drum and its distance (cm).
    pub head_magnet: MagnetKind,
    pub head_magnet_cm: f64,
    /// Shielding of the case and cables against radio fields, dB.
    pub shielding_db: f64,
    /// Seconds over which the phone and the magnets close in from four times their distance.
    pub approach_s: f64,
}

impl Default for Condition {
    fn default() -> Self {
        Self {
            tape_age_years: 0.0,
            tape_storage: 0.0,
            tape_quality: 0.8,
            tape_passes: 1.0,
            tape_stretch: 0.0,
            tape_position_m: 60.0,
            tape_speed: TapeSpeed::Sp,
            head_wear: 0.0,
            head_clog: 0.0,
            drum_wear: 0.0,
            deck_age_years: 2.0,
            deck_temp_c: 55.0,
            deck_humidity: 0.3,
            psu_smps: false,
            mains_hz: 60.0,
            mains_sag: 0.0,
            tracking_error_um: 0.0,
            tracking_slope_um: 0.0,
            tension_error: 0.0,
            rec_deck_age_years: 2.0,
            rec_deck_temp_c: 55.0,
            phone_mode: PhoneMode::Off,
            phone_distance_cm: 30.0,
            radio_mod: RadioMod::Off,
            radio_freq_mhz: 3.9,
            radio_field_vpm: 2.0,
            ground_loop_mv: 0.0,
            stray_field_ut: 0.0,
            dimmer_angle_deg: 0.0,
            crt_magnet: MagnetKind::Absent,
            crt_magnet_z_cm: 35.0,
            crt_magnet_cm: 15.0,
            crt_magnet_angle_deg: 0.0,
            head_magnet: MagnetKind::Absent,
            head_magnet_cm: 5.0,
            shielding_db: 10.0,
            approach_s: 0.0,
        }
    }
}

/// Aluminium electrolytic ageing.
#[derive(Clone, Copy, Debug)]
pub struct Electrolytic {
    /// Capacitance relative to new.
    pub c_ratio: f64,
    /// ESR relative to new.
    pub esr_mult: f64,
    /// Fraction of the rated life used.
    pub life_used: f64,
}

/// Electrolyte loss through the seal: life halves per +10 C (Arrhenius 10-degree
/// rule; Chemi-Con/TDK application notes); rated 2000 h at 105 C. Capacitance
/// falls ~20-30% and ESR rises 5-20x at end of life (failure literature).
/// The powered duty cycle (14% incl. shelf-equivalent) and the exact curves are [est].
pub fn electrolytic(age_years: f64, temp_c: f64) -> Electrolytic {
    let life_h = 2000.0 * 2f64.powf((105.0 - temp_c) / 10.0);
    let hours = age_years * 8760.0 * 0.14;
    let a = hours / life_h;
    Electrolytic {
        c_ratio: (1.0 - 0.30 * a.min(1.5).powf(0.9)).max(0.25),
        esr_mult: 1.0 + 14.0 * a.min(2.0).powi(2),
        life_used: a,
    }
}

/// Fractional upward drift of carbon-film resistors: moisture absorption grows
/// with sqrt(time). Magnitude [est] (service anecdotes: a few % over decades).
pub fn resistor_drift(age_years: f64, humidity: f64) -> f64 {
    (0.004 + 0.012 * humidity) * age_years.max(0.0).sqrt()
}

/// Supply ripple that actually reaches the video path.
#[derive(Clone, Copy, Debug)]
pub struct Supply {
    pub ripple_pp_v: f64,
    pub v_min: f64,
    pub deficit_v: f64,
    pub hum_ire: f64,
    pub smps_ire: f64,
}

/// A 12 V rail from a ~17 V raw supply through a regulator that needs 2 V of
/// headroom: ripple `Vr = I/(f_r C) + 4 I ESR` (full-wave, charging-pulse ESR drop).
/// While the raw minimum stays above 14 V the regulator hides it (PSRR ~ 62 dB);
/// once it dips below, the rail follows the ripple and hum reaches the picture.
/// Load, nominal voltages and the IRE-per-volt coupling are [est].
pub fn supply(c_ratio: f64, esr_mult: f64, mains_hz: f64, sag: f64, smps: bool) -> Supply {
    let (i_load, c0, esr0, v_nom, v_need) = (0.35, 2200e-6, 0.06, 17.0, 14.0);
    let f_r = 2.0 * mains_hz;
    let ripple = i_load / (f_r * c0 * c_ratio) + 4.0 * i_load * esr0 * esr_mult;
    let v_min = v_nom * (1.0 - sag) - ripple;
    let deficit = (v_need - v_min).max(0.0);
    let leak = ripple * 10f64.powf(-62.0 / 20.0) * 20.0;
    Supply {
        ripple_pp_v: ripple,
        v_min,
        deficit_v: deficit,
        hum_ire: if smps { 0.0 } else { leak + deficit * 7.0 },
        smps_ire: if smps { 0.05 * esr_mult + deficit * 3.0 } else { 0.0 },
    }
}

impl Condition {
    pub fn preset_names() -> &'static [&'static str] {
        &[
            "pristine", "good", "worn", "bad", "ruined", "dry-caps", "dirty-heads", "mistracked", "stretched", "humid-attic",
            "phone-buzz", "ham-radio", "am-station", "ground-loop", "magnet-screen", "magnet-head", "dimmer",
        ]
    }

    /// New premium tape, played three times, on a healthy deck in a quiet room.
    fn pristine() -> Self {
        Self { tape_quality: 1.0, tape_passes: 3.0, ..Self::default() }
    }

    pub fn preset(name: &str) -> Option<Self> {
        let d = Self::default();
        Some(match name {
            "pristine" => Self::pristine(),
            "good" => Self { tape_age_years: 8.0, tape_quality: 0.7, tape_passes: 4.0, deck_age_years: 10.0, head_wear: 0.15, rec_deck_age_years: 8.0, ..d },
            "worn" => Self {
                tape_age_years: 25.0, tape_storage: 0.5, tape_quality: 0.5, tape_passes: 6.0, tape_stretch: 0.2, head_wear: 0.4, head_clog: 0.15,
                drum_wear: 0.3, deck_age_years: 28.0, deck_temp_c: 60.0, deck_humidity: 0.5, tracking_error_um: 6.0, rec_deck_age_years: 25.0, ..d
            },
            "bad" => Self {
                tape_age_years: 32.0, tape_storage: 0.75, tape_quality: 0.35, tape_passes: 3.0, tape_stretch: 0.45, head_wear: 0.5, head_clog: 0.3,
                drum_wear: 0.5, deck_age_years: 34.0, deck_temp_c: 66.0, deck_humidity: 0.7, mains_sag: 0.05, tracking_error_um: 10.0,
                tension_error: -0.4, rec_deck_age_years: 32.0, rec_deck_temp_c: 62.0, ..d
            },
            "ruined" => Self {
                tape_age_years: 40.0, tape_storage: 1.0, tape_quality: 0.2, tape_passes: 1.0, tape_stretch: 0.8, head_wear: 0.9, head_clog: 0.7,
                drum_wear: 0.8, deck_age_years: 40.0, deck_temp_c: 72.0, deck_humidity: 0.9, mains_sag: 0.1, tracking_error_um: 22.0,
                tension_error: -0.7, rec_deck_age_years: 40.0, rec_deck_temp_c: 68.0, ..d
            },
            // isolated faults, for studying one mechanism at a time
            "dry-caps" => Self { deck_age_years: 38.0, deck_temp_c: 68.0, mains_sag: 0.06, ..d },
            "dirty-heads" => Self { head_wear: 0.3, head_clog: 0.8, ..d },
            "mistracked" => Self { tracking_error_um: 20.0, tracking_slope_um: 55.0, ..d },
            "stretched" => Self { tape_stretch: 0.8, tension_error: -0.5, ..d },
            "humid-attic" => Self { tape_age_years: 30.0, tape_storage: 1.0, deck_humidity: 0.9, ..d },
            // one outside mechanism each, on the otherwise pristine machine
            "phone-buzz" => Self { phone_mode: PhoneMode::Gsm, phone_distance_cm: 20.0, ..Self::pristine() },
            // a legal-limit (1.5 kW) station 10 m away: sqrt(30 P G) / d is 27 V/m
            "ham-radio" => Self { radio_mod: RadioMod::Ssb, radio_freq_mhz: 3.9, radio_field_vpm: 30.0, ..Self::pristine() },
            "am-station" => Self { radio_mod: RadioMod::Am, radio_freq_mhz: 0.9, radio_field_vpm: 6.0, ..Self::pristine() },
            // the mains of this room is off nominal, as a loaded line is
            "ground-loop" => Self { ground_loop_mv: 300.0, mains_hz: 59.4, ..Self::pristine() },
            "magnet-screen" => Self {
                crt_magnet: MagnetKind::Neodymium, crt_magnet_z_cm: 33.0, crt_magnet_cm: 20.0, crt_magnet_angle_deg: 20.0, ..Self::pristine()
            },
            "magnet-head" => Self { head_magnet: MagnetKind::Neodymium, head_magnet_cm: 4.5, ..Self::pristine() },
            "dimmer" => Self { dimmer_angle_deg: 100.0, ..Self::pristine() },
            _ => return None,
        })
    }

    /// The outside world in physical units: centimetres become metres, megahertz hertz.
    fn interference(&self, seed: u64) -> InterferenceParams {
        let default = InterferenceParams::default();
        InterferenceParams {
            seed,
            shielding_db: self.shielding_db,
            approach_s: self.approach_s,
            phone: PhoneParams { mode: self.phone_mode, distance_m: self.phone_distance_cm / CM_PER_M, ..default.phone },
            radio: RadioParams {
                modulation: self.radio_mod,
                freq_hz: self.radio_freq_mhz * HZ_PER_MHZ,
                field_vpm: self.radio_field_vpm,
                ..default.radio
            },
            mains: MainsParams {
                freq_hz: self.mains_hz,
                // the mains is not locked to anything in the equipment, so its phase at the start of the clip is arbitrary
                phase: 2.0 * std::f64::consts::PI * hash_unit(seed, STREAM_MAINS_PHASE, 0),
                ground_loop_mv: self.ground_loop_mv,
                stray_field_ut: self.stray_field_ut,
                dimmer_angle_deg: self.dimmer_angle_deg,
                ..default.mains
            },
            crt: CrtParams {
                magnet: CrtMagnet {
                    kind: self.crt_magnet,
                    z_m: self.crt_magnet_z_cm / CM_PER_M,
                    rho_m: self.crt_magnet_cm / CM_PER_M,
                    angle_deg: self.crt_magnet_angle_deg,
                },
                ..default.crt
            },
            head_magnet: HeadMagnet { kind: self.head_magnet, distance_m: self.head_magnet_cm / CM_PER_M, ..default.head_magnet },
            ..default
        }
    }

    /// Build the full physical parameter set.
    pub fn to_params(&self, seed: u64) -> Params {
        let mut p = Params { seed, ..Params::default() };
        p.speed = self.tape_speed;
        p.interference = self.interference(seed);
        let mut rng = Rng::new(seed, STREAM_COND, 0);
        let mut g = move || rng.gauss() as f64;

        // ageing figures
        let tape_af = (self.tape_age_years / 30.0).min(1.5) * (0.4 + 1.2 * self.tape_storage);
        let ecap = electrolytic(self.deck_age_years, self.deck_temp_c);
        let rec_ecap = electrolytic(self.rec_deck_age_years, self.rec_deck_temp_c);
        let r_drift = resistor_drift(self.deck_age_years, self.deck_humidity);
        let rec_r_drift = resistor_drift(self.rec_deck_age_years, 0.5);
        let deck_af = (self.deck_age_years / 30.0).min(1.5);
        let rec_af = (self.rec_deck_age_years / 30.0).min(1.5);

        // ---------------- tape ----------------
        let t = &mut p.tape;
        t.position_m = self.tape_position_m;
        t.defects_per_mm2 = 0.004 * (1.5 - self.tape_quality) * (1.0 + 6.0 * tape_af);
        t.void_fraction = 0.25 + 0.4 * self.tape_storage;
        t.passes = self.tape_passes;
        t.creases_per_m = 0.01 + 0.25 * self.tape_stretch + 0.15 * tape_af * self.tape_storage;
        t.crease_lift_um = 0.6 + 0.8 * self.tape_stretch;
        t.rough_um = 0.01 + 0.04 * tape_af;
        t.edge_top_um = 0.6 * self.tape_stretch + 1.5 * (-self.tension_error).max(0.0);
        t.edge_bottom_um = 0.3 * self.tape_stretch + 0.4 * self.tension_error.abs();
        t.edge_lines = 5.0;

        // ---------------- heads ----------------
        let clog_head = (seed % 2) as usize;
        for (k, h) in p.heads.iter_mut().enumerate() {
            h.h0_um = 0.36 + 0.25 * self.head_wear + 0.03 * g().abs();
            h.extra_spacing_um = 0.06 * self.head_wear;
            h.gain = 1.0 - 0.35 * self.head_wear + 0.03 * g() * deck_af;
            h.azimuth_error_deg = (0.1 + 0.9 * self.head_wear) * (0.6 + 0.4 * g().abs());
            h.cnr_db = 40.0 - 6.0 * self.head_wear - 2.5 * deck_af;
            h.media_am_noise = 0.02 + 0.03 * tape_af;
            // [est] the deck switches to the narrower head pair of a four-head machine at
            // LP and EP, so each head is as wide as the track it reads
            h.width_um = self.tape_speed.track_pitch_um();
            h.gain *= 1.0 - 0.10 * (self.tape_age_years / 30.0).min(1.0) * (0.5 + self.tape_storage);
            // resonance (tuned network) drifts with its capacitor
            h.q_freq_hz = 4.8e6 * (1.0 + 0.04 * deck_af * g());
            h.q_gain_db = 7.0 + 1.5 * deck_af * g();
            if k == clog_head {
                h.extra_spacing_um += 0.7 * self.head_clog;
                h.gain *= 1.0 - 0.5 * self.head_clog;
            }
        }

        // ---------------- mechanics ----------------
        let wear = self.drum_wear;
        let m = &mut p.mech;
        m.drum_ns = 12.0 + 120.0 * wear + 40.0 * deck_af;
        m.tension_ns = 15.0 + 60.0 * self.tension_error.abs() + 30.0 * deck_af;
        m.stick_ns = 6.0 + 30.0 * (0.5 * self.deck_humidity + 0.5 * deck_af);
        m.slow_us = 0.15 + 1.0 * deck_af + 0.5 * self.tape_stretch;
        m.line_ns = 3.0 + 15.0 * deck_af + 10.0 * self.head_wear;
        m.head_step_us = 0.05 + 0.4 * self.head_wear;
        m.flag_us = 0.8 * (-self.tension_error).max(0.0) + 0.5 * self.tape_stretch;
        m.switch_us = 0.3 + 0.8 * self.tape_stretch + 0.5 * self.tension_error.abs();

        // ---------------- tracking ----------------
        let tr = &mut p.tracking;
        tr.offset_um = self.tracking_error_um;
        tr.slope_um = self.tracking_slope_um + 50.0 * self.tape_stretch * if g() > 0.0 { 1.0 } else { -1.0 };
        tr.bow_um = 1.5 + 4.0 * self.tape_stretch;
        tr.wander_um = 1.0 + 4.0 * wear;

        // ---------------- playback deck electronics ----------------
        let d = &mut p.deck;
        let sup = supply(ecap.c_ratio, ecap.esr_mult, self.mains_hz, self.mains_sag, self.psu_smps);
        d.supply.hum_ire = sup.hum_ire;
        d.supply.smps_ire = sup.smps_ire;
        d.supply.smps_hz = 38_000.0;
        d.supply.mains_hz = self.mains_hz;
        d.supply.ripple_peakiness = (0.3 + 0.7 * (ecap.esr_mult - 1.0) / 14.0).min(1.0);
        d.supply.phase = g().abs() * 3.0;
        d.coupling_hpf_hz = 15.0 / ecap.c_ratio;
        d.deemph_tau_scale = 1.0 + r_drift + 0.01 * deck_af * g();
        d.deemph_ratio_scale = 1.0 + 0.5 * r_drift * g();
        d.demod_delay = 3.0 * (1.0 + (0.004 + 0.012 * deck_af) * g());
        d.dc_error_ire = (1.0 + 4.0 * deck_af) * g() * 0.5;
        d.sharp_db = (3.0 + 1.0 * deck_af * g()).max(0.0);
        d.luma_lpf_hz *= 1.0 + 0.03 * deck_af * g();
        d.doc_recirculation = if self.deck_age_years > 25.0 { 0.85 } else { 1.0 };
        for pk in d.eq_peaks.iter_mut() {
            pk.2 += 0.8 * deck_af * g();
        }

        // ---------------- recording deck (what was written to tape) ----------------
        let r = &mut p.record;
        r.emph_tau *= 1.0 + rec_r_drift + 0.01 * rec_af * g();
        r.emph_ratio *= 1.0 + 0.5 * rec_r_drift * g();
        r.chroma_level *= 1.0 + 0.10 * rec_af * g();
        r.fm_deviation_scale = 1.0 + 0.02 * rec_af * g();
        r.fm_freq_error = 0.003 * rec_af * g();
        r.drive_offset = 0.03 * rec_af * g() * (rec_ecap.esr_mult / 5.0).min(2.0);
        r.relay_threshold *= 1.0 + 0.10 * self.head_wear * g().abs();
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacitors_age_with_temperature_and_time() {
        let new = electrolytic(0.0, 55.0);
        assert!((new.c_ratio - 1.0).abs() < 1e-9 && (new.esr_mult - 1.0).abs() < 1e-9);
        let cool = electrolytic(30.0, 45.0);
        let hot = electrolytic(30.0, 65.0);
        // life halves per +10 C => ten degrees hotter ages the part >2x faster
        assert!(hot.life_used > 1.9 * cool.life_used);
        assert!(hot.c_ratio < cool.c_ratio && hot.esr_mult > cool.esr_mult);
        // end-of-life figures from the failure literature
        let eol = electrolytic(1000.0, 105.0 - 10.0 * (1000.0_f64 * 8760.0 * 0.14 / 2000.0).log2());
        assert!(eol.life_used > 0.99 && eol.c_ratio < 0.85 && eol.esr_mult > 10.0);
    }

    #[test]
    fn supply_hides_ripple_until_the_regulator_drops_out() {
        let healthy = supply(1.0, 1.0, 60.0, 0.0, false);
        assert_eq!(healthy.deficit_v, 0.0);
        assert!(healthy.hum_ire < 0.05, "{}", healthy.hum_ire);
        let tired = electrolytic(38.0, 68.0);
        let bad = supply(tired.c_ratio, tired.esr_mult, 60.0, 0.06, false);
        assert!(bad.deficit_v > 0.5 && bad.hum_ire > 3.0, "{bad:?}");
    }

    #[test]
    fn presets_are_ordered_by_severity() {
        let score = |n: &str| {
            let p = Condition::preset(n).unwrap().to_params(1);
            p.tape.defects_per_mm2 + p.mech.drum_ns / 1000.0 + p.tracking.offset_um / 50.0
        };
        let order = ["pristine", "good", "worn", "bad", "ruined"];
        for w in order.windows(2) {
            assert!(score(w[0]) < score(w[1]), "{} < {}", w[0], w[1]);
        }
        for n in Condition::preset_names() {
            assert!(Condition::preset(n).is_some());
        }
    }
}
