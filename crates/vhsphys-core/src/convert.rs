//! Standards conversion between the 625/50 and the 525/59.94 rasters, on signal samples.
//!
//! A converter is a change of sampling lattice in space and time. Each output sample is a
//! weighted sum of source samples at their true positions: lines of the two interlaced fields
//! lie half a field-line apart and one field period apart, and are never treated as one
//! simultaneous picture. Nothing here sees an RGB frame: the input and the output are
//! luminance and colour-difference samples on the engines' baseband grids.
//!
//! Signal path for the direct Y/C reference port on both sides:
//!
//! ```text
//! source Y, U, V (IRE of the source system)
//!   -> component basis: y = (Y - setup) / span, u = U / span, v = V / span
//!   -> vertical-temporal interpolation over the source field/line lattice (`aperture`)
//!   -> horizontal resampling of each line from the source active line to the target's
//!   -> target levels: Y' = setup' + span' y, U' = span' u, V' = span' v
//!   -> regenerated target sync, blanking and burst
//! ```
//!
//! Both engines use the same luminance matrix and the same U, V scaling (0.492, 0.877), so the
//! component basis is common and only the black level and the span differ. The colour
//! subcarrier is never interpolated: the decks hand over demodulated U and V, and the target
//! deck modulates them itself.
//!
//! Geometry. Both rasters carry the same 4:3 picture in their active aperture, so the source
//! active height maps onto the target active height and the source active line onto the
//! target active line (52.66 us of the 525-line grid, 52.04 us of the 625-line grid). Sample
//! counts are never compared: positions are fractions of the aperture.
//!
//! Time. Output field times and the phase of each against the source fields are exact
//! fractions (`signal::Rational`); only the coefficients are floating point. A symmetric
//! aperture reaches two source field periods ahead, which is lookahead, not delay: the
//! converted picture carries the same programme time as its source.
//!
//! Edges, as declared for every profile here:
//!
//! * outside the active picture the signal is at black (blanking), horizontally and vertically,
//!   as it is on the wire; the first and last lines and samples are therefore softened;
//! * before the first field and after the last field of the programme the nearest field of the
//!   same parity is held, so the aperture always has its sixteen samples;
//! * an output frame exists when it starts inside the programme: at or after the first source
//!   field and before the end of the last source frame. The output count follows from that and
//!   from the target clock, not from the source frame count.
//!
//! The profiles are mathematical references. None is a model of a commercial converter and
//! their transfer characteristics are uncalibrated against hardware.

use crate::aperture::{Aperture, ResponseTable, Tap, APERTURE_FIELDS, APERTURE_LINES, TABLE_11, TABLE_12, TABLE_13, TABLE_9};
use crate::signal::{LinkTiming, Planes, Raster, RasterSpec, Rational, SignalChunk, SignalForm, CHROMA_BANDWIDTH_HZ};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::f64::consts::PI;

/// Source samples on each side of an output sample in the horizontal resampler.
const H_HALF_TAPS: usize = 16;
/// Kaiser window of the horizontal resampler: beta 6 puts its side lobes about 60 dB down.
const H_KAISER_BETA: f64 = 6.0;
/// Revision of the coefficient generation; part of the render record.
const PROFILE_REVISION: u32 = 1;

/// A named conversion algorithm. Each is a complete, published rule; none is chosen silently.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConverterProfile {
    /// Four-field aperture from BBC RD 1984/20: luminance table 9 (625 to 525) or 11 (525 to
    /// 625), colour difference table 12.
    BbcFourField,
    /// The same with colour-difference table 13, for noisy sources.
    BbcFourFieldNoisyChroma,
    /// Diagnostic baseline: each output field repeats the latest source field, with linear
    /// interpolation between its lines. It judders and is not a model of a good converter.
    FieldHold,
}

impl ConverterProfile {
    pub const ALL: [ConverterProfile; 3] = [ConverterProfile::BbcFourField, ConverterProfile::BbcFourFieldNoisyChroma, ConverterProfile::FieldHold];

