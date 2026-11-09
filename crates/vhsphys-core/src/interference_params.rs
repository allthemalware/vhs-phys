//! Parameters of the outside world: radio and magnetic fields, the mains and the room, and how
//! they couple into the deck and the TV.
//!
//! Every field is a physical quantity in SI units (V/m, T, A m^2, m, Hz) unless the name says
//! otherwise. The field physics itself is closed form (`interference.rs`, `crt.rs`,
//! `docs/INTERFERENCE.md`). The [`Coupling`] block holds how much of a field a real circuit
//! picks up, which depends on the particular equipment; its values are engineering estimates,
//! tagged `[est]`, meant to be tuned against real equipment.
//!
//! Frozen contract: the field names, types, defaults and the `*_on()` helpers are fixed.

use serde::{Deserialize, Serialize};

/// A transmitting mobile phone near the equipment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PhoneMode {
    #[default]
    Off,
    /// GSM voice call: one 577 us burst in every 4.615 ms frame (217 Hz, duty 1/8).
    Gsm,
    /// GPRS data with two adjacent uplink slots per frame (duty 1/4).
    Gprs,
    /// LTE uplink: 1 ms subframes, each transmitted with probability 0.3 (scheduling).
    Lte,
}

/// Modulation of a radio signal picked up by the equipment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RadioMod {
    #[default]
    Off,
    /// Amplitude modulated carrier (broadcast, depth 0.8).
    Am,
    /// Unmodulated carrier (telegraphy, tuning, data).
    Cw,
    /// Single sideband voice: no carrier, upper sideband of the programme.
    Ssb,
}

/// A permanent magnet, by its magnetic moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MagnetKind {
    #[default]
    #[serde(rename = "none")]
    Absent,
    /// A piece of flexible bonded-ferrite sheet.
    #[serde(rename = "fridge")]
    Fridge,
    /// A 20 mm x 5 mm neodymium disc (N42).
    #[serde(rename = "neodymium")]
    Neodymium,
    /// The ferrite ring magnet of a 100 mm loudspeaker.
    #[serde(rename = "speaker")]
    Speaker,
}

impl MagnetKind {
    /// Magnetic moment, A m^2: remanence x volume / mu0 for the typical piece.
    pub fn moment_am2(self) -> f64 {
        match self {
            MagnetKind::Absent => 0.0,
            MagnetKind::Fridge => 0.3,
            MagnetKind::Neodymium => 1.6,
            MagnetKind::Speaker => 42.0,
        }
    }
}

/// A mobile phone transmitting at `distance_m` from the equipment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhoneParams {
    pub mode: PhoneMode,
    /// Distance to the equipment, metres (its position at the end of the approach).
    pub distance_m: f64,
    /// Peak transmit power during a burst, W (GSM900 class 4: 2 W).
    pub peak_power_w: f64,
    /// Antenna gain, linear (a handset antenna is about 0 dBi).
    pub antenna_gain: f64,
    pub freq_hz: f64,
}

impl Default for PhoneParams {
    fn default() -> Self {
        Self { mode: PhoneMode::Off, distance_m: 0.30, peak_power_w: 2.0, antenna_gain: 1.0, freq_hz: 900.0e6 }
    }
}

/// A radio transmitter whose field reaches the equipment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RadioParams {
    pub modulation: RadioMod,
    /// Carrier frequency, Hz. Below `0.45 * FS_RF` the carrier is added to the head signal.
    pub freq_hz: f64,
    /// Field strength at the equipment, V/m (before the shielding of the case).
    pub field_vpm: f64,
    /// AM modulation depth.
    pub depth: f64,
}

impl Default for RadioParams {
    fn default() -> Self {
        Self { modulation: RadioMod::Off, freq_hz: 3.9e6, field_vpm: 2.0, depth: 0.8 }
    }
}

/// The mains and what rides on it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MainsParams {
    /// Actual mains frequency, Hz (the video runs on its own crystal at 59.94 Hz).
    pub freq_hz: f64,
    /// Phase of the mains at t = 0, rad.
    pub phase: f64,
    /// Voltage between the grounds of the two devices, mV rms at the mains frequency.
    pub ground_loop_mv: f64,
    /// Stray 50/60 Hz magnetic field at the deck's audio head and at the TV tube, uT rms.
    pub stray_field_ut: f64,
    /// Direction of that field in the plane across the tube, degrees from +x.
    pub stray_angle_deg: f64,
    /// Phase-cut (triac) dimmer on the same circuit: firing angle after each zero crossing,
    /// degrees; 0 = no dimmer.
    pub dimmer_angle_deg: f64,
}

