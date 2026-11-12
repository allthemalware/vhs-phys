//! The picture tube as the pipeline uses it: `crt::scan_to_frame` takes the continuous baseband
//! and puts it through the electron beam, writing the frame to video at the very end.

mod common;

use common::{flat_frame, fnv, mean_abs_error};
use vhsphys_core::consts::*;
use vhsphys_core::crt::*;
use vhsphys_core::interference_params::*;
use vhsphys_core::video::{decode_ideal, encode_frame, test_pattern, Frame};

/// A neodymium disc nearer the axis, where half the screen is 0.135 m wide: it crowds and spreads the beams there and throws
/// some of them off the raster.
const MAGNET_CLOSE: (f64, f64, f64) = (0.34, 0.10, 0.0);

/// The frame index at which the tube is scanned in the tests that do not compare frames with each other.
const FRAME: i64 = 2;

/// Mean absolute error, in levels, above which a picture counts as clearly moved. Measured on the test pattern, whose fine
/// stripes make any shift large: 75 levels for `MAGNET_CLOSE`, 82 to 86 for the stray field.
const MOVED_LEVELS: f64 = 20.0;

/// How much more the picture is damaged on the side of the magnet than on the far side (measured 1.7 on the test pattern),
/// and the number of columns taken at the near and at the far edge of the picture.
const NEAR_OVER_FAR: f64 = 1.25;
const NEAR_COLUMNS: usize = 200;
const FAR_COLUMNS: usize = 120;

/// Fractions of the pixels of a flat picture that the strong magnet must darken, brighten or leave black. Measured: 3.2 % of a
/// white picture darkened, 5.3 % of a grey one brightened and 2.1 % of either black.
const DARKENED_FRACTION: f64 = 0.01;
const BRIGHTENED_FRACTION: f64 = 0.01;
const BLACK_FRACTION: f64 = 0.005;

/// Fraction of a white picture that the strong magnet leaves white: it only reaches part of the picture (measured 80 %).
const STILL_WHITE_FRACTION: f64 = 0.5;

/// Levels from the original at which a pixel of a flat picture counts as darkened or brightened.
const BRIGHTNESS_STEP_LEVELS: i32 = 16;

/// The stray field of the tests: 100 uT along +y at the mains frequency below. At 50 Hz the mains advances by two thirds of a
/// cycle between one frame and the next, so consecutive frames meet different fields (measured 81 and 108 uT in the middle row).
const STRAY_UT: f64 = 100.0;
const STRAY_ANGLE_DEG: f64 = 90.0;
const MAINS_HZ: f64 = 50.0;

/// Difference of the stray field between the two frames, tesla, above which they count as meeting different fields.
const FIELD_CHANGE_T: f64 = 10.0e-6;

/// Levels of the flat pictures.
const WHITE: u8 = 255;
const GREY: u8 = 128;

fn magnet((z, rho, angle): (f64, f64, f64)) -> InterferenceParams {
    let mut p = InterferenceParams::default();
    p.crt.magnet = CrtMagnet { kind: MagnetKind::Neodymium, z_m: z, rho_m: rho, angle_deg: angle };
    p
}

fn stray_only() -> InterferenceParams {
    let mut p = InterferenceParams::default();
    p.mains.stray_field_ut = STRAY_UT;
    p.mains.stray_angle_deg = STRAY_ANGLE_DEG;
    p.mains.freq_hz = MAINS_HZ;
    p
}

/// Mean absolute error of a frame against another over the columns `x0..x1`, all rows, in levels.
fn columns_error(a: &Frame, b: &Frame, x0: usize, x1: usize) -> f64 {
    let mut sum = 0.0;
    for y in 0..ACTIVE_H {
        for x in x0..x1 {
            for c in 0..3 {
                let at = (y * ACTIVE_W + x) * 3 + c;
                sum += (a.rgb[at] as f64 - b.rgb[at] as f64).abs();
            }
        }
    }
    sum / ((x1 - x0) * ACTIVE_H * 3) as f64
}

/// Number of pixels of `f` whose green level satisfies `test`.
fn count_pixels(f: &Frame, test: impl Fn(i32) -> bool) -> f64 {
    f.rgb.chunks_exact(3).filter(|px| test(px[1] as i32)).count() as f64
}

