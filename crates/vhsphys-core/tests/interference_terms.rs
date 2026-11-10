//! The per-line evaluation of the outside world's video terms against the per-pixel functions it
//! stands in for, and the dimmer's pickup where one firing's ring is still going when the next fires.

use vhsphys_core::condition::*;
use vhsphys_core::consts::*;
use vhsphys_core::interference::*;
use vhsphys_core::interference_params::*;

const WIDTH: usize = 720;
/// Pixels from the start of one line to the next in the bases below; enough lines to cover more than
/// a mains cycle, so the phone's bursts, the hum and the dimmer's windows all fall inside.
const LINE_PX: f64 = 910.0;
const ROWS: usize = 400;

/// Every source that reaches the video stages, with rings that outlast the half cycle of the mains,
/// so the windows of successive firings overlap.
fn everything_on(approach_s: f64) -> InterferenceParams {
    let c = Condition {
        phone_mode: PhoneMode::Gsm,
        phone_distance_cm: 20.0,
        radio_mod: RadioMod::Am,
        radio_freq_mhz: 0.9,
        radio_field_vpm: 6.0,
        ground_loop_mv: 300.0,
        dimmer_angle_deg: 100.0,
        approach_s,
        ..Condition::default()
    };
    let mut p = c.to_params(3).interference;
    p.coupling.dimmer_ring_tau_s = 2.0e-3;
    p.coupling.dimmer_rf_tau_s = 2.0e-3;
    p
}

/// The global RF sample of pixel `x` of a line whose first pixel sits at `base`, as the decoder places it.
fn pixel_rf(base: f64, x: usize) -> i64 {
    ((base + x as f64) * OS as f64).round() as i64
}

#[test]
fn the_line_terms_equal_the_per_pixel_functions() {
    for (p, label) in [(everything_on(0.0), "still"), (everything_on(1.0), "approaching")] {
        assert!(p.phone_on() && p.radio_on() && p.ground_on() && p.dimmer_on(), "{label}: every source is on");
        let (mut gain, mut add) = (vec![0f32; WIDTH], vec![0f32; WIDTH]);
        let mut largest = 0.0f64;
        for k in 0..ROWS {
            let base = k as f64 * LINE_PX + 0.5;
            video_line_terms(&p, base, &mut gain, &mut add);
            let direct: Vec<(f32, f32)> = (0..WIDTH)
                .map(|x| {
                    let g = pixel_rf(base, x);
                    (video_gain(&p, g), video_add_ire(&p, g))
                })
                .collect();
            let scale = direct.iter().map(|d| d.1.abs() as f64).fold(1.0, f64::max);
            largest = largest.max(scale);
            for (x, &(gn, ad)) in direct.iter().enumerate() {
                assert_eq!(gain[x], gn, "{label}: gain at line {k}, pixel {x}");
                let err = (add[x] as f64 - ad as f64).abs() / scale;
                assert!(err <= 1e-5, "{label}: offset at line {k}, pixel {x}: {} against {ad} (scale {scale})", add[x]);
            }
        }
        assert!(largest > 1.0, "{label}: the sources do reach the picture (largest offset {largest} IRE)");
    }
}

#[test]
fn the_rf_pickup_sums_overlapping_dimmer_rings_once() {
    let mut p = Condition { dimmer_angle_deg: 100.0, ..Condition::default() }.to_params(5).interference;
    p.coupling.dimmer_rf_tau_s = 2.0e-3;
    assert!(p.dimmer_on() && !p.radio_on() && !p.phone_on());
    let (hz, tau, ratio) = (p.coupling.dimmer_rf_hz, p.coupling.dimmer_rf_tau_s, p.coupling.dimmer_rf_ratio);
    // a ring is kept for ten time constants: with this tau that is longer than a full cycle of the mains,
    // so at any moment more than one firing is still ringing
    assert!(10.0 * tau > 1.0 / p.mains.freq_hz);

    let g0: i64 = 12_345;
    let n = (0.03 * FS_RF) as usize;
    let mut whole = vec![0f32; n];
    rf_pickup_add(&p, g0, &mut whole, 1.0);
    let direct: Vec<f64> = (0..n).map(|i| ratio * dimmer_ring((g0 + i as i64) as f64 / FS_RF, &p.mains, hz, tau)).collect();
    let peak = direct.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(peak > 0.0, "the dimmer rings");
    for i in 0..n {
        assert!((whole[i] as f64 - direct[i]).abs() <= 1e-5 * peak, "sample {i}: {} against {}", whole[i], direct[i]);
    }

    // in two pieces, the same to the bit
    let cut = n / 3;
    let (mut a, mut b) = (vec![0f32; cut], vec![0f32; n - cut]);
    rf_pickup_add(&p, g0, &mut a, 1.0);
    rf_pickup_add(&p, g0 + cut as i64, &mut b, 1.0);
    assert!(a.iter().chain(b.iter()).zip(&whole).all(|(x, y)| x == y), "pieces differ from the whole");
}
