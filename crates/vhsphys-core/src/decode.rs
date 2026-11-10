//! Display/capture side: recover the picture from the deck's baseband output
//! the way a TV or capture card does.
//!
//! * horizontal sync is found per line with sub-sample accuracy and followed by
//!   a first-order AFC flywheel (`afc_alpha`: 1 = lines locked to sync like a
//!   time-base corrector, 0 = free running); any timing error the AFC does not
//!   follow shows up as horizontal displacement;
//! * the back porch is clamped (DC restoration);
//! * colour-under is synchronously demodulated with the line-rotated carrier,
//!   referenced to the burst (APC phase, ACC gain, colour killer) and passed
//!   through the 1 H comb that cancels adjacent-track crosstalk;
//! * optionally, the picture then goes through a composite cable and the TV's Y/C
//!   separator (`composite`) before it is converted to RGB.

use serde::{Deserialize, Serialize};
use crate::composite::{self, CompositeParams};
use crate::consts::*;
use crate::crt;
use crate::dsp::*;
use crate::interference::{video_add_ire, video_gain};
use crate::interference_params::InterferenceParams;
use crate::record::line_rotation;
use crate::video::{decode_ideal, Baseband, Frame};
use std::f64::consts::PI;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DecodeParams {
    pub afc_alpha: f64,
    pub clamp: bool,
    pub comb: bool,
    /// Per-line smoothing of the APC phase and ACC gain estimates (1 = follow
    /// every burst measurement, small = slow loops as in a real deck).
    pub apc_alpha: f64,
    pub acc_alpha: f64,
    pub acc_max_gain: f64,
    /// Colour killer: burst magnitude below this fraction of nominal mutes colour.
    pub kill_threshold: f64,
    /// Baseband chroma smoothing after synchronous demodulation.
    pub chroma_lpf_hz: f64,
    /// Nominal demodulated burst magnitude (set by deck alignment).
    pub burst_nominal: f64,
    /// Record-side burst boost that the ACC undoes (standard: 2x = +6 dB).
    pub burst_boost: f64,
    /// How much the chroma path lags the luma path, baseband samples
    /// (measured at alignment; the deck's luma delay line cancels it).
    pub chroma_delay: f64,
}

impl Default for DecodeParams {
    fn default() -> Self {
        Self {
            afc_alpha: 0.35,
            clamp: true,
            comb: true,
            apc_alpha: 0.2,
            acc_alpha: 0.1,
            acc_max_gain: 8.0,
            kill_threshold: 0.18,
            chroma_lpf_hz: 0.8e6,
            burst_nominal: 1.0,
            burst_boost: 2.0,
            chroma_delay: 0.0,
        }
    }
}

/// Contiguous baseband data from the deck, starting at global index `bb0`.
pub struct StreamView<'a> {
    pub bb0: i64,
    pub luma: &'a [f32],
    pub cunder: &'a [f32],
}

impl StreamView<'_> {
    pub fn end(&self) -> i64 {
        self.bb0 + self.luma.len() as i64
    }
}

/// Head parity (0/1) that supplies global baseband sample `n`.
pub fn head_parity_at(n: i64) -> usize {
    let g = n * OS as i64 + SWITCH_LEAD_RF as i64;
    g.div_euclid(FIELD_RF as i64).rem_euclid(2) as usize
}

