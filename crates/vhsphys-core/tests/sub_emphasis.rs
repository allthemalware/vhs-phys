// CONTRACT
//! The sub-preemphasis of the LP and EP modes (ST 32 clause 4.3.1.1.1, table 4): a level-dependent
//! boost of the high frequencies before the main emphasis, undone after the main de-emphasis on
//! playback. The record stage is measured the way the format measures it; the playback stage must
//! be its inverse; through the whole chain it takes noise out of the slower tapes; SP has none of it.

mod common;

use common::{flat_frame, luma_noise};
use std::f64::consts::TAU;
use vhsphys_core::condition::Condition;
use vhsphys_core::consts::*;
use vhsphys_core::emphasis::*;
use vhsphys_core::format::*;
use vhsphys_core::pipeline::{Params, Session};

/// Sync tip to peak white, IRE: the 0 dB input level of table 4.
const SPAN_IRE: f64 = 140.0;
/// The table is measured at a higher rate than the chain's, so no peak of a 3 MHz sine is missed;
/// a separate test holds the stage at the chain's own rate to the same figures.
const MEASURE_FS: f64 = 4.0 * FS_RF;

/// Peak to peak of a sampled waveform, each extremum refined by the parabola through its neighbours.
fn peak_to_peak(x: &[f32]) -> f64 {
    let (mut hi, mut lo) = (f64::MIN, f64::MAX);
    for i in 1..x.len() - 1 {
        let (a, b, c) = (x[i - 1] as f64, x[i] as f64, x[i + 1] as f64);
        let curve = a + c - 2.0 * b;
        let v = if curve.abs() < 1e-12 { b } else { b - (c - a).powi(2) / (8.0 * curve) };
        if b >= a && b >= c {
            hi = hi.max(v);
        }
        if b <= a && b <= c {
            lo = lo.min(v);
        }
    }
    hi - lo
}

/// Response of the record stage to a sine of `pp_ire` peak to peak at `f_hz`, in dB, over its last 20 cycles.
fn response_db(stage: &mut SubPreemphasis, fs: f64, f_hz: f64, pp_ire: f64) -> f64 {
    let n = (60.0 * fs / f_hz).round() as usize;
    let a = pp_ire / 2.0;
    let mut x: Vec<f32> = (0..n).map(|i| (a * (TAU * f_hz * i as f64 / fs).sin()) as f32).collect();
    stage.settle(0.0);
    stage.process(&mut x);
    let tail = &x[n - (20.0 * fs / f_hz) as usize..];
    20.0 * (peak_to_peak(tail) / pp_ire).log10()
}

fn figures(speed: TapeSpeed) -> SubEmphasisParams {
    SubEmphasisParams::for_speed(speed).unwrap_or_else(|| panic!("{speed:?}: the sub-preemphasis is part of the mode"))
}

fn lp_or_ep_params(speed: TapeSpeed) -> Params {
    let mut c = Condition::preset("pristine").unwrap();
    c.tape_speed = speed;
    c.to_params(3)
}

/// A line's worth of picture: 10 us steps between black and white, mid and fine detail, and noise.
fn test_signal(n: usize) -> Vec<f32> {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64 * 4.0 - 2.0
    };
    (0..n)
        .map(|i| {
            let t = i as f64 / FS_RF;
            let step = if (t * 1.0e5).floor() as i64 % 2 == 0 { 0.0 } else { 100.0 };
            (step + 10.0 * (TAU * 600.0e3 * t).sin() + 2.5 * (TAU * 2.0e6 * t).sin() + noise()) as f32
        })
        .collect()
}

#[test]
fn the_record_stage_follows_table_4_of_the_format() {
    for speed in [TapeSpeed::Lp, TapeSpeed::Ep] {
        let (table, tol) = speed.sub_emphasis_table().expect("table 4 covers LP and EP");
        let mut stage = SubPreemphasis::new(&figures(speed), MEASURE_FS);
        for (i, &level_db) in SUB_EMPHASIS_LEVEL_DB.iter().enumerate() {
            let pp = SPAN_IRE * 10f64.powf(level_db / 20.0);
            let reference = response_db(&mut stage, MEASURE_FS, 10.0e3, pp);
            assert!(reference.abs() < 0.1, "{speed:?}, input {level_db} dB: 0 dB at 10 kHz, got {reference:.2}");
            for (j, &f) in SUB_EMPHASIS_HZ.iter().enumerate() {
                let r = response_db(&mut stage, MEASURE_FS, f, pp) - reference;
                assert!(
                    (r - table[i][j]).abs() <= tol[i][j],
                    "{speed:?}, input {level_db} dB at {f} Hz: {r:.2} dB; table 4 says {} +/- {}",
                    table[i][j],
                    tol[i][j]
                );
            }
        }
    }
}

