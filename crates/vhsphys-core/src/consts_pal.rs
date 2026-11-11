//! Timing and format constants for PAL VHS (625 lines, 50 fields/s, SP and LP).
//!
//! The same names as `consts.rs` (the 525-line NTSC set), so one engine source compiles
//! against either. Sources (details in `docs/RESEARCH.md`): ITU-R BT.470 / BT.1700 (625-line
//! timing and levels), vhs-decode `format_defs/vhs.py` (PAL VHS FM, colour-under, rotation),
//! the 625/50 VHS track figures (IEC 60774-1 via the service literature), and the derivations
//! noted per item. `[est]` marks a figure carried over from the 525-line format without a
//! 625-line source.

use crate::standard::TvStandard;
use std::f64::consts::PI;

/// The standard this constant set describes.
pub const STANDARD: TvStandard = TvStandard::Pal;

// ---------------------------------------------------------------- video timing
/// PAL colour subcarrier, 283.75 fH + 25 Hz. Not line locked: it is used only where a real
/// 4.43 MHz carrier exists (the composite stage); the baseband chroma is a complex envelope.
pub const FSC: f64 = 4.433_618_75e6;
/// Horizontal line rate, exactly 15 625 Hz.
pub const FH: f64 = 15_625.0;
pub const LINES: usize = 625;
/// 25 frames (50 fields) per second.
pub const FRAME_RATE: f64 = FH / LINES as f64;

/// RF-domain samples per line: the same 2730 as the 525-line grid, so the grid stays line
/// locked (42.656 MS/s) and the oversampling to baseband stays 3.
pub const RF_PER_LINE: usize = 2730;
/// Baseband samples per line (14.22 MS/s, 70.3 ns each).
pub const BB_PER_LINE: usize = 910;
pub const OS: usize = RF_PER_LINE / BB_PER_LINE;
pub const FS_RF: f64 = RF_PER_LINE as f64 * FH;
pub const FS_BB: f64 = BB_PER_LINE as f64 * FH;

pub const FIELD_RF: usize = (LINES / 2) * RF_PER_LINE + RF_PER_LINE / 2;
pub const FRAME_RF: usize = LINES * RF_PER_LINE;
pub const FIELD_BB: usize = FIELD_RF / OS;
pub const FRAME_BB: usize = LINES * BB_PER_LINE;
pub const HALF_LINE_BB: usize = BB_PER_LINE / 2;

// Levels in "IRE" units, 100 = 0.7 V of picture. Sync is 0.3 V, so -300/7.
pub const SYNC_IRE: f32 = -300.0 / 7.0;
pub const BLANK_IRE: f32 = 0.0;
/// No setup pedestal in 625-line systems.
pub const SETUP_IRE: f32 = 0.0;
pub const WHITE_IRE: f32 = 100.0;
pub const VIDEO_SPAN_IRE: f32 = WHITE_IRE - SETUP_IRE;
/// Burst 0.3 V peak to peak: 0.15 V peak = 150/7 units.
pub const BURST_AMP_IRE: f32 = 150.0 / 7.0;

// Baseband line layout (samples at 14.21875 MS/s, 70.33 ns each).
pub const HSYNC_W: usize = 67; // 4.7 us
pub const EQ_PULSE_W: usize = 33; // 2.35 us
pub const BURST_START: usize = 80; // 5.6 us after the sync leading edge
pub const BURST_LEN: usize = 32; // 10 cycles of 4.43 MHz (2.25 us)
pub const ACTIVE_START: usize = 149; // 10.5 us
pub const ACTIVE_W: usize = 740; // 52.0 us (even, for 4:2:0 encoders)
pub const ACTIVE_H: usize = 576;
pub const FIELD_ACTIVE_LINES: usize = 288;
/// First visible line (0-based frame line; frame line 0 = PAL line 1) of each field:
/// field 1 lines 23..310, field 2 lines 336..623.
pub const FIELD_FIRST_LINE: [usize; 2] = [22, 335];

/// Vertical interval per field, in half lines counted from the field's start (frame half-line
/// 0 for field 1, LINES for field 2): pre-equalising pulses start this many half lines
/// before it (PAL line 1 opens with the broad pulses; NTSC line 1 with the equalising ones).
pub const VSYNC_FIRST_HALF_LINE: i64 = -5;
/// Equalising pulses before and after the broad pulses, half lines each side.
pub const VSYNC_EQ_HALF_LINES: usize = 5;
/// Broad (field sync) pulses, half lines.
pub const VSYNC_BROAD_HALF_LINES: usize = 5;