    pub fn name(self) -> &'static str {
        match self {
            ConverterProfile::BbcFourField => "bbc_four_field",
            ConverterProfile::BbcFourFieldNoisyChroma => "bbc_four_field_noisy_chroma",
            ConverterProfile::FieldHold => "field_hold",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    /// The luminance and colour-difference tables for a source raster, or `None` for the
    /// profile without an aperture.
    fn tables(self, src: Raster) -> Option<(&'static ResponseTable, &'static ResponseTable)> {
        let luma = match src {
            Raster::L625 => &TABLE_9,
            Raster::L525 => &TABLE_11,
        };
        match self {
            ConverterProfile::BbcFourField => Some((luma, &TABLE_12)),
            ConverterProfile::BbcFourFieldNoisyChroma => Some((luma, &TABLE_13)),
            ConverterProfile::FieldHold => None,
        }
    }
}

/// What a converter was, for the record of a render.
#[derive(Debug, Serialize)]
pub struct ConverterRecord {
    pub profile: &'static str,
    pub revision: u32,
    pub source: &'static str,
    pub target: &'static str,
    /// Field periods, seconds, as exact fractions.
    pub source_field_period: String,
    pub target_field_period: String,
    pub luma_table: Option<&'static ResponseTable>,
    pub chroma_table: Option<&'static ResponseTable>,
    pub aperture_fields: usize,
    pub aperture_picture_lines: usize,
    pub luma_checksum: Option<String>,
    pub chroma_checksum: Option<String>,
    pub phase: &'static str,
    pub coefficients: &'static str,
    pub edges: &'static str,
    pub horizontal: String,
    pub lookahead_s: String,
    pub signal_delay_s: String,
    pub provenance: &'static str,
    pub calibrated_against_hardware: bool,
}

/// The active picture of one source frame in the component basis, row by row.
struct Picture {
    frame: i64,
    y: Vec<f32>,
    u: Vec<f32>,
    v: Vec<f32>,
}

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, mut k) = (1.0, 1.0, 1.0);
    while term > 1e-12 * sum {
        term *= (x / (2.0 * k)) * (x / (2.0 * k));
        sum += term;
        k += 1.0;
    }
    sum
}

/// Band-limited resampling of a line from `w_src` to `w_dst` samples across the same picture
/// width: a Kaiser-windowed sinc, -6 dB at `cutoff` cycles per source sample, unit gain at DC.
struct LineResampler {
    /// Source index of the first tap of each output sample.
    first: Vec<i64>,
    /// `2 * H_HALF_TAPS` weights per output sample.
    weights: Vec<f32>,
}

impl LineResampler {
    fn new(w_src: usize, w_dst: usize, cutoff: f64) -> Self {
        let n = 2 * H_HALF_TAPS;
        let mut first = Vec::with_capacity(w_dst);
        let mut weights = Vec::with_capacity(w_dst * n);
        let i0_beta = bessel_i0(H_KAISER_BETA);
        for x in 0..w_dst {
            // sample centres: the middle of output sample x lies here on the source line
            let c = (x as f64 + 0.5) * w_src as f64 / w_dst as f64 - 0.5;
            let i0 = c.floor() as i64 - H_HALF_TAPS as i64 + 1;
            let mut set: Vec<f64> = (0..n)
                .map(|k| {
                    let d = (i0 + k as i64) as f64 - c;
                    let u = d / H_HALF_TAPS as f64;
                    let window = if u.abs() < 1.0 { bessel_i0(H_KAISER_BETA * (1.0 - u * u).sqrt()) / i0_beta } else { 0.0 };
                    let a = 2.0 * cutoff * d;
                    let s = if a.abs() < 1e-12 { 1.0 } else { (PI * a).sin() / (PI * a) };
                    2.0 * cutoff * s * window
                })
                .collect();
            let sum: f64 = set.iter().sum();
            set.iter_mut().for_each(|w| *w /= sum);
            first.push(i0);
            weights.extend(set.iter().map(|w| *w as f32));
        }
        Self { first, weights }
    }

    /// Resample `src` (black beyond its ends) into `out`.
    fn apply(&self, src: &[f32], out: &mut [f32]) {
        let n = 2 * H_HALF_TAPS;
        for (x, o) in out.iter_mut().enumerate() {
            let i0 = self.first[x];
            let w = &self.weights[x * n..(x + 1) * n];
            let mut acc = 0.0f32;
            for (k, wk) in w.iter().enumerate() {
                let i = i0 + k as i64;
                if i >= 0 && (i as usize) < src.len() {
                    acc += wk * src[i as usize];
                }
            }
            *o = acc;
        }
    }
}

