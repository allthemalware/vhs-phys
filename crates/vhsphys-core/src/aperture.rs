//! Four-field interpolation apertures for standards conversion, synthesised from the fixed
//! frequency-response points of Clarke and Tanton, "Digital Standards Conversion:
//! Interpolation Theory and Aperture Synthesis", BBC Research Department Report 1984/20,
//! section 5 (tables 9, 11, 12 and 13).
//!
//! The report fixes the vertical-temporal response of the interpolator at a grid of points
//! spaced by the reciprocal of the aperture size, and obtains the aperture as the sum of
//! `cos t . cos y` terms weighted by those values, truncated to four field periods by eight
//! picture lines (its section 3.4.1). That is what `Aperture::value` computes:
//!
//! ```text
//! a(dy, dt) = 1/(Y T) * sum over m, n of e_m e_n H[m][n] cos(2 pi m dy / Y) cos(2 pi n dt / T)
//! Y = 8 picture lines, T = 4 field periods, e_0 = 1, e_k = 2 otherwise
//! ```
//!
//! for `-Y/2 <= dy < Y/2`, `-T/2 <= dt < T/2` and zero elsewhere. Its transform passes
//! through every fixed point exactly, and between them it is the points spread by
//! `sinc . sinc` (`Aperture::response`).
//!
//! Units. `dy` is in picture lines and `dt` in field periods of the *source* standard: the
//! aperture weights source samples, so its response points are multiples of 1/8 cycle per
//! source picture line and 1/4 cycle per source field period. For a 625/50 source those are
//! the report's 78.125 c/ph and 12.5 Hz. The report labels the other standard 525/60; this
//! project's 525-line clock is 60000/1001 fields per second, so there the temporal points
//! are multiples of 59.94/4 Hz: the report's 15 Hz scaled by 1000/1001. That scaling is this
//! project's derivation, not something the report studied. The chroma tables are given in
//! 625/50 coordinates and the report uses the same aperture, scaled, in the other direction;
//! in source units that is the same table again.
//!
//! An interlaced source has one sample per two picture lines per field, on a quincunx whose
//! reciprocal lattice lies on the grid of fixed points, at points where every table is zero.
//! So the weights that fall on the source lattice add up to the same total wherever the
//! output sample lies: a flat field converts flat, with no brightness change from one set of
//! coefficients to the next. The report quantised its coefficients to 128ths in sets of 16 and
//! had to force each set to unit sum; here a set is computed in `f64` at the exact phase of
//! each output line and field and scaled to unit sum, which removes only rounding.
//!
//! These are algorithmic reference profiles. They are not the response of any commercial
//! standards converter.

use crate::signal::Rational;
use serde::Serialize;
use std::f64::consts::PI;

/// Aperture height, picture lines of the source (four lines of each field).
pub const APERTURE_LINES: usize = 8;
/// Aperture duration, field periods of the source.
pub const APERTURE_FIELDS: usize = 4;
/// Fixed points along each axis: 0 to half the picture-line rate, 0 to the field rate.
pub const POINTS: usize = 5;
/// Source samples under the aperture: four lines in each of four fields.
pub const TAPS: usize = (APERTURE_LINES / 2) * APERTURE_FIELDS;

/// Positions per field period and per picture line at which the report's experimental
/// converter stored its aperture; `Aperture::checksum` samples the same grid.
const STORE_PER_FIELD: usize = 8;
const STORE_PER_LINE: usize = 16;
/// Resolution to which a stored value is rounded for the checksum.
const STORE_STEP: f64 = 1.0e-6;

/// Fixed response points of one aperture. `points[m][n]` is the response at vertical
/// frequency `m / 8` cycles per source picture line and temporal frequency `n / 4` cycles per
/// source field period.
#[derive(Debug, Serialize)]
pub struct ResponseTable {
    pub name: &'static str,
    /// Where the figures are printed.
    pub source: &'static str,
    pub points: [[f64; POINTS]; POINTS],
}

/// Luminance, 625/50 to 525/60, the report's extended temporal response: its general-use
/// compromise between judder and blur.
pub const TABLE_9: ResponseTable = ResponseTable {
    name: "luminance 625 to 525",
    source: "BBC RD 1984/20 table 9",
    points: [
        [1.00, 0.90, 0.32, 0.0, 0.0],
        [0.95, 0.60, 0.07, 0.0, 0.0],
        [0.50, 0.15, 0.0, 0.0, 0.0],
        [0.02, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0],
    ],
};

