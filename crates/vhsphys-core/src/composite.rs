//! Composite video: what a television's Y/C separator does to the deck's output.
//!
//! The deck's decoded luma and chroma are summed into one composite signal on the NTSC
//! subcarrier and separated again, the way a TV connected by a composite cable does. A
//! notch separator leaks fine luma detail into colour ("rainbow", cross-colour); a comb
//! separator leaks vertical colour edges into luma ("hanging dots", cross-luma); both
//! show the line- and frame-alternating dot pattern of the subcarrier ("dot crawl").
//! The 4 x fsc baseband grid makes the modulation trivial: the subcarrier advances 90
//! degrees per sample, 180 degrees per line (227.5 cycles) and 180 degrees per frame.
//!
//! Signal model, with `phi` the subcarrier phase of a sample (a multiple of 90 degrees):
//! `composite = Y + cu sin(phi) + cv cos(phi)`. The burst is `-BURST_AMP` on `cu`, i.e.
//! `-sin(phi)`, 180 degrees from +U as in NTSC, so the TV's burst-locked demodulator
//! recovers `cu = 2 LPF(C sin(phi))`, `cv = 2 LPF(C cos(phi))`. The TV's lock is taken as
//! perfect: the subcarrier used for demodulation is the one used for modulation.

use crate::consts::*;
use crate::dsp::{fir_centred, kaiser_lowpass};
use crate::video::Baseband;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompositeDecoder {
    /// Luma notch at fsc and chroma band-pass: cross-colour on fine luma detail.
    #[default]
    Notch,
    /// One-line comb: separates by adding/subtracting adjacent lines.
    Comb1h,
    /// Two-line (three-line adaptive-free) comb.
    Comb2h,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CompositeParams {
    /// Off (the default): S-Video style output, no composite artefacts.
    pub enabled: bool,
    pub decoder: CompositeDecoder,
}

impl Default for CompositeParams {
    fn default() -> Self {
        Self { enabled: false, decoder: CompositeDecoder::Notch }
    }
}

/// Chroma band-pass around fsc: `sin^6(pi f / (2 fsc))`, unity at fsc, zero at DC and at
/// 2 fsc, -3 dB at 2.5 and 4.7 MHz (Q about 1.7). On the 4 x fsc grid it is a short
/// symmetric FIR with taps on even offsets only, so it has no delay and no ringing tail.
/// `1 - band-pass` is the matching luma notch: the two always add up to the input.
const CHROMA_BANDPASS: [f32; 13] = [
    -1.0 / 64.0, 0.0, 6.0 / 64.0, 0.0, -15.0 / 64.0, 0.0, 20.0 / 64.0, 0.0, -15.0 / 64.0, 0.0, 6.0 / 64.0, 0.0, -1.0 / 64.0,
];

/// NTSC I axis: 123 degrees from +U; Q is at 33 degrees.
const IQ_ANGLE_DEG: f32 = 33.0;

/// Low-pass filters applied to the demodulated colour difference signals. A standard
/// NTSC decoder keeps about 1.3 MHz of I (orange-cyan, where the eye resolves colour
/// best) and 0.6 MHz of Q. Both also remove the 2 fsc product of the demodulation.
struct ChromaFilters {
    i: Vec<f32>,
    q: Vec<f32>,
}

fn chroma_filters() -> &'static ChromaFilters {
    static F: OnceLock<ChromaFilters> = OnceLock::new();
    F.get_or_init(|| ChromaFilters {
        // -6 dB at the centre of the 0.6 MHz transition band: 1.3 MHz and 0.6 MHz
        i: kaiser_lowpass(1.0e6, 0.6e6, 50.0, FS_BB),
        q: kaiser_lowpass(0.3e6, 0.6e6, 50.0, FS_BB),
    })
}

/// `(sin(phi), cos(phi))` of the subcarrier at sample `x` of frame line `line` of frame
/// `frame_index`. The phase advances 90 degrees per sample; a line (910 samples) and a
/// frame (525 lines) are both an odd number of half cycles, hence the `2 *` terms. Only
/// the parity of the frame matters: the pattern repeats every two frames.
fn subcarrier(frame_index: i64, line: usize, x: usize) -> (f32, f32) {
    const SIN: [f32; 4] = [0.0, 1.0, 0.0, -1.0];
    const COS: [f32; 4] = [1.0, 0.0, -1.0, 0.0];
    let q = (2 * frame_index.rem_euclid(2) as usize + 2 * line + x) % 4;
    (SIN[q], COS[q])
}

/// Frame lines carrying picture, field by field.
fn field_lines(field: usize) -> std::ops::Range<usize> {
    FIELD_FIRST_LINE[field]..FIELD_FIRST_LINE[field] + FIELD_ACTIVE_LINES
}