/// A streaming standards converter: source frames in, target frames out, with bounded memory.
pub struct StandardsConverter {
    profile: ConverterProfile,
    src: RasterSpec,
    dst: RasterSpec,
    apertures: Option<(Aperture, Aperture)>,
    luma_x: LineResampler,
    chroma_x: LineResampler,
    /// Where the horizontal filters are 6 dB down, as frequencies of the source line.
    luma_cutoff_hz: f64,
    chroma_cutoff_hz: f64,
    pictures: VecDeque<Picture>,
    /// First source frame of the programme and the next one expected.
    src_first: i64,
    src_next: i64,
    finished: bool,
    /// End of the programme, seconds, when it is earlier than the end of the last source frame.
    programme_end: Option<Rational>,
    next_out: i64,
}

impl StandardsConverter {
    /// A converter from `src` to `dst` whose first source frame is `src_start` and whose first
    /// output frame is `dst_start`, each on its own raster's clock. The first output frame
    /// must not start before the first source frame.
    pub fn new(profile: ConverterProfile, src: Raster, dst: Raster, src_start: i64, dst_start: i64) -> Result<Self, String> {
        if src == dst {
            return Err(format!("a standards converter joins two different rasters, not {} to itself", src.name()));
        }
        if dst.frame_time(dst_start) < src.frame_time(src_start) {
            return Err(format!(
                "the converter's first output frame ({} s) starts before its first source frame ({} s)",
                dst.frame_time(dst_start),
                src.frame_time(src_start)
            ));
        }
        let (s, d) = (src.spec(), dst.spec());
        // The narrower of the two systems' bandwidths, as it falls on the source line: a
        // target frequency f is f * (target line time / source line time) on the source.
        let src_rate = s.samples_per_line as f64 / s.line_period.to_f64();
        let dst_rate = d.samples_per_line as f64 / d.line_period.to_f64();
        let stretch = d.active_w as f64 / s.active_w as f64;
        let cut = |src_hz: f64, dst_hz: f64| (src_hz / src_rate).min(dst_hz / dst_rate * stretch);
        let luma_cut = cut(s.luma_bandwidth_hz, d.luma_bandwidth_hz);
        let chroma_cut = cut(CHROMA_BANDWIDTH_HZ, CHROMA_BANDWIDTH_HZ);
        Ok(Self {
            profile,
            apertures: profile.tables(src).map(|(l, c)| (Aperture::new(l), Aperture::new(c))),
            luma_x: LineResampler::new(s.active_w, d.active_w, luma_cut),
            chroma_x: LineResampler::new(s.active_w, d.active_w, chroma_cut),
            luma_cutoff_hz: luma_cut * src_rate,
            chroma_cutoff_hz: chroma_cut * src_rate,
            src: s,
            dst: d,
            pictures: VecDeque::new(),
            src_first: src_start,
            src_next: src_start,
            finished: false,
            programme_end: None,
            next_out: dst_start,
        })
    }

    pub fn profile(&self) -> ConverterProfile {
        self.profile
    }

    /// The aperture reaches half its duration ahead of an output field. The hold reaches
    /// nowhere, but an output frame is released only once a source frame that starts at or
    /// after it has arrived (see `can_make`), which is up to one source frame of waiting.
    pub fn timing(&self) -> LinkTiming {
        let lookahead = match self.apertures {
            Some(_) => self.src.field_period.mul_int(APERTURE_FIELDS as i64 / 2),
            None => self.src.field_period.mul_int(2),
        };
        LinkTiming { signal_delay: Rational::ZERO, lookahead }
    }