/// Sub-sample position of the sync leading edge near `n0`.
///
/// Like a TV's sync separator this is *tip referenced*: the slice level sits
/// half a nominal sync amplitude (20 IRE) above the sync tip, and a candidate is
/// only accepted if its tip really is at sync-tip level (about -40 IRE, allowing
/// +/-25 IRE of DC error and hum) and stays there for the sync width. That
/// rejects the steep fall from a bright picture into the front porch, which is
/// larger than the sync pulse itself. Candidate edges are tried strongest first.
/// Returns `None` if no plausible pulse is found.
pub fn find_sync(luma: &[f32], bb0: i64, n0: f64, search: i64) -> Option<f64> {
    let c = n0.round() as i64 - bb0;
    let lo = (c - search).max(10);
    let hi = (c + search).min(luma.len() as i64 - 60);
    let drop = |i: i64| luma[i as usize - 2] - luma[i as usize + 2];
    let mut cands: Vec<(f32, i64)> = (lo..hi)
        .filter(|&i| {
            let d = drop(i);
            d >= 14.0 && d >= drop(i - 1) && d >= drop(i + 1)
        })
        .map(|i| (drop(i), i))
        .collect();
    cands.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    for (_, i) in cands.into_iter().take(6) {
        let i = i as usize;
        let tip = luma[i + 8..i + 24].iter().sum::<f32>() / 16.0;
        if !(-65.0..=-15.0).contains(&tip) {
            continue;
        }
        let mid = tip + 0.5 * 40.0;
        if luma[i - 6] < mid + 5.0 {
            continue;
        }
        let mut j = i - 6;
        while j < i + 6 && !(luma[j] >= mid && luma[j + 1] < mid) {
            j += 1;
        }
        if !(luma[j] >= mid && luma[j + 1] < mid) {
            continue;
        }
        let low = luma[j + 1..j + 51].iter().filter(|v| **v < mid + 10.0).count();
        if low < 44 {
            continue;
        }
        let frac = ((luma[j] - mid) / (luma[j] - luma[j + 1])) as f64;
        return Some(bb0 as f64 + j as f64 + frac);
    }
    None
}

/// Demodulate the colour-under over `[start, start+len)` (global baseband
/// indices) into complex baseband `(re, im)`.
pub(crate) fn demod_chroma(view: &StreamView, start: i64, len: usize, p: &DecodeParams) -> (Vec<f32>, Vec<f32>) {
    let mut re = vec![0.0f32; len];
    let mut im = vec![0.0f32; len];
    for k in 0..len {
        let n = start + k as i64;
        let idx = n - view.bb0;
        if idx < 0 || idx >= view.cunder.len() as i64 {
            continue;
        }
        let line = n.div_euclid(BB_PER_LINE as i64);
        let col = n.rem_euclid(BB_PER_LINE as i64) as f64;
        let th = 2.0 * PI * 40.0 * col / BB_PER_LINE as f64;
        let (rc, rs) = line_rotation(head_parity_at(n), line);
        let (c, s) = (th.cos() as f32, th.sin() as f32);
        // carrier phasor e^{j(th+rho)}
        let (pc, ps) = (c * rc - s * rs, c * rs + s * rc);
        let u = 2.0 * view.cunder[idx as usize];
        re[k] = u * pc;
        im[k] = -u * ps;
    }
    for ch in [&mut re, &mut im] {
        let mut f = butter_lp(4, p.chroma_lpf_hz, FS_BB);
        f.process(ch);
    }
    (re, im)
}

/// Decode frame `k` from the deck output to RGB. With `composite.enabled` the decoded
/// luma and chroma first pass through a composite connection to the TV. `itf` is the
/// outside world: what it adds in the TV's video stages and what it does to the picture tube.
pub fn decode_frame(k: i64, view: &StreamView, p: &DecodeParams, composite: &CompositeParams, itf: &InterferenceParams) -> Frame {
    let mut bb = decode_planes(k, view, p, itf);
    if composite.enabled {
        composite::roundtrip(&mut bb, k, composite);
    }
    let mut frame = decode_ideal(&bb);
    // nothing acts on the tube without a magnet or a stray field
    if itf.crt_on() {
        crt::apply(&mut frame, k, itf);
    }
    frame
}

