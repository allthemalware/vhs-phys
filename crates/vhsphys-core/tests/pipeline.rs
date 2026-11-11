//! End-to-end invariants of the whole record -> tape -> deck -> display chain.

use vhsphys_core::config;
use vhsphys_core::consts::*;
use vhsphys_core::pipeline::{Params, Session};
use vhsphys_core::video::{test_pattern, Frame};

fn bar_colours(f: &Frame) -> Vec<[f32; 3]> {
    // mean colour in the middle of each of the 7 bars, rows 40..280 (away from the head switch)
    (0..7)
        .map(|b| {
            let (x0, x1) = ((b as f32 + 0.3) * ACTIVE_W as f32 / 7.0, (b as f32 + 0.7) * ACTIVE_W as f32 / 7.0);
            let mut acc = [0.0f32; 3];
            let mut n = 0.0;
            for y in 40..280 {
                for x in x0 as usize..x1 as usize {
                    for k in 0..3 {
                        acc[k] += f.rgb[(y * ACTIVE_W + x) * 3 + k] as f32;
                    }
                    n += 1.0;
                }
            }
            [acc[0] / n, acc[1] / n, acc[2] / n]
        })
        .collect()
}

fn run(preset: &str, frames: usize, seed: u64) -> Vec<Frame> {
    let p = config::build(Some(preset), "", seed).unwrap();
    let input: Vec<Frame> = (0..frames).map(|_| test_pattern()).collect();
    Session::run(p, &input)
}

#[test]
fn pristine_chain_reproduces_the_colour_bars() {
    let out = run("pristine", 6, 1);
    assert_eq!(out.len(), 6);
    let want = bar_colours(&test_pattern());
    let got = bar_colours(&out[3]);
    for (w, g) in want.iter().zip(got.iter()) {
        for k in 0..3 {
            assert!((w[k] - g[k]).abs() < 28.0, "bar colour {w:?} came out {g:?}");
        }
    }
}

#[test]
fn output_is_deterministic_for_a_seed_and_differs_between_seeds() {
    let a = run("worn", 5, 42);
    let b = run("worn", 5, 42);
    let c = run("worn", 5, 43);
    assert_eq!(a[2].rgb, b[2].rgb);
    assert_ne!(a[2].rgb, c[2].rgb);
}

#[test]
fn damage_increases_with_severity() {
    let reference = test_pattern();
    let err = |preset: &str| -> f64 {
        let out = run(preset, 6, 7);
        let f = &out[3];
        f.rgb.iter().zip(reference.rgb.iter()).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / f.rgb.len() as f64
    };
    let (p, w, r) = (err("pristine"), err("worn"), err("ruined"));
    assert!(p < w && w < r, "{p} < {w} < {r}");
}

#[test]
fn windowed_session_matches_a_full_run() {
    // rendering frames 4.. from a fresh session positioned at frame 4 must give the
    // same tape (defects, timebase) as the full run, so a plugin can render any range
    let p = config::build(Some("worn"), "", 9).unwrap();
    let input: Vec<Frame> = (0..10).map(|_| test_pattern()).collect();
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
    // frames away from the start of the window agree closely (the first frames
    // see a blank tape before the window, which only affects neighbouring tracks)
    let diff: f64 = full[7].rgb.iter().zip(part[3].rgb.iter()).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / full[7].rgb.len() as f64;
    assert!(diff < 1.0, "mean abs difference {diff}");
}

#[test]
fn default_params_survive_a_config_round_trip() {
    let p = Params::default();
    let text = toml::to_string(&p).unwrap();
    let q = config::apply_overrides(&Params::default(), &text).unwrap();
    assert_eq!(format!("{p:?}"), format!("{q:?}"));
}
