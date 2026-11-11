//! The 625-line PAL engine: raster, line switching, colour-under, FM, tape geometry, the
//! PAL television, and the same windowed rendering as the 525-line engine.

use std::f64::consts::PI;
use vhsphys_core::frame::Frame;
use vhsphys_core::pal::{config, consts::*, format::TapeSpeed, pipeline::Session, record, video};
use vhsphys_core::standard::{ColourTv, TvStandard};

fn bar_colours(f: &Frame) -> Vec<[f32; 3]> {
    (0..7)
        .map(|b| {
            let (x0, x1) = ((b as f32 + 0.3) * f.w as f32 / 7.0, (b as f32 + 0.7) * f.w as f32 / 7.0);
            let (mut acc, mut n) = ([0.0f32; 3], 0.0);
            for y in 60..330 {
                for x in x0 as usize..x1 as usize {
                    let i = (y * f.w + x) * 3;
                    for k in 0..3 {
                        acc[k] += f.rgb[i + k] as f32;
                    }
                    n += 1.0;
                }
            }
            acc.map(|a| a / n)
        })
        .collect()
}

fn flat(rgb: [u8; 3]) -> Frame {
    let mut f = Frame::new(ACTIVE_W, ACTIVE_H);
    for p in f.rgb.chunks_mut(3) {
        p.copy_from_slice(&rgb);
    }
    f
}

/// Mean (U, V) of one output row over the middle of the picture (BT.601 weights).
fn row_uv(f: &Frame, y: usize) -> (f64, f64) {
    let (mut u, mut v, mut n) = (0.0, 0.0, 0.0);
    for x in f.w / 4..3 * f.w / 4 {
        let i = (y * f.w + x) * 3;
        let (r, g, b) = (f.rgb[i] as f64, f.rgb[i + 1] as f64, f.rgb[i + 2] as f64);
        let l = 0.299 * r + 0.587 * g + 0.114 * b;
        u += 0.492 * (b - l);
        v += 0.877 * (r - l);
        n += 1.0;
    }
    (u / n, v / n)
}

fn hue_sat(uv: (f64, f64)) -> (f64, f64) {
    (uv.1.atan2(uv.0).to_degrees(), (uv.0 * uv.0 + uv.1 * uv.1).sqrt())
}

fn wrap_deg(a: f64) -> f64 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

const ORANGE: [u8; 3] = [210, 120, 40];

fn render(text: &str, input: &[Frame], seed: u64) -> Vec<Frame> {
    Session::run(config::build(Some("pristine"), text, seed).unwrap(), input)
}

#[test]
fn the_engine_is_625_lines_at_25_frames() {
    assert_eq!(STANDARD, TvStandard::Pal);
    assert_eq!((LINES, ACTIVE_W, ACTIVE_H), (625, 740, 576));
    let p = config::build(Some("pristine"), "", 1).unwrap();
    assert_eq!(p.standard, TvStandard::Pal);
    assert_eq!(p.decode.tv, ColourTv::Pal);
    assert!(p.decode.delay_line);
}

#[test]
fn the_raster_has_the_625_line_vertical_interval() {
    let bb = video::encode_frame(&video::test_pattern());
    assert_eq!(bb.y.len(), FRAME_BB);
    let at = |half_line: usize, off: usize| bb.y[half_line * HALF_LINE_BB + off];
    let sync = |x: f32| (x - SYNC_IRE).abs() < 0.5;
    let blank = |x: f32| (x - BLANK_IRE).abs() < 0.5;
    // field 1: broad pulses on half lines 0..5, equalising 5..10 and 1245..1250
    for h in 0..5 {
        assert!(sync(at(h, 200)), "broad pulse at half line {h}");
    }
    for h in (5..10).chain(1245..1250) {
        assert!(sync(at(h, 10)) && blank(at(h, 60)), "equalising pulse at half line {h}");
    }
    // field 2: the same 312.5 lines later
    for h in 625..630 {
        assert!(sync(at(h, 200)), "broad pulse at half line {h}");
    }
    for h in (620..625).chain(630..635) {
        assert!(sync(at(h, 10)) && blank(at(h, 60)), "equalising pulse at half line {h}");
    }
    // an ordinary line: 4.7 us sync, then blanking
    let l = 100 * BB_PER_LINE;
    assert!(sync(bb.y[l + HSYNC_W / 2]) && blank(bb.y[l + HSYNC_W + 3]));
}

