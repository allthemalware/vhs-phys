//! The plain path must never change.

mod common;
use common::*;
use vhsphys_core::{config, pipeline::Session, video::test_pattern};

/// FNV-1a of the output of the plain path (SP, no composite, one generation): the `worn`
/// preset, seed 42, five frames of the test pattern, as the prototype produced them.
/// Every feature must leave this path bit-identical when it is switched off.
const GOLDEN: u64 = 0x64948fb7a6413cac;

#[test]
fn the_plain_path_is_bit_identical_to_the_prototype() {
    let p = config::build(Some("worn"), "", 42).unwrap();
    let input: Vec<_> = (0..5).map(|_| test_pattern()).collect();
    assert_eq!(fnv(&Session::run(p, &input)), GOLDEN);
}
