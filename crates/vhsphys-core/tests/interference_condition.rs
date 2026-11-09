//! How the outside world is described to a person (the `Condition`) and what physical parameters
//! that becomes. Everything is off by default and in every older preset.

use vhsphys_core::condition::Condition;
use vhsphys_core::config;
use vhsphys_core::interference_params::*;

const OLD_PRESETS: [&str; 10] = ["pristine", "good", "worn", "bad", "ruined", "dry-caps", "dirty-heads", "mistracked", "stretched", "humid-attic"];
const NEW_PRESETS: [&str; 7] = ["phone-buzz", "ham-radio", "am-station", "ground-loop", "magnet-screen", "magnet-head", "dimmer"];

#[test]
fn the_new_conditions_default_to_a_quiet_room() {
    let c = Condition::default();
    assert_eq!(c.phone_mode, PhoneMode::Off);
    assert_eq!(c.phone_distance_cm, 30.0);
    assert_eq!(c.radio_mod, RadioMod::Off);
    assert_eq!((c.radio_freq_mhz, c.radio_field_vpm), (3.9, 2.0));
    assert_eq!((c.ground_loop_mv, c.stray_field_ut, c.dimmer_angle_deg), (0.0, 0.0, 0.0));
    assert_eq!(c.crt_magnet, MagnetKind::Absent);
    assert_eq!((c.crt_magnet_z_cm, c.crt_magnet_cm, c.crt_magnet_angle_deg), (35.0, 15.0, 0.0));
    assert_eq!(c.head_magnet, MagnetKind::Absent);
    assert_eq!(c.head_magnet_cm, 5.0);
    assert_eq!((c.shielding_db, c.approach_s), (10.0, 0.0));
    assert_eq!(c.mains_hz, 60.0);
    let p = c.to_params(1);
    assert!(p.interference.is_off());
    // the parameters carry the same defaults as a fresh `InterferenceParams`
    assert_eq!(p.interference, InterferenceParams { seed: 1, mains: MainsParams { phase: p.interference.mains.phase, ..MainsParams::default() }, ..InterferenceParams::default() });
}

#[test]
fn every_older_preset_is_silent_outside() {
    for name in OLD_PRESETS {
        assert!(Condition::preset(name).unwrap().to_params(3).interference.is_off(), "{name}");
        assert!(Condition::preset_names().contains(&name));
    }
}

#[test]
fn a_condition_becomes_physical_parameters() {
    let c = Condition {
        phone_mode: PhoneMode::Gprs,
        phone_distance_cm: 25.0,
        radio_mod: RadioMod::Am,
        radio_freq_mhz: 1.0,
        radio_field_vpm: 3.0,
        ground_loop_mv: 120.0,
        mains_hz: 59.7,
        stray_field_ut: 12.0,
        dimmer_angle_deg: 80.0,
        crt_magnet: MagnetKind::Neodymium,
        crt_magnet_z_cm: 20.0,
        crt_magnet_cm: 12.0,
        crt_magnet_angle_deg: 45.0,
        head_magnet: MagnetKind::Speaker,
        head_magnet_cm: 18.0,
        shielding_db: 6.0,
        approach_s: 12.0,
        ..Condition::default()
    };
    let p = c.to_params(7);
    let i = &p.interference;
    assert_eq!(i.seed, 7);
    assert_eq!((i.phone.mode, i.phone.distance_m), (PhoneMode::Gprs, 0.25));
    assert_eq!((i.phone.peak_power_w, i.phone.freq_hz), (2.0, 900.0e6));
    assert_eq!((i.radio.modulation, i.radio.freq_hz, i.radio.field_vpm), (RadioMod::Am, 1.0e6, 3.0));
    assert_eq!((i.mains.ground_loop_mv, i.mains.stray_field_ut, i.mains.dimmer_angle_deg), (120.0, 12.0, 80.0));
    assert_eq!(i.mains.freq_hz, 59.7);
    assert_eq!(p.deck.supply.mains_hz, 59.7, "the deck's own supply ripple follows the same mains");
    assert_eq!((i.crt.magnet.kind, i.crt.magnet.z_m, i.crt.magnet.rho_m, i.crt.magnet.angle_deg), (MagnetKind::Neodymium, 0.20, 0.12, 45.0));
    assert_eq!((i.head_magnet.kind, i.head_magnet.distance_m), (MagnetKind::Speaker, 0.18));
    assert_eq!((i.shielding_db, i.approach_s), (6.0, 12.0));
    assert!(!i.is_off() && i.phone_on() && i.radio_on() && i.ground_on() && i.stray_on() && i.dimmer_on() && i.crt_magnet_on() && i.head_magnet_on());
}

