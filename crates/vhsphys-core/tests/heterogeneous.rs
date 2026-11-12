//! Decks of different rasters in one rack: tagged signals, explicit links, exact clocks and
//! the four-field standards converter.

mod common;
use common::*;
use std::f64::consts::PI;
use vhsphys_core::convert::{ConverterProfile, StandardsConverter};
use vhsphys_core::frame::Frame;
use vhsphys_core::pipeline::{Chain, Session};
use vhsphys_core::rack::{LinkSpec, NamedLink, Rack, RackPlan, Separator, StageParams, StageSpec};
use vhsphys_core::signal::{Planes, Raster, Rational, SignalChunk, SignalPort};
use vhsphys_core::{ntsc, pal};

fn ntsc_stage(preset: &str, text: &str, seed: u64) -> StageParams {
    StageParams::L525(Box::new(ntsc::config::build(Some(preset), text, seed).unwrap()))
}

fn pal_stage(preset: &str, text: &str, seed: u64) -> StageParams {
    StageParams::L625(Box::new(pal::config::build(Some(preset), &format!("standard = \"pal\"\n{text}"), seed).unwrap()))
}

fn plan(stages: Vec<StageParams>, links: Vec<LinkSpec>) -> RackPlan {
    RackPlan {
        stages: stages.into_iter().enumerate().map(|(i, params)| StageSpec { name: format!("deck {i}"), params }).collect(),
        links: links.into_iter().enumerate().map(|(i, spec)| NamedLink { name: format!("cable {i}"), spec }).collect(),
    }
}

const BBC: LinkSpec = LinkSpec::StandardsConverter { profile: ConverterProfile::BbcFourField };

/// One frame of `raster` whose active picture is `f(row, x, field time in seconds)` as
/// (luma, u, v) in the component basis (black 0, white 1).
fn chunk(raster: Raster, frame: i64, f: impl Fn(usize, usize, f64) -> [f32; 3]) -> SignalChunk {
    let s = raster.spec();
    let mut planes = (s.blank)();
    for row in 0..s.active_h {
        let t = s.field_period.mul_int(2 * frame + (row % 2) as i64).to_f64();
        let line = s.field_first_line[row % 2] + row / 2;
        let base = line * s.samples_per_line + s.active_start;
        for x in 0..s.active_w {
            let [y, u, v] = f(row, x, t);
            planes.y[base + x] = s.setup_ire + s.span_ire * y;
            planes.cu[base + x] = s.span_ire * u;
            planes.cv[base + x] = s.span_ire * v;
        }
    }
    SignalChunk::new(raster, frame, planes)
}

/// The component values at (row, x) of a chunk's active picture.
fn at(c: &SignalChunk, row: usize, x: usize) -> [f64; 3] {
    let s = c.port.raster.spec();
    let i = (s.field_first_line[row % 2] + row / 2) * s.samples_per_line + s.active_start + x;
    [((c.planes.y[i] - s.setup_ire) / s.span_ire) as f64, (c.planes.cu[i] / s.span_ire) as f64, (c.planes.cv[i] / s.span_ire) as f64]
}

/// Convert `frames` source frames of a synthetic picture, all at once.
fn convert(profile: ConverterProfile, src: Raster, dst: Raster, frames: i64, f: impl Fn(usize, usize, f64) -> [f32; 3]) -> Vec<SignalChunk> {
    let mut c = StandardsConverter::new(profile, src, dst, 0, 0).unwrap();
    let mut out = Vec::new();
    for k in 0..frames {
        c.push(chunk(src, k, &f)).unwrap();
        out.extend(c.pull());
    }
    c.finish();
    out.extend(c.pull());
    out
}

fn other(r: Raster) -> Raster {
    if r == Raster::L525 {
        Raster::L625
    } else {
        Raster::L525
    }
}

// ------------------------------------------------------------------ the existing path

#[test]
fn a_rack_of_one_engine_with_direct_links_is_the_chain() {
    let input: Vec<_> = (0..7).map(|_| ntsc::video::test_pattern()).collect();
    let stages = ntsc::config::generation_params(Some("worn"), "", 11, 3).unwrap();
    let chained = Chain::run(stages.clone(), &input);
    let racked = Rack::run(RackPlan::direct(stages.into_iter().map(|p| StageParams::L525(Box::new(p))).collect()), &input).unwrap();
    assert_eq!(chained.len(), racked.len());
    assert_eq!(fnv(&chained), fnv(&racked));

    let input: Vec<_> = (0..6).map(|_| pal::video::test_pattern()).collect();
    let stages = pal::config::generation_params(Some("worn"), "standard = \"pal\"\n", 4, 2).unwrap();
    let chained = pal::pipeline::Chain::run(stages.clone(), &input);
    let racked = Rack::run(RackPlan::direct(stages.into_iter().map(|p| StageParams::L625(Box::new(p))).collect()), &input).unwrap();
    assert_eq!(fnv(&chained), fnv(&racked));
}

