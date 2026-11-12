//! Schedule files and scoping: the names a file may use, the order of its keyframes, which deck of a dub
//! a schedule reaches, and what the head model does with values it cannot work with.

mod common;
use common::*;
use vhsphys_core::automation::{AutomationSchedule, AutomationTrack, Keyframe};
use vhsphys_core::config;
use vhsphys_core::pipeline::{Chain, Params, Session};
use vhsphys_core::video::{test_pattern, Frame};
use std::path::PathBuf;

/// Frames per run: enough for the tracking ramp to finish inside the clip.
const CLIP_FRAMES: usize = 6;

/// Frame at which the tracking ramp reaches its end value.
const RAMP_END_FRAME: f64 = 5.0;

/// End value of the tracking ramp, micrometres (severe mistracking).
const RAMP_END_UM: f64 = 25.0;

/// Frames drawn from the first deck of a hand-built dub in one go.
const HAND_ON_BATCH: usize = 4;

/// A pulse half-width far below zero, micrometres: the head model must hold it at a usable value.
const NEGATIVE_H0_UM: f64 = -5.0;

/// A spacing, gain or wander far below zero: the head model must hold each at zero.
const NEGATIVE_OFFSET: f64 = -2.0;

/// Frames in the runs that drive a head value out of range.
const SHORT_CLIP_FRAMES: usize = 4;

fn clip() -> Vec<Frame> {
    (0..CLIP_FRAMES).map(|_| test_pattern()).collect()
}

fn tracking_ramp() -> AutomationSchedule {
    let mut schedule = AutomationSchedule::new();
    schedule.set_track("tracking_offset_um", AutomationTrack::linear_ramp(0.0, 0.0, RAMP_END_FRAME, RAMP_END_UM));
    schedule
}

fn two_decks() -> Vec<Params> {
    config::generation_params(Some("good"), "", 1, 2).unwrap()
}

/// Move what the first deck can play back into the second, as `Chain` does between generations.
fn hand_on(first: &mut Session, second: &mut Session) {
    while first.ready() > 0 {
        let (_, bbs) = first.process_baseband(HAND_ON_BATCH);
        for bb in bbs {
            second.push_baseband(bb);
        }
    }
}

/// A two-deck dub wired by hand, with `schedule` on the second (last) deck only.
fn dub_with_schedule_on_last_deck(input: &[Frame], schedule: Option<AutomationSchedule>) -> Vec<Frame> {
    let mut decks = two_decks().into_iter();
    let mut first = Session::new(decks.next().unwrap());
    let mut second = Session::new(decks.next().unwrap());
    second.set_automation(schedule);
    let mut out = Vec::new();
    for f in input {
        first.push(f);
        hand_on(&mut first, &mut second);
        while second.ready() > 0 {
            out.extend(second.process(HAND_ON_BATCH));
        }
    }
    first.finish();
    hand_on(&mut first, &mut second);
    second.finish();
    while second.ready() > 0 {
        out.extend(second.process(HAND_ON_BATCH));
    }
    out
}

fn assert_same_frames(a: &[Frame], b: &[Frame], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: frame count");
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert!(x.rgb == y.rgb, "{what}: frame {i} differs");
    }
}

#[test]
fn csv_files_may_use_the_short_names() {
    let csv = "\
frame, iris, tracking, clog, magnet_angle, slope, bow, wear, tension, head_magnet
0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 10.0
10, 2.0, 10.0, 0.4, 90.0, 5.0, 3.0, 0.5, -0.4, 2.0
";
    let schedule = AutomationSchedule::from_csv_str(csv).expect("short names are known names");
    assert_eq!(schedule.iris_stops(5.0), Some(1.0));
    assert_eq!(schedule.tracking_offset(5.0), Some(5.0));
    assert_eq!(schedule.head_spacing(5.0), Some(0.2));
    assert_eq!(schedule.eval("crt_magnet_angle_deg", 5.0), Some(45.0));
    assert_eq!(schedule.eval("tracking_slope_um", 5.0), Some(2.5));
    assert_eq!(schedule.eval("tracking_bow_um", 5.0), Some(1.5));
    assert_eq!(schedule.head_wear(5.0), Some(0.25));
    assert_eq!(schedule.tension(5.0), Some(-0.2));
    assert_eq!(schedule.head_magnet_cm(5.0), Some(6.0));

    let events = "\
frame, param, value
0, wander, 0.0
10, wander, 3.0
";
    let schedule = AutomationSchedule::from_csv_str(events).unwrap();
    assert_eq!(schedule.eval("tracking_wander_um", 5.0), Some(1.5));
}

#[test]
fn names_resolve_in_both_directions() {
    let mut schedule = AutomationSchedule::new();
    schedule.set_track("iris_stops", AutomationTrack::constant(1.5));
    assert_eq!(schedule.eval("iris", 3.0), Some(1.5));
    assert_eq!(schedule.eval("exposure", 3.0), Some(1.5));
    assert_eq!(schedule.eval("iris_stops", 3.0), Some(1.5));

    let mut schedule = AutomationSchedule::new();
    schedule.set_track("iris", AutomationTrack::constant(2.5));
    assert_eq!(schedule.iris_stops(3.0), Some(2.5));
    assert_eq!(schedule.eval("iris", 3.0), Some(2.5));

    // The programmatic API keeps any other name as given.
    schedule.set_track("my_own_curve", AutomationTrack::constant(7.0));
    assert_eq!(schedule.eval("my_own_curve", 0.0), Some(7.0));
}