#[test]
fn the_burst_sits_at_135_degrees_before_the_line_switch() {
    // the baseband is the unswitched (U, V) of the camera; the switch is applied per line
    // by the recorder, which knows the line's place in the four-field sequence
    let bb = video::encode_frame(&video::test_pattern());
    for line in [40usize, 41, 200, 401] {
        let i = line * BB_PER_LINE + BURST_START + BURST_LEN / 2;
        let a = (bb.cv[i] as f64).atan2(bb.cu[i] as f64).to_degrees();
        let m = ((bb.cu[i] * bb.cu[i] + bb.cv[i] * bb.cv[i]) as f64).sqrt();
        assert!((a - 135.0).abs() < 0.5, "line {line}: burst at {a} deg");
        assert!((m - BURST_AMP_IRE as f64).abs() < 0.2, "line {line}: burst {m}");
    }
}

#[test]
fn v_is_switched_on_alternate_lines_and_the_sequence_runs_across_frames() {
    for l in 0..2000i64 {
        assert_ne!(record::v_switched(l), record::v_switched(l + 1));
    }
    // 625 is odd: frame k+1 starts on the opposite switch state
    assert_ne!(record::v_switched(0), record::v_switched(LINES as i64));
}

#[test]
fn the_colour_under_advances_an_eighth_of_a_cycle_per_line_and_channel_2_turns_back_a_quarter() {
    assert!((F_CU - 626_953.125).abs() < 1e-6);
    for line in 0..16i64 {
        for (parity, quarter) in [(0usize, 0.0), (1, -0.25)] {
            let want = 2.0 * PI * (0.125 + quarter) * line as f64;
            let (c, s) = record::line_rotation(parity, line);
            let got = (s as f64).atan2(c as f64);
            let d = (got - want + PI).rem_euclid(2.0 * PI) - PI;
            assert!(d.abs() < 1e-4, "parity {parity} line {line}: {got} vs {want}");
        }
    }
}

#[test]
fn tape_speeds_are_sp_and_lp_only() {
    assert!((TapeSpeed::Sp.tape_speed_m_s() - 0.023_39).abs() < 1e-9);
    assert!((TapeSpeed::Sp.track_pitch_um() - 49.0).abs() < 1e-9);
    assert!((TapeSpeed::Lp.track_pitch_um() - 24.5).abs() < 0.2);
    assert!(config::build(Some("pristine"), "[condition]\ntape_speed = \"lp\"\n", 1).is_ok());
    let e = config::build(Some("pristine"), "[condition]\ntape_speed = \"ep\"\n", 1);
    assert!(e.is_err(), "EP is not a 625-line VHS speed");
}

#[test]
fn the_mains_is_50_hz_unless_the_condition_says_otherwise() {
    let p = config::build(None, "", 1).unwrap();
    assert!((p.deck.supply.mains_hz - 50.0).abs() < 1e-9, "{}", p.deck.supply.mains_hz);
}

#[test]
fn pristine_chain_reproduces_the_colour_bars() {
    let input: Vec<Frame> = (0..6).map(|_| video::test_pattern()).collect();
    let out = render("", &input, 1);
    assert_eq!((out[3].w, out[3].h), (ACTIVE_W, ACTIVE_H));
    let want = bar_colours(&video::test_pattern());
    let got = bar_colours(&out[3]);
    for (w, g) in want.iter().zip(got.iter()) {
        for k in 0..3 {
            assert!((w[k] - g[k]).abs() < 28.0, "bar colour {w:?} came out {g:?}");
        }
    }
}