#[test]
fn a_rack_of_one_generation_is_the_plain_session() {
    let input: Vec<_> = (0..6).map(|_| ntsc::video::test_pattern()).collect();
    let p = ntsc::config::build(Some("worn"), "", 11).unwrap();
    let plain = Session::run(p.clone(), &input);
    let racked = Rack::run(RackPlan::direct(vec![StageParams::L525(Box::new(p))]), &input).unwrap();
    assert_eq!(fnv(&plain), fnv(&racked));
}

#[test]
fn a_windowed_rack_starts_where_the_chain_does() {
    let input: Vec<_> = (0..10).map(|_| ntsc::video::test_pattern()).collect();
    let stages = ntsc::config::generation_params(Some("worn"), "", 9, 2).unwrap();
    let mut chain = Chain::with_start(stages.clone(), 4);
    let start = Raster::L525.frame_time(4);
    let mut rack = Rack::new(RackPlan::direct(stages.into_iter().map(|p| StageParams::L525(Box::new(p))).collect()), start).unwrap();
    assert_eq!((rack.input_start_frame(), rack.output_start_time()), (4, start));
    let (mut a, mut b) = (vec![], vec![]);
    for f in &input[4..] {
        chain.push(f);
        rack.push(f);
    }
    chain.finish();
    rack.finish();
    while chain.ready() > 0 {
        a.extend(chain.process(3));
    }
    while rack.ready() > 0 {
        b.extend(rack.process(5));
    }
    assert_eq!(a.len(), 6);
    assert_eq!(fnv(&a), fnv(&b));
}

// ------------------------------------------------------------------ ports and validation

#[test]
fn a_change_of_raster_without_a_converter_is_refused_and_the_edge_is_named() {
    for link in [LinkSpec::DirectYcReference, LinkSpec::SeparateYc, LinkSpec::Composite { separator: Separator::Notch }] {
        let p = plan(vec![pal_stage("good", "", 1), ntsc_stage("good", "", 2)], vec![link]);
        let err = p.validate().unwrap_err();
        for part in ["link 0", "cable 0", "deck 0", "deck 1", "625/50", "525/59.94", "standards_converter"] {
            assert!(err.contains(part), "'{part}' is missing from: {err}");
        }
        assert!(Rack::new(p, Rational::ZERO).is_err());
    }
}

#[test]
fn a_converter_between_like_rasters_and_a_wrong_link_count_are_refused() {
    let same = plan(vec![ntsc_stage("good", "", 1), ntsc_stage("good", "", 2)], vec![BBC]);
    assert!(same.validate().unwrap_err().contains("two different rasters"));
    let short = plan(vec![ntsc_stage("good", "", 1), ntsc_stage("good", "", 2)], vec![]);
    assert!(short.validate().unwrap_err().contains("need 1 links"));
    let none = plan(vec![], vec![]);
    assert!(none.validate().is_err());
}

#[test]
fn pal_60_is_a_colour_system_on_the_525_raster_not_a_change_of_raster() {
    let pal60 = || ntsc_stage("good", "standard = \"pal60\"\n", 1);
    // decoded components pass to an NTSC recorder: the raster is the same
    assert!(plan(vec![pal60(), ntsc_stage("good", "", 2)], vec![LinkSpec::DirectYcReference]).validate().is_ok());
    // on a subcarrier they do not: the recorder's input takes M/NTSC
    for link in [LinkSpec::Composite { separator: Separator::Comb1h }, LinkSpec::SeparateYc] {
        let err = plan(vec![pal60(), ntsc_stage("good", "", 2)], vec![link]).validate().unwrap_err();
        assert!(err.contains("PAL-60") && err.contains("M/NTSC"), "{err}");
    }
    // and it is still not 625/50
    assert!(plan(vec![pal60(), pal_stage("good", "", 2)], vec![LinkSpec::DirectYcReference]).validate().is_err());
    let ports = plan(vec![pal60(), pal_stage("good", "", 2)], vec![BBC]).validate().unwrap();
    assert_eq!(ports[0], (SignalPort::reference(Raster::L525), SignalPort::reference(Raster::L625)));
}

