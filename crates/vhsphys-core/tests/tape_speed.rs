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
fn a_slower_tape_is_noisier_until_the_format_steps_in() {
    let flat: Vec<_> = (0..6).map(|_| flat_frame(110)).collect();
    let noise = |s: &str, stages: bool| {
        let mut p = params("good", s, "", 3);
        if !stages {
            p.record.sub_emphasis = None;
            p.deck.sub_deemphasis = None;
        }
        luma_noise(&Session::run(p, &flat)[3])
    };
    // the medium: a narrower track reads back more noise
    let (sp, lp, ep) = (noise("sp", false), noise("lp", false), noise("ep", false));
    assert!(sp < lp && lp < ep, "luma noise of the tape alone {sp:.2} < {lp:.2} < {ep:.2}");
    assert!(ep > 1.15 * sp, "EP {ep:.2} should be clearly noisier than SP {sp:.2} without the format's stages");
    // the format: the sub-emphasis of LP and EP (ST 32 table 4) takes high-frequency noise out of the slower
    // tapes, and SP has no such stage, so with the stages a slower tape is no noisier than SP on a flat field
    assert_eq!(noise("sp", true), sp, "SP has no stage to switch off");
    let (lp_f, ep_f) = (noise("lp", true), noise("ep", true));
    assert!(lp_f < lp && ep_f < ep, "the stages lower the noise: LP {lp_f:.2} against {lp:.2}, EP {ep_f:.2} against {ep:.2}");
    assert!(lp_f <= 1.15 * sp && ep_f <= 1.15 * sp, "with the format's stages LP {lp_f:.2} and EP {ep_f:.2} sit at or below SP {sp:.2}");
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
