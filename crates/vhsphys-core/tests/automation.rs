//! Integration tests for time-varying parameter automation.

use vhsphys_core::automation::{AutomationSchedule, AutomationTrack};
use vhsphys_core::config;
use vhsphys_core::pipeline::{Chain, Session};
use vhsphys_core::video::{test_pattern, Frame};

fn mean_pixel_difference(a: &Frame, b: &Frame) -> f64 {
    assert_eq!(a.rgb.len(), b.rgb.len());
    let sum: f64 = a.rgb.iter().zip(b.rgb.iter()).map(|(&x, &y)| (x as f64 - y as f64).abs()).sum();
    sum / a.rgb.len() as f64
}

#[test]
fn automated_tracking_offset_progressively_degrades_picture() {
    let p = config::build(Some("good"), "", 1).unwrap();
    let mut schedule = AutomationSchedule::new();
    // Ramp tracking offset from 0.0 um (perfect alignment) to 25.0 um (severe mistracking)
    schedule.set_track("tracking_offset_um", AutomationTrack::linear_ramp(0.0, 0.0, 5.0, 25.0));

    let input: Vec<Frame> = (0..6).map(|_| test_pattern()).collect();
    let out = Session::run_with_automation(p, &input, schedule);
    assert_eq!(out.len(), 6);

    let pattern = test_pattern();
    // Frame 1 (near zero offset) should be close to reference
    let diff_early = mean_pixel_difference(&out[1], &pattern);
    // Frame 5 (severe offset) should be noticeably more degraded
    let diff_late = mean_pixel_difference(&out[5], &pattern);
    assert!(diff_late > diff_early + 2.0, "early diff: {diff_early}, late diff: {diff_late}");
}

#[test]
fn automated_head_clogging_attenuates_signal_progressively() {
    let p = config::build(Some("good"), "", 2).unwrap();
    let mut schedule = AutomationSchedule::new();
    // Progressive oxide buildup in head gap: 0.0 um -> 0.7 um extra spacing (Wallace spacing loss)
    schedule.set_track("head_spacing_um", AutomationTrack::smooth_ramp(0.0, 0.0, 5.0, 0.7));

    let input: Vec<Frame> = (0..6).map(|_| test_pattern()).collect();
    let out = Session::run_with_automation(p, &input, schedule);
    assert_eq!(out.len(), 6);

    let diff_between_start_and_end = mean_pixel_difference(&out[1], &out[5]);
    assert!(diff_between_start_and_end > 3.0, "clogging should alter output over time");
}

#[test]
fn automation_schedule_from_csv_and_chain_execution() {
    let csv = "\
frame, param, value, interp
0, tracking_offset_um, 0.0, linear
5, tracking_offset_um, 20.0, linear
";
    let schedule = AutomationSchedule::from_csv_str(csv).expect("valid CSV schedule");
    let p = config::build(Some("pristine"), "", 3).unwrap();
    let input: Vec<Frame> = (0..6).map(|_| test_pattern()).collect();

    let out = Chain::run_with_automation(vec![p], &input, schedule);
    assert_eq!(out.len(), 6);
}