#[test]
fn a_converter_refuses_a_signal_that_is_not_its_source() {
    let mut c = StandardsConverter::new(ConverterProfile::BbcFourField, Raster::L625, Raster::L525, 0, 0).unwrap();
    let flat = |_: usize, _: usize, _: f64| [0.5f32, 0.0, 0.0];
    assert!(c.push(chunk(Raster::L525, 0, flat)).unwrap_err().contains("525/59.94"));
    assert!(c.push(chunk(Raster::L625, 1, flat)).unwrap_err().contains("expects source frame 0"));
    assert!(c.push(chunk(Raster::L625, 0, flat)).is_ok());
    assert!(c.push(chunk(Raster::L625, 0, flat)).is_err(), "a repeated frame");
}

// ------------------------------------------------------------------ clocks

#[test]
fn output_frames_follow_the_target_clock_exactly() {
    for (src, n_src) in [(Raster::L625, 25i64), (Raster::L525, 30)] {
        let dst = other(src);
        let out = convert(ConverterProfile::BbcFourField, src, dst, n_src, |_, _, _| [0.4, 0.0, 0.0]);
        // every target frame that starts before the end of the last source frame
        let end = src.frame_time(n_src);
        let want = (0..).take_while(|&k| dst.frame_time(k) < end).count();
        assert_eq!(out.len(), want, "{} -> {}", src.name(), dst.name());
        for (k, c) in out.iter().enumerate() {
            assert_eq!(c.frame, k as i64);
            assert_eq!(c.pts, dst.frame_period().mul_int(k as i64));
            assert_eq!(c.port, SignalPort::reference(dst));
            c.check().unwrap();
        }
    }
    // one second of 625/50 is 25 frames in and 30 out (30 x 1001/30000 = 1.001 s is past the end)
    assert_eq!(convert(ConverterProfile::FieldHold, Raster::L625, Raster::L525, 25, |_, _, _| [0.0; 3]).len(), 30);
    // A source that is itself a conversion overhangs its programme: 30 frames of 525 (1.001 s)
    // are 26 of 625, the last of which ends at 1.04 s. Converted back, the programme's end
    // bounds the output at 30 frames, whether the frames are taken as they come or at the end.
    let end = Raster::L525.frame_time(30);
    for profile in ConverterProfile::ALL {
        for streamed in [true, false] {
            let mut c = StandardsConverter::new(profile, Raster::L625, Raster::L525, 0, 0).unwrap();
            let mut n = 0;
            for k in 0..26 {
                c.push(chunk(Raster::L625, k, |_, _, _| [0.3, 0.0, 0.0])).unwrap();
                if streamed {
                    n += c.pull().len();
                }
            }
            c.finish_at(end);
            n += c.pull().len();
            assert_eq!(n, 30, "{} streamed {streamed}", profile.name());
        }
    }
    // and 1001 s would be 25 025 frames in and 30 000 out, with nothing accumulated
    assert_eq!(Raster::L525.first_frame_at_or_after(Raster::L625.frame_time(25_025)), 30_000);
}

#[test]
fn the_converter_gives_the_same_frames_however_they_are_asked_for() {
    let moving = |row: usize, x: usize, t: f64| {
        let edge = 200.0 + 400.0 * t;
        [if (x as f64) < edge + row as f64 * 0.3 { 0.8 } else { 0.1 }, 0.1, -0.05]
    };
    let streamed = convert(ConverterProfile::BbcFourField, Raster::L625, Raster::L525, 9, moving);
    // everything pushed first, then everything pulled
    let mut c = StandardsConverter::new(ConverterProfile::BbcFourField, Raster::L625, Raster::L525, 0, 0).unwrap();
    for k in 0..9 {
        c.push(chunk(Raster::L625, k, moving)).unwrap();
    }
    c.finish();
    let batched = c.pull();
    assert_eq!(streamed.len(), batched.len());
    for (a, b) in streamed.iter().zip(&batched) {
        assert_eq!(a.frame, b.frame);
        // before the input ends a frame waits for its lookahead; at the end the last field is
        // held instead. Frames made both ways must agree wherever the lookahead was available.
        if Raster::L525.frame_time(a.frame + 1).add(Raster::L625.field_period().mul_int(2)) <= Raster::L625.frame_time(9) {
            assert!(a.planes.y == b.planes.y && a.planes.cu == b.planes.cu, "frame {}", a.frame);
        }
    }
    // a converter that starts later on the same timeline makes the same frames once its
    // aperture is clear of the start
    let mut late = StandardsConverter::new(ConverterProfile::BbcFourField, Raster::L625, Raster::L525, 3, 4).unwrap();
    for k in 3..9 {
        late.push(chunk(Raster::L625, k, moving)).unwrap();
    }
    late.finish();
    let late = late.pull();
    assert_eq!(late[0].frame, 4);
    let a = streamed.iter().find(|c| c.frame == 7).unwrap();
    let b = late.iter().find(|c| c.frame == 7).unwrap();
    assert!(a.planes.y == b.planes.y);
}