impl Default for MainsParams {
    fn default() -> Self {
        Self { freq_hz: 60.0, phase: 0.0, ground_loop_mv: 0.0, stray_field_ut: 0.0, stray_angle_deg: 30.0, dimmer_angle_deg: 0.0 }
    }
}

/// A magnet near the picture tube. Tube coordinates: z along the axis from the electron gun
/// (z = 0) to the screen (z = depth), x to the right and y up when looking at the screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrtMagnet {
    pub kind: MagnetKind,
    /// Position along the tube, m (0 = gun/neck, `CrtParams::depth_m` = screen).
    pub z_m: f64,
    /// Distance from the tube axis, m (position at the end of the approach).
    pub rho_m: f64,
    /// Angle around the axis, degrees from +x towards +y.
    pub angle_deg: f64,
}

impl Default for CrtMagnet {
    fn default() -> Self {
        Self { kind: MagnetKind::Absent, z_m: 0.35, rho_m: 0.15, angle_deg: 0.0 }
    }
}

/// The picture tube of the TV.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrtParams {
    /// Anode voltage, kV (a colour set: 25 kV).
    pub anode_kv: f64,
    /// Gun to screen, m.
    pub depth_m: f64,
    /// Where the deflection yoke bends the beam, m from the gun.
    pub deflection_z_m: f64,
    /// Visible screen size, m.
    pub screen_w_m: f64,
    pub screen_h_m: f64,
    /// Width of one phosphor stripe, mm (an aperture-grille tube of 0.7 mm triad pitch).
    pub stripe_mm: f64,
    pub magnet: CrtMagnet,
}

impl Default for CrtParams {
    fn default() -> Self {
        Self { anode_kv: 25.0, depth_m: 0.35, deflection_z_m: 0.27, screen_w_m: 0.27, screen_h_m: 0.204, stripe_mm: 0.7 / 3.0, magnet: CrtMagnet::default() }
    }
}

/// A magnet near the head drum and the tape path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeadMagnet {
    pub kind: MagnetKind,
    /// Distance to the head, m (its position at the end of the approach).
    pub distance_m: f64,
    /// Initial relative permeability of the ferrite head core (Mn-Zn).
    pub core_mu_i: f64,
    /// Saturation flux density of the core, T.
    pub core_b_sat_t: f64,
    /// Fraction of an external flux density that enters the core path [est]: the drum and the
    /// head geometry shield and misalign most of it.
    pub core_coupling: f64,
    /// Coercivity of the tape, A/m (VHS cobalt-modified oxide, about 650 Oe).
    pub tape_hc_a_per_m: f64,
    /// Spread of the switching field of the particles, relative to the coercivity.
    pub tape_sfd: f64,
}

impl Default for HeadMagnet {
    fn default() -> Self {
        Self { kind: MagnetKind::Absent, distance_m: 0.05, core_mu_i: 1500.0, core_b_sat_t: 0.45, core_coupling: 0.1, tape_hc_a_per_m: 52_000.0, tape_sfd: 0.3 }
    }
}

/// How much of a field a real circuit picks up. All values `[est]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Coupling {
    /// Rectified (square-law) output of the video amplifiers, IRE per (V/m)^2 at the case.
    pub video_ire_per_vpm2: f64,
    /// Rectified output of the audio head amplifier, full scale per (V/m)^2.
    pub audio_fs_per_vpm2: f64,
    /// Area of the loop formed by the head leads, m^2: induced emf = (2 pi f / c) E A.
    pub rf_loop_area_m2: f64,
    /// Nominal head output amplitude, V: what an interferer is compared with.
    pub head_signal_v: f64,
    /// Corner of the second-order low-pass formed by the head preamplifier, Hz.
    pub preamp_lpf_hz: f64,
    /// Fraction of the ground-loop voltage that appears across the video input.
    pub ground_to_video: f64,
    /// Fraction of that hum which the TV's clamp does not remove again and which stays as an offset in the picture.
    pub clamp_residual: f64,
    /// Gain modulation of the video stages per IRE of hum at the input: the ground current also moves the bias of the
    /// amplifiers, as supply ripple does (the same 0.8 % per IRE as in `electronics::supply_disturbance`).
    pub ground_gain_per_ire: f64,
    /// Fraction of the ground-loop voltage that appears across the audio output.
    pub ground_to_audio: f64,
    /// Voltage induced in the audio head per uT per Hz: 2 pi N A_eff.
    pub head_emf_v_per_ut_hz: f64,
    /// Audio head voltage that corresponds to full scale in the audio path, V.
    pub audio_head_ref_v: f64,
    /// Ringing of a dimmer step in the supply as it reaches the video, IRE at 90 degrees.
    pub dimmer_video_ire: f64,
    pub dimmer_ring_hz: f64,
    pub dimmer_ring_tau_s: f64,
    /// The same step as radio-frequency burst on the head leads, relative to the head signal at 90 degrees.
    pub dimmer_rf_ratio: f64,
    pub dimmer_rf_hz: f64,
    pub dimmer_rf_tau_s: f64,
    /// The same step as a click in the audio path, full scale at 90 degrees.
    pub dimmer_audio_fs: f64,
    pub dimmer_audio_hz: f64,
    pub dimmer_audio_tau_s: f64,
}