#[test]
fn the_phase_of_the_mains_belongs_to_the_seed() {
    let phase = |seed: u64| Condition::default().to_params(seed).interference.mains.phase;
    assert_eq!(phase(5), phase(5));
    assert_ne!(phase(5), phase(6));
    for seed in 0..50 {
        assert!((0.0..2.0 * std::f64::consts::PI).contains(&phase(seed)), "{}", phase(seed));
    }
}

#[test]
fn the_new_conditions_can_be_written_in_a_config_file() {
    let text = "[condition]\nphone_mode = \"gsm\"\nphone_distance_cm = 40.0\nradio_mod = \"cw\"\nradio_freq_mhz = 4.6\nradio_field_vpm = 5.0\ncrt_magnet = \"neodymium\"\nhead_magnet = \"fridge\"\nmains_hz = 49.6\nground_loop_mv = 80.0\n";
    let p = config::build(Some("pristine"), text, 2).unwrap();
    let i = &p.interference;
    assert_eq!((i.phone.mode, i.phone.distance_m), (PhoneMode::Gsm, 0.40));
    assert_eq!((i.radio.modulation, i.radio.freq_hz, i.radio.field_vpm), (RadioMod::Cw, 4.6e6, 5.0));
    assert_eq!((i.crt.magnet.kind, i.head_magnet.kind), (MagnetKind::Neodymium, MagnetKind::Fridge));
    assert_eq!((i.mains.freq_hz, i.mains.ground_loop_mv), (49.6, 80.0));
    assert!(config::build(Some("pristine"), "[condition]\nphone_mode = \"umts\"\n", 1).is_err());
}

#[test]
fn the_interference_presets_each_isolate_one_mechanism() {
    for name in NEW_PRESETS {
        assert!(Condition::preset_names().contains(&name), "{name} is not listed");
        let i = Condition::preset(name).unwrap_or_else(|| panic!("{name} is missing")).to_params(1).interference;
        assert!(!i.is_off(), "{name}");
    }
    let get = |n: &str| Condition::preset(n).unwrap().to_params(1).interference;
    let only = |i: &InterferenceParams, flags: [bool; 7]| {
        [i.phone_on(), i.radio_on(), i.ground_on(), i.stray_on(), i.dimmer_on(), i.crt_magnet_on(), i.head_magnet_on()] == flags
    };
    let buzz = get("phone-buzz");
    assert!(only(&buzz, [true, false, false, false, false, false, false]) && buzz.phone.mode == PhoneMode::Gsm);
    let ham = get("ham-radio");
    assert!(only(&ham, [false, true, false, false, false, false, false]) && ham.radio.modulation == RadioMod::Ssb && (3.5e6..=4.0e6).contains(&ham.radio.freq_hz));
    let am = get("am-station");
    assert!(only(&am, [false, true, false, false, false, false, false]) && am.radio.modulation == RadioMod::Am && (0.53e6..=1.7e6).contains(&am.radio.freq_hz));
    let gl = get("ground-loop");
    assert!(only(&gl, [false, false, true, false, false, false, false]) && (gl.mains.freq_hz - 60.0).abs() > 0.1, "a ground loop preset also has a mains that is off the nominal frequency");
    assert!(only(&get("magnet-screen"), [false, false, false, false, false, true, false]));
    assert!(only(&get("magnet-head"), [false, false, false, false, false, false, true]));
    assert!(only(&get("dimmer"), [false, false, false, false, true, false, false]));
    // and none of them touches the wear of the machine: the deck is the one of the 'pristine' preset. The deck's own
    // supply ripple does follow the mains frequency (it falls as 1 / f), so a mains off 60 Hz moves it by about a percent.
    let wear = |n: &str| {
        let p = Condition::preset(n).unwrap().to_params(1);
        (p.tape.defects_per_mm2, p.heads[0].gain, p.deck.supply.hum_ire)
    };
    let clean = wear("pristine");
    for name in NEW_PRESETS {
        let w = wear(name);
        assert_eq!((w.0, w.1), (clean.0, clean.1), "{name}");
        assert!((w.2 / clean.2 - 1.0).abs() < 0.03, "{name}: supply hum {} against {}", w.2, clean.2);
    }
}