// ------------------------------------------------------------------ the converter's response

#[test]
fn flat_fields_convert_flat_at_the_target_levels() {
    for src in Raster::ALL {
        let dst = other(src);
        for profile in ConverterProfile::ALL {
            let out = convert(profile, src, dst, 4, |_, _, _| [0.6, 0.2, -0.15]);
            let d = dst.spec();
            let c = &out[2];
            for &(row, x) in &[(d.active_h / 2, d.active_w / 2), (d.active_h / 3 + 1, 100), (40, d.active_w - 60)] {
                let v = at(c, row, x);
                assert!((v[0] - 0.6).abs() < 1e-4 && (v[1] - 0.2).abs() < 1e-4 && (v[2] + 0.15).abs() < 1e-4, "{} {:?}: {v:?}", profile.name(), src);
            }
            // sync, blanking and burst are the target's own, not interpolated
            let blank = (d.blank)();
            let line = d.field_first_line[0] + 50;
            let s0 = line * d.samples_per_line;
            assert_eq!(&c.planes.y[s0..s0 + d.active_start], &blank.y[s0..s0 + d.active_start]);
            assert_eq!(&c.planes.cu[s0..s0 + d.active_start], &blank.cu[s0..s0 + d.active_start]);
            assert_eq!(&c.planes.y[..d.samples_per_line * 6], &blank.y[..d.samples_per_line * 6]);
        }
    }
    // black is the target's black: 7.5 IRE of setup at 525 lines, none at 625
    let to_525 = convert(ConverterProfile::BbcFourField, Raster::L625, Raster::L525, 4, |_, _, _| [0.0; 3]);
    let s = Raster::L525.spec();
    let i = (s.field_first_line[0] + 100) * s.samples_per_line + s.active_start + 300;
    assert!((to_525[1].planes.y[i] - 7.5).abs() < 1e-3);
}

/// A still vertical sine at a fixed point of the luminance table comes out with the table's
/// gain, in step with the picture height (not with the line count).
#[test]
fn still_vertical_detail_follows_the_published_response() {
    for (src, table) in [(Raster::L625, [0.95, 0.50]), (Raster::L525, [0.90, 0.50])] {
        let dst = other(src);
        let (hs, hd) = (src.spec().active_h as f64, dst.spec().active_h as f64);
        for (m, want) in table.iter().enumerate() {
            let fy = (m + 1) as f64 / 8.0; // cycles per source picture line
            let out = convert(ConverterProfile::BbcFourField, src, dst, 4, move |row, _, _| [(0.5 + 0.4 * (2.0 * PI * fy * row as f64).cos()) as f32, 0.0, 0.0]);
            let c = &out[2];
            let mut worst = 0.0f64;
            for row in 40..dst.spec().active_h - 40 {
                let y_src = (row as f64 + 0.5) * hs / hd - 0.5;
                let expect = 0.5 + 0.4 * want * (2.0 * PI * fy * y_src).cos();
                worst = worst.max((at(c, row, 300)[0] - expect).abs());
            }
            assert!(worst < 2e-4, "{} point {}: off by {worst}", src.name(), m + 1);
        }
    }
}

