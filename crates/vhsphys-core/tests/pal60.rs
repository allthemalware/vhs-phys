//! PAL-60: a 525-line NTSC tape played on a multi-standard PAL deck into a PAL television.
//! The tape is the NTSC tape; only the television's colour decoding changes.

mod common;
use common::*;
use vhsphys_core::standard::{ColourTv, TvStandard};
use vhsphys_core::{config, consts::*, pipeline::Session, video::{test_pattern, Frame}};

fn flat(rgb: [u8; 3]) -> Frame {
    let mut f = Frame::new(ACTIVE_W, ACTIVE_H);
    for p in f.rgb.chunks_mut(3) {
        p.copy_from_slice(&rgb);
    }
    f
}

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

fn hue_sat(f: &Frame, rows: std::ops::Range<usize>) -> (f64, f64) {
    let (mut u, mut v) = (0.0, 0.0);
    for y in rows.clone() {
        let (a, b) = row_uv(f, y);
        u += a;
        v += b;
    }
    let n = rows.len() as f64;
    let (u, v) = (u / n, v / n);
    (v.atan2(u).to_degrees(), (u * u + v * v).sqrt())
}

fn wrap_deg(a: f64) -> f64 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

/// A muted orange: turned 30 degrees either way it stays inside the RGB cube, so no channel
/// clips and the measured saturation is the decoder's.
const ORANGE: [u8; 3] = [190, 130, 80];

fn render(text: &str, input: &[Frame]) -> Vec<Frame> {
    Session::run(config::build(Some("pristine"), text, 2).unwrap(), input)
}

#[test]
fn pal60_is_the_525_line_engine_with_a_pal_television() {
    let p = config::build(Some("pristine"), "standard = \"pal60\"\n", 1).unwrap();
    assert_eq!(p.standard, TvStandard::Pal60);
    assert_eq!(p.decode.tv, ColourTv::Pal);
    assert!(p.decode.delay_line);
    assert_eq!(LINES, 525);
    let q = config::build(Some("pristine"), "", 1).unwrap();
    assert_eq!((q.standard, q.decode.tv), (TvStandard::Ntsc, ColourTv::Ntsc));
    // the 625-line standard is the other engine, not a setting of this one
    assert!(config::build(Some("pristine"), "standard = \"pal\"\n", 1).is_err());
}

#[test]
fn pal60_reproduces_the_colour_bars() {
    let input: Vec<Frame> = (0..6).map(|_| test_pattern()).collect();
    let out = render("standard = \"pal60\"\n", &input);
    let want = bar_colours(&test_pattern());
    let got = bar_colours(&out[3]);
    for (w, g) in want.iter().zip(got.iter()) {
        for k in 0..3 {
            assert!((w[k] - g[k]).abs() < 28.0, "bar colour {w:?} came out {g:?}");
        }
    }
}

/// The same tape into the same misadjusted set (its colour reference 30 degrees off): an NTSC
/// set shows a hue shift (hence its tint control), a PAL set a loss of saturation with the hue kept.
#[test]
fn the_same_phase_error_is_a_hue_shift_in_ntsc_and_a_saturation_loss_in_pal60() {
    let input: Vec<Frame> = (0..5).map(|_| flat(ORANGE)).collect();
    let rows = 60..420;
    let (h0, s0) = hue_sat(&render("", &input)[3], rows.clone());
    let (hn, sn) = hue_sat(&render("[decode]\ntv_phase_deg = 30.0\n", &input)[3], rows.clone());
    assert!((wrap_deg(hn - h0).abs() - 30.0).abs() < 6.0, "NTSC hue {h0} -> {hn}");
    assert!((sn / s0 - 1.0).abs() < 0.1, "NTSC saturation {s0} -> {sn}");

    let (hp0, sp0) = hue_sat(&render("standard = \"pal60\"\n", &input)[3], rows.clone());
    let (hp, sp) = hue_sat(&render("standard = \"pal60\"\n[decode]\ntv_phase_deg = 30.0\n", &input)[3], rows.clone());
    assert!(wrap_deg(hp0 - h0).abs() < 6.0, "PAL-60 without error moved the hue {h0} -> {hp0}");
    assert!(wrap_deg(hp - hp0).abs() < 6.0, "PAL-60 hue {hp0} -> {hp}");
    let ratio = sp / sp0;
    assert!((ratio - (30f64).to_radians().cos()).abs() < 0.1, "PAL-60 saturation ratio {ratio}");
}

#[test]
fn pal60_without_the_delay_line_shows_hanover_bars() {
    let input: Vec<Frame> = (0..5).map(|_| flat(ORANGE)).collect();
    let f = &render("standard = \"pal60\"\n[decode]\ntv_phase_deg = 30.0\ndelay_line = false\n", &input)[3];
    let (mut alt, mut n) = (0.0, 0.0);
    for y in (60..420).step_by(2) {
        let (ua, va) = row_uv(f, y);
        let (ub, vb) = row_uv(f, y + 1);
        alt += wrap_deg(va.atan2(ua).to_degrees() - vb.atan2(ub).to_degrees()).abs();
        n += 1.0;
    }
    let alt = alt / n;
    assert!((alt - 60.0).abs() < 12.0, "hue difference between neighbouring rows {alt}");
}

#[test]
fn pal60_colour_bars_survive_composite_with_every_decoder() {
    let input: Vec<Frame> = (0..6).map(|_| test_pattern()).collect();
    let plain = &render("standard = \"pal60\"\n", &input)[3];
    for d in ["notch", "comb1h", "comb2h"] {
        let out = &render(&format!("standard = \"pal60\"\n[composite]\nenabled = true\ndecoder = \"{d}\"\n"), &input)[3];
        assert_eq!((out.w, out.h), (plain.w, plain.h));
        let diff: f64 = out.rgb.iter().zip(plain.rgb.iter()).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / out.rgb.len() as f64;
        assert!(diff > 0.1, "{d}: composite should visibly affect the picture");
    }
}