    pub fn record(&self) -> ConverterRecord {
        let t = self.timing();
        let (luma, chroma) = match &self.apertures {
            Some((l, c)) => (Some(l), Some(c)),
            None => (None, None),
        };
        ConverterRecord {
            profile: self.profile.name(),
            revision: PROFILE_REVISION,
            source: self.src.name,
            target: self.dst.name,
            source_field_period: self.src.field_period.to_string(),
            target_field_period: self.dst.field_period.to_string(),
            luma_table: luma.map(|a| a.table),
            chroma_table: chroma.map(|a| a.table),
            aperture_fields: if luma.is_some() { APERTURE_FIELDS } else { 1 },
            aperture_picture_lines: if luma.is_some() { APERTURE_LINES } else { 4 },
            luma_checksum: luma.map(|a| format!("{:016x}", a.checksum())),
            chroma_checksum: chroma.map(|a| format!("{:016x}", a.checksum())),
            phase: "exact rational phase of every output line and field; a source sample half an aperture early is inside, half an aperture late is outside",
            coefficients: "f64, each set of 16 scaled to unit sum; applied to f32 samples",
            edges: "black outside the active picture; first and last fields of the programme held; output frames start inside the programme",
            horizontal: format!(
                "Kaiser-windowed sinc, {} taps, beta {}, luminance -6 dB at {:.2} MHz and colour difference at {:.2} MHz of the source line; active line mapped to active line",
                2 * H_HALF_TAPS,
                H_KAISER_BETA,
                self.luma_cutoff_hz / 1e6,
                self.chroma_cutoff_hz / 1e6
            ),
            lookahead_s: t.lookahead.to_string(),
            signal_delay_s: t.signal_delay.to_string(),
            provenance: "[derived] from BBC RD 1984/20 and the engines' exact clocks; the 525-line temporal points are the report's scaled by 1000/1001",
            calibrated_against_hardware: false,
        }
    }

    /// Queue the next source frame. It must carry the direct Y/C reference form on the source
    /// raster and be the frame that follows the last one.
    pub fn push(&mut self, chunk: SignalChunk) -> Result<(), String> {
        if self.finished {
            return Err("the converter's input has ended".into());
        }
        if chunk.port.raster != self.src.raster {
            return Err(format!("the converter takes {}, the signal is {}", self.src.name, chunk.port.raster.name()));
        }
        if chunk.port.form != SignalForm::DirectYc {
            return Err(format!("the converter takes decoded components, the signal is {}", chunk.port.form.name()));
        }
        chunk.check()?;
        if chunk.frame != self.src_next {
            return Err(format!("the converter expects source frame {}, got {}", self.src_next, chunk.frame));
        }
        self.pictures.push_back(self.picture_of(&chunk));
        self.src_next += 1;
        Ok(())
    }

    /// No more source frames will arrive; the programme ends with the last one.
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// No more source frames will arrive, and the programme ended at `end` seconds. A source
    /// that is itself a conversion can run a little past the programme it was made from (its
    /// last frame starts inside it and ends outside); the end of the original programme is
    /// what bounds the output, so the count does not grow from one conversion to the next.
    pub fn finish_at(&mut self, end: Rational) {
        self.finished = true;
        self.programme_end = Some(end);
    }

    /// Every output frame that can be made now, in order.
    pub fn pull(&mut self) -> Vec<SignalChunk> {
        let mut out = Vec::new();
        while self.can_make(self.next_out) {
            let k = self.next_out;
            out.push(SignalChunk::new(self.dst.raster, k, self.make(k)));
            self.next_out += 1;
            // source frames the next output frame can no longer reach
            let (lo, _) = self.reach(self.next_out);
            while self.pictures.len() > 1 && self.pictures[0].frame < lo {
                self.pictures.pop_front();
            }
        }
        out
    }

    /// Time of field `field` (0 or 1) of output frame `k`, in source field periods.
    fn tau(&self, k: i64, field: i64) -> Rational {
        self.dst.field_period.mul_int(2 * k + field).div(self.src.field_period)
    }

    /// Source frames an output frame reads, first and last (before the hold at the edges).
    fn reach(&self, k: i64) -> (i64, i64) {
        let half = Rational::int(APERTURE_FIELDS as i64 / 2);
        match self.apertures {
            Some(_) => {
                let first = self.tau(k, 0).sub(half).ceil();
                let last = self.tau(k, 1).sub(half).ceil() + APERTURE_FIELDS as i64 - 1;
                (first.div_euclid(2), last.div_euclid(2))
            }
            None => (self.tau(k, 0).floor().div_euclid(2), self.tau(k, 1).floor().div_euclid(2)),
        }
    }

