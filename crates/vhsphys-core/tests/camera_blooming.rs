//! Acceptance test for camera tube beam-bending blooming.

mod common;
use common::*;
use vhsphys_core::{config, pipeline::Session, video::test_pattern};
use vhsphys_core::camera::{CameraParams, CameraTube};
use vhsphys_core::tube::TubeKind;
use vhsphys_core::video::{decode_ideal, Frame};
use vhsphys_core::consts::{ACTIVE_W, ACTIVE_H};

const GOLDEN: u64 = 0x64948fb7a6413cac;
/// Exposure that drives the test pattern well past what the beam takes off, in stops.
const BLOOMING_IRIS_STOPS: f64 = 3.0;
/// Blooming share of the tubes that are compared.
const BLOOMING_RATIO: f64 = 0.35;
/// Frames the tubes are exposed to the pattern: enough for the charge to build up.
const BLOOMING_FRAMES: usize = 4;
/// The baseband takes a trip through the encoder and the ideal decoder, which costs a level or two.
const BLOOMING_AGREEMENT_LEVELS: i32 = 2;

#[test]
fn camera_blooming_zero_ratio_is_bit_identical_to_golden() {
    let mut p = config::build(Some("worn"), "", 42).unwrap();
    p.camera.blooming = 0.0;
    let input: Vec<_> = (0..5).map(|_| test_pattern()).collect();
    assert_eq!(fnv(&Session::run(p, &input)), GOLDEN);
}

#[test]
fn camera_blooming_spreads_overexposed_highlight() {
    let mut p_off = CameraParams::default();
    p_off.tube = TubeKind::Plumbicon;
    p_off.iris_stops = 3.0;
    p_off.blooming = 0.0;

    let mut p_on = CameraParams::default();
    p_on.tube = TubeKind::Plumbicon;
    p_on.iris_stops = 3.0;
    p_on.blooming = 0.35;

    let mut tube_off = CameraTube::new(&p_off).unwrap();
    let mut tube_on = CameraTube::new(&p_on).unwrap();

    let mut f = Frame::new(ACTIVE_W, ACTIVE_H);
    // Draw bright point highlight in the center
    let center_x = ACTIVE_W / 2;
    let center_y = ACTIVE_H / 2;
    let idx = (center_y * ACTIVE_W + center_x) * 3;
    f.rgb[idx] = 255;
    f.rgb[idx + 1] = 255;
    f.rgb[idx + 2] = 255;

    // Pre-run several frames to build charge above saturation
    for _ in 0..6 {
        tube_off.process_frame(&f);
        tube_on.process_frame(&f);
    }

    let out_off = tube_off.process_frame(&f);
    let out_on = tube_on.process_frame(&f);

    // Neighbor pixels with blooming should be elevated due to lateral electron deflection
    let n_idx = (center_y * ACTIVE_W + center_x + 1) * 3;
    let neighbor_off = out_off.rgb[n_idx];
    let neighbor_on = out_on.rgb[n_idx];

    assert!(neighbor_on > neighbor_off, "Blooming should elevate adjacent pixel charge: on={neighbor_on}, off={neighbor_off}");
}

/// Largest difference in levels between two frames, leaving out the last row (at 625 lines
/// the right half of it falls on the half line that holds the equalising pulses).
fn worst_level_error(a: &Frame, b: &Frame) -> i32 {
    let carried = (ACTIVE_H - 1) * ACTIVE_W * 3;
    a.rgb[..carried].iter().zip(b.rgb[..carried].iter()).map(|(x, y)| (*x as i32 - *y as i32).abs()).max().unwrap()
}

/// The camera hands the recorder either the baseband of its output (`encode_baseband`) or
/// the frame of it (`process_frame`); with blooming on both must be the same picture.
#[test]
fn camera_blooming_encode_baseband_agrees_with_process_frame() {
    let params = |blooming: f64| CameraParams { tube: TubeKind::Plumbicon, iris_stops: BLOOMING_IRIS_STOPS, blooming, ..CameraParams::default() };
    let mut as_baseband = CameraTube::new(&params(BLOOMING_RATIO)).unwrap();
    let mut as_frame = CameraTube::new(&params(BLOOMING_RATIO)).unwrap();
    let mut without = CameraTube::new(&params(0.0)).unwrap();
    let input = test_pattern();
    let mut spread = 0;
    for _ in 0..BLOOMING_FRAMES {
        let from_baseband = decode_ideal(&as_baseband.encode_baseband(&input));
        let from_frame = as_frame.process_frame(&input);
        let plain = without.process_frame(&input);
        let err = worst_level_error(&from_baseband, &from_frame);
        assert!(err <= BLOOMING_AGREEMENT_LEVELS, "the two outputs differ by {err} levels");
        spread = spread.max(worst_level_error(&from_frame, &plain));
    }
    assert!(spread > BLOOMING_AGREEMENT_LEVELS, "blooming left the picture alone ({spread} levels), so the test did not exercise it");
}
