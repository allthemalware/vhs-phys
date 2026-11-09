//! Dubbing: each generation re-records the previous generation's playback.

mod common;
use common::*;
use vhsphys_core::pipeline::{Chain, Params, Session};
use vhsphys_core::{config, video::test_pattern};

fn stages(preset: &str, seed: u64, n: usize) -> Vec<Params> {
    config::generation_params(Some(preset), "", seed, n).unwrap()
}

#[test]
fn one_generation_is_the_plain_session() {
    let input: Vec<_> = (0..6).map(|_| test_pattern()).collect();
    let p = config::build(Some("worn"), "", 11).unwrap();
    let plain = Session::run(p.clone(), &input);
    let chained = Chain::run(vec![p], &input);
    assert_eq!(plain.len(), chained.len());
    assert_eq!(fnv(&plain), fnv(&chained));
}

#[test]
fn generation_zero_is_the_plain_build_and_later_ones_are_new_tapes() {
    let s = stages("worn", 5, 3);
    assert_eq!(s.len(), 3);
    assert_eq!(format!("{:?}", s[0]), format!("{:?}", config::build(Some("worn"), "", 5).unwrap()));
    assert!(s[0].seed != s[1].seed && s[1].seed != s[2].seed && s[0].seed != s[2].seed);
    assert_ne!(format!("{:?}", s[1]), format!("{:?}", s[2]));
    assert_eq!(config::generation_params(Some("worn"), "", 5, 1).unwrap().len(), 1);
    assert!(config::generation_params(Some("worn"), "", 5, 0).is_err(), "zero generations is an error");
    assert!(config::generation_params(Some("nope"), "", 5, 2).is_err());
}

#[test]
fn a_copy_of_a_copy_is_noisier_every_time() {
    let flat: Vec<_> = (0..6).map(|_| flat_frame(110)).collect();
    let noise = |n: usize| luma_noise(&Chain::run(stages("good", 3, n), &flat)[3]);
    let (g1, g2, g3) = (noise(1), noise(2), noise(3));
    assert!(g1 < g2 && g2 < g3, "luma noise {g1:.2} < {g2:.2} < {g3:.2}");
    // A deck does not pass all of the noise it is fed (its low-passes remove most of the
    // high-frequency part) and adds its own, so the second generation is not sqrt(2) noisier:
    // measured over seeds 1 to 8 the ratio is 1.12 to 1.21.
    assert!(g2 > 1.08 * g1, "the second generation {g2:.2} should add visible noise to {g1:.2}");
}

#[test]
fn the_picture_degrades_cumulatively() {
    let reference = test_pattern();
    let input: Vec<_> = (0..6).map(|_| test_pattern()).collect();
    let err = |n: usize| mean_abs_error(&Chain::run(stages("worn", 7, n), &input)[3], &reference);
    let (e1, e2, e3) = (err(1), err(2), err(3));
    assert!(e1 < e2 && e2 < e3, "error {e1:.2} < {e2:.2} < {e3:.2}");
}

#[test]
fn chains_keep_the_frame_count_and_are_deterministic() {
    let input: Vec<_> = (0..7).map(|_| test_pattern()).collect();
    let a = Chain::run(stages("worn", 2, 2), &input);
    let b = Chain::run(stages("worn", 2, 2), &input);
    let c = Chain::run(stages("worn", 3, 2), &input);
    assert_eq!(a.len(), 7);
    assert_eq!(fnv(&a), fnv(&b));
    assert_ne!(fnv(&a), fnv(&c));
}

#[test]
fn a_windowed_chain_reproduces_the_full_run() {
    let input: Vec<_> = (0..12).map(|_| test_pattern()).collect();
    let full = Chain::run(stages("worn", 9, 2), &input);
    let mut chain = Chain::with_start(stages("worn", 9, 2), 4);
    let mut part = vec![];
    for f in &input[4..] {
        chain.push(f);
        while chain.ready() > 0 {
            part.extend(chain.process(2));
        }
    }
    chain.finish();
    while chain.ready() > 0 {
        part.extend(chain.process(2));
    }
    assert_eq!(part.len(), 8);
    let diff = mean_abs_error(&full[10], &part[6]);
    assert!(diff < 1.5, "mean abs difference {diff}");
}
