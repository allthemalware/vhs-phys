//! Acceptance test for VHS HQ non-linear luminance noise reduction.

mod common;
use common::*;
use vhsphys_core::{config, pipeline::Session, video::test_pattern};

const GOLDEN: u64 = 0x64948fb7a6413cac;

#[test]
fn hq_noise_reduction_disabled_is_bit_identical_to_golden() {
    let p = config::build(Some("worn"), "", 42).unwrap();
    let input: Vec<_> = (0..5).map(|_| test_pattern()).collect();
    assert_eq!(fnv(&Session::run(p, &input)), GOLDEN);
}

#[test]
fn hq_noise_reduction_runs_in_sp_mode() {
    let p = config::build(Some("worn"), "hq = true\n", 42).unwrap();
    assert!(p.record.sub_emphasis.is_some());
    assert!(p.deck.sub_deemphasis.is_some());
    let input: Vec<_> = (0..2).map(|_| test_pattern()).collect();
    let out = Session::run(p, &input);
    assert_eq!(out.len(), 2);
}

#[test]
fn hq_pre_and_deemphasis_are_invertible_in_noiseless_channel() {
    use vhsphys_core::consts::FS_BB;
    use vhsphys_core::emphasis::{SubDeemphasis, SubPreemphasis, HQ_FIGURES};

    let mut pre = SubPreemphasis::new(&HQ_FIGURES, FS_BB);
    let mut de = SubDeemphasis::new(&HQ_FIGURES, FS_BB);
    pre.settle(10.0);
    de.settle(10.0);

    let original: Vec<f32> = (0..1000)
        .map(|k| 20.0 + 30.0 * (k as f32 * 0.05).sin() + if k > 500 { 25.0 } else { 0.0 })
        .collect();
    let mut sig = original.clone();
    pre.process(&mut sig);
    de.process(&mut sig);

    let err: f32 = original[50..]
        .iter()
        .zip(&sig[50..])
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(err < 0.25, "HQ noise reduction loop must invert with <0.25 IRE error, got {err}");
}

#[test]
fn hq_is_taken_out_again_when_a_speed_override_leaves_sp() {
    use vhsphys_core::format::TapeSpeed;

    // the top-level `speed` is merged after the condition has been mapped, as `--set speed=lp` is
    let p = config::build(Some("worn"), "speed = \"lp\"\nhq = true\n", 42).unwrap();
    assert_eq!(p.speed, TapeSpeed::Lp);
    assert_eq!(p.record.sub_emphasis, None);
    assert_eq!(p.deck.sub_deemphasis, None);
}

#[test]
fn hq_is_taken_out_of_parameters_built_at_sp_when_the_speed_changes() {
    use vhsphys_core::format::TapeSpeed;

    let sp = config::build(Some("worn"), "hq = true\n", 42).unwrap();
    assert_eq!(sp.speed, TapeSpeed::Sp);
    assert!(sp.record.sub_emphasis.is_some() && sp.deck.sub_deemphasis.is_some());
    let p = config::apply_overrides(&sp, "speed = \"lp\"\n").unwrap();
    assert_eq!(p.speed, TapeSpeed::Lp);
    assert_eq!(p.record.sub_emphasis, None);
    assert_eq!(p.deck.sub_deemphasis, None);
}

#[test]
fn hq_does_not_disturb_the_figures_of_the_speed_the_condition_sets() {
    use vhsphys_core::emphasis::{SubEmphasisParams, HQ_FIGURES};
    use vhsphys_core::format::TapeSpeed;

    let lp = config::build(Some("worn"), "hq = true\n[condition]\ntape_speed = \"lp\"\n", 42).unwrap();
    assert_eq!(lp.speed, TapeSpeed::Lp);
    let want_lp = SubEmphasisParams::for_speed(TapeSpeed::Lp);
    assert!(want_lp.is_some(), "the 525-line format has LP figures");
    assert_eq!(lp.record.sub_emphasis, want_lp);
    assert_eq!(lp.deck.sub_deemphasis, want_lp);

    let sp = config::build(Some("worn"), "hq = true\n", 42).unwrap();
    assert_eq!(sp.speed, TapeSpeed::Sp);
    assert_eq!(sp.record.sub_emphasis, Some(HQ_FIGURES));
    assert_eq!(sp.deck.sub_deemphasis, Some(HQ_FIGURES));
}

#[test]
fn hq_is_a_switch_on_the_decks_and_not_part_of_the_condition() {
    use vhsphys_core::condition::Condition;

    let names = toml::Value::try_from(Condition::default()).unwrap();
    assert!(names.get("hq_noise_reduction").is_none() && names.get("hq").is_none());
    let sets = config::sets_to_toml(&["hq=true".to_string()]).unwrap();
    let on = config::build(Some("worn"), &sets, 42).unwrap();
    assert!(on.hq && on.record.sub_emphasis.is_some());
    let off = config::apply_overrides(&on, "hq = false\n").unwrap();
    assert_eq!(off.record.sub_emphasis, None);
    assert_eq!(off.deck.sub_deemphasis, None);
}
