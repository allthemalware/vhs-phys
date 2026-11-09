//! The physics of the outside world, one law at a time: free-space fields, the shape of a phone's
//! transmissions, loop pickup, the mains, dimmers, dipoles, the saturation of a head core and the
//! erasure of tape. Every expected number below is worked out from the formulas in
//! `docs/INTERFERENCE.md`, not read back from the code.

use std::f64::consts::PI;
use vhsphys_core::consts::*;
use vhsphys_core::interference::*;
use vhsphys_core::interference_params::*;

fn near(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(1e-300)
}

/// Relative agreement with a floor, for values that may pass through zero.
fn agree(a: f64, b: f64, rel: f64, floor: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(floor)
}

fn params() -> InterferenceParams {
    InterferenceParams::default()
}

// --------------------------------------------------------------------------------- time

#[test]
fn a_moving_source_closes_in_linearly_from_four_times_its_distance() {
    assert_eq!(approach_scale(0.0, 3.0), 1.0);
    assert!(near(approach_scale(10.0, 0.0), 4.0, 1e-12));
    assert!(near(approach_scale(10.0, 5.0), 2.5, 1e-12));
    assert!(near(approach_scale(10.0, 10.0), 1.0, 1e-12));
    assert_eq!(approach_scale(10.0, 25.0), 1.0);
}

// --------------------------------------------------------------------------------- phone

#[test]
fn a_phone_field_is_a_dipole_field_with_its_near_field_correction() {
    // far field: sqrt(30 P G) / d; at 1 m and 900 MHz the correction is 0.9986
    assert!(near(phone_field_vpm(2.0, 1.0, 1.0, 900.0e6), 7.7351, 2e-4));
    assert!(near(phone_field_vpm(2.0, 1.0, 2.0, 900.0e6), 3.8716, 2e-4));
    // 20 mm from the antenna (kd = 0.38) the reactive field is 6.6 times the radiated one
    assert!(near(phone_field_vpm(2.0, 1.0, 0.02, 900.0e6), 2549.8, 2e-3));
    // the field goes as the square root of the power and of the gain
    assert!(near(phone_field_vpm(8.0, 1.0, 0.5, 900.0e6), 2.0 * phone_field_vpm(2.0, 1.0, 0.5, 900.0e6), 1e-12));
    assert!(near(phone_field_vpm(2.0, 4.0, 0.5, 900.0e6), 2.0 * phone_field_vpm(2.0, 1.0, 0.5, 900.0e6), 1e-12));
}

#[test]
fn shielding_is_measured_in_decibels_of_field() {
    assert_eq!(shielded_vpm(5.0, 0.0), 5.0);
    assert!(near(shielded_vpm(5.0, 20.0), 0.5, 1e-12));
    assert!(near(shielded_vpm(5.0, 6.0), 5.0 * 0.501187, 1e-5));
}

#[test]
fn a_gsm_phone_transmits_one_577_us_burst_every_4_615_ms() {
    let s = 1234u64;
    let env = |t: f64| phone_envelope(PhoneMode::Gsm, t, s);
    assert!(near(env(GSM_SLOT_S / 2.0), 1.0, 1e-12));
    assert_eq!(env(2.0e-3), 0.0);
    assert_eq!(env(4.5e-3), 0.0);
    assert_eq!(env(0.0), 0.0);
    // raised-cosine ramps: half power in the middle of each ramp
    assert!(near(env(GSM_RAMP_S / 2.0), 0.5, 1e-9));
    assert!(near(env(GSM_SLOT_S - GSM_RAMP_S / 2.0), 0.5, 1e-9));
    // periodic with the frame
    for t in [0.00001, 0.0001, 0.00033, 0.0005, 0.0006, 0.002] {
        for k in [1.0, 7.0, 1000.0] {
            assert!((env(t + k * GSM_FRAME_S) - env(t)).abs() < 1e-6, "t {t} k {k}");
        }
    }
    // mean over a frame is the analytic mean (about 0.119 = duty 1/8 less the ramps)
    let n = 200_000;
    let mean = (0..n).map(|i| env((i as f64 + 0.5) / n as f64 * GSM_FRAME_S)).sum::<f64>() / n as f64;
    assert!(near(mean, phone_mean_envelope(PhoneMode::Gsm), 3e-3), "{mean}");
    assert!(near(phone_mean_envelope(PhoneMode::Gsm), 0.118933, 1e-4));
    // the repetition rate of the pattern is 216.7 Hz
    assert!(near(1.0 / GSM_FRAME_S, 216.667, 1e-4));
}

#[test]
fn a_gprs_phone_uses_two_adjacent_slots() {
    let env = |t: f64| phone_envelope(PhoneMode::Gprs, t, 5);
    assert!(near(env(0.0003), 1.0, 1e-12));
    assert!(near(env(0.0009), 1.0, 1e-12));
    assert_eq!(env(0.0013), 0.0);
    let n = 200_000;
    let mean = (0..n).map(|i| env((i as f64 + 0.5) / n as f64 * GSM_FRAME_S)).sum::<f64>() / n as f64;
    assert!(near(mean, phone_mean_envelope(PhoneMode::Gprs), 3e-3), "{mean}");
    assert!(near(phone_mean_envelope(PhoneMode::Gprs), 0.243933, 1e-4));
}