#[test]
fn with_nothing_acting_on_the_tube_the_picture_is_the_ideal_decode() {
    let bb = encode_frame(&test_pattern());
    let ideal = decode_ideal(&bb);
    assert_eq!(scan_to_frame(&bb, 5, &InterferenceParams::default()).rgb, ideal.rgb);
    // a dimmer, a phone, a ground loop or a head magnet do not touch the tube
    let mut p = InterferenceParams::default();
    p.phone.mode = PhoneMode::Gsm;
    p.mains.dimmer_angle_deg = 60.0;
    p.mains.ground_loop_mv = 300.0;
    p.head_magnet.kind = MagnetKind::Neodymium;
    assert_eq!(scan_to_frame(&bb, 5, &p).rgb, ideal.rgb);
}

#[test]
fn a_strong_magnet_close_to_the_screen_displaces_the_picture_and_changes_its_brightness() {
    let p = magnet(MAGNET_CLOSE);
    // the test pattern is moved well beyond the rounding of the encode, and more on the side of the magnet than far from it
    let bb = encode_frame(&test_pattern());
    let ideal = decode_ideal(&bb);
    let scanned = scan_to_frame(&bb, FRAME, &p);
    let near_side = columns_error(&scanned, &ideal, ACTIVE_W - NEAR_COLUMNS, ACTIVE_W);
    let far_side = columns_error(&scanned, &ideal, 0, FAR_COLUMNS);
    assert!(mean_abs_error(&scanned, &ideal) > MOVED_LEVELS, "{}", mean_abs_error(&scanned, &ideal));
    assert!(near_side > NEAR_OVER_FAR * far_side, "near {near_side}, far {far_side}");

    // a flat white picture is not white any more: beams that spread darken, beams aimed off the raster leave black
    let pixels = (ACTIVE_W * ACTIVE_H) as f64;
    let white = scan_to_frame(&encode_frame(&flat_frame(WHITE)), FRAME, &p);
    assert_ne!(white.rgb, flat_frame(WHITE).rgb);
    let darkened = count_pixels(&white, |v| (1..WHITE as i32 - BRIGHTNESS_STEP_LEVELS).contains(&v));
    let black = count_pixels(&white, |v| v == 0);
    let still_white = count_pixels(&white, |v| v >= WHITE as i32 - BRIGHTNESS_STEP_LEVELS);
    assert!(darkened > DARKENED_FRACTION * pixels, "{darkened} pixels are darkened");
    assert!(black > BLACK_FRACTION * pixels, "{black} pixels are black");
    assert!(still_white > STILL_WHITE_FRACTION * pixels, "{still_white} pixels are still white");

    // white cannot get brighter, so crowding shows on a grey picture
    let grey = scan_to_frame(&encode_frame(&flat_frame(GREY)), FRAME, &p);
    let brightened = count_pixels(&grey, |v| v > GREY as i32 + BRIGHTNESS_STEP_LEVELS);
    assert!(brightened > BRIGHTENED_FRACTION * pixels, "{brightened} pixels are brightened");
}

#[test]
fn the_stray_mains_field_alone_moves_the_picture_differently_at_different_mains_phases() {
    let p = stray_only();
    let bb = encode_frame(&test_pattern());
    let ideal = decode_ideal(&bb);
    let middle_field = |k: i64| stray_field_t(&p, line_time_s(k, ACTIVE_H / 2));
    assert!((middle_field(0)[1] - middle_field(1)[1]).abs() > FIELD_CHANGE_T, "{:?} {:?}", middle_field(0), middle_field(1));
    let (a, b) = (scan_to_frame(&bb, 0, &p), scan_to_frame(&bb, 1, &p));
    assert!(mean_abs_error(&a, &ideal) > MOVED_LEVELS, "frame 0 is hardly moved: {}", mean_abs_error(&a, &ideal));
    assert!(mean_abs_error(&b, &ideal) > MOVED_LEVELS, "frame 1 is hardly moved: {}", mean_abs_error(&b, &ideal));
    assert!(mean_abs_error(&a, &b) > MOVED_LEVELS, "the two frames are alike: {}", mean_abs_error(&a, &b));
}

#[test]
fn scanning_is_deterministic() {
    // the rows are rendered side by side: however the work is shared out, the same picture must come out
    let mut p = magnet(MAGNET_CLOSE);
    p.mains = stray_only().mains;
    let bb = encode_frame(&test_pattern());
    let first = scan_to_frame(&bb, FRAME, &p);
    for _ in 0..3 {
        assert_eq!(scan_to_frame(&bb, FRAME, &p).rgb, first.rgb);
    }
    let alone = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(|| scan_to_frame(&bb, FRAME, &p));
    assert_eq!(fnv(&[alone]), fnv(&[first]));
}
