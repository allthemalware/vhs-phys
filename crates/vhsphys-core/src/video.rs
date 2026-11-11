//! Raster of the engine's television standard (525 lines in the `ntsc` engine, 625 in the
//! `pal` one): RGB frame <-> baseband luma/chroma at 4 x fsc, including the
//! full vertical-sync structure (equalising pulses, serrated vsync, half-line
//! field offset) so that the head-switch position relative to vsync is real.
//!
//! Chroma is carried as a complex baseband value `C = cu + j*cv` in IRE, with
//! `cu` along the +U axis. The colour burst is `-BURST_AMP` on `cu` (180 deg) at 525 lines;
//! at 625 lines it is `BURST_AMP` at +135 deg (-U + V), the unswitched half of the PAL
//! swinging burst. The planes hold the unswitched (U, V); the recorder applies the PAL V
//! switch line by line, because only it knows each line's place in the sequence.

use super::consts::*;
use crate::standard::TvStandard;
use std::sync::OnceLock;

/// The RGB frame is shared by both engines; it lives in `crate::frame`.
pub use crate::frame::Frame;

/// One frame in the baseband domain (FRAME_BB samples each).
#[derive(Clone)]
pub struct Baseband {
    pub y: Vec<f32>,
    pub cu: Vec<f32>,
    pub cv: Vec<f32>,
}

impl Baseband {
    pub fn blank() -> Self {
        let (y, _) = structure();
        Self { y: y.clone(), cu: vec![0.0; FRAME_BB], cv: vec![0.0; FRAME_BB] }
    }
}

struct Structure {
    sync: Vec<f32>,
    burst_line: Vec<bool>,
    /// Half lines of the vertical interval (equalising or broad pulses), which carry no picture.
    interval_half: Vec<bool>,
}

fn structure_all() -> &'static Structure {
    static S: OnceLock<Structure> = OnceLock::new();
    S.get_or_init(build_structure)
}

fn structure() -> (&'static Vec<f32>, &'static Vec<bool>) {
    let s = structure_all();
    (&s.sync, &s.burst_line)
}

fn pulse(buf: &mut [f32], start: usize, width: usize) {
    for i in 0..width {
        let idx = start + i;
        if idx >= buf.len() {
            break;
        }
        buf[idx] = if i == 0 || i == width - 1 { SYNC_IRE / 2.0 } else { SYNC_IRE };
    }
}

/// What a half line of the frame carries in the vertical interval.
#[derive(Clone, Copy, PartialEq)]
enum Interval {
    None,
    Equalising,
    Broad,
}

fn build_structure() -> Structure {
    let mut sync = vec![BLANK_IRE; FRAME_BB];
    let mut burst_line = vec![false; LINES];
    let half_lines = 2 * LINES;
    // The interval of each field is the equalising pulses, the broad pulses, the equalising
    // pulses again; field 1 starts at half line 0, field 2 at half line LINES. An interval
    // that starts before the field's first half line wraps round to the end of the frame.
    let mut interval = vec![Interval::None; half_lines];
    for field_start in [0, LINES] {
        let first = field_start as i64 + VSYNC_FIRST_HALF_LINE;
        let broad = VSYNC_EQ_HALF_LINES..VSYNC_EQ_HALF_LINES + VSYNC_BROAD_HALF_LINES;
        for j in 0..(2 * VSYNC_EQ_HALF_LINES + VSYNC_BROAD_HALF_LINES) {
            let h = (first + j as i64).rem_euclid(half_lines as i64) as usize;
            interval[h] = if broad.contains(&j) { Interval::Broad } else { Interval::Equalising };
        }
    }
    for h in 0..half_lines {
        let s0 = h * HALF_LINE_BB;
        match interval[h] {
            Interval::Equalising => pulse(&mut sync, s0, EQ_PULSE_W),
            Interval::Broad => pulse(&mut sync, s0, HALF_LINE_BB - HSYNC_W),
            Interval::None if h % 2 == 0 => {
                pulse(&mut sync, s0, HSYNC_W);
                burst_line[h / 2] = true;
            }
            Interval::None => {}
        }
    }
    let interval_half = interval.iter().map(|i| *i != Interval::None).collect();
    Structure { sync, burst_line, interval_half }
}

#[inline]
pub fn rgb_to_ycc(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let y = 0.299 * r + 0.587 * g + 0.114 * b;
    let u = 0.492 * (b - y);
    let v = 0.877 * (r - y);
    (SETUP_IRE + VIDEO_SPAN_IRE * y, VIDEO_SPAN_IRE * u, VIDEO_SPAN_IRE * v)
}

