//! Composite video: Y/C summed on the subcarrier and separated again.

mod common;
use common::*;
use vhsphys_core::composite::{roundtrip, CompositeDecoder, CompositeParams};
use vhsphys_core::consts::*;
use vhsphys_core::pipeline::Session;
use vhsphys_core::video::{test_pattern, Baseband};
use vhsphys_core::config;

const DECODERS: [CompositeDecoder; 3] = [CompositeDecoder::Notch, CompositeDecoder::Comb1h, CompositeDecoder::Comb2h];
// a patch of the active picture inside one field, away from the edges of the filled region
const FILL_LINES: std::ops::Range<usize> = 100..260;
const FILL_X: std::ops::Range<usize> = 200..760;
const MEASURE_LINES: std::ops::Range<usize> = 150..210;
const MEASURE_X: std::ops::Range<usize> = 350..610;

fn on(d: CompositeDecoder) -> CompositeParams {
    CompositeParams { enabled: true, decoder: d }
}

fn idx(line: usize, x: usize) -> usize {
    line * BB_PER_LINE + x
}

fn filled(y: f32, cu: f32, cv: f32) -> Baseband {
    let mut bb = Baseband::blank();
    for l in FILL_LINES {
        for x in FILL_X {
            bb.y[idx(l, x)] = y;
            bb.cu[idx(l, x)] = cu;
            bb.cv[idx(l, x)] = cv;
        }
    }
    bb
}

/// Luma stripes at the subcarrier frequency: period four samples at 4 x fsc.
fn fsc_stripes() -> Baseband {
    let mut bb = filled(50.0, 0.0, 0.0);
    for l in FILL_LINES {
        for x in FILL_X {
            bb.y[idx(l, x)] = 50.0 + if (x / 2) % 2 == 0 { 15.0 } else { -15.0 };
        }
    }
    bb
}

fn mean(bb: &[f32]) -> f64 {
    let mut s = 0.0;
    let mut n = 0.0;
    for l in MEASURE_LINES {
        for x in MEASURE_X {
            s += bb[idx(l, x)] as f64;
            n += 1.0;
        }
    }
    s / n
}

/// Amplitude of the fsc-period stripe pattern left in the luma.
fn stripe_amplitude(bb: &Baseband) -> f64 {
    let mut s = 0.0;
    let mut n = 0.0;
    for l in MEASURE_LINES {
        for x in MEASURE_X {
            s += bb.y[idx(l, x)] as f64 * if (x / 2) % 2 == 0 { 1.0 } else { -1.0 };
            n += 1.0;
        }
    }
    s / n
}

fn chroma_magnitude(bb: &Baseband) -> f64 {
    let mut s = 0.0;
    let mut n = 0.0;
    for l in MEASURE_LINES {
        for x in MEASURE_X {
            s += ((bb.cu[idx(l, x)] as f64).powi(2) + (bb.cv[idx(l, x)] as f64).powi(2)).sqrt();
            n += 1.0;
        }
    }
    s / n
}

#[test]
fn flat_colour_survives_every_decoder() {
    for d in DECODERS {
        let mut bb = filled(50.0, 15.0, -10.0);
        roundtrip(&mut bb, 0, &on(d));
        let (y, cu, cv) = (mean(&bb.y), mean(&bb.cu), mean(&bb.cv));
        assert!((y - 50.0).abs() < 2.0 && (cu - 15.0).abs() < 3.0 && (cv + 10.0).abs() < 3.0, "{d:?}: y {y:.1} cu {cu:.1} cv {cv:.1}");
    }
}

#[test]
fn fine_luma_detail_becomes_colour_with_a_notch_but_not_with_a_comb() {
    let mut notch = fsc_stripes();
    roundtrip(&mut notch, 0, &on(CompositeDecoder::Notch));
    assert!(stripe_amplitude(&notch) < 6.0, "the notch should remove the stripes from luma: {:.1}", stripe_amplitude(&notch));
    assert!(chroma_magnitude(&notch) > 4.0, "and they turn up as colour: {:.1}", chroma_magnitude(&notch));
    for d in [CompositeDecoder::Comb1h, CompositeDecoder::Comb2h] {
        let mut bb = fsc_stripes();
        roundtrip(&mut bb, 0, &on(d));
        assert!(stripe_amplitude(&bb) > 12.0, "{d:?} keeps the detail in luma: {:.1}", stripe_amplitude(&bb));
        assert!(chroma_magnitude(&bb) < 2.0, "{d:?} makes no colour from it: {:.1}", chroma_magnitude(&bb));
    }
}

#[test]
fn the_dot_pattern_repeats_every_two_frames() {
    let at = |k: i64| {
        let mut bb = fsc_stripes();
        roundtrip(&mut bb, k, &on(CompositeDecoder::Notch));
        bb
    };
    let (f0, f1, f2) = (at(0), at(1), at(2));
    assert!(f0.cu == f2.cu && f0.cv == f2.cv && f0.y == f2.y, "frame 2 must equal frame 0");
    let rms = (MEASURE_LINES.flat_map(|l| MEASURE_X.map(move |x| (l, x))).map(|(l, x)| ((f0.cu[idx(l, x)] - f1.cu[idx(l, x)]) as f64).powi(2) + ((f0.cv[idx(l, x)] - f1.cv[idx(l, x)]) as f64).powi(2)).sum::<f64>()
        / (MEASURE_LINES.len() * MEASURE_X.len()) as f64)
        .sqrt();
    assert!(rms > 2.0, "consecutive frames should differ (dot crawl): {rms:.2}");
}

#[test]
fn sync_blanking_and_burst_are_left_alone() {
    let mut before = fsc_stripes();
    for l in FILL_LINES {
        for x in 80..116 {
            before.cu[idx(l, x)] = -20.0; // a burst
        }
    }
    let mut after = before.clone();
    roundtrip(&mut after, 0, &on(CompositeDecoder::Comb1h));
    for l in FILL_LINES {
        for x in 0..ACTIVE_START - 10 {
            assert_eq!(before.y[idx(l, x)], after.y[idx(l, x)], "luma at line {l} sample {x}");
            assert_eq!(before.cu[idx(l, x)], after.cu[idx(l, x)], "chroma at line {l} sample {x}");
        }
    }
}

#[test]
fn it_is_configured_like_everything_else() {
    let p = config::build(Some("worn"), "[composite]\nenabled = true\ndecoder = \"comb1h\"\n", 1).unwrap();
    assert!(p.composite.enabled && p.composite.decoder == CompositeDecoder::Comb1h);
    let off = config::build(Some("worn"), "", 1).unwrap();
    assert!(!off.composite.enabled, "composite is off unless asked for");
}

#[test]
fn colour_bars_survive_the_whole_chain_with_every_decoder() {
    let want = bar_colours(&test_pattern());
    let input: Vec<_> = (0..6).map(|_| test_pattern()).collect();
    let plain = Session::run(config::build(Some("pristine"), "", 1).unwrap(), &input);
    for d in ["notch", "comb1h", "comb2h"] {
        let p = config::build(Some("pristine"), &format!("[composite]\nenabled = true\ndecoder = \"{d}\"\n"), 1).unwrap();
        let out = Session::run(p, &input);
        for (w, g) in want.iter().zip(bar_colours(&out[3]).iter()) {
            for k in 0..3 {
                assert!((w[k] - g[k]).abs() < 40.0, "{d}: bar colour {w:?} came out {g:?}");
            }
        }
        assert!(mean_abs_error(&out[3], &plain[3]) > 0.1, "{d}: composite should visibly change the picture");
    }
}
