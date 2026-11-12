//! Acceptance test for closed-loop second-order chroma APC PLL.

mod common;
use common::*;
use vhsphys_core::{config, pipeline::{Chain, Session}, video::{test_pattern, Frame}};

const GOLDEN: u64 = 0x64948fb7a6413cac;

/// How far, in levels, a bar of the last frame may move when the PLL replaces the first-order
/// loop (one generation, five frames). Measured: 0.8. The other frames of the run differ by up
/// to 17, because the PLL follows the burst's phase noise more closely, so this only holds the
/// last frame; a loop that has gone wrong moves the bars by 60 levels or more.
const PLL_BAR_TOLERANCE: f32 = 10.0;

/// The same for two generations, where the second deck adds its own noise to the burst.
/// Measured: 31.9.
const PLL_CHAIN_BAR_TOLERANCE: f32 = 48.0;

/// How far a bar of the last frame must move at least when the loop bandwidth is far beyond the
/// region where the loop settles: the gains are taken as given, so the hue rings and jumps as on a
/// mis-adjusted deck. A loop that has gone wrong moves the bars by 60 levels or more (149.6 was
/// measured for this run before the gains were ever held).
const PLL_UNSTABLE_BAR_MIN: f32 = 30.0;

#[test]
fn chroma_pll_disabled_is_bit_identical_to_golden() {
    let mut p = config::build(Some("worn"), "", 42).unwrap();
    p.decode.chroma_pll = false;
    let input: Vec<_> = (0..5).map(|_| test_pattern()).collect();
    assert_eq!(fnv(&Session::run(p, &input)), GOLDEN);
}

/// Largest difference, in levels, between the mean colours of the bars of two frames.
fn bar_distance(a: &Frame, b: &Frame) -> f32 {
    bar_colours(a)
        .iter()
        .zip(bar_colours(b).iter())
        .flat_map(|(x, y)| (0..3).map(move |k| (x[k] - y[k]).abs()))
        .fold(0.0, f32::max)
}

#[test]
fn chroma_pll_executes_and_stabilizes() {
    let mut p = config::build(Some("worn"), "", 42).unwrap();
    p.decode.chroma_pll = true;
    p.decode.pll_omega_n = 0.25;
    p.decode.pll_zeta = 0.707;
    let mut first_order = p.clone();
    first_order.decode.chroma_pll = false;
    let input: Vec<_> = (0..5).map(|_| test_pattern()).collect();
    let out = Session::run(p, &input);
    assert_eq!(out.len(), 5);
    let reference = Session::run(first_order, &input);
    // the switch must do something ...
    assert!(out.iter().zip(&reference).any(|(a, b)| a.rgb != b.rgb), "the PLL must change the output");
    // ... without moving the colours
    let d = bar_distance(&out[4], &reference[4]);
    assert!(d < PLL_BAR_TOLERANCE, "the PLL moved a bar by {d} levels");
}

#[test]
fn chroma_pll_tracks_across_generations() {
    let with = config::generation_params(Some("worn"), "[decode]\nchroma_pll = true\n", 42, 2).unwrap();
    let without = config::generation_params(Some("worn"), "[decode]\nchroma_pll = false\n", 42, 2).unwrap();
    assert!(with.iter().all(|p| p.decode.chroma_pll));
    let input: Vec<_> = (0..2).map(|_| test_pattern()).collect();
    let out = Chain::run(with, &input);
    assert_eq!(out.len(), 2);
    let reference = Chain::run(without, &input);
    let d = bar_distance(&out[1], &reference[1]);
    assert!(d < PLL_CHAIN_BAR_TOLERANCE, "the PLL moved a bar by {d} levels over two generations");
}

#[test]
fn chroma_pll_gains_outside_the_stable_region_disturb_the_hue() {
    let mut nominal = config::build(Some("worn"), "", 42).unwrap();
    nominal.decode.chroma_pll = true;
    let mut hot = nominal.clone();
    hot.decode.pll_omega_n = 5.0;
    let input: Vec<_> = (0..5).map(|_| test_pattern()).collect();
    let a = Session::run(nominal, &input);
    let b = Session::run(hot, &input);
    let d = bar_distance(&a[4], &b[4]);
    assert!(d > PLL_UNSTABLE_BAR_MIN, "an out-of-range loop bandwidth moved a bar by only {d} levels");
}

#[test]
fn chroma_pll_gains_are_taken_as_given() {
    // Two bandwidths that are both far outside the region give two different loops, and so
    // two different pictures, rather than one loop held at its edge.
    let mut a = config::build(Some("worn"), "", 42).unwrap();
    a.decode.chroma_pll = true;
    a.decode.pll_omega_n = 5.0;
    let mut b = a.clone();
    b.decode.pll_omega_n = 50.0;
    let input: Vec<_> = (0..2).map(|_| test_pattern()).collect();
    let (out_a, out_b) = (Session::run(a, &input), Session::run(b, &input));
    assert!(out_a.iter().zip(&out_b).any(|(x, y)| x.rgb != y.rgb));
}

#[test]
fn chroma_pll_takes_nonsense_gains_as_zero() {
    // A NaN bandwidth gives both gains as 0, the same open loop as a bandwidth of 0.
    let mut p = config::build(Some("worn"), "", 42).unwrap();
    p.decode.chroma_pll = true;
    let input: Vec<_> = (0..2).map(|_| test_pattern()).collect();
    let mut zero = p.clone();
    zero.decode.pll_omega_n = 0.0;
    let reference = Session::run(zero, &input);
    let mut q = p.clone();
    q.decode.pll_omega_n = f64::NAN;
    let out = Session::run(q, &input);
    assert!(out.iter().zip(&reference).all(|(x, y)| x.rgb == y.rgb));
}