impl Default for Coupling {
    fn default() -> Self {
        Self {
            video_ire_per_vpm2: 0.12,
            audio_fs_per_vpm2: 8.0e-4,
            rf_loop_area_m2: 5.0e-4,
            head_signal_v: 1.0e-3,
            preamp_lpf_hz: 8.0e6,
            ground_to_video: 0.15,
            clamp_residual: 0.1,
            ground_gain_per_ire: 0.008,
            ground_to_audio: 0.15,
            head_emf_v_per_ut_hz: 9.5e-8,
            audio_head_ref_v: 1.0e-4,
            dimmer_video_ire: 4.0,
            dimmer_ring_hz: 80.0e3,
            dimmer_ring_tau_s: 40.0e-6,
            dimmer_rf_ratio: 0.05,
            dimmer_rf_hz: 2.0e6,
            dimmer_rf_tau_s: 2.0e-6,
            dimmer_audio_fs: 0.02,
            dimmer_audio_hz: 5.0e3,
            dimmer_audio_tau_s: 300.0e-6,
        }
    }
}

/// Everything outside the circuit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InterferenceParams {
    pub seed: u64,
    /// Shielding of the case and cables against radio-frequency fields, dB.
    pub shielding_db: f64,
    /// Seconds from the start of the video during which the phone and the magnets close in from
    /// four times their distance to it (0 = they are there from the start).
    pub approach_s: f64,
    pub phone: PhoneParams,
    pub radio: RadioParams,
    pub mains: MainsParams,
    pub crt: CrtParams,
    pub head_magnet: HeadMagnet,
    pub coupling: Coupling,
}

impl Default for InterferenceParams {
    fn default() -> Self {
        Self {
            seed: 1,
            shielding_db: 10.0,
            approach_s: 0.0,
            phone: PhoneParams::default(),
            radio: RadioParams::default(),
            mains: MainsParams::default(),
            crt: CrtParams::default(),
            head_magnet: HeadMagnet::default(),
            coupling: Coupling::default(),
        }
    }
}

impl InterferenceParams {
    pub fn phone_on(&self) -> bool {
        self.phone.mode != PhoneMode::Off && self.phone.distance_m > 0.0
    }
    pub fn radio_on(&self) -> bool {
        self.radio.modulation != RadioMod::Off && self.radio.field_vpm > 0.0
    }
    pub fn ground_on(&self) -> bool {
        self.mains.ground_loop_mv > 0.0
    }
    pub fn stray_on(&self) -> bool {
        self.mains.stray_field_ut > 0.0
    }
    pub fn dimmer_on(&self) -> bool {
        self.mains.dimmer_angle_deg > 0.0
    }
    pub fn crt_magnet_on(&self) -> bool {
        self.crt.magnet.kind != MagnetKind::Absent
    }
    pub fn head_magnet_on(&self) -> bool {
        self.head_magnet.kind != MagnetKind::Absent
    }
    /// Something adds to the head signal before the deck electronics.
    pub fn rf_on(&self) -> bool {
        self.radio_on() || self.dimmer_on()
    }
    /// Something adds to the video after the deck electronics.
    pub fn video_on(&self) -> bool {
        self.phone_on() || (self.radio_on() && self.radio.modulation == RadioMod::Am) || self.ground_on() || self.dimmer_on()
    }
    /// Something reaches the audio path.
    pub fn audio_on(&self) -> bool {
        self.phone_on() || self.radio_on() || self.ground_on() || self.stray_on() || self.dimmer_on() || self.head_magnet_on()
    }
    /// Something acts on the picture tube.
    pub fn crt_on(&self) -> bool {
        self.crt_magnet_on() || self.stray_on()
    }
    /// Nothing from outside reaches anything: the chain must then be bit-identical to one without this module.
    pub fn is_off(&self) -> bool {
        !(self.rf_on() || self.video_on() || self.audio_on() || self.crt_on() || self.head_magnet_on())
    }
}
