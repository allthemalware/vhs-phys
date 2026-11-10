// CONTRACT
//! Two details of the LP and EP modes in the format (ST 32 clause 4.3.1): the FM carrier recorded by
//! the channel-1 head sits half a line frequency above the channel-2 head's, and the colour burst is
//! not doubled at LP.

use vhsphys_core::condition::Condition;
use vhsphys_core::consts::*;
use vhsphys_core::format::TapeSpeed;
use vhsphys_core::pipeline::Params;
use vhsphys_core::record::{record_track, VideoSource};

/// A flat luminance with no colour: every reversal on tape is one half cycle of the FM carrier.
struct Flat(f32);

impl VideoSource for Flat {
    fn fill(&self, _bb_start: i64, y: &mut [f32], cu: &mut [f32], cv: &mut [f32]) {
        y.fill(self.0);
        cu.fill(0.0);
        cv.fill(0.0);
    }
}

fn params(speed: TapeSpeed) -> Params {
    let mut c = Condition::preset("pristine").unwrap();
    c.tape_speed = speed;
    c.to_params(1)
}

#[test]
fn the_format_says_which_speeds_interleave_and_double() {
    assert_eq!(TapeSpeed::Sp.carrier_interleave_hz(), 0.0);
    assert!((TapeSpeed::Lp.carrier_interleave_hz() - FH / 2.0).abs() < 1e-6);
    assert!((TapeSpeed::Ep.carrier_interleave_hz() - FH / 2.0).abs() < 1e-6);
    assert!(TapeSpeed::Sp.burst_doubled() && !TapeSpeed::Lp.burst_doubled() && TapeSpeed::Ep.burst_doubled());
}

#[test]
fn the_burst_is_doubled_except_at_lp() {
    for s in TapeSpeed::ALL {
        let p = params(s);
        let expect = if s.burst_doubled() { 2.0 } else { 1.0 };
        assert_eq!(p.record.burst_boost, expect, "{s:?}");
    }
}

#[test]
fn the_channel_1_carrier_sits_half_a_line_frequency_above_channel_2_at_lp_and_ep() {
    const LINES_MEASURED: usize = 200;
    let n_rf = LINES_MEASURED * RF_PER_LINE;
    for s in TapeSpeed::ALL {
        let p = params(s);
        assert!((p.record.carrier_interleave_hz - s.carrier_interleave_hz()).abs() < 1e-6, "{s:?}: the condition passes the format's figure on");
        let reversals = |parity| record_track(&Flat(50.0), &p.record, parity, 0, n_rf).len() as f64;
        let extra = reversals(1) - reversals(0);
        // fH/2 more carrier cycles per second is one more reversal per line
        let expect = if s == TapeSpeed::Sp { 0.0 } else { LINES_MEASURED as f64 };
        assert!((extra - expect).abs() <= 3.0, "{s:?}: channel 1 makes {extra} more reversals than channel 2 over {LINES_MEASURED} lines, expected {expect}");
    }
}