/// Encode one frame's luma/chroma (in place) to a composite signal and decode it again
/// with the chosen separator. `frame_index` is the absolute frame number: the subcarrier
/// phase pattern repeats every two frames. Only the active picture is processed; sync,
/// blanking and burst are left alone.
pub fn roundtrip(bb: &mut Baseband, frame_index: i64, p: &CompositeParams) {
    // Composite of every picture line. The whole line goes through the filters so they
    // see real blanking around the picture rather than an artificial edge; colour is
    // gated to the active picture, as the TV's demodulator ignores everything outside it
    // (the burst serves only the subcarrier lock, which is ideal here).
    let mut composite: Vec<Vec<f32>> = vec![Vec::new(); LINES];
    for field in 0..2 {
        for line in field_lines(field) {
            let base = line * BB_PER_LINE;
            composite[line] = (0..BB_PER_LINE)
                .map(|x| {
                    let n = base + x;
                    let (s, c) = subcarrier(frame_index, line, x);
                    let chroma = if is_active(x) { bb.cu[n] * s + bb.cv[n] * c } else { 0.0 };
                    bb.y[n] + chroma
                })
                .collect();
        }
    }

    for field in 0..2 {
        let lines = field_lines(field);
        for line in lines.clone() {
            let chroma = separate_chroma(&composite, line, &lines, p.decoder);
            let (cu, cv) = demodulate(&chroma, frame_index, line);
            let base = line * BB_PER_LINE;
            for x in ACTIVE_START..ACTIVE_START + ACTIVE_W {
                bb.y[base + x] = composite[line][x] - chroma[x];
                bb.cu[base + x] = cu[x];
                bb.cv[base + x] = cv[x];
            }
        }
    }
}

fn is_active(x: usize) -> bool {
    (ACTIVE_START..ACTIVE_START + ACTIVE_W).contains(&x)
}

/// The modulated chroma the separator extracts from the composite of `line`; the luma is
/// what is left over.
///
/// The subcarrier is inverted on the neighbouring lines, so a comb's line difference
/// keeps the chroma and cancels whatever luma the lines share, at every horizontal
/// frequency: fine luma detail stays luma, but colour that changes from line to line
/// leaves half its difference in the luma (hanging dots), and the luma is averaged
/// vertically (1H: with the line above; 2H: 1-2-1 over three lines). The luma difference
/// that lands in the comb's chroma is demodulated to fsc and removed by the chroma
/// low-pass. A comb needs the neighbouring lines of the same field; where one is missing
/// (the first or last picture line of a field) the separator falls back to the notch.
fn separate_chroma(composite: &[Vec<f32>], line: usize, field: &std::ops::Range<usize>, decoder: CompositeDecoder) -> Vec<f32> {
    let has = |l: usize| field.contains(&l);
    let c = &composite[line];
    match decoder {
        CompositeDecoder::Comb1h if has(line - 1) => {
            let a = &composite[line - 1];
            c.iter().zip(a).map(|(c, a)| 0.5 * (c - a)).collect()
        }
        CompositeDecoder::Comb2h if has(line - 1) && has(line + 1) => {
            let (a, b) = (&composite[line - 1], &composite[line + 1]);
            c.iter().zip(a).zip(b).map(|((c, a), b)| 0.25 * (2.0 * c - a - b)).collect()
        }
        _ => fir_centred(c, &CHROMA_BANDPASS),
    }
}

/// Synchronous demodulation of the separated chroma of `line` to `(cu, cv)` in IRE, with
/// the I and Q low-pass filters of an NTSC decoder.
fn demodulate(chroma: &[f32], frame_index: i64, line: usize) -> (Vec<f32>, Vec<f32>) {
    let (sr, cr) = IQ_ANGLE_DEG.to_radians().sin_cos();
    let mut i = Vec::with_capacity(chroma.len());
    let mut q = Vec::with_capacity(chroma.len());
    for (x, &c) in chroma.iter().enumerate() {
        let (s, co) = subcarrier(frame_index, line, x);
        let (u, v) = (2.0 * c * s, 2.0 * c * co);
        i.push(-u * sr + v * cr);
        q.push(u * cr + v * sr);
    }
    let f = chroma_filters();
    let (i, q) = (fir_centred(&i, &f.i), fir_centred(&q, &f.q));
    i.iter().zip(&q).map(|(&i, &q)| (-i * sr + q * cr, i * cr + q * sr)).unzip()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subcarrier_steps_a_quarter_cycle_per_sample_and_flips_per_line_and_frame() {
        assert_eq!(subcarrier(0, 100, 0), (0.0, 1.0));
        assert_eq!(subcarrier(0, 100, 1), (1.0, 0.0));
        assert_eq!(subcarrier(0, 101, 1), (-1.0, 0.0));
        assert_eq!(subcarrier(1, 100, 1), (-1.0, 0.0));
        assert_eq!(subcarrier(-1, 100, 1), subcarrier(1, 100, 1));
    }

    #[test]
    fn band_pass_and_notch_split_the_spectrum_at_fsc() {
        let gain = |w: f32| {
            let half = CHROMA_BANDPASS.len() as i32 / 2;
            CHROMA_BANDPASS.iter().enumerate().map(|(k, h)| h * (w * (k as i32 - half) as f32).cos()).sum::<f32>()
        };
        let pi = std::f32::consts::PI;
        assert!((gain(pi / 2.0) - 1.0).abs() < 1e-6, "unity at fsc");
        assert!(gain(0.0).abs() < 1e-6 && gain(pi).abs() < 1e-6, "nothing at DC or 2 fsc");
    }
}