    fn can_make(&self, k: i64) -> bool {
        if self.src_next == self.src_first {
            return false;
        }
        if self.finished {
            // the frame starts before the end of the last source frame and of the programme
            let source_end = self.src.raster.frame_time(self.src_next);
            let end = self.programme_end.map_or(source_end, |e| e.min(source_end));
            return self.dst.raster.frame_time(k) < end;
        }
        // Before the input has ended the end of the programme is not known. Every source frame
        // starts inside the programme, so a source frame that starts at or after this output
        // frame shows that the output frame does too; until one has arrived the frame waits.
        self.reach(k).1 < self.src_next && self.dst.raster.frame_time(k) <= self.src.raster.frame_time(self.src_next - 1)
    }

    fn picture_of(&self, chunk: &SignalChunk) -> Picture {
        let s = &self.src;
        let (w, h) = (s.active_w, s.active_h);
        let mut p = Picture { frame: chunk.frame, y: vec![0.0; w * h], u: vec![0.0; w * h], v: vec![0.0; w * h] };
        let inv = 1.0 / s.span_ire;
        for row in 0..h {
            let line = s.field_first_line[row % 2] + row / 2;
            let base = line * s.samples_per_line + s.active_start;
            for x in 0..w {
                // where the raster carries no picture the signal is blanking: black
                if (s.carries_picture)(line, x) {
                    p.y[row * w + x] = (chunk.planes.y[base + x] - s.setup_ire) * inv;
                    p.u[row * w + x] = chunk.planes.cu[base + x] * inv;
                    p.v[row * w + x] = chunk.planes.cv[base + x] * inv;
                }
            }
        }
        p
    }

    /// The picture of source frame `frame`, holding the first and the last of the programme.
    fn picture(&self, frame: i64) -> &Picture {
        let first = self.pictures[0].frame;
        let i = (frame.clamp(self.src_first.max(first), self.src_next - 1) - first) as usize;
        &self.pictures[i]
    }

    /// The source samples and weights for an output sample at source row `y` and time `tau`.
    fn taps(&self, aperture: Option<&Aperture>, y: Rational, tau: Rational) -> Vec<Tap> {
        match aperture {
            Some(a) => a.taps(y, tau).0.to_vec(),
            None => {
                // the latest field, and the two of its lines either side of the output line
                let field = tau.floor();
                let mut r0 = y.floor();
                if (r0 - field).rem_euclid(2) != 0 {
                    r0 -= 1;
                }
                let f = (y.to_f64() - r0 as f64) / 2.0;
                vec![Tap { field, row: r0, weight: 1.0 - f }, Tap { field, row: r0 + 2, weight: f }]
            }
        }
    }

