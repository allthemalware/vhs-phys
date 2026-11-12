//! Acceptance test for video head pole-tip contour resonance ("head bumps").

mod common;
use common::*;
use vhsphys_core::{config, pipeline::Session, video::test_pattern};
use vhsphys_core::head::{head_front_end, HeadParams};
use vhsphys_core::rng::Rng;

const GOLDEN: u64 = 0x64948fb7a6413cac;

#[test]
fn head_bumps_zero_amplitude_is_bit_identical_to_golden() {
    let mut p = config::build(Some("worn"), "", 42).unwrap();
    p.heads[0].contour_amp = 0.0;
    p.heads[1].contour_amp = 0.0;
    let input: Vec<_> = (0..5).map(|_| test_pattern()).collect();
    assert_eq!(fnv(&Session::run(p, &input)), GOLDEN);
}

#[test]
fn head_bumps_runs_in_session() {
    let p = config::build(Some("worn"), "[head]\ncontour_amp = 0.08\npole_length_um = 60.0\n", 42).unwrap();
    assert_eq!(p.heads[0].contour_amp, 0.08);
    let input: Vec<_> = (0..2).map(|_| test_pattern()).collect();
    let out = Session::run(p, &input);
    assert_eq!(out.len(), 2);
}

#[test]
fn head_bumps_modulates_low_frequency_readback() {
    let mut hp_off = HeadParams::default();
    hp_off.contour_amp = 0.0;

    let mut hp_on = HeadParams::default();
    hp_on.contour_amp = 0.15;
    hp_on.pole_length_um = 60.0;

    let n = 20_000;
    let mut buf_off = vec![0.0f32; n];
    let mut buf_on = vec![0.0f32; n];

    // Inject low-frequency carrier pulse in middle
    for i in 1000..n - 1000 {
        let v = (2.0 * std::f32::consts::PI * 629_000.0 * (i as f32) / 42_954_545.0).sin();
        buf_off[i] = v;
        buf_on[i] = v;
    }

    let mut rng1 = Rng::new(12345, 1, 0);
    let mut rng2 = Rng::new(12345, 1, 0);
    head_front_end(&mut buf_off, &hp_off, &mut rng1);
    head_front_end(&mut buf_on, &hp_on, &mut rng2);

    // Difference reflects the secondary pole contour coupling
    let max_diff: f32 = buf_off
        .iter()
        .zip(&buf_on)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(max_diff > 0.05, "Contour resonance should visibly modulate low frequencies; max diff was {max_diff}");
}

/// Run `head_front_end` on a copy of `input` with the given contour amplitude and a fixed noise seed.
fn front_end(input: &[f32], contour_amp: f64) -> Vec<f32> {
    let mut hp = HeadParams::default();
    hp.contour_amp = contour_amp;
    hp.pole_length_um = 60.0;
    let mut buf = input.to_vec();
    let mut rng = Rng::new(12345, 1, 0);
    head_front_end(&mut buf, &hp, &mut rng);
    buf
}

#[test]
fn head_bumps_act_on_the_tape_signal_not_on_the_amplifier_noise() {
    // With nothing on the tape the head amplifier's noise is all there is, and the pole-face
    // contour does not see it: the output must not depend on the contour amplitude.
    let silent = vec![0.0f32; 20_000];
    let off = front_end(&silent, 0.0);
    let on = front_end(&silent, 0.15);
    assert!(off.iter().any(|v| *v != 0.0), "the amplifier noise must be present");
    assert!(off == on, "the contour must not echo the amplifier noise");
}

#[test]
fn head_bumps_leave_the_luma_carrier_alone() {
    // The pole-face echo is a low-band effect: a luma FM carrier must come through unchanged.
    // The carrier starts at the first sample, so the window edges are part of the test.
    // Largest difference measured: 0.0017 (0.058 with a one-pole low-pass at 1.2 MHz).
    let w = 2.0 * std::f64::consts::PI * 3.9e6 / vhsphys_core::consts::FS_RF;
    let carrier: Vec<f32> = (0..20_000).map(|i| (w * i as f64).sin() as f32).collect();
    let off = front_end(&carrier, 0.0);
    let on = front_end(&carrier, 0.15);
    let max_diff: f32 = off.iter().zip(&on).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    assert!(max_diff < 0.01, "the contour echo must not reach the luma carrier; max diff was {max_diff}");
}