#[inline]
pub fn ycc_to_rgb(y_ire: f32, cu: f32, cv: f32) -> [f32; 3] {
    let y = (y_ire - SETUP_IRE) / VIDEO_SPAN_IRE;
    let u = cu / VIDEO_SPAN_IRE;
    let v = cv / VIDEO_SPAN_IRE;
    let r = y + v / 0.877;
    let b = y + u / 0.492;
    let g = (y - 0.299 * r - 0.114 * b) / 0.587;
    [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)]
}

/// Frame line (0-based) and field of display row `r` (0..480).
pub fn row_to_line(row: usize) -> (usize, usize) {
    let field = row % 2;
    (FIELD_FIRST_LINE[field] + row / 2, field)
}

pub fn burst_present(line: usize) -> bool {
    structure().1[line]
}

/// Write standard color burst to baseband chroma planes on lines that carry burst.
pub fn write_standard_burst(bb: &mut Baseband) {
    for line in 0..LINES {
        if burst_present(line) {
            let s0 = line * BB_PER_LINE + BURST_START;
            for i in 0..BURST_LEN {
                let e = (i.min(BURST_LEN - 1 - i) as f32 / 4.0).min(1.0);
                let w = 0.5 - 0.5 * (std::f32::consts::PI * e).cos();
                match STANDARD {
                    TvStandard::Pal => {
                        let h = std::f32::consts::FRAC_1_SQRT_2;
                        bb.cu[s0 + i] = -BURST_AMP_IRE * h * w;
                        bb.cv[s0 + i] = BURST_AMP_IRE * h * w;
                    }
                    _ => bb.cu[s0 + i] = -BURST_AMP_IRE * w,
                }
            }
        }
    }
}

/// Encode an RGB frame (ACTIVE_W x ACTIVE_H) to baseband Y / C.
pub fn encode_frame(frame: &Frame) -> Baseband {
    assert!(frame.w == ACTIVE_W && frame.h == ACTIVE_H, "frame must be {ACTIVE_W}x{ACTIVE_H}");
    encode_from_rgb(|row, x| {
        let p = (row * ACTIVE_W + x) * 3;
        [
            frame.rgb[p] as f32 / 255.0,
            frame.rgb[p + 1] as f32 / 255.0,
            frame.rgb[p + 2] as f32 / 255.0,
        ]
    })
}

/// Encode continuous floating-point RGB gun signals (`0.0..=1.0`) directly to baseband Y / C
/// without quantising to an intermediate `u8` video frame.
pub fn encode_from_rgb(mut sample_rgb: impl FnMut(usize, usize) -> [f32; 3]) -> Baseband {
    let (sync, _) = structure();
    let mut bb = Baseband { y: sync.clone(), cu: vec![0.0; FRAME_BB], cv: vec![0.0; FRAME_BB] };
    write_standard_burst(&mut bb);

    // The last visible line of the second 625-line field (PAL line 623) is a half line: its
    // second half carries the equalising pulses, so the picture stops at mid line there.
    let interval_half = &structure_all().interval_half;
    for row in 0..ACTIVE_H {
        let (line, _) = row_to_line(row);
        let base = line * BB_PER_LINE + ACTIVE_START;
        for x in 0..ACTIVE_W {
            if interval_half[(base + x) / HALF_LINE_BB] {
                continue;
            }
            let [r, g, b] = sample_rgb(row, x);
            let (y, u, v) = rgb_to_ycc(r, g, b);
            bb.y[base + x] = y;
            bb.cu[base + x] = u;
            bb.cv[base + x] = v;
        }
    }
    bb
}

/// Perfect-timing decode used for tests and the `--bypass` reference mode.
pub fn decode_ideal(bb: &Baseband) -> Frame {
    let mut f = Frame::new(ACTIVE_W, ACTIVE_H);
    for row in 0..ACTIVE_H {
        let (line, _) = row_to_line(row);
        let base = line * BB_PER_LINE + ACTIVE_START;
        for x in 0..ACTIVE_W {
            let rgb = ycc_to_rgb(bb.y[base + x], bb.cu[base + x], bb.cv[base + x]);
            let p = (row * ACTIVE_W + x) * 3;
            for k in 0..3 {
                f.rgb[p + k] = (rgb[k] * 255.0 + 0.5) as u8;
            }
        }
    }
    f
}

