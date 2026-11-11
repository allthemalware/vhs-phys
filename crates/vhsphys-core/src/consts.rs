//! Timing and format constants for NTSC VHS (SP mode).
//!
//! Sources (details in `docs/RESEARCH.md`): SMPTE ST 32-1998 (VHS type H),
//! Sencore TT189, vhs-decode format tables.

use std::f64::consts::PI;

// ---------------------------------------------------------------- video timing
/// NTSC colour subcarrier, 315/88 MHz.
pub const FSC: f64 = 315.0e6 / 88.0;
/// Horizontal line rate, fsc * 2 / 455.
pub const FH: f64 = FSC * 2.0 / 455.0;
pub const LINES: usize = 525;
pub const FRAME_RATE: f64 = FH / LINES as f64;

/// RF-domain samples per video line. 12 x fsc / (fsc*2/455) = 2730, so the RF
/// grid is exactly line locked (42.95 MS/s).
pub const RF_PER_LINE: usize = 2730;
/// Baseband (4 x fsc) samples per line.
pub const BB_PER_LINE: usize = 910;
/// RF samples per baseband sample.
pub const OS: usize = RF_PER_LINE / BB_PER_LINE;
pub const FS_RF: f64 = RF_PER_LINE as f64 * FH;
pub const FS_BB: f64 = BB_PER_LINE as f64 * FH;

pub const FIELD_RF: usize = 262 * RF_PER_LINE + RF_PER_LINE / 2;
pub const FRAME_RF: usize = LINES * RF_PER_LINE;
pub const FIELD_BB: usize = FIELD_RF / OS;
pub const FRAME_BB: usize = LINES * BB_PER_LINE;
pub const HALF_LINE_BB: usize = BB_PER_LINE / 2;

// Levels in IRE.
pub const SYNC_IRE: f32 = -40.0;
pub const BLANK_IRE: f32 = 0.0;
pub const SETUP_IRE: f32 = 7.5;
pub const SETUP_IRE_PAL: f32 = 0.0;
pub const WHITE_IRE: f32 = 100.0;
pub const VIDEO_SPAN_IRE: f32 = 92.5;
pub const VIDEO_SPAN_IRE_PAL: f32 = 100.0;
pub const BURST_AMP_IRE: f32 = 20.0;

// Baseband line layout (samples at 4 x fsc, 69.8 ns each).
pub const HSYNC_W: usize = 67; // 4.7 us
pub const EQ_PULSE_W: usize = 33; // 2.3 us
pub const BURST_START: usize = 80; // 5.7 us after sync leading edge
pub const BURST_LEN: usize = 36; // 9 cycles
pub const ACTIVE_START: usize = 135; // 9.4 us
pub const ACTIVE_W: usize = 754; // 52.6 us
pub const ACTIVE_H: usize = 480;
pub const FIELD_ACTIVE_LINES: usize = 240;
/// First visible line (0-based frame line) of each field.
pub const FIELD_FIRST_LINE: [usize; 2] = [21, 284];

// ------------------------------------------------------------------ VHS format
pub const DRUM_DIAMETER_M: f64 = 0.062;
/// Head-to-tape speed: pi * D * revolutions per second (one rev per frame).
pub const HEAD_SPEED: f64 = PI * DRUM_DIAMETER_M * FRAME_RATE;
/// Tape distance travelled by the head per RF sample, micrometres.
pub const UM_PER_SAMPLE: f64 = HEAD_SPEED / FS_RF * 1.0e6;
pub const TAPE_SPEED_SP: f64 = 0.03335;
pub const TRACK_PITCH_UM: f64 = 58.0;
pub const TRACK_PITCH_UM_PAL: f64 = 49.0;
pub const TRACK_ANGLE_DEG: f64 = 5.0 + 58.0 / 60.0 + 9.9 / 3600.0;
pub const VIDEO_AREA_MM: f64 = 10.07;
pub const VIDEO_CENTRE_MM: f64 = 6.2;
pub const TAPE_WIDTH_MM: f64 = 12.65;
pub const AZIMUTH_DEG: f64 = 6.0;