#[test]
fn files_naming_an_unknown_parameter_are_refused() {
    for bad in ["crinkle_density", "typo_name"] {
        let csv = format!("frame, {bad}\n0, 1.0\n10, 2.0\n");
        let err = AutomationSchedule::from_csv_str(&csv).expect_err("unknown name in a CSV table");
        assert!(err.contains(bad), "CSV error should name '{bad}': {err}");

        let events = format!("frame, param, value\n0, {bad}, 1.0\n");
        let err = AutomationSchedule::from_csv_str(&events).expect_err("unknown name in a CSV event list");
        assert!(err.contains(bad), "CSV event error should name '{bad}': {err}");

        let json = format!(r#"{{"tracks": {{"{bad}": {{"keyframes": [{{"frame": 0, "value": 1}}]}}}}}}"#);
        let err = AutomationSchedule::from_json_str(&json).expect_err("unknown name in JSON");
        assert!(err.contains(bad), "JSON error should name '{bad}': {err}");

        let toml = format!("[tracks.{bad}]\nkeyframes = [{{ frame = 0.0, value = 1.0 }}]\n");
        let err = AutomationSchedule::from_toml_str(&toml).expect_err("unknown name in TOML");
        assert!(err.contains(bad), "TOML error should name '{bad}': {err}");
    }
}

#[test]
fn the_refusal_lists_the_known_names() {
    let err = AutomationSchedule::from_csv_str("frame, crinkle_density\n0, 1.0\n").unwrap_err();
    assert!(err.contains("tracking_offset_um") && err.contains("iris_stops"), "{err}");
}

#[test]
fn json_and_toml_files_may_use_the_short_names() {
    let json = r#"{"tracks": {"iris": {"keyframes": [{"frame": 0, "value": 0}, {"frame": 10, "value": 2}]}}}"#;
    assert_eq!(AutomationSchedule::from_json_str(json).unwrap().iris_stops(5.0), Some(1.0));

    let toml = "[tracks.clog]\nkeyframes = [{ frame = 0.0, value = 0.0 }, { frame = 10.0, value = 0.4 }]\n";
    assert_eq!(AutomationSchedule::from_toml_str(toml).unwrap().head_spacing(5.0), Some(0.2));
}

/// A file in the temporary directory that is removed again when dropped.
struct TempFile(PathBuf);

impl TempFile {
    fn new(name: &str, content: &str) -> Self {
        let path = std::env::temp_dir().join(format!("vhsphys_schedule_{}_{name}", std::process::id()));
        std::fs::write(&path, content).unwrap();
        Self(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn files_of_any_extension_are_checked_and_the_error_names_the_track() {
    let bad = r#"{"tracks": {"typo_name": {"keyframes": [{"frame": 0, "value": 1}]}}}"#;
    for name in ["bad.json", "bad.unknown"] {
        let file = TempFile::new(name, bad);
        let err = AutomationSchedule::from_file(&file.0).expect_err(name);
        assert!(err.contains("typo_name"), "{name}: {err}");
    }

    let good = r#"{"tracks": {"iris": {"keyframes": [{"frame": 0, "value": 2}]}}}"#;
    let file = TempFile::new("good.unknown", good);
    assert_eq!(AutomationSchedule::from_file(&file.0).unwrap().iris_stops(0.0), Some(2.0));
}

#[test]
fn keyframes_listed_out_of_order_are_sorted() {
    let json = r#"{"tracks": {"tracking_offset_um": {"keyframes": [{"frame": 60, "value": 10}, {"frame": 0, "value": 0}]}}}"#;
    let schedule = AutomationSchedule::from_json_str(json).unwrap();
    assert_eq!(schedule.tracking_offset(30.0), Some(5.0));

    let toml = "[tracks.tracking_offset_um]\nkeyframes = [{ frame = 60.0, value = 10.0 }, { frame = 0.0, value = 0.0 }]\n";
    let schedule = AutomationSchedule::from_toml_str(toml).unwrap();
    assert_eq!(schedule.tracking_offset(30.0), Some(5.0));

    let mut schedule = AutomationSchedule::new();
    let track = AutomationTrack { keyframes: vec![Keyframe::linear(60.0, 10.0), Keyframe::linear(0.0, 0.0)] };
    schedule.set_track("tracking_offset_um", track);
    assert_eq!(schedule.tracking_offset(30.0), Some(5.0));
}

#[test]
fn files_with_a_value_or_frame_that_is_not_finite_are_refused() {
    for bad in ["nan", "inf", "-inf"] {
        let table = format!("frame, tracking_offset_um\n0, 0.0\n10, {bad}\n");
        let err = AutomationSchedule::from_csv_str(&table).expect_err("non-finite value in a CSV table");
        assert!(err.contains("tracking_offset_um"), "the error should name the track: {err}");

        let events = format!("frame, param, value\n{bad}, head_spacing_um, 0.1\n");
        let err = AutomationSchedule::from_csv_str(&events).expect_err("non-finite frame in a CSV event list");
        assert!(err.contains("head_spacing_um"), "the error should name the track: {err}");

        let toml = format!("[tracks.iris_stops]\nkeyframes = [{{ frame = 0.0, value = {bad} }}]\n");
        let err = AutomationSchedule::from_toml_str(&toml).expect_err("non-finite value in TOML");
        assert!(err.contains("iris_stops"), "the error should name the track: {err}");

        let toml = format!("[tracks.iris_stops]\nkeyframes = [{{ frame = {bad}, value = 1.0 }}]\n");
        assert!(AutomationSchedule::from_toml_str(&toml).is_err(), "non-finite frame in TOML");
    }
    // JSON has no spelling for them, but a number past the range of f64 must not slip through either.
    let json = r#"{"tracks": {"iris_stops": {"keyframes": [{"frame": 0, "value": 1e999}]}}}"#;
    assert!(AutomationSchedule::from_json_str(json).is_err());
}

#[test]
fn the_schedule_reaches_only_the_last_deck_of_a_dub() {
    let input = clip();
    let reference = dub_with_schedule_on_last_deck(&input, Some(tracking_ramp()));
    assert_eq!(reference.len(), CLIP_FRAMES);

    // The hand-built dub is the chain: with no schedule the two agree.
    let plain_chain = Chain::run(two_decks(), &input);
    let plain_hand = dub_with_schedule_on_last_deck(&input, None);
    assert_same_frames(&plain_hand, &plain_chain, "hand-built dub without a schedule");
    assert_ne!(fnv(&reference), fnv(&plain_chain), "the ramp changes nothing, so the comparison below proves nothing");

    let scoped = Chain::run_with_automation(two_decks(), &input, tracking_ramp());
    assert_same_frames(&scoped, &reference, "chain with a tracking ramp");
}

#[test]
fn the_camera_track_alone_changes_nothing_when_there_is_no_tube() {
    let input = clip();
    let with_tracking = Chain::run_with_automation(two_decks(), &input, tracking_ramp());
    let mut schedule = tracking_ramp();
    schedule.set_track("iris", AutomationTrack::constant(2.0));
    let with_iris_too = Chain::run_with_automation(two_decks(), &input, schedule);
    assert_same_frames(&with_iris_too, &with_tracking, "extra iris track");
}

/// Run the "good" preset over a short clip with one constant track.
fn run_with_constant(name: &str, value: f64) -> Vec<Frame> {
    let p = config::build(Some("good"), "", 7).unwrap();
    let mut schedule = AutomationSchedule::new();
    schedule.set_track(name, AutomationTrack::constant(value));
    let input: Vec<Frame> = (0..SHORT_CLIP_FRAMES).map(|_| test_pattern()).collect();
    Session::run_with_automation(p, &input, schedule)
}

#[test]
fn a_pulse_width_driven_below_zero_still_gives_a_picture() {
    let out = run_with_constant("h0_um", NEGATIVE_H0_UM);
    assert_eq!(out.len(), SHORT_CLIP_FRAMES);
    for (i, f) in out.iter().enumerate() {
        assert!(f.rgb.iter().any(|&b| b != f.rgb[0]), "frame {i} is one flat value");
    }
    // Every value below the floor gives the same head, so how far below makes no difference.
    let further = run_with_constant("h0_um", 10.0 * NEGATIVE_H0_UM);
    assert_same_frames(&out, &further, "h0_um below its floor");
}

/// A value far below zero and one further below it must give the same output once both are held at the floor.
fn assert_held_at_floor(name: &str) {
    let a = run_with_constant(name, NEGATIVE_OFFSET);
    let b = run_with_constant(name, 10.0 * NEGATIVE_OFFSET);
    assert_same_frames(&a, &b, name);
}

#[test]
fn head_spacing_driven_below_zero_is_held_at_zero() {
    assert_held_at_floor("head_spacing_um");
}

#[test]
fn head_gain_driven_below_zero_is_held_at_zero() {
    assert_held_at_floor("head_gain");
}

#[test]
fn tracking_wander_driven_below_zero_is_held_at_zero() {
    assert_held_at_floor("tracking_wander_um");
}

#[test]
fn physical_tracks_change_the_rendered_picture() {
    let baseline = run_with_constant("tracking_offset_um", 0.0);
    for (name, val) in [
        ("head_wear_um", 0.5),
        ("tension_offset", -0.5),
        ("tracking_slope_um", 20.0),
        ("tracking_bow_um", 10.0),
        ("head_magnet_cm", 3.0),
    ] {
        let changed = run_with_constant(name, val);
        assert_eq!(changed.len(), SHORT_CLIP_FRAMES);
        assert!(
            changed.iter().zip(&baseline).any(|(a, b)| a.rgb != b.rgb),
            "track {name} with value {val} produced identical output to baseline"
        );
    }
}
