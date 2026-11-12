//! Acceptance test for true field-sequential interlaced video ingestion.

mod common;
use common::*;
use vhsphys_core::{
    automation::{AutomationSchedule, AutomationTrack},
    config,
    consts::*,
    pipeline::{Params, Session},
    tube::TubeKind,
    video::{encode_fields, encode_frame, test_pattern, Frame},
};

const GOLDEN: u64 = 0x64948fb7a6413cac;
/// Grey level of the lit frames of the clips that test the camera.
const LIT_GREY: u8 = 160;
/// Frames of light, then of darkness: long enough for the tube's charge to build up and for its tail to show.
const LIT_FRAMES: usize = 8;
const DARK_FRAMES: usize = 6;
/// Iris offset that darkens the picture a lot, in stops.
const IRIS_OFFSET_STOPS: f64 = -3.0;

#[test]
fn field_sequential_with_identical_fields_matches_encode_frame_identically() {
    let f = test_pattern();
    let bb_frame = encode_frame(&f);
    let bb_fields = encode_fields(&f, &f);

    assert_eq!(bb_frame.y, bb_fields.y);
    assert_eq!(bb_frame.cu, bb_fields.cu);
    assert_eq!(bb_frame.cv, bb_fields.cv);
}

#[test]
fn field_sequential_interlaces_distinct_temporal_motion() {
    let mut f0 = Frame::new(ACTIVE_W, ACTIVE_H);
    let mut f1 = Frame::new(ACTIVE_W, ACTIVE_H);

    // In field 0 (t0), top half is white, bottom is black
    for y in 0..ACTIVE_H / 2 {
        for x in 0..ACTIVE_W {
            let p = (y * ACTIVE_W + x) * 3;
            f0.rgb[p] = 255;
            f0.rgb[p + 1] = 255;
            f0.rgb[p + 2] = 255;
        }
    }

    // In field 1 (t1), bottom half is white, top is black
    for y in ACTIVE_H / 2..ACTIVE_H {
        for x in 0..ACTIVE_W {
            let p = (y * ACTIVE_W + x) * 3;
            f1.rgb[p] = 255;
            f1.rgb[p + 1] = 255;
            f1.rgb[p + 2] = 255;
        }
    }

    let bb = encode_fields(&f0, &f1);

    // Verify even lines come from f0 and odd lines come from f1
    let row_even = 100; // in top half (< ACTIVE_H/2)
    let row_odd = 101;

    let line_even = FIELD_FIRST_LINE[0] + row_even / 2;
    let line_odd = FIELD_FIRST_LINE[1] + row_odd / 2;

    let idx_even = line_even * BB_PER_LINE + ACTIVE_START + 50;
    let idx_odd = line_odd * BB_PER_LINE + ACTIVE_START + 50;

    // Line even (from f0, top half) is white (Y ~ 100 IRE)
    assert!(bb.y[idx_even] > 70.0, "even line should be bright white from f0, got {}", bb.y[idx_even]);
    // Line odd (from f1, top half) is black (Y ~ 7.5 IRE)
    assert!(bb.y[idx_odd] < 20.0, "odd line should be black from f1, got {}", bb.y[idx_odd]);
}

/// Drive a session by hand the way `Session::run` does, handing each input frame to `ingest`.
fn drive(params: Params, automation: Option<AutomationSchedule>, input: &[Frame], ingest: fn(&mut Session, &Frame)) -> Vec<Frame> {
    let mut s = Session::new(params);
    s.set_automation(automation);
    let mut out = Vec::new();
    for f in input {
        ingest(&mut s, f);
        while s.ready() > 0 {
            out.extend(s.process(4));
        }
    }
    s.finish();
    while s.ready() > 0 {
        out.extend(s.process(4));
    }
    out
}

fn by_frame(s: &mut Session, f: &Frame) {
    s.push(f);
}

/// Both fields of the frame sampled at the same moment, so the result is the frame itself.
fn by_fields(s: &mut Session, f: &Frame) {
    s.push_fields(f, f);
}

/// Lit frames, then dark ones: the light goes out and the tube's tail shows.
fn light_going_out() -> Vec<Frame> {
    let mut input = vec![flat_frame(LIT_GREY); LIT_FRAMES];
    input.extend(vec![flat_frame(0); DARK_FRAMES]);
    input
}

fn plumbicon_params() -> Params {
    let mut p = config::build(Some("pristine"), "", 1).unwrap();
    p.camera.tube = TubeKind::Plumbicon;
    p
}

fn assert_same_frames(a: &[Frame], b: &[Frame]) {
    assert_eq!(a.len(), b.len());
    for (k, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert!(x.rgb == y.rgb, "frame {k} differs, mean abs error {}", mean_abs_error(x, y));
    }
}

#[test]
fn field_sequential_session_push_preserves_golden() {
    let p = config::build(Some("worn"), "", 42).unwrap();
    let input: Vec<_> = (0..5).map(|_| test_pattern()).collect();
    assert_eq!(fnv(&drive(p, None, &input, by_fields)), GOLDEN);
}

/// The tube's lag is fitted for one scan of each element per frame time, so a clip that goes
/// in as fields must age the charge as often as one that goes in as frames.
#[test]
fn field_sequential_scans_each_camera_element_once_per_frame() {
    let input = light_going_out();
    let frames = drive(plumbicon_params(), None, &input, by_frame);
    let fields = drive(plumbicon_params(), None, &input, by_fields);
    assert_same_frames(&frames, &fields);
}

#[test]
fn field_sequential_applies_the_iris_automation() {
    let input = vec![flat_frame(LIT_GREY); LIT_FRAMES];
    let mut schedule = AutomationSchedule::new();
    schedule.set_track("iris_stops", AutomationTrack::constant(IRIS_OFFSET_STOPS));
    let frames = drive(plumbicon_params(), Some(schedule.clone()), &input, by_frame);
    let fields = drive(plumbicon_params(), Some(schedule), &input, by_fields);
    assert_same_frames(&frames, &fields);
    let plain = drive(plumbicon_params(), None, &input, by_fields);
    assert_ne!(fnv(&fields), fnv(&plain), "the automation did not reach the camera");
}