#[test]
fn an_lte_phone_transmits_in_a_random_third_of_its_subframes() {
    let on = |seed: u64| (0..20_000).filter(|s| phone_envelope(PhoneMode::Lte, (*s as f64 + 0.5) * LTE_SUBFRAME_S, seed) > 0.5).count();
    let share = on(7) as f64 / 20_000.0;
    assert!((0.28..=0.32).contains(&share), "{share}");
    // mid-subframe the envelope is exactly on or off, and the pattern is a function of the seed only
    for s in 0..200 {
        let v = phone_envelope(PhoneMode::Lte, (s as f64 + 0.5) * LTE_SUBFRAME_S, 7);
        assert!(v == 0.0 || v == 1.0);
        assert_eq!(v, phone_envelope(PhoneMode::Lte, (s as f64 + 0.5) * LTE_SUBFRAME_S, 7));
    }
    let pattern = |seed: u64| (0..400).map(|s| phone_envelope(PhoneMode::Lte, (s as f64 + 0.5) * LTE_SUBFRAME_S, seed) > 0.5).collect::<Vec<_>>();
    assert_ne!(pattern(7), pattern(8));
    let n = 100_000;
    let mean = (0..n).map(|i| phone_envelope(PhoneMode::Lte, (i as f64 + 0.5) * 20.0e-6, 7)).sum::<f64>() / n as f64;
    assert!((mean - phone_mean_envelope(PhoneMode::Lte)).abs() < 0.02, "{mean}");
    assert_eq!(phone_envelope(PhoneMode::Off, 0.0003, 7), 0.0);
    assert_eq!(phone_mean_envelope(PhoneMode::Off), 0.0);
}

// --------------------------------------------------------------------------------- radio

#[test]
fn the_programme_is_a_bounded_zero_mean_sum_of_ten_partials() {
    let parts = program_partials(3);
    assert_eq!(parts.len(), 10);
    assert!(near(parts.iter().map(|p| p.1).sum::<f64>(), 1.0, 1e-9));
    for (k, p) in parts.iter().enumerate() {
        assert!(near(p.0, 180.0 * 1.35f64.powi(k as i32), 1e-9));
        assert!(near(p.1 / parts[0].1, 1.35f64.powf(-0.5 * k as f64), 1e-9));
    }
    let mut peak = 0.0f64;
    let (mut sum, mut sum2, n) = (0.0, 0.0, 160_000);
    for i in 0..n {
        let t = i as f64 / 8000.0;
        let v = program(t, 3);
        let direct: f64 = parts.iter().map(|p| p.1 * (2.0 * PI * p.0 * t + p.2).sin()).sum();
        assert!((v - direct).abs() < 1e-9);
        peak = peak.max(v.abs());
        sum += v;
        sum2 += v * v;
    }
    assert!(peak <= 1.0 + 1e-12);
    assert!((sum / n as f64).abs() < 0.01);
    let rms = (sum2 / n as f64).sqrt();
    assert!((0.15..=0.5).contains(&rms), "{rms}");
    assert!((0..200).any(|i| program(i as f64 / 8000.0, 3) != program(i as f64 / 8000.0, 4)));
}

#[test]
fn an_am_carrier_is_one_plus_depth_times_the_programme() {
    for t in [0.0, 0.001, 0.0173, 0.4] {
        assert!(near(radio_envelope(RadioMod::Am, 0.8, t, 9), 1.0 + 0.8 * program(t, 9), 1e-12));
        assert_eq!(radio_envelope(RadioMod::Cw, 0.8, t, 9), 1.0);
        assert_eq!(radio_envelope(RadioMod::Off, 0.8, t, 9), 0.0);
        assert_eq!(radio_envelope(RadioMod::Ssb, 0.8, t, 9), 0.0);
    }
}

#[test]
fn radio_pickup_is_loop_induction_through_the_head_preamplifier() {
    let c = Coupling::default();
    let r = |f: f64, e: f64| RadioParams { modulation: RadioMod::Cw, freq_hz: f, field_vpm: e, depth: 0.8 };
    // 1 V/m at 3.9 MHz induces 2 pi f/c * 5e-4 m^2 = 41 uV, 0.973 of it passes the preamplifier: 4.0 % of a 1 mV head signal
    assert!(near(radio_carrier_ratio(&r(3.9e6, 1.0), 0.0, &c), 0.039762, 5e-3));
    // linear in the field, in decibels of shielding, and (well below the preamplifier corner) in frequency
    assert!(near(radio_carrier_ratio(&r(3.9e6, 3.0), 0.0, &c), 3.0 * radio_carrier_ratio(&r(3.9e6, 1.0), 0.0, &c), 1e-12));
    assert!(near(radio_carrier_ratio(&r(3.9e6, 1.0), 20.0, &c), 0.1 * radio_carrier_ratio(&r(3.9e6, 1.0), 0.0, &c), 1e-12));
    assert!(near(radio_carrier_ratio(&r(0.5e6, 1.0), 0.0, &c), 0.005240, 5e-3));
    assert!(near(radio_carrier_ratio(&r(1.0e6, 1.0), 0.0, &c) / radio_carrier_ratio(&r(0.5e6, 1.0), 0.0, &c), 2.0, 5e-3));
    // above the corner the preamplifier rolls the pickup off at 12 dB per octave
    let hi = radio_carrier_ratio(&r(16.0e6, 1.0), 0.0, &c) / radio_carrier_ratio(&r(8.0e6, 1.0), 0.0, &c);
    assert!(near(hi, 2.0 * 2.0f64.sqrt() / 17.0f64.sqrt(), 5e-3), "{hi}");
}