/// Luma FM: sync tip 3.4 MHz, 1 MHz per 140 IRE (white = 4.4 MHz).
pub const F_SYNC_TIP: f64 = 3.4e6;
/// PAL VHS Luma FM: sync tip 3.8 MHz, 1 MHz per 140 IRE (white = 4.8 MHz).
pub const F_SYNC_TIP_PAL: f64 = 3.8e6;
pub const HZ_PER_IRE: f64 = 1.0e6 / 140.0;
pub const PRE_EMPH_TAU: f64 = 1.3e-6;
pub const PRE_EMPH_RATIO: f64 = 5.0; // 1 + Rb/Ra, Rb/Ra = 4
pub const WHITE_CLIP_IRE: f32 = (-40.0 + 1.6 * 140.0) as f32; // 184
pub const DARK_CLIP_IRE: f32 = (-40.0 - 0.4 * 140.0) as f32; // -96
/// Colour-under carrier, 40 x fH.
pub const F_CU: f64 = 40.0 * FH;
/// PAL VHS colour-under carrier cycles per line (40 + 1/8 fH = 626.95 kHz).
pub const PAL_CU_CYCLES_PER_LINE: f64 = 40.125;

/// Along-track offset of adjacent tracks: the tape advance per field equals 1.5 H of
/// head travel, so neighbouring tracks are line aligned (see docs/RESEARCH.md 3.4).
pub const ADJACENT_TRACK_OFFSET_RF: usize = RF_PER_LINE * 3 / 2;

/// Head switch happens this many lines before vertical sync.
pub const SWITCH_LEAD_LINES: f64 = 6.5;
pub const SWITCH_LEAD_RF: usize = (SWITCH_LEAD_LINES * RF_PER_LINE as f64) as usize; // 17 745
/// Data margin recorded/rendered on each side of a track window (overlap).
pub const TRACK_MARGIN_RF: usize = 4 * RF_PER_LINE;
/// Extra warm-up for the common electronics at each end of a segment.
pub const SEGMENT_WARM_RF: usize = 2 * RF_PER_LINE;

/// Track length in micrometres (video area / sin(track angle)).
pub fn track_length_um() -> f64 {
    VIDEO_AREA_MM * 1000.0 / TRACK_ANGLE_DEG.to_radians().sin()
}

/// Global RF sample index at which head `n` takes over (start of its playback).
pub fn switch_time(field: i64) -> i64 {
    field * FIELD_RF as i64 - SWITCH_LEAD_RF as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_is_line_locked() {
        assert_eq!(FIELD_RF * 2, FRAME_RF);
        assert_eq!(FIELD_RF % OS, 0);
        assert_eq!(SWITCH_LEAD_RF % OS, 0);
        assert_eq!(TRACK_MARGIN_RF % OS, 0);
        assert!((FS_RF - 42_954_545.45).abs() < 1.0);
    }

    #[test]
    fn head_speed_matches_standard() {
        // ST 32 gives 5.80 m/s nominal; pi*D*29.97 = 5.837.
        assert!((HEAD_SPEED - 5.837).abs() < 0.01);
        assert!((UM_PER_SAMPLE - 0.1359).abs() < 0.001);
    }

    #[test]
    fn track_length_is_half_a_drum_turn() {
        let l = track_length_um() / 1000.0; // mm
        assert!((l - 96.8).abs() < 0.5, "{l}");
        // half circumference 97.4 mm
        assert!((PI * 62.0 / 2.0 - l).abs() < 1.5);
    }

    #[test]
    fn tape_advance_equals_one_and_a_half_lines_of_head_travel() {
        let adv_um = TAPE_SPEED_SP / (FRAME_RATE * 2.0) * 1e6;
        let h15_um = 1.5 * RF_PER_LINE as f64 * UM_PER_SAMPLE;
        assert!((adv_um - h15_um).abs() < 2.0, "{adv_um} vs {h15_um}");
    }
}