/// What the outside world does to one luma sample after the TV's clamp: a ground loop moves
/// the bias of the video stages (black stays, whites move) and the pulses, the hum that the
/// clamp leaves and the rings add on top. `pos` is the baseband position of the pixel; the
/// deck's RF time axis is `OS` times finer.
#[inline]
fn outside_luma(itf: &InterferenceParams, y: f32, pos: f64) -> f32 {
    let g = (pos * OS as f64).round() as i64;
    y * (1.0 + video_gain(itf, g)) + video_add_ire(itf, g)
}

/// Luma and chroma of frame `k` as the deck delivers them, placed on the nominal raster
/// (the active picture of each line, timed by the AFC); the rest of the frame is blank.
/// `itf` adds to the luma what reaches the TV's video stages (nothing while `!itf.video_on()`).
pub fn decode_planes(k: i64, view: &StreamView, p: &DecodeParams, itf: &InterferenceParams) -> Baseband {
    let outside = itf.video_on();
    let frame0 = k * FRAME_BB as i64;
    let (cs, ce) = (frame0 + 20 * BB_PER_LINE as i64, frame0 + 525 * BB_PER_LINE as i64);
    assert!(view.bb0 <= cs - 2000 && view.end() >= ce, "stream view does not cover frame {k}");
    let cstart = cs - 600;
    let clen = (ce - cs) as usize + 1200;
    let (cre, cim) = demod_chroma(view, cstart, clen, p);
    let cat = |n: f64, c: &[f32]| -> f32 { sample_cubic(c, n - cstart as f64) };

    let mut planes = Baseband::blank();
    for field in 0..2 {
        let mut s_est = 0.0f64;
        let (mut phi_s, mut g_s) = (0.0f64, 0.0f64);
        let mut chroma_lines: Vec<Vec<[f32; 2]>> = Vec::with_capacity(FIELD_ACTIVE_LINES);
        let mut luma_lines: Vec<Vec<f32>> = Vec::with_capacity(FIELD_ACTIVE_LINES);
        for j in 0..FIELD_ACTIVE_LINES {
            let line = (k * LINES as i64) + (FIELD_FIRST_LINE[field] + j) as i64;
            let n0 = (line * BB_PER_LINE as i64) as f64;
            let meas = find_sync(view.luma, view.bb0, if j == 0 { n0 } else { s_est + BB_PER_LINE as f64 }, 45);
            let pred = if j == 0 { n0 } else { s_est + BB_PER_LINE as f64 };
            s_est = match (meas, j) {
                (Some(m), 0) => m,
                (Some(m), _) => pred + p.afc_alpha * (m - pred),
                (None, _) => pred,
            };
            let s_blank_ref = meas.unwrap_or(s_est);

            // clamp on the back porch
            let mut off = 0.0f32;
            if p.clamp {
                let b0 = (s_blank_ref + 120.0).round() as i64 - view.bb0;
                if b0 >= 0 && (b0 + 14) < view.luma.len() as i64 {
                    off = view.luma[b0 as usize..b0 as usize + 14].iter().sum::<f32>() / 14.0;
                }
            }
            // burst measurement (complex mean over the burst middle)
            let (mut br, mut bi) = (0.0f64, 0.0f64);
            let b0 = (s_blank_ref + 88.0 + p.chroma_delay).round() as i64;
            for i in 0..20 {
                let idx = (b0 + i - cstart) as usize;
                br += cre[idx.min(clen - 1)] as f64;
                bi += cim[idx.min(clen - 1)] as f64;
            }
            let (br, bi) = (br / 20.0, bi / 20.0);
            let bmag = (br * br + bi * bi).sqrt();
            let phi = bi.atan2(br) - PI;
            let wrap = |a: f64| (a + PI).rem_euclid(2.0 * PI) - PI;
            if j == 0 {
                phi_s = wrap(phi);
                g_s = (p.burst_nominal / bmag.max(1e-9)).ln();
            } else {
                phi_s += p.apc_alpha * wrap(wrap(phi) - phi_s);
                let g = (p.burst_nominal / bmag.max(1e-9)).ln();
                g_s += p.acc_alpha * (g - g_s);
            }
            let gain = g_s.exp().min(p.acc_max_gain);
            let killed = bmag < p.kill_threshold * p.burst_nominal;
            let (cr, sr) = ((-phi_s).cos(), (-phi_s).sin());
            // ACC brings the burst to its nominal demodulated size; that size
            // corresponds to a recorded burst of BURST_AMP * boost, so this
            // converts chroma back to picture IRE.
            let scale = gain * (BURST_AMP_IRE as f64 * p.burst_boost) / p.burst_nominal.max(1e-12);

            let base = s_est + ACTIVE_START as f64;
            let mut yl = Vec::with_capacity(ACTIVE_W);
            let mut cl = Vec::with_capacity(ACTIVE_W);
            for x in 0..ACTIVE_W {
                let pos = base + x as f64;
                let mut y = sample_cubic(view.luma, pos - view.bb0 as f64) - off;
                if outside {
                    y = outside_luma(itf, y, pos);
                }
                yl.push(y);
                if killed {
                    cl.push([0.0, 0.0]);
                } else {
                    let (r, i) = (cat(pos + p.chroma_delay, &cre) as f64, cat(pos + p.chroma_delay, &cim) as f64);
                    // rotate by -phi, scale to the burst reference amplitude
                    let (rr, ii) = (r * cr - i * sr, r * sr + i * cr);
                    cl.push([(rr * scale) as f32, (ii * scale) as f32]);
                }
            }
            luma_lines.push(yl);
            chroma_lines.push(cl);
        }
        for j in 0..FIELD_ACTIVE_LINES {
            let base = (FIELD_FIRST_LINE[field] + j) * BB_PER_LINE + ACTIVE_START;
            for x in 0..ACTIVE_W {
                let c = if p.comb && j > 0 {
                    let (a, b) = (chroma_lines[j][x], chroma_lines[j - 1][x]);
                    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
                } else {
                    chroma_lines[j][x]
                };
                planes.y[base + x] = luma_lines[j][x];
                planes.cu[base + x] = c[0];
                planes.cv[base + x] = c[1];
            }
        }
    }
    planes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_with_sync(at: usize, level: f32) -> Vec<f32> {
        let mut v = vec![level; 600];
        for k in 0..HSYNC_W {
            v[at + k] = level - 40.0;
        }
        v
    }

    /// A bright picture tail ahead of the sync (as at the end of an active line)
    /// must not hide the pulse or move it.
    #[test]
    fn bright_picture_before_sync_does_not_matter() {
        let base = find_sync(&line_with_sync(300, 0.0), 0, 300.0, 40).unwrap();
        for lead in [60.0f32, 95.0, 120.0] {
            let mut v = line_with_sync(300, 0.0);
            for x in v[200..296].iter_mut() {
                *x = lead;
            }
            let t = find_sync(&v, 0, 300.0, 40).unwrap();
            assert!((t - base).abs() < 0.6, "lead {lead}: {t} vs {base}");
        }
    }

    #[test]
    fn sync_slicer_is_referenced_to_the_local_level() {
        // the same pulse found at the same place whether the line sits at 0 IRE
        // or is lifted/dropped by hum
        let base = find_sync(&line_with_sync(300, 0.0), 0, 300.0, 40).unwrap();
        for level in [-15.0f32, 12.0, 22.0] {
            let t = find_sync(&line_with_sync(300, level), 0, 300.0, 40).unwrap();
            assert!((t - base).abs() < 0.05, "level {level}: {t} vs {base}");
        }
        // sub-sample accuracy of the 50% point of a hard edge
        assert!((base - 299.5).abs() < 0.6, "{base}");
    }

    #[test]
    fn noise_without_a_pulse_is_not_sync() {
        let v: Vec<f32> = (0..600).map(|i| ((i * 7919) % 23) as f32 - 11.0).collect();
        assert!(find_sync(&v, 0, 300.0, 40).is_none());
    }
}
