//! One engine source, compiled once per raster standard; the crate root is the 525-line
//! engine, as before.

use vhsphys_core::frame::Frame;
use vhsphys_core::standard::TvStandard;

fn takes_shared_frame(f: &Frame) -> (usize, usize) {
    (f.w, f.h)
}

#[test]
fn each_engine_has_its_own_raster() {
    assert_eq!(vhsphys_core::ntsc::consts::STANDARD, TvStandard::Ntsc);
    assert_eq!(vhsphys_core::pal::consts::STANDARD, TvStandard::Pal);
    assert_eq!(vhsphys_core::ntsc::consts::LINES, 525);
    assert_eq!(vhsphys_core::pal::consts::LINES, 625);
    // the root is the 525-line engine
    assert_eq!(vhsphys_core::consts::LINES, 525);
}

#[test]
fn frames_are_one_type_across_engines() {
    let n: vhsphys_core::video::Frame = vhsphys_core::ntsc::video::test_pattern();
    let p: vhsphys_core::pal::video::Frame = vhsphys_core::pal::video::test_pattern();
    assert_eq!(takes_shared_frame(&n), (754, 480));
    assert_eq!(takes_shared_frame(&p), (740, 576));
}

#[test]
fn both_engines_build_their_parameters() {
    let _ = vhsphys_core::pal::pipeline::Params::default();
    let _ = vhsphys_core::ntsc::config::build(Some("pristine"), "", 1).unwrap();
    let _ = vhsphys_core::pal::config::build(Some("pristine"), "", 1).unwrap();
}