/// A whole-picture brightness change at a quarter of the source field rate comes out with the
/// table's gain at its first temporal point, at the output fields' own times: luminance by
/// table 9 or 11, colour difference by table 12.
#[test]
fn temporal_response_is_taken_at_the_true_field_times() {
    for (src, luma_gain) in [(Raster::L625, 0.90), (Raster::L525, 0.75)] {
        let dst = other(src);
        let f = 0.25 / src.field_period().to_f64(); // 12.5 Hz, or 14.985 Hz at 525 lines
        let wave = move |t: f64| (2.0 * PI * f * t).cos();
        let out = convert(ConverterProfile::BbcFourField, src, dst, 12, move |_, _, t| [(0.5 + 0.3 * wave(t)) as f32, (0.2 * wave(t)) as f32, 0.0]);
        let d = dst.spec();
        let (mut worst_y, mut worst_u) = (0.0f64, 0.0f64);
        for c in &out[3..out.len() - 4] {
            for field in 0..2usize {
                let t = d.field_period.mul_int(2 * c.frame + field as i64).to_f64();
                let v = at(c, 200 + field, 350);
                worst_y = worst_y.max((v[0] - (0.5 + 0.3 * luma_gain * wave(t))).abs());
                worst_u = worst_u.max((v[1] - 0.2 * 0.38 * wave(t)).abs());
            }
        }
        assert!(worst_y < 2e-4, "{}: luminance off by {worst_y}", src.name());
        assert!(worst_u < 2e-4, "{}: colour difference off by {worst_u}", src.name());
    }
}

/// Interlace. One field white and the other black, standing still, is the same set of samples
/// as a flat picture that flickers at the frame rate: on the interlaced lattice the two cannot
/// be told apart. A frame resize would take it for vertical detail and keep the line structure.
/// The aperture has a zero at the vertical limit and passes table 9's 0.32 at 25 Hz, so it
/// comes out as a flat field at each output field's own time, never as lines.
#[test]
fn field_difference_is_not_taken_for_vertical_detail() {
    let out = convert(ConverterProfile::BbcFourField, Raster::L625, Raster::L525, 8, |row, _, _| [if row % 2 == 0 { 0.9 } else { 0.1 }, 0.0, 0.0]);
    let (d, s) = (Raster::L525.spec(), Raster::L625.spec());
    for c in &out[2..6] {
        for field in 0..2usize {
            let tau = d.field_period.mul_int(2 * c.frame + field as i64).div(s.field_period).to_f64();
            // both images of the 25 Hz component (at +25 and -25 Hz) pass with gain 0.32
            let want = 0.5 + 0.4 * 2.0 * 0.32 * (PI * tau).cos();
            for row in (100 + field..300).step_by(2) {
                let v = at(c, row, 300)[0];
                assert!((v - want).abs() < 2e-4, "frame {} field {field} row {row}: {v} vs {want}", c.frame);
            }
        }
    }
}

/// A moving vertical edge keeps moving: each output field shows it where it was at that
/// field's time, between the positions of the neighbouring source fields.
#[test]
fn a_moving_edge_is_placed_by_time_not_by_frame() {
    let speed = 1500.0; // source samples per second
    let edge = move |t: f64| 150.0 + speed * t;
    let out = convert(ConverterProfile::BbcFourField, Raster::L625, Raster::L525, 10, move |_, x, t| [if (x as f64) < edge(t) { 0.9 } else { 0.1 }, 0.0, 0.0]);
    let (ws, wd) = (Raster::L625.spec().active_w as f64, Raster::L525.spec().active_w as f64);
    let d = Raster::L525.spec();
    let mut last = 0.0;
    for c in &out[2..out.len() - 3] {
        for field in 0..2usize {
            let t = d.field_period.mul_int(2 * c.frame + field as i64).to_f64();
            let row = 240 + field;
            // where the luminance crosses mid-level
            let x = (0..d.active_w - 1).find(|&x| at(c, row, x)[0] >= 0.5 && at(c, row, x + 1)[0] < 0.5).unwrap();
            let (a, b) = (at(c, row, x)[0], at(c, row, x + 1)[0]);
            let cross = x as f64 + (a - 0.5) / (a - b);
            let want = (edge(t) - 0.5) * wd / ws;
            // the aperture blends the edge's positions in four fields, so the mid-level crossing
            // lies between its positions one source field before and after
            let field_motion = speed * Raster::L625.field_period().to_f64() * wd / ws;
            assert!((cross - want).abs() < field_motion, "field at {t:.4} s: edge at {cross:.1}, moved to {want:.1}");
            assert!(cross > last, "the edge went backwards");
            last = cross;
        }
    }
}

