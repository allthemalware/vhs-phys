//! Colour saturation through the 625-line deck: an aligned deck plays back the saturation it
//! was given, and a copy of a copy does not gain colour.

use vhsphys_core::frame::Frame;
use vhsphys_core::{ntsc, pal};

/// Saturation of a pristine first generation relative to the source, either way.
const FIRST_GENERATION_TOLERANCE: f64 = 0.05;
/// Change of saturation from one pristine generation to the next, either way.
const PER_GENERATION_TOLERANCE: f64 = 0.04;
/// The two engines' pristine decks agree on saturation to this.
const ENGINE_TOLERANCE: f64 = 0.05;

/// The six coloured bars of the test pattern (bar 0 is grey).
const COLOURED_BARS: std::ops::Range<usize> = 1..7;

/// Chroma magnitude (the length of (U, V), in levels) in the middle of each bar.
fn bar_saturation(f: &Frame) -> Vec<f64> {
    (0..7)
        .map(|b| {
            let (x0, x1) = ((b as f64 + 0.3) * f.w as f64 / 7.0, (b as f64 + 0.7) * f.w as f64 / 7.0);
            let (mut u, mut v, mut n) = (0.0, 0.0, 0.0);
            for y in f.h / 12..f.h * 7 / 12 {
                for x in x0 as usize..x1 as usize {
                    let i = (y * f.w + x) * 3;
                    let (r, g, bl) = (f.rgb[i] as f64, f.rgb[i + 1] as f64, f.rgb[i + 2] as f64);
                    let luma = 0.299 * r + 0.587 * g + 0.114 * bl;
                    u += 0.492 * (bl - luma);
                    v += 0.877 * (r - luma);
                    n += 1.0;
                }
            }
            (u / n).hypot(v / n)
        })
        .collect()
}

/// Mean over the coloured bars of `a`'s saturation relative to `b`'s.
fn relative(a: &[f64], b: &[f64]) -> f64 {
    COLOURED_BARS.map(|i| a[i] / b[i]).sum::<f64>() / COLOURED_BARS.len() as f64
}

fn pal_generations(n: usize) -> Vec<f64> {
    let input: Vec<_> = (0..7).map(|_| pal::video::test_pattern()).collect();
    let stages = pal::config::generation_params(Some("pristine"), "standard = \"pal\"\n", 3, n).unwrap();
    bar_saturation(&pal::pipeline::Chain::run(stages, &input)[3])
}

fn ntsc_generations(n: usize) -> Vec<f64> {
    let input: Vec<_> = (0..7).map(|_| ntsc::video::test_pattern()).collect();
    let stages = ntsc::config::generation_params(Some("pristine"), "", 3, n).unwrap();
    bar_saturation(&ntsc::pipeline::Chain::run(stages, &input)[3])
}

#[test]
fn a_pristine_625_line_deck_plays_back_the_saturation_it_recorded() {
    let source = bar_saturation(&pal::video::test_pattern());
    let played = pal_generations(1);
    let ratio = relative(&played, &source);
    assert!((ratio - 1.0).abs() < FIRST_GENERATION_TOLERANCE, "saturation {ratio:.3} of the source");
    for i in COLOURED_BARS {
        let r = played[i] / source[i];
        assert!((r - 1.0).abs() < 2.0 * FIRST_GENERATION_TOLERANCE, "bar {i}: saturation {r:.3} of the source");
    }
}

#[test]
fn a_625_line_copy_of_a_copy_does_not_gain_colour() {
    let (g1, g2, g3) = (pal_generations(1), pal_generations(2), pal_generations(3));
    for (name, later, earlier) in [("second", &g2, &g1), ("third", &g3, &g2)] {
        let ratio = relative(later, earlier);
        assert!((ratio - 1.0).abs() < PER_GENERATION_TOLERANCE, "the {name} generation has {ratio:.3} of the saturation of the one before");
    }
}

#[test]
fn both_engines_play_back_the_same_saturation() {
    let pal = relative(&pal_generations(1), &bar_saturation(&pal::video::test_pattern()));
    let ntsc = relative(&ntsc_generations(1), &bar_saturation(&ntsc::video::test_pattern()));
    assert!((ntsc - 1.0).abs() < FIRST_GENERATION_TOLERANCE, "525 lines: saturation {ntsc:.3} of the source");
    assert!((pal - ntsc).abs() < ENGINE_TOLERANCE, "625 lines {pal:.3}, 525 lines {ntsc:.3}");
}