#[test]
fn the_stage_at_the_chain_rate_gives_the_same_figures() {
    for speed in [TapeSpeed::Lp, TapeSpeed::Ep] {
        let p = figures(speed);
        for &(f, pp) in &[(500.0e3, 44.0), (1.0e6, 14.0), (2.0e6, 140.0), (3.0e6, 14.0)] {
            let fine = response_db(&mut SubPreemphasis::new(&p, MEASURE_FS), MEASURE_FS, f, pp);
            let chain = response_db(&mut SubPreemphasis::new(&p, FS_RF), FS_RF, f, pp);
            assert!((fine - chain).abs() < 0.3, "{speed:?} at {f} Hz, {pp} IRE: {fine:.2} dB at the fine rate, {chain:.2} at the chain's");
        }
    }
}

#[test]
fn the_playback_stage_undoes_the_record_stage() {
    for speed in [TapeSpeed::Lp, TapeSpeed::Ep] {
        let p = figures(speed);
        let x = test_signal(3 * RF_PER_LINE);
        let mut y = x.clone();
        let mut pre = SubPreemphasis::new(&p, FS_RF);
        pre.settle(x[0] as f64);
        pre.process(&mut y);
        let changed = x.iter().zip(&y).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(changed > 5.0, "{speed:?}: the record stage changes the picture (largest change {changed} IRE)");
        let mut de = SubDeemphasis::new(&p, FS_RF);
        de.settle(y[0] as f64);
        de.process(&mut y);
        let err = x.iter().zip(&y).skip(2000).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(err < 0.05, "{speed:?}: record then play differs from the input by up to {err} IRE");
    }
}

#[test]
fn a_flat_field_passes_both_stages_unchanged() {
    for speed in [TapeSpeed::Lp, TapeSpeed::Ep] {
        let p = figures(speed);
        for level in [-40.0f32, 0.0, 50.0, 100.0] {
            let mut x = vec![level; 20_000];
            let mut pre = SubPreemphasis::new(&p, FS_RF);
            pre.settle(level as f64);
            pre.process(&mut x);
            let mut de = SubDeemphasis::new(&p, FS_RF);
            de.settle(level as f64);
            de.process(&mut x);
            let err = x.iter().map(|v| (v - level).abs()).fold(0.0f32, f32::max);
            assert!(err < 0.01, "{speed:?}: a flat {level} IRE moved by {err}");
        }
    }
}

#[test]
fn a_slower_tape_is_quieter_with_its_sub_emphasis_than_without() {
    let input: Vec<_> = (0..4).map(|_| flat_frame(110)).collect();
    for speed in [TapeSpeed::Lp, TapeSpeed::Ep] {
        let p = lp_or_ep_params(speed);
        assert!(p.record.sub_emphasis.is_some() && p.deck.sub_deemphasis.is_some(), "{speed:?}: the condition switches both stages on");
        assert_eq!(p.record.sub_emphasis, p.deck.sub_deemphasis, "{speed:?}: record and play use the same figures");
        let with = luma_noise(&Session::run(p, &input)[3]);
        let mut q = lp_or_ep_params(speed);
        q.record.sub_emphasis = None;
        q.deck.sub_deemphasis = None;
        let without = luma_noise(&Session::run(q, &input)[3]);
        assert!(with < 0.9 * without && with > 0.4 * without, "{speed:?}: luma noise {with:.2} with the stages, {without:.2} without");
    }
}

#[test]
fn sp_has_only_the_main_emphasis() {
    assert!(SubEmphasisParams::for_speed(TapeSpeed::Sp).is_none());
    let p = Condition::preset("pristine").unwrap().to_params(3);
    assert_eq!(p.speed, TapeSpeed::Sp);
    assert!(p.record.sub_emphasis.is_none() && p.deck.sub_deemphasis.is_none());
}