/// The hold repeats source fields: it is the judder baseline, and it differs from the aperture.
#[test]
fn the_field_hold_baseline_repeats_fields() {
    let ramp = |_: usize, _: usize, t: f64| [(0.2 + t) as f32, 0.0, 0.0];
    let hold = convert(ConverterProfile::FieldHold, Raster::L625, Raster::L525, 8, ramp);
    let d = Raster::L525.spec();
    let s = Raster::L625.spec();
    for c in &hold[1..6] {
        for field in 0..2usize {
            let t = d.field_period.mul_int(2 * c.frame + field as i64);
            // the level of the latest source field at or before this output field
            let n = t.div(s.field_period).floor();
            let want = 0.2 + s.field_period.mul_int(n).to_f64();
            assert!((at(c, 200 + field, 300)[0] - want).abs() < 1e-5);
        }
    }
    let smooth = convert(ConverterProfile::BbcFourField, Raster::L625, Raster::L525, 8, ramp);
    // the aperture follows a steady ramp at the output's own time
    let c = &smooth[4];
    let t = d.field_period.mul_int(2 * c.frame).to_f64();
    assert!((at(c, 200, 300)[0] - (0.2 + t)).abs() < 2e-3);
}

// ------------------------------------------------------------------ whole racks

/// Mean colour in the middle of each of the seven bars of a test pattern of either raster.
fn bars(f: &Frame) -> Vec<[f32; 3]> {
    (0..7)
        .map(|b| {
            let (x0, x1) = ((b as f32 + 0.3) * f.w as f32 / 7.0, (b as f32 + 0.7) * f.w as f32 / 7.0);
            let mut acc = [0.0f32; 3];
            let mut n = 0.0;
            for y in f.h / 12..f.h * 7 / 12 {
                for x in x0 as usize..x1 as usize {
                    for k in 0..3 {
                        acc[k] += f.rgb[(y * f.w + x) * 3 + k] as f32;
                    }
                    n += 1.0;
                }
            }
            [acc[0] / n, acc[1] / n, acc[2] / n]
        })
        .collect()
}

fn colour_bars_625() -> Vec<Frame> {
    (0..8).map(|_| pal::video::test_pattern()).collect()
}

#[test]
fn a_625_tape_dubbed_to_a_525_deck_keeps_the_picture_and_changes_the_clock() {
    let p = plan(vec![pal_stage("pristine", "", 3), ntsc_stage("pristine", "", 4)], vec![BBC]);
    let out = Rack::run(p, &colour_bars_625()).unwrap();
    // 8 frames of 625/50 are 0.32 s: ten frames of 525/59.94 start inside it
    assert_eq!(out.len(), 10);
    assert!(out.iter().all(|f| (f.w, f.h) == (754, 480)));
    // The converter adds no colour error of its own: converted with no deck either side, the
    // bars are the bars exactly.
    let bb = pal::video::encode_frame(&pal::video::test_pattern());
    let mut c = StandardsConverter::new(ConverterProfile::BbcFourField, Raster::L625, Raster::L525, 0, 0).unwrap();
    for k in 0..4 {
        c.push(SignalChunk::new(Raster::L625, k, Planes { y: bb.y.clone(), cu: bb.cu.clone(), cv: bb.cv.clone() })).unwrap();
    }
    c.finish();
    let p = c.pull().swap_remove(2).planes;
    let converted = ntsc::video::decode_ideal(&ntsc::video::Baseband { y: p.y, cu: p.cu, cv: p.cv });
    for (g, r) in bars(&converted).iter().zip(&bars(&ntsc::video::test_pattern())) {
        for k in 0..3 {
            assert!((g[k] - r[k]).abs() < 1.0, "converter alone: {} vs {}", g[k], r[k]);
        }
    }
    // Through the rack the bars are those the 625-line deck plays, less what one more deck does.
    let played = pal::pipeline::Session::run(pal::config::build(Some("pristine"), "standard = \"pal\"\n", 3).unwrap(), &colour_bars_625());
    for (b, (g, r)) in bars(&out[5]).iter().zip(&bars(&played[4])).enumerate() {
        for k in 0..3 {
            assert!((g[k] - r[k]).abs() < 14.0, "bar {b} channel {k}: {} vs {}", g[k], r[k]);
        }
    }
}