// --------------------------------------------------------------------------------- mains

#[test]
fn the_mains_is_a_unit_rms_line_with_third_and_fifth_harmonics() {
    let f = 60.0;
    let n = 240_000; // 60 cycles of 4000 samples
    let (mut s2, mut a1, mut b1, mut a3, mut b3, mut a5, mut b5) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for i in 0..n {
        let t = i as f64 / (4000.0 * f);
        let v = mains_waveform(t, f, 0.3);
        let th = 2.0 * PI * f * t + 0.3;
        s2 += v * v;
        a1 += v * th.sin();
        b1 += v * th.cos();
        a3 += v * (3.0 * th).sin();
        b3 += v * (3.0 * th).cos();
        a5 += v * (5.0 * th).sin();
        b5 += v * (5.0 * th).cos();
    }
    let k = 2.0 / n as f64;
    assert!(near((s2 / n as f64).sqrt(), 1.0, 1e-3));
    assert!(near((a1 * a1 + b1 * b1).sqrt() * k, 1.38013, 3e-3));
    assert!(near((a3 * a3 + b3 * b3).sqrt() * k, 0.276026, 3e-3));
    assert!(near((a5 * a5 + b5 * b5).sqrt() * k, 0.138013, 3e-3));
    // a period is 1 / f and the phase moves the waveform in time
    assert!((mains_waveform(0.0123, f, 0.0) - mains_waveform(0.0123 + 1.0 / f, f, 0.0)).abs() < 1e-9);
    assert!((mains_waveform(0.0123, f, 0.7) - mains_waveform(0.0123 + 0.7 / (2.0 * PI * f), f, 0.0)).abs() < 1e-9);
    // 50 Hz is a different waveform in time
    assert!((mains_waveform(0.0071, 50.0, 0.0) - mains_waveform(0.0071, 60.0, 0.0)).abs() > 0.05);
}

#[test]
fn the_mains_derivative_is_the_derivative() {
    for (t, f, ph) in [(0.0, 60.0, 0.0), (0.0031, 60.0, 1.1), (0.0172, 50.0, 0.4), (0.5, 59.4, 2.0)] {
        let h = 1.0e-8;
        let num = (mains_waveform(t + h, f, ph) - mains_waveform(t - h, f, ph)) / (2.0 * h);
        assert!(near(mains_waveform_derivative(t, f, ph), num, 1e-4), "{t} {f}: {} vs {num}", mains_waveform_derivative(t, f, ph));
    }
}

#[test]
fn a_dimmer_rings_after_each_firing_and_the_ring_dies_away() {
    let m = |angle: f64| MainsParams { freq_hz: 60.0, phase: 0.0, dimmer_angle_deg: angle, ..MainsParams::default() };
    let (ring, tau) = (80.0e3, 40.0e-6);
    let t0 = 0.5 / 120.0; // first firing at 90 degrees after the zero crossing
    let quarter = 1.0 / (4.0 * ring); // the ring is at its first peak
    let v = |angle: f64, t: f64| dimmer_ring(t, &m(angle), ring, tau);
    assert!(near(v(90.0, t0 + quarter), (-quarter / tau).exp(), 1e-6));
    // one full ring period later: decayed by exp(-T/tau)
    assert!(near(v(90.0, t0 + quarter + 1.0 / ring), (-(quarter + 1.0 / ring) / tau).exp(), 1e-6));
    // the next half cycle fires with the other sign
    assert!(near(v(90.0, t0 + 1.0 / 120.0 + quarter), -(-quarter / tau).exp(), 1e-6));
    // nothing before the first event of a cycle, nothing long after it, nothing with the dimmer off
    assert_eq!(v(90.0, t0 - 1.0e-6), 0.0);
    assert_eq!(v(90.0, t0 + 500.0e-6), 0.0);
    assert_eq!(v(0.0, t0 + quarter), 0.0);
    // the step is the mains voltage at the firing angle: proportional to sin(angle); 30 and 150 degrees are alike
    let at = |angle: f64| {
        let t_k = angle / 180.0 / 120.0;
        dimmer_ring(t_k + quarter, &m(angle), ring, tau)
    };
    assert!(near(at(30.0), 0.5 * (-quarter / tau).exp(), 1e-6));
    assert!(near(at(150.0), at(30.0), 1e-6));
    // the mains phase moves the events in time
    let mp = MainsParams { phase: 0.9, ..m(90.0) };
    let t_shift = 0.9 / (2.0 * PI * 60.0);
    assert!(near(dimmer_ring(t0 - t_shift + quarter, &mp, ring, tau), (-quarter / tau).exp(), 1e-6));
}