// ------------------------------------------------------------------ VHS format
pub const DRUM_DIAMETER_M: f64 = 0.062;
/// Head-to-tape speed: pi * D * 25 rev/s = 4.87 m/s.
pub const HEAD_SPEED: f64 = PI * DRUM_DIAMETER_M * FRAME_RATE;
/// Tape distance travelled by the head per RF sample, micrometres (0.1141).
pub const UM_PER_SAMPLE: f64 = HEAD_SPEED / FS_RF * 1.0e6;
/// 23.39 mm/s.
pub const TAPE_SPEED_SP: f64 = 0.023_39;
pub const TRACK_PITCH_UM: f64 = 49.0;
/// 5 deg 57' 50.3": the stationary 5 deg 56' 7.4" opened by the tape motion at 23.39 mm/s
/// against 4.87 m/s (the same law as `format::TapeSpeed::track_angle_deg`).
pub const TRACK_ANGLE_DEG: f64 = 5.0 + 57.0 / 60.0 + 50.3 / 3600.0;
pub const VIDEO_AREA_MM: f64 = 10.07;
pub const VIDEO_CENTRE_MM: f64 = 6.2;
pub const TAPE_WIDTH_MM: f64 = 12.65;
pub const AZIMUTH_DEG: f64 = 6.0;
/// The 625-line VHS format defines SP and LP only.
pub const HAS_EP: bool = false;
/// SMPTE ST 32's slow-speed luminance rules (table 4 sub-emphasis, the fH/2 carrier
/// interleave, the undoubled LP burst) are 525-line rules. No 625-line source: off.
pub const ST32_SLOW_SPEED_RULES: bool = false;

/// Luma FM: sync tip 3.8 MHz, white 4.8 MHz, 1 MHz per volt (sync tip to white = 1 V).
pub const F_SYNC_TIP: f64 = 3.8e6;
pub const HZ_PER_IRE: f64 = 1.0e6 / (WHITE_IRE - SYNC_IRE) as f64;
/// vhs-decode uses 1.3 us for PAL VHS as for NTSC.
pub const PRE_EMPH_TAU: f64 = 1.3e-6;
pub const PRE_EMPH_RATIO: f64 = 5.0; // [est] as the 525-line format
/// Clip levels at the same fractions of the sync-to-white span as the 525-line format [est].
pub const WHITE_CLIP_IRE: f32 = SYNC_IRE + 1.6 * (WHITE_IRE - SYNC_IRE);
pub const DARK_CLIP_IRE: f32 = SYNC_IRE - 0.4 * (WHITE_IRE - SYNC_IRE);
/// Colour-under carrier, (40 + 1/8) x fH = 626.953 kHz.
pub const CU_CYCLES_PER_LINE: f64 = 40.125;
pub const F_CU: f64 = CU_CYCLES_PER_LINE * FH;
/// Colour-under phase rotation per line, in quarter turns, for head parity 0 and 1:
/// channel 1 unrotated, channel 2 retarded 90 deg per line (vhs-decode PAL_ROTATION [0, -1]).
pub const CU_ROTATION_QUARTERS: [i64; 2] = [0, -1];

/// Mains frequency the equipment is expected to meet in a 625-line country.
pub const MAINS_HZ_DEFAULT: f64 = 50.0;

/// Along-track offset of adjacent tracks: the tape advance per field (467.8 um) equals
/// 1.5 H of head travel (467.5 um), as in the 525-line format.
pub const ADJACENT_TRACK_OFFSET_RF: usize = RF_PER_LINE * 3 / 2;

/// Head switch lead before the field's first equalising pulse, lines: 6.5 H before the
/// vertical interval as in the 525-line engine, whose field starts with the equalising
/// pulses; here they start 2.5 H before the field.
pub const SWITCH_LEAD_LINES: f64 = 6.5 + 2.5;
pub const SWITCH_LEAD_RF: usize = (SWITCH_LEAD_LINES * RF_PER_LINE as f64) as usize; // 24 570
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
        assert!((FS_RF - 42_656_250.0).abs() < 1e-6);
        assert!((FRAME_RATE - 25.0).abs() < 1e-12);
    }

    #[test]
    fn levels_are_one_volt_sync_to_white() {
        assert!((HZ_PER_IRE * (WHITE_IRE - SYNC_IRE) as f64 - 1.0e6).abs() < 1e-3);
        assert!((F_SYNC_TIP + HZ_PER_IRE * (WHITE_IRE - SYNC_IRE) as f64 - 4.8e6).abs() < 1e-3);
        assert!((F_CU - 626_953.125).abs() < 1e-6);
    }

    #[test]
    fn head_speed_and_track_geometry() {
        assert!((HEAD_SPEED - 4.869).abs() < 0.001);
        let l = track_length_um() / 1000.0;
        assert!((PI * 62.0 / 2.0 - l).abs() < 1.5, "{l}");
        let adv_um = TAPE_SPEED_SP / (FRAME_RATE * 2.0) * 1e6;
        let h15_um = 1.5 * RF_PER_LINE as f64 * UM_PER_SAMPLE;
        assert!((adv_um - h15_um).abs() < 2.0, "{adv_um} vs {h15_um}");
    }

    #[test]
    fn active_picture_fits_the_line_and_the_fields() {
        assert!(ACTIVE_START + ACTIVE_W < BB_PER_LINE);
        assert_eq!(2 * FIELD_ACTIVE_LINES, ACTIVE_H);
        assert!(FIELD_FIRST_LINE[0] + FIELD_ACTIVE_LINES <= LINES / 2);
        assert!(FIELD_FIRST_LINE[1] + FIELD_ACTIVE_LINES <= LINES);
    }
}