#[test]
fn a_rack_through_both_rasters_is_deterministic_and_returns_to_its_raster() {
    let make = |seed: u64| {
        plan(
            vec![ntsc_stage("good", "", seed), pal_stage("good", "", seed + 1), ntsc_stage("good", "", seed + 2)],
            vec![BBC, LinkSpec::StandardsConverter { profile: ConverterProfile::BbcFourFieldNoisyChroma }],
        )
    };
    let input: Vec<_> = (0..8).map(|_| ntsc::video::test_pattern()).collect();
    let a = Rack::run(make(5), &input).unwrap();
    let b = Rack::run(make(5), &input).unwrap();
    let c = Rack::run(make(6), &input).unwrap();
    assert_eq!(fnv(&a), fnv(&b));
    assert_ne!(fnv(&a), fnv(&c));
    assert!(a.iter().all(|f| (f.w, f.h) == (754, 480)));
    // 8 frames of 525 are 0.267 s: 7 frames of 625 start inside that, and the last of them
    // ends after it (0.28 s). The last generation is bounded by the programme, not by that
    // overhang, so it plays 8 frames and not 9.
    assert_eq!(a.len(), 8);
    // two conversions and three tapes later the bars are still the bars
    let reference = bar_colours(&ntsc::video::test_pattern());
    for (g, r) in bar_colours(&a[4]).iter().zip(&reference) {
        for k in 0..3 {
            assert!((g[k] - r[k]).abs() < 30.0, "{} vs {}", g[k], r[k]);
        }
    }
}

#[test]
fn each_generation_starts_inside_the_one_before() {
    let p = plan(vec![ntsc_stage("good", "", 1), pal_stage("good", "", 2), ntsc_stage("good", "", 3)], vec![BBC, BBC]);
    let start = Raster::L525.frame_time(7);
    let rack = Rack::new(p, start).unwrap();
    let rec = rack.record();
    // frame 7 of 525 is at 0.2336 s; the 625 deck starts at its frame 6 (0.24 s); the last
    // deck at the first 525 frame at or after that, frame 8 (0.2669 s)
    assert_eq!(rec.generations.iter().map(|g| g.first_frame).collect::<Vec<_>>(), vec![7, 6, 8]);
    assert_eq!(rack.input_start_time(), start);
    assert_eq!(rack.output_start_time(), Raster::L525.frame_time(8));
    assert_eq!(rec.links[0].lookahead_s, "1001/30000");
    assert_eq!(rec.links[1].lookahead_s, "1/25");
    assert_eq!(rec.links[0].signal_delay_s, "0");
    let c = rec.links[0].converter.as_ref().unwrap();
    assert_eq!((c.source, c.target, c.profile), ("525/59.94", "625/50", "bbc_four_field"));
    assert!(rec.summary().contains("not calibrated against hardware"));
}

#[test]
fn a_composite_link_adds_its_cross_effects_between_the_decks() {
    let stages = || vec![ntsc_stage("pristine", "", 1), ntsc_stage("pristine", "", 2)];
    let input: Vec<_> = (0..6).map(|_| ntsc::video::test_pattern()).collect();
    let direct = Rack::run(plan(stages(), vec![LinkSpec::DirectYcReference]), &input).unwrap();
    let separate = Rack::run(plan(stages(), vec![LinkSpec::SeparateYc]), &input).unwrap();
    let composite = Rack::run(plan(stages(), vec![LinkSpec::Composite { separator: Separator::Notch }]), &input).unwrap();
    // separate conductors have no cross-effects to add
    assert_eq!(fnv(&direct), fnv(&separate));
    assert_ne!(fnv(&direct), fnv(&composite));
    // the notch separator takes luma near the subcarrier out with the chroma: the fine stripes
    // at the bottom of the pattern lose contrast
    let contrast = |f: &Frame| {
        let (mut s, mut s2, mut n) = (0.0, 0.0, 0.0);
        for y in 420..460 {
            for x in 40..300 {
                let i = (y * f.w + x) * 3;
                let v = 0.299 * f.rgb[i] as f64 + 0.587 * f.rgb[i + 1] as f64 + 0.114 * f.rgb[i + 2] as f64;
                s += v;
                s2 += v * v;
                n += 1.0;
            }
        }
        (s2 / n - (s / n) * (s / n)).sqrt()
    };
    assert!(contrast(&composite[3]) < 0.9 * contrast(&direct[3]), "{} vs {}", contrast(&composite[3]), contrast(&direct[3]));
}

// ------------------------------------------------------------------ configuration

const BOUNCE: &str = r#"
[condition]
tape_age_years = 3.0

[[generation]]
name = "camera original"
standard = "pal"
preset = "good"

[[generation]]
name = "export copy"
standard = "ntsc"
seed = 77
[generation.tape]
passes = 12.0

[[link]]
name = "converter"
type = "standards_converter"
profile = "bbc_four_field"
"#;