/// Luminance, 525/60 to 625/50, the extended temporal response of that direction.
pub const TABLE_11: ResponseTable = ResponseTable {
    name: "luminance 525 to 625",
    source: "BBC RD 1984/20 table 11",
    points: [
        [1.00, 0.75, 0.15, 0.0, 0.0],
        [0.90, 0.50, 0.02, 0.0, 0.0],
        [0.50, 0.15, 0.0, 0.0, 0.0],
        [0.10, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0],
    ],
};

/// Colour difference, both directions: a rapid temporal cut and a zero at a quarter of the
/// field rate and a quarter of the picture-line rate (Hanover bars).
pub const TABLE_12: ResponseTable = ResponseTable {
    name: "chrominance",
    source: "BBC RD 1984/20 table 12",
    points: [
        [1.00, 0.38, 0.0, 0.0, 0.0],
        [0.70, 0.20, 0.0, 0.0, 0.0],
        [0.075, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0],
    ],
};

/// Colour difference for noisy sources: more suppression of chroma noise and cross-colour,
/// less vertical colour resolution.
pub const TABLE_13: ResponseTable = ResponseTable {
    name: "chrominance, noisy source",
    source: "BBC RD 1984/20 table 13",
    points: [
        [1.00, 0.35, 0.0, 0.0, 0.0],
        [0.35, 0.07, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0],
    ],
};

/// One source sample under the aperture and its weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tap {
    /// Field index on the source timeline (its parity is the field's parity).
    pub field: i64,
    /// Display row of the source picture (a picture line); rows outside the picture are
    /// returned as they fall and are the caller's edge to handle.
    pub row: i64,
    pub weight: f64,
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

/// An interpolation aperture built from a table of fixed points.
#[derive(Clone, Copy)]
pub struct Aperture {
    pub table: &'static ResponseTable,
}

impl Aperture {
    pub fn new(table: &'static ResponseTable) -> Self {
        Self { table }
    }

    /// The aperture at a displacement of `dy` source picture lines and `dt` source field
    /// periods, per picture line per field period.
    pub fn value(&self, dy: f64, dt: f64) -> f64 {
        let (hy, ht) = (APERTURE_LINES as f64 / 2.0, APERTURE_FIELDS as f64 / 2.0);
        if !(-hy..hy).contains(&dy) || !(-ht..ht).contains(&dt) {
            return 0.0;
        }
        let mut cy = [0.0; POINTS];
        let mut ct = [0.0; POINTS];
        for k in 0..POINTS {
            let e = if k == 0 { 1.0 } else { 2.0 };
            cy[k] = e * (2.0 * PI * k as f64 * dy / APERTURE_LINES as f64).cos();
            ct[k] = e * (2.0 * PI * k as f64 * dt / APERTURE_FIELDS as f64).cos();
        }
        let mut sum = 0.0;
        for m in 0..POINTS {
            for n in 0..POINTS {
                sum += self.table.points[m][n] * cy[m] * ct[n];
            }
        }
        sum / (APERTURE_LINES * APERTURE_FIELDS) as f64
    }

    /// Frequency response of the continuous aperture at `fy` cycles per source picture line
    /// and `ft` cycles per source field period. Equal to `table.points[m][n]` at
    /// `(m / 8, n / 4)`.
    pub fn response(&self, fy: f64, ft: f64) -> f64 {
        let (uy, ut) = (fy * APERTURE_LINES as f64, ft * APERTURE_FIELDS as f64);
        // one fixed point contributes sinc around +k and around -k (the same lobe when k = 0)
        let lobe = |u: f64, k: usize| if k == 0 { sinc(u) } else { sinc(u - k as f64) + sinc(u + k as f64) };
        let mut sum = 0.0;
        for m in 0..POINTS {
            for n in 0..POINTS {
                sum += self.table.points[m][n] * lobe(uy, m) * lobe(ut, n);
            }
        }
        sum
    }

