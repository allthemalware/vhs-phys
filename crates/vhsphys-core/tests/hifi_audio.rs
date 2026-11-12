//! Acceptance test for VHS Hi-Fi depth-multiplex stereo audio.

use std::f64::consts::PI;
use vhsphys_core::audio::AudioProcessor;
use vhsphys_core::config;
use vhsphys_core::pipeline::Params;

const RATE: f64 = 48_000.0;

fn build(preset: &str, text: &str, seed: u64) -> Params {
    config::build(Some(preset), text, seed).unwrap()
}

fn run(p: &Params, x: &[f32], channels: usize, start: i64) -> Vec<f32> {
    AudioProcessor::new(p, RATE, channels).process(x, start)
}

fn tone(f: f64, secs: f64, amp: f64) -> Vec<f32> {
    (0..(secs * RATE) as usize).map(|n| (amp * (2.0 * PI * f * n as f64 / RATE).sin()) as f32).collect()
}

fn amp_at(x: &[f32], f: f64) -> f64 {
    let (mut re, mut im) = (0.0, 0.0);
    for (n, v) in x.iter().enumerate() {
        let w = 2.0 * PI * f * n as f64 / RATE;
        re += *v as f64 * w.cos();
        im += *v as f64 * w.sin();
    }
    2.0 * (re * re + im * im).sqrt() / x.len() as f64
}

fn tone_gain_db(p: &Params, f: f64) -> f64 {
    let y = run(p, &tone(f, 1.0, 0.1), 1, 0);
    20.0 * (amp_at(&y[24_000..], f) / 0.1).max(1e-12).log10()
}

#[test]
fn hifi_disabled_matches_linear_baseline_bit_identically() {
    let mut p1 = build("worn", "", 42);
    p1.audio.hifi = false;
    let p2 = build("worn", "", 42);
    assert!(!p2.audio.hifi);

    let t = tone(1000.0, 0.5, 0.5);
    let out1 = run(&p1, &t, 1, 0);
    let out2 = run(&p2, &t, 1, 0);
    assert_eq!(out1, out2);
}

#[test]
fn hifi_audio_has_extended_high_frequency_bandwidth() {
    let mut p_linear = build("worn", "", 42);
    p_linear.audio.hifi = false;

    let mut p_hifi = build("worn", "", 42);
    p_hifi.audio.hifi = true;

    // Linear track cuts off heavily above 10 kHz
    let g_linear_15k = tone_gain_db(&p_linear, 15_000.0);
    // Hi-Fi audio maintains flat full-range response
    let g_hifi_15k = tone_gain_db(&p_hifi, 15_000.0);

    assert!(g_linear_15k < -10.0, "Linear track should roll off at 15 kHz, got {g_linear_15k:.1} dB");
    assert!(g_hifi_15k > -3.0, "Hi-Fi audio should remain flat at 15 kHz, got {g_hifi_15k:.1} dB");
}

#[test]
fn hifi_audio_exhibits_field_rate_head_switch_buzz() {
    let mut p = build("worn", "", 42);
    p.audio.hifi = true;
    p.audio.head_switch_buzz = 0.005;

    // Pass silence through Hi-Fi processor
    let silence = vec![0.0f32; 48_000];
    let out = run(&p, &silence, 1, 0);

    // Measure buzz at field frequency (~59.94 Hz)
    let buzz_amp = amp_at(&out[24_000..], 59.94);
    assert!(buzz_amp > 1e-4, "Head-switching buzz should be present at 59.94 Hz: got {buzz_amp}");
}

#[test]
fn hifi_audio_falls_back_to_linear_under_extreme_tracking_error() {
    let mut p = build("worn", "", 42);
    p.audio.hifi = true;
    p.tracking.offset_um = 25.0; // above 18 um threshold

    let g_15k = tone_gain_db(&p, 15_000.0);
    // When fallen back to linear track, 15 kHz is heavily rolled off
    assert!(g_15k < -8.0, "Large tracking error should force fallback to linear track, got {g_15k:.1} dB");
}

#[test]
fn hifi_unlocks_at_ep_when_the_head_strays_onto_the_next_track() {
    // an EP track is 19.3 um wide, so 17 um is most of a track pitch off centre
    let p = build("pristine", "[condition]\ntape_speed = \"ep\"\ntracking_error_um = 17.0\n[audio]\nhifi = true\n", 1);
    let g_15k = tone_gain_db(&p, 15_000.0);
    assert!(g_15k < -8.0, "an off-track head should drop Hi-Fi to the linear track, got {g_15k:.1} dB");
}

#[test]
fn hifi_stays_locked_at_ep_for_an_offset_well_inside_the_track() {
    let p = build("pristine", "[condition]\ntape_speed = \"ep\"\ntracking_error_um = 1.0\n[audio]\nhifi = true\n", 1);
    let g_15k = tone_gain_db(&p, 15_000.0);
    assert!(g_15k > -3.0, "a small offset should keep Hi-Fi locked at EP, got {g_15k:.1} dB");
}

#[test]
fn hifi_unlocks_when_the_tracking_slope_carries_the_head_off_the_track() {
    let p = build("worn", "[condition]\ntracking_slope_um = 100.0\n[audio]\nhifi = true\n", 42);
    let g_15k = tone_gain_db(&p, 15_000.0);
    assert!(g_15k < -8.0, "a large tracking slope should force fallback to the linear track, got {g_15k:.1} dB");
}

#[test]
fn hifi_unlocks_when_the_track_bow_peaks_away_from_both_ends() {
    // offset 12 um and bow 8 um: 12 um at both ends of the track but 20 um at its middle
    let mut p = build("pristine", "[audio]\nhifi = true\n", 1);
    p.tracking.offset_um = 12.0;
    p.tracking.slope_um = 0.0;
    p.tracking.bow_um = 8.0;
    let g_bowed = tone_gain_db(&p, 15_000.0);
    assert!(g_bowed < -8.0, "the worst point of the track decides the lock, got {g_bowed:.1} dB");
    p.tracking.bow_um = 0.0;
    let g_flat = tone_gain_db(&p, 15_000.0);
    assert!(g_flat > -3.0, "12 um all along the track is inside the limit, got {g_flat:.1} dB");
}

#[test]
fn hifi_does_not_compress_or_distort_a_loud_tone() {
    let mut p = build("worn", "", 42);
    p.audio.hifi = true;
    let y = run(&p, &tone(1000.0, 1.0, 0.9), 1, 0);
    let second_half = &y[24_000..];
    let fundamental = amp_at(second_half, 1000.0);
    let third = amp_at(second_half, 3000.0);
    let level_db = 20.0 * (fundamental / 0.9).log10();
    let third_db = 20.0 * (third / fundamental).max(1e-12).log10();
    assert!(level_db.abs() < 0.2, "the frequency-modulated carrier is recorded at constant level, got {level_db:.2} dB");
    assert!(third_db < -50.0, "no tape saturation in the Hi-Fi path, third harmonic at {third_db:.1} dB");
}

#[test]
fn hifi_output_does_not_depend_on_how_the_input_is_split() {
    let mut p = build("worn", "", 42);
    p.audio.hifi = true;
    let x = tone(1000.0, 1.0, 0.5);
    let a = 10_007;
    assert_ne!(a % 2048, 0);
    let whole = AudioProcessor::new(&p, RATE, 1).process(&x, 0);
    let mut split = AudioProcessor::new(&p, RATE, 1);
    let mut parts = split.process(&x[..a], 0);
    parts.extend(split.process(&x[a..], a as i64));
    assert_eq!(whole, parts);
}
