//! Helpers shared by the integration tests.
#![allow(dead_code)]

use vhsphys_core::consts::*;
use vhsphys_core::video::Frame;

/// A frame of one grey level.
pub fn flat_frame(grey: u8) -> Frame {
    let mut f = Frame::new(ACTIVE_W, ACTIVE_H);
    f.rgb.fill(grey);
    f
}

fn luma(f: &Frame, x: usize, y: usize) -> f64 {
    let i = (y * f.w + x) * 3;
    0.299 * f.rgb[i] as f64 + 0.587 * f.rgb[i + 1] as f64 + 0.114 * f.rgb[i + 2] as f64
}

/// RMS deviation of the luma from its mean over the part of the picture that is clear of
/// the head switch and the sync-edge settling (rows 40..400, columns 60..W-60).
pub fn luma_noise(f: &Frame) -> f64 {
    let (mut s, mut s2, mut n) = (0.0, 0.0, 0.0);
    for y in 40..400 {
        for x in 60..f.w - 60 {
            let v = luma(f, x, y);
            s += v;
            s2 += v * v;
            n += 1.0;
        }
    }
    let m = s / n;
    (s2 / n - m * m).max(0.0).sqrt()
}

/// Mean absolute difference of two frames, in levels.
pub fn mean_abs_error(a: &Frame, b: &Frame) -> f64 {
    a.rgb.iter().zip(b.rgb.iter()).map(|(x, y)| (*x as f64 - *y as f64).abs()).sum::<f64>() / a.rgb.len() as f64
}

/// FNV-1a over the pixels of every frame.
pub fn fnv(frames: &[Frame]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for f in frames {
        for b in &f.rgb {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// Mean colour (r, g, b) in the middle of each of the 7 bars of the test pattern.
pub fn bar_colours(f: &Frame) -> Vec<[f32; 3]> {
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