/// The television's colour reference 30 degrees off: the PAL set keeps the hue and loses
/// saturation by cos(phi); without the delay line, alternate lines go wrong in opposite directions.
#[test]
fn a_phase_error_costs_saturation_not_hue_and_without_the_delay_line_shows_hanover_bars() {
    let input: Vec<Frame> = (0..5).map(|_| flat(ORANGE)).collect();
    let rows = 100..460;
    let mean = |f: &Frame| {
        let (mut u, mut v) = (0.0, 0.0);
        for y in rows.clone() {
            let (a, b) = row_uv(f, y);
            u += a;
            v += b;
        }
        let n = rows.len() as f64;
        hue_sat((u / n, v / n))
    };
    let (h0, s0) = mean(&render("", &input, 2)[3]);
    let (h1, s1) = mean(&render("[decode]\ntv_phase_deg = 30.0\n", &input, 2)[3]);
    assert!(wrap_deg(h1 - h0).abs() < 6.0, "hue moved {h0} -> {h1}");
    let ratio = s1 / s0;
    assert!((ratio - (30f64).to_radians().cos()).abs() < 0.1, "saturation ratio {ratio}");

    let simple = render("[decode]\ntv_phase_deg = 30.0\ndelay_line = false\n", &input, 2);
    let f = &simple[3];
    let (mut alt, mut n) = (0.0, 0.0);
    for y in (100..460).step_by(2) {
        let (ha, _) = hue_sat(row_uv(f, y));
        let (hb, _) = hue_sat(row_uv(f, y + 1));
        alt += wrap_deg(ha - hb).abs();
        n += 1.0;
    }
    let alt = alt / n;
    assert!((alt - 60.0).abs() < 12.0, "hue difference between neighbouring rows {alt}");
}

#[test]
fn windowed_session_matches_a_full_run() {
    let p = config::build(Some("worn"), "", 9).unwrap();
    let input: Vec<Frame> = (0..10).map(|_| video::test_pattern()).collect();
    let full = Session::run(p.clone(), &input);
    let mut s = Session::with_start(p, 4);
    let mut part = vec![];
    for f in &input[4..] {
        s.push(f);
        while s.ready() > 0 {
            part.extend(s.process(2));
        }
    }
    s.finish();
    while s.ready() > 0 {
        part.extend(s.process(2));
    }
    assert_eq!(part.len(), 6);
    let diff: f64 = full[7].rgb.iter().zip(part[3].rgb.iter()).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / full[7].rgb.len() as f64;
    assert!(diff < 1.0, "mean abs difference {diff}");
}

#[test]
fn output_is_deterministic_for_a_seed() {
    let input: Vec<Frame> = (0..4).map(|_| video::test_pattern()).collect();
    let a = Session::run(config::build(Some("worn"), "", 5).unwrap(), &input);
    let b = Session::run(config::build(Some("worn"), "", 5).unwrap(), &input);
    assert!(a.iter().zip(b.iter()).all(|(x, y)| x.rgb == y.rgb));
}

#[test]
fn pal_colour_bars_survive_composite_with_every_decoder() {
    let want = bar_colours(&video::test_pattern());
    let input: Vec<Frame> = (0..6).map(|_| video::test_pattern()).collect();
    let plain = render("", &input, 1);
    for d in ["notch", "comb1h", "comb2h"] {
        let out = render(&format!("[composite]\nenabled = true\ndecoder = \"{d}\"\n"), &input, 1);
        let got = bar_colours(&out[3]);
        for (w, g) in want.iter().zip(got.iter()) {
            for k in 0..3 {
                assert!((w[k] - g[k]).abs() < 40.0, "{d}: bar colour {w:?} came out {g:?}");
            }
        }
        let diff: f64 = out[3].rgb.iter().zip(plain[3].rgb.iter()).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / out[3].rgb.len() as f64;
        assert!(diff > 0.1, "{d}: composite should visibly affect the picture");
    }
}