    /// The sixteen source samples that make the output sample at source row coordinate `y`
    /// (picture lines, row 0 the top line of the even field) and time `tau` (source field
    /// periods), with weights that add up to one. The second value is the sum before scaling,
    /// which the lattice argument above puts at one for every phase.
    ///
    /// Phase convention: the aperture is centred on the output sample; a source sample at
    /// exactly half the aperture before it is inside, one at exactly half after it is outside.
    pub fn taps(&self, y: Rational, tau: Rational) -> ([Tap; TAPS], f64) {
        let half_t = Rational::int(APERTURE_FIELDS as i64 / 2);
        let half_y = Rational::int(APERTURE_LINES as i64 / 2);
        let n0 = tau.sub(half_t).ceil();
        let (yf, tf) = (y.to_f64(), tau.to_f64());
        let mut taps = [Tap { field: 0, row: 0, weight: 0.0 }; TAPS];
        let mut sum = 0.0;
        let mut i = 0;
        for n in n0..n0 + APERTURE_FIELDS as i64 {
            let mut r = y.sub(half_y).ceil();
            if (r - n).rem_euclid(2) != 0 {
                r += 1;
            }
            for _ in 0..APERTURE_LINES / 2 {
                // two picture lines and one field period per source sample
                let w = 2.0 * self.value(r as f64 - yf, n as f64 - tf);
                taps[i] = Tap { field: n, row: r, weight: w };
                sum += w;
                i += 1;
                r += 2;
            }
        }
        for t in taps.iter_mut() {
            t.weight /= sum;
        }
        (taps, sum)
    }