    fn make(&self, k: i64) -> Planes {
        let (s, d) = (&self.src, &self.dst);
        let (ws, wd, hs, hd) = (s.active_w, d.active_w, s.active_h as i64, d.active_h);
        let (luma, chroma) = match &self.apertures {
            Some((l, c)) => (Some(l), Some(c)),
            None => (None, None),
        };
        let rows: Vec<[Vec<f32>; 3]> = (0..hd)
            .into_par_iter()
            .map(|row| {
                let tau = self.tau(k, (row % 2) as i64);
                // the middle of output row `row`, as a row coordinate of the source picture
                let y = Rational::new((2 * row as i64 + 1) * hs, 2 * hd as i64).sub(Rational::new(1, 2));
                let mut acc = [vec![0.0f32; ws], vec![0.0f32; ws], vec![0.0f32; ws]];
                let mut add = |taps: &[Tap], planes: &[usize]| {
                    for t in taps {
                        if t.row < 0 || t.row >= hs {
                            continue;
                        }
                        let p = self.picture(t.field.div_euclid(2));
                        let (a, b) = (t.row as usize * ws, (t.row as usize + 1) * ws);
                        let w = t.weight as f32;
                        for &i in planes {
                            let src = match i {
                                0 => &p.y[a..b],
                                1 => &p.u[a..b],
                                _ => &p.v[a..b],
                            };
                            for (o, v) in acc[i].iter_mut().zip(src) {
                                *o += w * v;
                            }
                        }
                    }
                };
                add(&self.taps(luma, y, tau), &[0]);
                add(&self.taps(chroma, y, tau), &[1, 2]);
                let mut out = [vec![0.0f32; wd], vec![0.0f32; wd], vec![0.0f32; wd]];
                self.luma_x.apply(&acc[0], &mut out[0]);
                self.chroma_x.apply(&acc[1], &mut out[1]);
                self.chroma_x.apply(&acc[2], &mut out[2]);
                out
            })
            .collect();

        let mut planes = (d.blank)();
        for (row, [y, u, v]) in rows.iter().enumerate() {
            let line = d.field_first_line[row % 2] + row / 2;
            let base = line * d.samples_per_line + d.active_start;
            for x in 0..wd {
                if (d.carries_picture)(line, x) {
                    planes.y[base + x] = d.setup_ire + d.span_ire * y[x];
                    planes.cu[base + x] = d.span_ire * u[x];
                    planes.cv[base + x] = d.span_ire * v[x];
                }
            }
        }
        planes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_resampler_keeps_levels_and_position() {
        let r = LineResampler::new(740, 754, 0.3);
        let flat = vec![0.7f32; 740];
        let mut out = vec![0.0f32; 754];
        r.apply(&flat, &mut out);
        // flat inside, softened only where the aperture reaches the black beyond the line
        assert!(out[40..714].iter().all(|v| (v - 0.7).abs() < 1e-5));
        assert!(out[0] > 0.2 && out[0] < 0.7);
        // a step in the middle of the line stays in the middle of the line
        let step: Vec<f32> = (0..740).map(|x| if x < 370 { 0.0 } else { 1.0 }).collect();
        r.apply(&step, &mut out);
        let cross = out.iter().position(|v| *v >= 0.5).unwrap();
        assert!((cross as i64 - 377).abs() <= 1, "step crosses at {cross}");
    }

    #[test]
    fn the_line_resampler_passes_the_band_and_stops_above_it() {
        let cut = 0.3;
        let r = LineResampler::new(754, 740, cut);
        let gain = |f: f64| {
            let src: Vec<f32> = (0..754).map(|x| (2.0 * PI * f * x as f64).cos() as f32).collect();
            let mut out = vec![0.0f32; 740];
            r.apply(&src, &mut out);
            out[100..640].iter().fold(0.0f32, |m, v| m.max(v.abs())) as f64
        };
        assert!((gain(0.1) - 1.0).abs() < 0.01, "pass band {}", gain(0.1));
        assert!((gain(0.2) - 1.0).abs() < 0.01, "pass band {}", gain(0.2));
        assert!((gain(cut) - 0.5).abs() < 0.03, "cutoff {}", gain(cut));
        assert!(gain(0.42) < 0.005, "stop band {}", gain(0.42));
    }

    #[test]
    fn a_converter_needs_two_rasters_and_a_start_inside_the_programme() {
        assert!(StandardsConverter::new(ConverterProfile::BbcFourField, Raster::L525, Raster::L525, 0, 0).is_err());
        assert!(StandardsConverter::new(ConverterProfile::BbcFourField, Raster::L625, Raster::L525, 5, 5).is_err());
        assert!(StandardsConverter::new(ConverterProfile::BbcFourField, Raster::L625, Raster::L525, 5, 6).is_ok());
    }

    #[test]
    fn profiles_are_named_and_pick_the_table_of_their_direction() {
        for p in ConverterProfile::ALL {
            assert_eq!(ConverterProfile::from_name(p.name()), Some(p));
        }
        assert!(ConverterProfile::from_name("catmull_rom").is_none());
        let down = StandardsConverter::new(ConverterProfile::BbcFourField, Raster::L625, Raster::L525, 0, 0).unwrap().record();
        let up = StandardsConverter::new(ConverterProfile::BbcFourField, Raster::L525, Raster::L625, 0, 0).unwrap().record();
        assert_eq!(down.luma_table.unwrap().source, "BBC RD 1984/20 table 9");
        assert_eq!(up.luma_table.unwrap().source, "BBC RD 1984/20 table 11");
        assert_eq!(down.chroma_table.unwrap().source, "BBC RD 1984/20 table 12");
        assert_eq!((down.lookahead_s.as_str(), up.lookahead_s.as_str()), ("1/25", "1001/30000"));
        assert!(!down.calibrated_against_hardware);
        let hold = StandardsConverter::new(ConverterProfile::FieldHold, Raster::L625, Raster::L525, 0, 0).unwrap();
        assert_eq!(hold.timing(), LinkTiming { signal_delay: Rational::ZERO, lookahead: Rational::new(1, 25) });
    }
}
