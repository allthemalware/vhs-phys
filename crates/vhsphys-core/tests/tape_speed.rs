//! SP, LP and EP: the speed sets the track pitch, and noise, crosstalk and tracking
//! tolerance follow from the geometry (nothing here is a filter painted on).

mod common;
use common::*;
use vhsphys_core::consts::*;
use vhsphys_core::format::TapeSpeed;
use vhsphys_core::pipeline::{Params, Session};
use vhsphys_core::{config, video::test_pattern};

fn params(preset: &str, speed: &str, extra: &str, seed: u64) -> Params {
    config::build(Some(preset), &format!("[condition]\ntape_speed = \"{speed}\"\n{extra}"), seed).unwrap()
}

#[test]
fn the_condition_selects_the_speed_and_a_speed_matched_head() {
    assert_eq!(Params::default().speed, TapeSpeed::Sp);
    for s in TapeSpeed::ALL {
        let p = params("good", s.name(), "", 1);
        assert_eq!(p.speed, s);
        for h in &p.heads {
            assert!((h.width_um - s.track_pitch_um()).abs() < 0.5, "{s:?}: head width {}", h.width_um);
        }
    }
}

#[test]
fn a_slower_tape_is_noisier() {
    let flat: Vec<_> = (0..6).map(|_| flat_frame(110)).collect();
    let noise = |s: &str| luma_noise(&Session::run(params("good", s, "", 3), &flat)[3]);
    let (sp, lp, ep) = (noise("sp"), noise("lp"), noise("ep"));
    assert!(sp < lp && lp < ep, "luma noise {sp:.2} < {lp:.2} < {ep:.2}");
    assert!(ep > 1.15 * sp, "EP {ep:.2} should be clearly noisier than SP {sp:.2}");
}

#[test]
fn a_tracking_error_hurts_narrow_tracks_more() {
    let flat: Vec<_> = (0..6).map(|_| flat_frame(110)).collect();
    let reference = flat_frame(110);
    let err = |s: &str| mean_abs_error(&Session::run(params("good", s, "tracking_error_um = 8.0\n", 3), &flat)[3], &reference);
    let (sp, ep) = (err("sp"), err("ep"));
    assert!(ep > 1.2 * sp, "an 8 um offset costs EP {ep:.2} but SP only {sp:.2}");
}

#[test]
fn colours_survive_at_every_speed() {
    let want = bar_colours(&test_pattern());
    for s in ["sp", "lp", "ep"] {
        let input: Vec<_> = (0..6).map(|_| test_pattern()).collect();
        let out = Session::run(params("pristine", s, "", 1), &input);
        for (w, g) in want.iter().zip(bar_colours(&out[3]).iter()) {
            for k in 0..3 {
                assert!((w[k] - g[k]).abs() < 50.0, "{s}: bar colour {w:?} came out {g:?}");
            }
        }
    }
}

#[test]
fn windowed_sessions_reproduce_the_tape_at_every_speed() {
    for s in ["lp", "ep"] {
        let p = params("worn", s, "", 9);
        let input: Vec<_> = (0..10).map(|_| test_pattern()).collect();
        let full = Session::run(p.clone(), &input);
        let mut part = vec![];
        let mut session = Session::with_start(p, 4);
        for f in &input[4..] {
            session.push(f);
            while session.ready() > 0 {
                part.extend(session.process(2));
            }
        }
        session.finish();
        while session.ready() > 0 {
            part.extend(session.process(2));
        }
        assert_eq!(part.len(), 6);
        let diff = mean_abs_error(&full[7], &part[3]);
        assert!(diff < 1.0, "{s}: mean abs difference {diff}");
    }
}

#[test]
fn neighbouring_tracks_are_offset_by_the_tape_advance() {
    // geometry that the picture's vertical structure depends on (see tape.rs)
    for s in TapeSpeed::ALL {
        let expect = s.advance_um() / UM_PER_SAMPLE;
        assert!((s.adjacent_track_offset_rf() - expect).abs() < 2.0, "{s:?}: {} vs {expect}", s.adjacent_track_offset_rf());
    }
}