#[test]
fn a_rack_is_read_from_generation_and_link_tables() {
    let p = RackPlan::from_config("worn", BOUNCE, 5, "").unwrap().unwrap();
    assert_eq!(p.stages.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["camera original", "export copy"]);
    assert_eq!(p.stages.iter().map(|s| s.params.raster()).collect::<Vec<_>>(), vec![Raster::L625, Raster::L525]);
    assert_eq!((p.stages[0].params.seed(), p.stages[1].params.seed()), (5, 77));
    assert_eq!(p.links[0].spec, BBC);
    assert_eq!(p.links[0].name, "converter");
    let StageParams::L525(second) = &p.stages[1].params else { panic!("the second generation is a 525-line deck") };
    assert_eq!(second.tape.passes, 12.0);
    // the first generation's own preset, the common condition on both
    let StageParams::L625(first) = &p.stages[0].params else { panic!("the first generation is a 625-line deck") };
    let plain = pal::config::build(Some("good"), "standard = \"pal\"\n[condition]\ntape_age_years = 3.0\n", 5).unwrap();
    assert_eq!(format!("{first:?}"), format!("{plain:?}"));
    // no rack in the text: an ordinary run
    assert!(RackPlan::from_config("worn", "[tape]\npasses = 2.0\n", 5, "").unwrap().is_none());
}

#[test]
fn a_rack_config_of_one_raster_is_the_generations_of_a_plain_run() {
    let text = "seed = 9\n[tape]\npasses = 4.0\n[[generation]]\n[[generation]]\n[[generation]]\n";
    let p = RackPlan::from_config("worn", text, 2, "[decode]\ncomb = false\n").unwrap().unwrap();
    let plain = ntsc::config::generation_params(Some("worn"), "seed = 9\n[tape]\npasses = 4.0\n", 2, 3).unwrap();
    assert_eq!(p.links.len(), 2);
    assert!(p.links.iter().all(|l| l.spec == LinkSpec::DirectYcReference));
    for (stage, plain) in p.stages.iter().zip(&plain) {
        let plain = ntsc::config::apply_overrides(plain, "[decode]\ncomb = false\n").unwrap();
        let StageParams::L525(got) = &stage.params else { panic!("a 525-line deck") };
        assert_eq!(format!("{got:?}"), format!("{plain:?}"));
    }
}

#[test]
fn config_errors_name_what_is_wrong() {
    let two = "[[generation]]\nstandard = \"pal\"\n[[generation]]\nstandard = \"ntsc\"\n";
    // no link given: the reference link is assumed, and it does not change the raster
    let err = RackPlan::from_config("worn", two, 1, "").unwrap_err();
    assert!(err.contains("standards_converter") && err.contains("625/50"), "{err}");
    // a converter must name its profile
    let err = RackPlan::from_config("worn", &format!("{two}[[link]]\ntype = \"standards_converter\"\n"), 1, "").unwrap_err();
    assert!(err.contains("profile") && err.contains("bbc_four_field"), "{err}");
    let err = RackPlan::from_config("worn", &format!("{two}[[link]]\ntype = \"standards_converter\"\nprofile = \"lanczos\"\n"), 1, "").unwrap_err();
    assert!(err.contains("link 0"), "{err}");
    let err = RackPlan::from_config("worn", &format!("{two}[[link]]\ntype = \"resize\"\n"), 1, "").unwrap_err();
    assert!(err.contains("direct_yc_reference"), "{err}");
    // what an engine cannot play is still refused, and the generation is named
    let err = RackPlan::from_config("worn", "[[generation]]\nname = \"slow\"\nstandard = \"pal\"\n[generation.condition]\ntape_speed = \"ep\"\n", 1, "").unwrap_err();
    assert!(err.contains("slow") && err.contains("EP"), "{err}");
    assert!(RackPlan::from_config("worn", "[[link]]\ntype = \"composite\"\n", 1, "").is_err());
    assert!(RackPlan::from_config("worn", "[[generation]]\nstandard = \"secam\"\n", 1, "").is_err());
}

#[test]
fn the_television_connection_belongs_to_the_last_generation_only() {
    let text = "[composite]\nenabled = true\n[[generation]]\n[[generation]]\n";
    let p = RackPlan::from_config("good", text, 1, "").unwrap().unwrap();
    let enabled: Vec<bool> = p
        .stages
        .iter()
        .map(|s| match &s.params {
            StageParams::L525(p) => p.composite.enabled,
            StageParams::L625(p) => p.composite.enabled,
        })
        .collect();
    assert_eq!(enabled, vec![false, true]);
}