    /// One quadrant of the aperture sampled on the grid of the report's experimental
    /// converter (8 positions per field period, 16 per picture line: 16 x 64 values), each
    /// rounded to a millionth, hashed with FNV-1a. It identifies the coefficient generation:
    /// the table, the support and the synthesis rule.
    pub fn checksum(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for j in 0..STORE_PER_LINE * APERTURE_LINES / 2 {
            for i in 0..STORE_PER_FIELD * APERTURE_FIELDS / 2 {
                let v = self.value(j as f64 / STORE_PER_LINE as f64, i as f64 / STORE_PER_FIELD as f64);
                let q = (v / STORE_STEP).round() as i32;
                for b in q.to_le_bytes() {
                    h ^= b as u64;
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLES: [&ResponseTable; 4] = [&TABLE_9, &TABLE_11, &TABLE_12, &TABLE_13];

    #[test]
    fn the_response_passes_through_every_fixed_point() {
        for table in TABLES {
            let a = Aperture::new(table);
            for m in 0..POINTS {
                for n in 0..POINTS {
                    let r = a.response(m as f64 / 8.0, n as f64 / 4.0);
                    assert!((r - table.points[m][n]).abs() < 1e-12, "{} at ({m}, {n}): {r}", table.name);
                }
            }
            // and is zero at the points beyond the table
            assert!(a.response(1.0, 0.0).abs() < 1e-12 && a.response(0.0, 2.0).abs() < 1e-12);
        }
    }

    /// The closed-form response is the transform of the aperture that `value` computes.
    #[test]
    fn the_response_is_the_transform_of_the_aperture() {
        let a = Aperture::new(&TABLE_9);
        let (ny, nt) = (400, 200);
        for &(fy, ft) in &[(0.0, 0.0), (0.07, 0.11), (0.19, 0.3), (0.3, 0.05), (0.45, 0.6)] {
            let mut sum = 0.0;
            for j in 0..ny {
                let dy = -4.0 + (j as f64 + 0.5) * 8.0 / ny as f64;
                for i in 0..nt {
                    let dt = -2.0 + (i as f64 + 0.5) * 4.0 / nt as f64;
                    sum += a.value(dy, dt) * (2.0 * PI * (fy * dy + ft * dt)).cos();
                }
            }
            let numeric = sum * (8.0 / ny as f64) * (4.0 / nt as f64);
            assert!((numeric - a.response(fy, ft)).abs() < 2e-4, "({fy}, {ft}): {numeric} vs {}", a.response(fy, ft));
        }
    }

    #[test]
    fn the_aperture_is_symmetric_in_all_four_quadrants() {
        let a = Aperture::new(&TABLE_11);
        for &(dy, dt) in &[(0.3, 0.2), (1.7, 1.1), (3.2, 0.4), (0.9, 1.9)] {
            let v = a.value(dy, dt);
            assert!((v - a.value(-dy, dt)).abs() < 1e-12 && (v - a.value(dy, -dt)).abs() < 1e-12 && (v - a.value(-dy, -dt)).abs() < 1e-12);
        }
        assert_eq!(a.value(4.0, 0.0), 0.0);
        assert_eq!(a.value(0.0, 2.0), 0.0);
    }

    /// The reciprocal lattice of interlaced scanning lies on zeros of every table, so the
    /// sixteen weights add up to one at any phase before they are scaled.
    #[test]
    fn a_set_of_taps_has_unit_sum_at_every_phase() {
        for table in TABLES {
            let a = Aperture::new(table);
            for yi in 0..40 {
                for ti in 0..40 {
                    let (y, tau) = (Rational::new(100 * 40 + yi * 7, 40), Rational::new(30 * 40 + ti * 3, 40));
                    let (taps, raw) = a.taps(y, tau);
                    assert!((raw - 1.0).abs() < 1e-9, "{}: raw sum {raw} at y {y} tau {tau}", table.name);
                    let sum: f64 = taps.iter().map(|t| t.weight).sum();
                    assert!((sum - 1.0).abs() < 1e-12);
                }
            }
        }
    }

    #[test]
    fn taps_lie_on_the_interlaced_lattice_inside_the_aperture() {
        let a = Aperture::new(&TABLE_9);
        for &(y, tau) in &[(Rational::new(203, 10), Rational::new(77, 6)), (Rational::int(20), Rational::int(11)), (Rational::new(-1, 2), Rational::new(1, 3))] {
            let (taps, _) = a.taps(y, tau);
            let mut fields: Vec<i64> = taps.iter().map(|t| t.field).collect();
            fields.dedup();
            assert_eq!(fields.len(), APERTURE_FIELDS);
            for t in taps {
                assert_eq!((t.row - t.field).rem_euclid(2), 0, "row {} is not a line of field {}", t.row, t.field);
                let (dy, dt) = (t.row as f64 - y.to_f64(), t.field as f64 - tau.to_f64());
                assert!((-4.0..4.0).contains(&dy) && (-2.0..2.0).contains(&dt), "tap at ({dy}, {dt})");
            }
        }
        // an output sample that coincides with a source sample: that sample dominates
        let (taps, _) = a.taps(Rational::int(20), Rational::int(10));
        let centre = taps.iter().find(|t| t.field == 10 && t.row == 20).unwrap();
        assert!(taps.iter().all(|t| t.weight <= centre.weight));
    }

    /// A still picture sees only the response along the vertical axis (the temporal zeros at
    /// the field rate remove the interlace component); a vertical sine through the taps comes
    /// out scaled by it.
    #[test]
    fn taps_reproduce_the_vertical_response_of_a_still_picture() {
        let a = Aperture::new(&TABLE_9);
        for m in 1..3 {
            let fy = m as f64 / 8.0;
            let (y, tau) = (Rational::new(4013, 100), Rational::new(1237, 100));
            let (taps, _) = a.taps(y, tau);
            let out: f64 = taps.iter().map(|t| t.weight * (2.0 * PI * fy * t.row as f64).cos()).sum();
            let want = TABLE_9.points[m][0] * (2.0 * PI * fy * y.to_f64()).cos();
            // the alias of the still sine about the interlace point (1/2, 1/2) is at a table zero
            assert!((out - want).abs() < 1e-9, "m {m}: {out} vs {want}");
        }
    }

    /// The coefficient generation is pinned: a change of table, support or synthesis shows here.
    #[test]
    fn the_apertures_have_their_recorded_checksums() {
        let sums: Vec<u64> = TABLES.iter().map(|t| Aperture::new(t).checksum()).collect();
        for (i, a) in sums.iter().enumerate() {
            for b in &sums[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert_eq!(sums, RECORDED_CHECKSUMS, "{sums:#x?}");
    }

    const RECORDED_CHECKSUMS: [u64; 4] = [0x05f0_ef7c_b3c7_227c, 0x1be0_9ec2_5779_359d, 0xbeb6_1812_7c32_8d49, 0x9703_0fcf_ed80_b464];
}