// --------------------------------------------------------------------------------- magnetism

#[test]
fn a_dipole_field_has_the_textbook_values() {
    // 1.6 A m^2 at 20 mm: on the axis 2e-7 m / r^3 = 40 mT, on the equator half of it and opposite
    let on_axis = dipole_field(1.6, [0.0, 0.0, 1.0], [0.0, 0.0, 0.02]);
    assert!(near(on_axis[2], 0.04, 1e-9) && on_axis[0].abs() < 1e-12 && on_axis[1].abs() < 1e-12);
    let equator = dipole_field(1.6, [0.0, 0.0, 1.0], [0.02, 0.0, 0.0]);
    assert!(near(equator[2], -0.02, 1e-9) && equator[0].abs() < 1e-12);
    // falls as 1/r^3, is even in r, and follows the axis
    let far = dipole_field(1.6, [0.0, 0.0, 1.0], [0.0, 0.0, 0.04]);
    assert!(near(far[2], on_axis[2] / 8.0, 1e-9));
    let back = dipole_field(1.6, [0.0, 0.0, 1.0], [0.0, 0.0, -0.02]);
    assert!(near(back[2], on_axis[2], 1e-9));
    let x_axis = dipole_field(1.6, [1.0, 0.0, 0.0], [0.03, 0.0, 0.0]);
    assert!(near(x_axis[0], 2.0e-7 * 1.6 / 0.03f64.powi(3), 1e-9));
    // an oblique point: |B| = (mu0 m / 4 pi r^3) sqrt(1 + 3 cos^2 theta)
    let r = [0.01, 0.0, 0.02];
    let b = dipole_field(1.0, [0.0, 0.0, 1.0], r);
    let rr = (0.01f64 * 0.01 + 0.02 * 0.02).sqrt();
    let cos_t = 0.02 / rr;
    assert!(near((b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt(), 1.0e-7 / rr.powi(3) * (1.0 + 3.0 * cos_t * cos_t).sqrt(), 1e-9));
}

#[test]
fn the_electrons_of_a_picture_tube_are_relativistic() {
    assert!(near(electron_momentum(25.0), 8.6463e-23, 5e-4));
    // slow electrons: p = sqrt(2 m e V)
    assert!(near(electron_momentum(1.0), 1.70933e-23, 5e-3));
    let nonrel = (2.0 * M_ELECTRON * E_CHARGE * 25.0e3).sqrt();
    let ratio = electron_momentum(25.0) / nonrel;
    assert!((1.010..=1.014).contains(&ratio), "{ratio}");
}

#[test]
fn tape_is_erased_by_the_distribution_of_its_switching_fields() {
    let hc = 52_000.0;
    assert!(near(erase_fraction(hc, hc, 0.3), 0.5, 1e-9));
    assert!(near(erase_fraction(1.3 * hc, hc, 0.3), 0.84134, 1e-4));
    assert!(near(erase_fraction(0.7 * hc, hc, 0.3), 0.15866, 1e-3));
    assert!(erase_fraction(0.0, hc, 0.3) < 1.0e-3);
    assert!(erase_fraction(2.0 * hc, hc, 0.3) > 0.999);
    let mut last = -1.0;
    for i in 0..60 {
        let v = erase_fraction(i as f64 * 2000.0, hc, 0.3);
        assert!(v > last);
        last = v;
    }
}

#[test]
fn a_ferrite_core_loses_incremental_permeability_as_it_saturates() {
    let (mu, bs, k) = (1500.0, 0.45, 0.1);
    assert_eq!(core_gain(0.0, mu, bs, k), 1.0);
    // x = mu k b / bs; half the gain where sech^2 x = 1/2, x = 0.88137
    let b_half = 0.881374 * bs / (mu * k);
    assert!(near(core_gain(b_half, mu, bs, k), 0.5, 1e-4));
    assert!(near(core_gain(1.0e-3, mu, bs, k), 0.89663, 2e-3));
    assert_eq!(core_gain(-1.0e-3, mu, bs, k), core_gain(1.0e-3, mu, bs, k));
    assert!(core_gain(0.05, mu, bs, k) < 1.0e-6);
}

fn with_head_magnet(kind: MagnetKind, d: f64) -> InterferenceParams {
    let mut p = params();
    p.head_magnet.kind = kind;
    p.head_magnet.distance_m = d;
    p
}

#[test]
fn a_magnet_near_the_head_dims_it_by_saturating_the_core_and_erasing_the_tape() {
    // far away: nothing (the Gaussian switching-field distribution leaves 4e-4 even at zero field)
    assert!(head_signal_gain(&with_head_magnet(MagnetKind::Neodymium, 1.0), 0.0) > 0.999);
    // a neodymium disc at 6 cm biases the core by 1.5 mT: 79 % of the signal remains
    assert!(near(head_signal_gain(&with_head_magnet(MagnetKind::Neodymium, 0.06), 0.0), 0.7905, 1e-2));
    // at 3 cm the core is deep in saturation, at 2 cm the tape is erased as well
    assert!(near(head_signal_gain(&with_head_magnet(MagnetKind::Neodymium, 0.03), 0.0), 0.001475, 0.05));
    assert!(head_signal_gain(&with_head_magnet(MagnetKind::Neodymium, 0.015), 0.0) < 1.0e-6);
    // weaker magnets matter at shorter range
    assert!(near(head_signal_gain(&with_head_magnet(MagnetKind::Fridge, 0.04), 0.0), 0.9079, 1e-2));
    assert!(near(head_signal_gain(&with_head_magnet(MagnetKind::Speaker, 0.30), 0.0), 0.9889, 5e-3));
    // monotone in distance
    let mut last = -1.0;
    for i in 1..40 {
        let g = head_signal_gain(&with_head_magnet(MagnetKind::Neodymium, 0.01 * i as f64), 0.0);
        assert!(g >= last - 1e-12, "d {i}");
        last = g;
    }
    // no magnet, no effect, exactly
    assert_eq!(head_signal_gain(&params(), 0.0), 1.0);
    assert_eq!(audio_gain(&params(), 1.0), 1.0);
}

#[test]
fn a_magnet_closes_in_over_the_approach_time() {
    let mut p = with_head_magnet(MagnetKind::Neodymium, 0.03);
    p.approach_s = 10.0;
    let far = head_signal_gain(&with_head_magnet(MagnetKind::Neodymium, 0.12), 0.0);
    let near_ = head_signal_gain(&with_head_magnet(MagnetKind::Neodymium, 0.03), 0.0);
    assert!(near(head_signal_gain(&p, 0.0), far, 1e-9));
    assert!(near(head_signal_gain(&p, 10.0), near_, 1e-9));
    assert!(near(head_signal_gain(&p, 60.0), near_, 1e-9));
    assert!(head_signal_gain(&p, 5.0) < head_signal_gain(&p, 0.0));
    assert!(near(audio_gain(&p, 5.0), head_signal_gain(&p, 5.0), 1e-12));
}

// ------------------------------------------------------------------------------- the nodes

#[test]
fn with_nothing_outside_every_node_is_untouched() {
    let p = params();
    assert!(p.is_off());
    let mut buf = vec![0.25f32; 4096];
    let before = buf.clone();
    rf_pickup_add(&p, 1234, &mut buf, 0.7);
    assert_eq!(buf, before);
    for g in [0i64, 1, 77_777, 123_456_789] {
        assert_eq!(video_add_ire(&p, g), 0.0);
    }
    let (mut pre, mut post) = (vec![0.0f64; 1000], vec![0.0f64; 1000]);
    audio_add(&p, 5000, 48_000.0, &mut pre, &mut post);
    assert!(pre.iter().chain(post.iter()).all(|v| *v == 0.0));
}

/// Amplitude and the quadrature of the component at `f` (Hz) of `x` sampled from global index `g0`.
fn iq(x: &[f32], g0: i64, f: f64) -> (f64, f64) {
    let (mut re, mut im) = (0.0, 0.0);
    for (i, v) in x.iter().enumerate() {
        let w = 2.0 * PI * f * (g0 + i as i64) as f64 / FS_RF;
        re += *v as f64 * w.cos();
        im -= *v as f64 * w.sin();
    }
    (2.0 * re / x.len() as f64, 2.0 * im / x.len() as f64)
}

#[test]
fn a_cw_carrier_is_added_to_the_head_signal_at_its_frequency_and_amplitude() {
    let n = 65_536usize;
    let f = 5950.0 * FS_RF / n as f64; // a whole number of cycles in the buffer
    let mut p = params();
    p.shielding_db = 10.0;
    p.radio = RadioParams { modulation: RadioMod::Cw, freq_hz: f, field_vpm: 2.0, depth: 0.8 };
    let a_nom = 0.8;
    let ratio = radio_carrier_ratio(&p.radio, p.shielding_db, &p.coupling);
    let want = a_nom * ratio;
    let mut buf = vec![0.0f32; n];
    rf_pickup_add(&p, 12_345, &mut buf, a_nom);
    let (a, b) = iq(&buf, 12_345, f);
    assert!(near((a * a + b * b).sqrt(), want, 1e-2), "{} vs {want}", (a * a + b * b).sqrt());
    // all of the energy is in that one line
    let e: f64 = buf.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / n as f64;
    assert!(near(e, want * want / 2.0, 1e-2));
    // a function of the absolute index only: two halves equal the whole
    let mut halves = vec![0.0f32; n];
    let (lo, hi) = halves.split_at_mut(n / 2);
    rf_pickup_add(&p, 12_345, lo, a_nom);
    rf_pickup_add(&p, 12_345 + (n / 2) as i64, hi, a_nom);
    let worst = buf.iter().zip(halves.iter()).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
    assert!(worst < 1e-6 * want as f32 + 1e-9, "{worst}");
    // it adds to what is there
    let mut busy = vec![0.5f32; n];
    rf_pickup_add(&p, 12_345, &mut busy, a_nom);
    assert!(busy.iter().zip(buf.iter()).all(|(b, c)| (b - 0.5 - c).abs() < 1e-6));
}

#[test]
fn an_am_carrier_follows_the_programme_and_ssb_has_no_carrier() {
    let n = 4_194_304usize; // 98 ms: many periods of the lowest partial of the programme
    let f = 6000.0 * FS_RF / 65_536.0;
    let a_nom = 1.0;
    let demod = |p: &InterferenceParams| -> Vec<(f64, f64)> {
        // I/Q demodulation, averaged over 256 samples (6 us: far wider than the audio band)
        let mut buf = vec![0.0f32; n];
        rf_pickup_add(p, 0, &mut buf, a_nom);
        let w = 256;
        (0..n / w)
            .map(|blk| {
                let (a, b) = iq(&buf[blk * w..(blk + 1) * w], (blk * w) as i64, f);
                (a, b)
            })
            .collect()
    };
    let mut p = params();
    p.radio = RadioParams { modulation: RadioMod::Am, freq_hz: f, field_vpm: 4.0, depth: 0.8 };
    let a = a_nom * radio_carrier_ratio(&p.radio, p.shielding_db, &p.coupling);
    let am = demod(&p);
    let mut err2 = 0.0;
    for (i, (re, im)) in am.iter().enumerate() {
        let t = ((i * 256) as f64 + 128.0) / FS_RF;
        let env = (re * re + im * im).sqrt();
        err2 += (env - a * radio_envelope(RadioMod::Am, 0.8, t, p.seed)).powi(2);
    }
    assert!((err2 / am.len() as f64).sqrt() < 0.04 * a, "rms envelope error {}", (err2 / am.len() as f64).sqrt() / a);
    // single sideband: the carrier is absent, the programme's power is that of its partials
    p.radio.modulation = RadioMod::Ssb;
    let ssb = demod(&p);
    let mean_re = ssb.iter().map(|z| z.0).sum::<f64>() / ssb.len() as f64;
    let mean_im = ssb.iter().map(|z| z.1).sum::<f64>() / ssb.len() as f64;
    assert!((mean_re * mean_re + mean_im * mean_im).sqrt() < 0.1 * a);
    let power = ssb.iter().map(|z| z.0 * z.0 + z.1 * z.1).sum::<f64>() / ssb.len() as f64;
    let want: f64 = program_partials(p.seed).iter().map(|q| q.1 * q.1).sum::<f64>() * a * a;
    assert!(near(power, want, 0.25), "{power} vs {want}");
}

#[test]
fn a_carrier_above_what_the_preamplifier_can_pass_is_not_added() {
    let mut p = params();
    p.radio = RadioParams { modulation: RadioMod::Cw, freq_hz: 0.46 * FS_RF, field_vpm: 10.0, depth: 0.8 };
    let mut buf = vec![0.0f32; 2048];
    rf_pickup_add(&p, 0, &mut buf, 1.0);
    assert!(buf.iter().all(|v| *v == 0.0));
}

#[test]
fn a_dimmer_adds_a_burst_to_the_head_signal_at_each_firing() {
    let mut p = params();
    p.mains.dimmer_angle_deg = 90.0;
    p.mains.freq_hz = 60.0;
    let a_nom = 0.6;
    let t0 = 0.5 / 120.0;
    let g0 = (t0 * FS_RF) as i64 - 50;
    let mut buf = vec![0.0f32; 4000];
    rf_pickup_add(&p, g0, &mut buf, a_nom);
    for i in (0..4000).step_by(37) {
        let t = (g0 + i as i64) as f64 / FS_RF;
        let want = a_nom * p.coupling.dimmer_rf_ratio * dimmer_ring(t, &p.mains, p.coupling.dimmer_rf_hz, p.coupling.dimmer_rf_tau_s);
        assert!((buf[i] as f64 - want).abs() < 1e-6, "{i}");
    }
    assert!(buf.iter().any(|v| v.abs() > 0.01));
}

fn video_only(f: impl Fn(&mut InterferenceParams)) -> InterferenceParams {
    let mut p = params();
    p.shielding_db = 0.0;
    f(&mut p);
    p
}

#[test]
fn a_ground_loop_modulates_the_video_stages_and_leaves_a_residue_after_the_clamp() {
    let p = video_only(|p| {
        p.mains.ground_loop_mv = 200.0;
        p.mains.freq_hz = 60.0;
    });
    // 200 mV x 0.15 = 30 mV rms = 4.2 IRE rms of hum at the input; the clamp leaves a tenth as an offset
    // and the bias shift modulates the gain by 0.8 % per IRE: 3.36 % rms
    let n = 240_000;
    let at = |i: usize| (i as f64 / (4000.0 * 60.0) * FS_RF) as i64;
    let rms_add = ((0..n).map(|i| video_add_ire(&p, at(i)) as f64).map(|v| v * v).sum::<f64>() / n as f64).sqrt();
    let rms_gain = ((0..n).map(|i| video_gain(&p, at(i)) as f64).map(|v| v * v).sum::<f64>() / n as f64).sqrt();
    assert!(near(rms_add, 0.42, 5e-3), "{rms_add}");
    assert!(near(rms_gain, 0.0336, 5e-3), "{rms_gain}");
    let g = 123_456i64;
    let period = (FS_RF / 60.0).round() as i64;
    assert!((video_gain(&p, g) - video_gain(&p, g + period)).abs() < 1e-5);
    // 50 Hz mains: a different period
    let q = video_only(|p| {
        p.mains.ground_loop_mv = 200.0;
        p.mains.freq_hz = 50.0;
    });
    let period50 = (FS_RF / 50.0).round() as i64;
    assert!((video_gain(&q, g) - video_gain(&q, g + period50)).abs() < 1e-5);
    assert!((video_gain(&q, g) - video_gain(&q, g + period)).abs() > 1e-3);
    // exactly the formulas
    let t = g as f64 / FS_RF;
    assert!(near(video_add_ire(&p, g) as f64, 0.2 * 0.15 * 0.1 * 140.0 * mains_waveform(t, 60.0, 0.0), 1e-4));
    assert!(near(video_gain(&p, g) as f64, 0.008 * 0.2 * 0.15 * 140.0 * mains_waveform(t, 60.0, 0.0), 1e-4));
    // nothing else modulates the gain
    let phone = video_only(|p| p.phone = PhoneParams { mode: PhoneMode::Gsm, distance_m: 0.3, ..PhoneParams::default() });
    assert_eq!(video_gain(&phone, 4_321), 0.0);
    assert_eq!(video_gain(&params(), 4_321), 0.0);
}

#[test]
fn a_phone_pulses_the_video_with_its_rectified_envelope() {
    let p = video_only(|p| {
        p.phone = PhoneParams { mode: PhoneMode::Gsm, distance_m: 1.0, ..PhoneParams::default() };
    });
    // 1 m, unshielded: E = 7.735 V/m, 0.12 IRE per (V/m)^2 minus the mean the clamp removes
    let g_mid = (GSM_SLOT_S / 2.0 * FS_RF) as i64;
    let g_out = (2.0e-3 * FS_RF) as i64;
    assert!(near(video_add_ire(&p, g_mid) as f64, 6.3259, 5e-3));
    assert!(near(video_add_ire(&p, g_out) as f64, -0.8539, 5e-3));
    // it repeats with the GSM frame
    let g_next = g_mid + (GSM_FRAME_S * FS_RF).round() as i64;
    assert!((video_add_ire(&p, g_mid) - video_add_ire(&p, g_next)).abs() < 0.01);
    // twice as far: a quarter of the pulse (square law)
    let q = video_only(|p| {
        p.phone = PhoneParams { mode: PhoneMode::Gsm, distance_m: 2.0, ..PhoneParams::default() };
    });
    assert!(near(video_add_ire(&q, g_mid) as f64 / video_add_ire(&p, g_mid) as f64, 0.25, 6e-3));
    // 20 dB of shielding: a hundredth
    let s = video_only(|p| {
        p.shielding_db = 20.0;
        p.phone = PhoneParams { mode: PhoneMode::Gsm, distance_m: 1.0, ..PhoneParams::default() };
    });
    assert!(near(video_add_ire(&s, g_mid) as f64 / video_add_ire(&p, g_mid) as f64, 0.01, 1e-6));
    // approaching: four times as far (a sixteenth of the pulse) at the start
    let mut a = p.clone();
    a.approach_s = 20.0;
    assert!(near(video_add_ire(&a, g_mid) as f64 / video_add_ire(&p, g_mid) as f64, 1.0 / approach_scale(20.0, g_mid as f64 / FS_RF).powi(2), 5e-3));
}

#[test]
fn an_am_station_is_rectified_into_the_video_and_a_plain_carrier_is_not() {
    let am = video_only(|p| p.radio = RadioParams { modulation: RadioMod::Am, freq_hz: 1.0e6, field_vpm: 3.0, depth: 0.8 });
    for g in [1000i64, 54_321, 999_999] {
        let t = g as f64 / FS_RF;
        let env = radio_envelope(RadioMod::Am, 0.8, t, am.seed);
        assert!(agree(video_add_ire(&am, g) as f64, 0.12 * 9.0 * (env * env - 1.0), 1e-4, 0.05), "g {g}");
    }
    let cw = video_only(|p| p.radio = RadioParams { modulation: RadioMod::Cw, freq_hz: 1.0e6, field_vpm: 3.0, depth: 0.8 });
    assert_eq!(video_add_ire(&cw, 54_321), 0.0);
}

#[test]
fn a_dimmer_rings_in_the_video_and_the_sources_add() {
    let d = video_only(|p| p.mains.dimmer_angle_deg = 90.0);
    let g = ((0.5 / 120.0 + 3.125e-6) * FS_RF) as i64;
    assert!(near(video_add_ire(&d, g) as f64, 4.0 * dimmer_ring(g as f64 / FS_RF, &d.mains, 80.0e3, 40.0e-6), 1e-3));
    assert!(video_add_ire(&d, g) > 3.0);
    let ground = video_only(|p| p.mains.ground_loop_mv = 100.0);
    let phone = video_only(|p| p.phone = PhoneParams { mode: PhoneMode::Gprs, distance_m: 0.5, ..PhoneParams::default() });
    let all = video_only(|p| {
        p.mains.dimmer_angle_deg = 90.0;
        p.mains.ground_loop_mv = 100.0;
        p.phone = PhoneParams { mode: PhoneMode::Gprs, distance_m: 0.5, ..PhoneParams::default() };
    });
    for g in [g, g + 777, 4_000_000] {
        let sum = video_add_ire(&d, g) + video_add_ire(&ground, g) + video_add_ire(&phone, g);
        assert!((video_add_ire(&all, g) - sum).abs() < 2e-3, "g {g}");
    }
}

#[test]
fn the_audio_path_hears_the_mains_the_phone_and_the_stray_field_in_the_right_places() {
    let rate = 48_000.0;
    let n = 48_000usize; // one second: 60 whole cycles
    let rms = |x: &[f64]| (x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64).sqrt();
    let run = |p: &InterferenceParams, start: i64| {
        let (mut pre, mut post) = (vec![0.0f64; n], vec![0.0f64; n]);
        audio_add(p, start, rate, &mut pre, &mut post);
        (pre, post)
    };
    // a ground loop enters after the equaliser: 200 mV x 0.15 = 30 mV rms
    let ground = video_only(|p| p.mains.ground_loop_mv = 200.0);
    let (pre, post) = run(&ground, 0);
    assert!(pre.iter().all(|v| *v == 0.0));
    assert!(near(rms(&post), 0.03, 5e-3), "{}", rms(&post));
    // a stray magnetic field is induced in the head, before the equaliser: 10 uT is 0.706 of full scale
    let stray = video_only(|p| p.mains.stray_field_ut = 10.0);
    let (pre, post) = run(&stray, 0);
    assert!(post.iter().all(|v| *v == 0.0));
    assert!(near(rms(&pre), 0.7058, 5e-3), "{}", rms(&pre));
    let t = 777.0 / rate;
    let want = 9.5e-8 / (2.0 * PI) * 10.0 * mains_waveform_derivative(t, 60.0, 0.0) / 1.0e-4;
    assert!(near(run(&stray, 0).0[777], want, 1e-6));
    // a phone: pre-equaliser pulses; sample 14 of a frame that starts at 0 lies in the flat part of the burst
    let phone = video_only(|p| p.phone = PhoneParams { mode: PhoneMode::Gsm, distance_m: 1.0, ..PhoneParams::default() });
    let (pre, post) = run(&phone, 0);
    assert!(post.iter().all(|v| *v == 0.0));
    assert!(near(pre[14], 0.042173, 5e-3), "{}", pre[14]);
    assert!(pre[50] < 0.0 && near(pre[50], -8.0e-4 * 7.7351f64.powi(2) * 0.118933, 5e-3));
    // the pattern repeats with the frame, at 48 kHz: 221.5 samples
    let (a, b) = (pre[14], pre[(14.0f64 + 221.538461538 * 3.0).round() as usize]);
    assert!((a - b).abs() < 2e-3 * a.abs().max(1.0));
    // an AM station: rectified programme before the equaliser
    let am = video_only(|p| p.radio = RadioParams { modulation: RadioMod::Am, freq_hz: 1.0e6, field_vpm: 3.0, depth: 0.8 });
    let (pre, _) = run(&am, 1000);
    let t = 1000.0 / rate + 100.0 / rate;
    let env = radio_envelope(RadioMod::Am, 0.8, t, am.seed);
    assert!(agree(pre[100], 8.0e-4 * 9.0 * (env * env - 1.0), 1e-4, 1.0e-4));
    // a dimmer clicks after the equaliser
    let dimmer = video_only(|p| p.mains.dimmer_angle_deg = 90.0);
    let (pre, post) = run(&dimmer, 0);
    assert!(pre.iter().all(|v| *v == 0.0));
    let i = (0.5 / 120.0 * rate).ceil() as usize + 2;
    let want = 0.02 * dimmer_ring(i as f64 / rate, &dimmer.mains, 5.0e3, 300.0e-6);
    assert!(near(post[i], want, 1e-6) && post[i].abs() > 1e-3);
    // the sources add (the stray field and the phone, both before the equaliser)
    let both = video_only(|p| {
        p.mains.stray_field_ut = 10.0;
        p.phone = PhoneParams { mode: PhoneMode::Gsm, distance_m: 1.0, ..PhoneParams::default() };
    });
    let (pre_both, _) = run(&both, 0);
    let (pre_phone, _) = run(&phone, 0);
    let (pre_stray, _) = run(&stray, 0);
    for i in [14, 500, 20_000] {
        assert!((pre_both[i] - pre_phone[i] - pre_stray[i]).abs() < 1e-9);
    }
}
