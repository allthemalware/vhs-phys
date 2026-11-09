//! The tape as a physical object: a deterministic map of defects in tape
//! coordinates, seen by the head along each helical track.
//!
//! Tape coordinates are `u` along the tape (micrometres) and `v` across it.
//! Track `n` starts at `u_n = position + n * advance` and runs at the track
//! angle, so *lateral position maps linearly to picture line*:
//!
//! * a longitudinal crease or edge defect appears at a fixed picture height on
//!   every field it spans;
//! * a point defect moves by about 1.5 lines per field at SP (the tape advance per
//!   field equals 1.5 H of head travel), 0.75 at LP and 0.5 at EP.
//!
//! Defects are generated on demand from a spatial hash, so any tape region is
//! reproducible without storing it, and the cost per track is small.
//!
//! Effects on the head signal:
//! * `debris` / bumps lift the tape off the head: extra spacing `dd` (um),
//!   which costs `54.6 dd / lambda` dB (Wallace), i.e. a dropout whose depth
//!   follows the lift height;
//! * `voids` (missing oxide) scale the magnetisation `mag` by the fraction of
//!   the head width they cover.

use serde::{Deserialize, Serialize};
use crate::consts::*;
use crate::format::TapeSpeed;
use crate::head::{Controls, STRIDE};
use crate::rng::Rng;

const STREAM_TAPE: u64 = 31;
const STREAM_CREASE: u64 = 32;
const STREAM_ROUGH: u64 = 33;
const CELL_U: f64 = 2000.0; // um
const CELL_V: f64 = 1000.0;
const CREASE_CELL: f64 = 250_000.0; // 250 mm of tape per crease cell

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TapeParams {
    /// Position on the tape (metres from the start) of the first track.
    pub position_m: f64,
    /// Total tape length for the "bathtub" (m). T-120 at SP is ~240 m.
    pub length_m: f64,
    /// Defects (r >= r_min) per mm^2 of tape.
    pub defects_per_mm2: f64,
    /// Size distribution: pdf ~ r^-alpha on [r_min, r_max] (um).
    pub size_alpha: f64,
    pub r_min_um: f64,
    pub r_max_um: f64,
    /// Fraction of defects that are missing oxide (rest: debris/lifts).
    pub void_fraction: f64,
    /// Density multiplier at the two tape ends (hub pressure).
    pub bathtub: f64,
    /// Number of previous playbacks: loose debris is removed (up to ~10x).
    pub passes: f64,
    pub wash_passes: f64,
    /// Extra spacing at the track start/end (tension, edge curl), um, and decay (lines).
    pub edge_top_um: f64,
    pub edge_bottom_um: f64,
    pub edge_lines: f64,
    /// Random spacing roughness (rms-ish, um) and correlation length (um).
    pub rough_um: f64,
    pub rough_corr_um: f64,
    /// Longitudinal creases per metre and their lift (um) / width (um).
    pub creases_per_m: f64,
    pub crease_lift_um: f64,
    pub crease_width_um: f64,
    /// Recording speed. It sets the track angle and the tape advance per field. It is
    /// copied from `Params.speed` when the tape is built, so it is not a separate setting.
    #[serde(skip)]
    pub speed: TapeSpeed,
}

impl Default for TapeParams {
    fn default() -> Self {
        Self {
            position_m: 30.0,
            length_m: 240.0,
            defects_per_mm2: 0.0,
            size_alpha: 2.5,
            r_min_um: 3.0,
            r_max_um: 400.0,
            void_fraction: 0.35,
            bathtub: 3.0,
            passes: 0.0,
            wash_passes: 3.0,
            edge_top_um: 0.0,
            edge_bottom_um: 0.0,
            edge_lines: 4.0,
            rough_um: 0.0,
            rough_corr_um: 80.0,
            creases_per_m: 0.0,
            crease_lift_um: 0.8,
            crease_width_um: 60.0,
            speed: TapeSpeed::Sp,
        }
    }
}

struct Geo {
    sin_a: f64,
    cos_a: f64,
    tan_a: f64,
    len_um: f64,
    v_lo: f64,
    advance: f64,
}