/// SMPTE-style colour bars plus a grey ramp, used for tests and examples.
pub fn test_pattern() -> Frame {
    let mut f = Frame::new(ACTIVE_W, ACTIVE_H);
    let bars: [[f32; 3]; 7] = [
        [0.75, 0.75, 0.75], [0.75, 0.75, 0.0], [0.0, 0.75, 0.75], [0.0, 0.75, 0.0],
        [0.75, 0.0, 0.75], [0.75, 0.0, 0.0], [0.0, 0.0, 0.75],
    ];
    for y in 0..ACTIVE_H {
        for x in 0..ACTIVE_W {
            let c = if y < ACTIVE_H * 2 / 3 {
                bars[x * 7 / ACTIVE_W]
            } else if y < ACTIVE_H * 5 / 6 {
                let g = x as f32 / (ACTIVE_W - 1) as f32;
                [g, g, g]
            } else {
                // black / white edge blocks with fine vertical lines (resolution)
                let stripe = (x / 3) % 2 == 0;
                if x < ACTIVE_W / 2 { if stripe { [1.0; 3] } else { [0.0; 3] } } else { [(x / 60 % 2) as f32; 3] }
            };
            let p = (y * ACTIVE_W + x) * 3;
            for k in 0..3 {
                f.rgb[p + k] = (c[k] * 255.0) as u8;
            }
        }
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::standard::TvStandard;

    /// These tests pin the figures of the 525-line format; the other engine runs the same
    /// file against its own constants, so they return early there.
    const NTSC: bool = matches!(STANDARD, TvStandard::Ntsc);

    #[test]
    fn vertical_sync_structure() {
        if !NTSC {
            return;
        }
        let (sync, burst) = structure();
        // 18 half-lines of vertical interval at each field start, no burst in them
        assert!(!burst[0] && !burst[8]);
        assert!(burst[9]);
        // field 2 begins at half-line 525 => burst resumes at line 272 (h = 544)
        assert!(!burst[262 + 5] && burst[272]);
        // serrated vsync: first broad pulse low until 67 samples before the next half-line
        assert_eq!(sync[6 * HALF_LINE_BB + 100], SYNC_IRE);
        assert_eq!(sync[6 * HALF_LINE_BB + HALF_LINE_BB - 10], BLANK_IRE);
        // normal line: sync then blanking
        let l = 100 * BB_PER_LINE;
        assert_eq!(sync[l + 10], SYNC_IRE);
        assert_eq!(sync[l + 200], BLANK_IRE);
        // 262 normal-ish lines + 1 half line: sync present at the field boundary half line
        assert_eq!(sync[525 * HALF_LINE_BB + 10], SYNC_IRE);
    }

    #[test]
    fn encode_decode_round_trips() {
        let f = test_pattern();
        let bb = encode_frame(&f);
        let g = decode_ideal(&bb);
        // every pixel the raster carries; at 625 lines the right half of the last row falls on
        // the half line that holds the equalising pulses (none are left out at 525 lines)
        let carried = |i: usize| {
            let (row, x) = (i / 3 / ACTIVE_W, i / 3 % ACTIVE_W);
            let (line, _) = row_to_line(row);
            !structure_all().interval_half[(line * BB_PER_LINE + ACTIVE_START + x) / HALF_LINE_BB]
        };
        let mut worst = 0i32;
        let mut skipped = 0;
        for (i, (a, b)) in f.rgb.iter().zip(g.rgb.iter()).enumerate() {
            if !carried(i) {
                skipped += 1;
                continue;
            }
            worst = worst.max((*a as i32 - *b as i32).abs());
        }
        assert!(worst <= 2, "worst error {worst}");
        assert!(skipped < 3 * ACTIVE_W, "{skipped} values not carried: more than part of one row");
        if NTSC {
            assert_eq!(skipped, 0);
        }
    }

    #[test]
    fn peak_composite_levels_are_legal_for_75_percent_bars() {
        if !NTSC {
            return;
        }
        let f = test_pattern();
        let bb = encode_frame(&f);
        // 75% yellow: Y' = 0.665 -> luma 69 IRE; chroma amplitude 31 IRE, so the
        // composite peaks at 100 IRE exactly as in a real NTSC 75% bar signal.
        let line = FIELD_FIRST_LINE[0] + 5;
        let x = ACTIVE_W * 3 / 14; // middle of bar 2 (yellow)
        let s = line * BB_PER_LINE + ACTIVE_START + x;
        let amp = (bb.cu[s].powi(2) + bb.cv[s].powi(2)).sqrt();
        assert!((bb.y[s] - 69.0).abs() < 1.5, "{}", bb.y[s]);
        assert!((amp - 31.0).abs() < 1.5, "{amp}");
        assert!((bb.y[s] + amp - 100.0).abs() < 2.0);
    }
}