/// Track geometry of a recording made at `speed`: a slower tape advances less between
/// two fields and leaves a slightly shallower track.
fn geo(speed: TapeSpeed) -> Geo {
    let a = speed.track_angle_deg().to_radians();
    Geo {
        sin_a: a.sin(),
        cos_a: a.cos(),
        tan_a: a.tan(),
        len_um: track_length_um(),
        v_lo: (VIDEO_CENTRE_MM - VIDEO_AREA_MM / 2.0) * 1000.0,
        advance: speed.advance_um(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Void,
    Debris,
}

/// A point defect in tape coordinates (um).
#[derive(Clone, Copy, Debug)]
pub struct Defect {
    pub u: f64,
    pub v: f64,
    pub r: f64,
    pub kind: Kind,
    /// Lift height (debris) or demagnetised fraction (void).
    pub strength: f64,
}

pub struct Tape {
    pub p: TapeParams,
    seed: u64,
}

impl Tape {
    pub fn new(p: TapeParams, seed: u64) -> Self {
        Self { p, seed }
    }

    fn bathtub(&self, u_um: f64) -> f64 {
        let l = self.p.length_m * 1e6;
        let e = 10.0e6; // 10 m scale at each end
        1.0 + (self.p.bathtub - 1.0) * ((-u_um / e).exp() + (-(l - u_um) / e).exp())
    }

    /// All point defects whose cell intersects track `n`'s neighbourhood.
    pub fn defects_near_track(&self, n: i64) -> Vec<Defect> {
        let g = geo(self.p.speed);
        let u_n = self.p.position_m * 1e6 + n as f64 * g.advance;
        let (iu0, iu1) = (
            ((u_n / CELL_U).floor() as i64) - 1,
            (((u_n + g.len_um * g.cos_a) / CELL_U).floor() as i64) + 1,
        );
        let margin = self.p.r_max_um + 100.0;
        let mut out = Vec::new();
        for iu in iu0..=iu1 {
            let vmin = g.v_lo + (iu as f64 * CELL_U - u_n) * g.tan_a - margin;
            let vmax = g.v_lo + ((iu + 1) as f64 * CELL_U - u_n) * g.tan_a + margin;
            let (iv0, iv1) = ((vmin / CELL_V).floor() as i64, (vmax / CELL_V).floor() as i64);
            for iv in iv0..=iv1 {
                if iv < 0 || (iv as f64) * CELL_V > TAPE_WIDTH_MM * 1000.0 {
                    continue;
                }
                let mut rng = Rng::new(self.seed, STREAM_TAPE, iu.wrapping_mul(100_003).wrapping_add(iv));
                let lam = self.p.defects_per_mm2 * CELL_U * CELL_V / 1.0e6 * self.bathtub((iu as f64 + 0.5) * CELL_U);
                for _ in 0..rng.poisson(lam) {
                    let u = (iu as f64 + rng.uniform()) * CELL_U;
                    let v = (iv as f64 + rng.uniform()) * CELL_V;
                    let r = rng.power_law(self.p.size_alpha, self.p.r_min_um, self.p.r_max_um);
                    let is_void = rng.uniform() < self.p.void_fraction;
                    let stick = rng.uniform();
                    let ln = rng.gauss() as f64;
                    let kind = if is_void { Kind::Void } else { Kind::Debris };
                    // loose debris is cleaned off by repeated playback; voids stay
                    if kind == Kind::Debris && stick > (-self.p.passes / self.p.wash_passes).exp() * 0.9 + 0.1 {
                        continue;
                    }
                    let strength = match kind {
                        Kind::Debris => (0.25 * (r / 20.0).powf(0.6) * (0.5 * ln).exp()).clamp(0.03, 3.0),
                        Kind::Void => (0.8 + 0.2 * stick).min(1.0),
                    };
                    out.push(Defect { u, v, r, kind, strength });
                }
            }
        }
        out
    }

    /// Paint spacing (`dd`) and magnetisation (`mag`) for track `n` over the
    /// window starting at global RF index `g0`.
    pub fn paint(&self, n: i64, g0: i64, head_width_um: f64, ctl: &mut Controls) {
        let g = geo(self.p.speed);
        let u_n = self.p.position_m * 1e6 + n as f64 * g.advance;
        let g_in = switch_time(n);
        let g_out = switch_time(n + 1);
        let um_per_g = g.len_um / FIELD_RF as f64; // along-track um per RF sample
        let m = ctl.dd.len();
        let idx_of_s = |s: f64| -> f64 { ((g_in as f64 + s / um_per_g) - g0 as f64) / STRIDE as f64 };

        // ---- roughness ----
        if self.p.rough_um > 0.0 {
            let mut rng = Rng::new(self.seed, STREAM_ROUGH, n);
            let a = (-(STRIDE as f64 * UM_PER_SAMPLE) / self.p.rough_corr_um).exp() as f32;
            let b = (1.0 - a * a).sqrt();
            let mut x = rng.gauss();
            for i in 0..m {
                x = a * x + b * rng.gauss();
                ctl.dd[i] += self.p.rough_um as f32 * x.abs();
            }
        }
        // ---- edge / tension profile ----
        if self.p.edge_top_um > 0.0 || self.p.edge_bottom_um > 0.0 {
            let tau = self.p.edge_lines * RF_PER_LINE as f64;
            for i in 0..m {
                let gi = g0 + (i * STRIDE) as i64;
                let top = self.p.edge_top_um * (-((gi - g_in).max(0) as f64) / tau).exp();
                let bot = self.p.edge_bottom_um * (-((g_out - gi).max(0) as f64) / tau).exp();
                ctl.dd[i] += (top + bot) as f32;
            }
        }
        // ---- point defects ----
        if self.p.defects_per_mm2 > 0.0 {
            for d in self.defects_near_track(n) {
                let (du, dv) = (d.u - u_n, d.v - g.v_lo);
                let s_d = du * g.cos_a + dv * g.sin_a;
                let rho = -du * g.sin_a + dv * g.cos_a;
                if s_d < -d.r || s_d > g.len_um + d.r || rho.abs() > d.r + head_width_um / 2.0 {
                    continue;
                }
                let (i0, i1) = (idx_of_s(s_d - d.r).floor().max(0.0) as usize, (idx_of_s(s_d + d.r).ceil() as usize).min(m - 1));
                for i in i0..=i1 {
                    let s = ((g0 + (i * STRIDE) as i64 - g_in) as f64) * um_per_g;
                    let ds = s - s_d;
                    match d.kind {
                        Kind::Debris => {
                            let x = ds / d.r;
                            if x.abs() < 1.0 {
                                let lat = 1.0 - (rho.abs() / (d.r + head_width_um / 2.0)).powi(2);
                                ctl.dd[i] += (d.strength * lat.max(0.0) * (1.0 - x * x).powi(2)) as f32;
                            }
                        }
                        Kind::Void => {
                            let hc = (d.r * d.r - ds * ds).max(0.0).sqrt();
                            let lo = (rho - hc).max(-head_width_um / 2.0);
                            let hi = (rho + hc).min(head_width_um / 2.0);
                            let cover = ((hi - lo).max(0.0) / head_width_um).min(1.0);
                            ctl.mag[i] *= (1.0 - d.strength * cover) as f32;
                        }
                    }
                }
            }
        }
        // ---- creases (longitudinal, fixed picture height) ----
        if self.p.creases_per_m > 0.0 {
            let (c0, c1) = (
                (u_n / CREASE_CELL).floor() as i64 - 1,
                ((u_n + g.len_um * g.cos_a) / CREASE_CELL).floor() as i64 + 1,
            );
            for c in c0..=c1 {
                let mut rng = Rng::new(self.seed, STREAM_CREASE, c);
                for _ in 0..rng.poisson(self.p.creases_per_m * CREASE_CELL / 1.0e6) {
                    let u0 = (c as f64 + rng.uniform()) * CREASE_CELL;
                    let len = rng.range(10_000.0, 120_000.0);
                    let v_c = g.v_lo + rng.uniform() * VIDEO_AREA_MM * 1000.0;
                    let slope = rng.gauss() as f64 * 0.002;
                    let width = self.p.crease_width_um * (0.4 * rng.gauss() as f64).exp();
                    let lift = self.p.crease_lift_um * (0.4 * rng.gauss() as f64).exp();
                    // crossing of the track with the crease line
                    let s_c = (v_c - g.v_lo + slope * (u_n - u0)) / (g.sin_a - slope * g.cos_a);
                    let u_at = u_n + s_c * g.cos_a;
                    if s_c < 0.0 || s_c > g.len_um || u_at < u0 || u_at > u0 + len {
                        continue;
                    }
                    let sigma = width / g.sin_a / 2.355;
                    let (i0, i1) = (
                        idx_of_s(s_c - 4.0 * sigma).floor().max(0.0) as usize,
                        (idx_of_s(s_c + 4.0 * sigma).ceil() as usize).min(m - 1),
                    );
                    for i in i0..=i1 {
                        let s = ((g0 + (i * STRIDE) as i64 - g_in) as f64) * um_per_g;
                        let x = (s - s_c) / sigma;
                        // core of the crease plus a shallow halo (lost contact before and after)
                        ctl.dd[i] += (lift * (-0.5 * x * x).exp() + 0.25 * lift * (-0.5 * (x / 3.0).powi(2)).exp()) as f32;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tape(p: TapeParams) -> Tape {
        Tape::new(p, 7)
    }

    fn picture_line_of_peak(t: &Tape, n: i64) -> Option<f64> {
        let len = FIELD_RF + 2 * TRACK_MARGIN_RF;
        let g0 = switch_time(n) - TRACK_MARGIN_RF as i64;
        let mut ctl = Controls::new(len);
        t.paint(n, g0, 58.0, &mut ctl);
        let (i, v) = ctl.dd.iter().enumerate().fold((0, 0.0f32), |a, (i, v)| if *v > a.1 { (i, *v) } else { a });
        if v < 0.2 {
            return None;
        }
        Some(((g0 + (i * STRIDE) as i64 - switch_time(n)) as f64) / RF_PER_LINE as f64) // lines from head-in
    }

    #[test]
    fn crease_sits_at_a_fixed_picture_height_on_every_field() {
        let t = tape(TapeParams { creases_per_m: 40.0, crease_lift_um: 1.0, ..Default::default() });
        let mut lines = vec![];
        for n in 0..400 {
            if let Some(l) = picture_line_of_peak(&t, n) {
                lines.push((n, l));
            }
        }
        assert!(lines.len() > 20, "creases should intersect many tracks: {}", lines.len());
        // find a run of consecutive tracks hitting the same crease: its line number drifts only slowly
        let mut best = 0;
        for w in lines.windows(2) {
            if w[1].0 == w[0].0 + 1 && (w[1].1 - w[0].1).abs() < 3.0 {
                best += 1;
            }
        }
        assert!(best >= 10, "expected persistent streaks, got {best}");
    }

    #[test]
    fn point_defect_slides_by_one_and_a_half_lines_per_field() {
        // place exactly one big debris particle by brute force: search seeds
        let p = TapeParams { defects_per_mm2: 0.02, void_fraction: 0.0, r_min_um: 300.0, r_max_um: 400.0, size_alpha: 1.0, ..Default::default() };
        let t = tape(p);
        let mut prev: Option<(i64, f64)> = None;
        let mut slopes = vec![];
        for n in 0..3000 {
            if let Some(l) = picture_line_of_peak(&t, n) {
                if let Some((pn, pl)) = prev {
                    if pn + 1 == n {
                        slopes.push(l - pl);
                    }
                }
                prev = Some((n, l));
            }
        }
        assert!(slopes.len() > 5);
        // a defect radius of 300+ um spans several tracks; adjacent-track shift of its peak ~ -1.5 H
        // (negative: later tracks meet the defect earlier along the track)
        let med = {
            let mut s = slopes.clone();
            s.sort_by(|a, b| a.partial_cmp(b).unwrap());
            s[s.len() / 2]
        };
        assert!((med.abs() - 1.5).abs() < 0.6, "median shift {med} lines per field");
    }

    #[test]
    fn dropout_rate_is_in_a_sane_range_and_cleans_with_passes() {
        let mut p = TapeParams { defects_per_mm2: 0.02, ..Default::default() };
        let count = |p: &TapeParams| -> usize {
            let t = tape(p.clone());
            let mut c = 0;
            for n in (0..2400).step_by(1) {
                let len = FIELD_RF + 2 * TRACK_MARGIN_RF;
                let g0 = switch_time(n) - TRACK_MARGIN_RF as i64;
                let mut ctl = Controls::new(len);
                t.paint(n, g0, 58.0, &mut ctl);
                // count dropout events deeper than 16 dB at 1.4 um wavelength: dd >= 0.41 um, or mag < 0.16
                let mut inside = false;
                for i in 0..ctl.dd.len() {
                    let deep = ctl.dd[i] > 0.41 || ctl.mag[i] < 0.16;
                    if deep && !inside {
                        c += 1;
                    }
                    inside = deep;
                }
            }
            c
        };
        let c0 = count(&p);
        // 2400 fields = 40 s: scale to per minute
        let per_min = c0 as f64 * 1.5;
        assert!(per_min > 3.0 && per_min < 2000.0, "{per_min} deep dropouts / min");
        p.passes = 20.0;
        let c1 = count(&p);
        assert!(c1 < c0, "repeated playback must reduce dropouts: {c0} -> {c1}");
    }
}
