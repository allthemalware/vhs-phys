//! Streaming simulation session: frames in, VHS-degraded frames out.
//!
//! Work is organised per *field*, because a field is exactly one helical track:
//!
//! 1. `record_track`        (parallel)  video -> magnetisation reversals on track n
//! 2. `render_rf`           (parallel)  playback head output of track n (+ neighbours)
//! 3. `segment`             (parallel)  head switch + common deck electronics
//! 4. `decode_frame`        (parallel)  sync/AFC/colour decoding
//!
//! Intermediate results are cached per field and evicted when no longer needed.

use serde::{Deserialize, Serialize};
use crate::audio::AudioParams;
use crate::composite::CompositeParams;
use crate::consts::*;
use crate::decode::{decode_frame, demod_chroma, DecodeParams, StreamView};
use crate::electronics::{process_segment, DeckParams};
use crate::format::TapeSpeed;
use crate::head::*;
use crate::mech::{Mech, MechParams};
use crate::record::*;
use crate::rng::Rng;
use crate::tape::{Tape, TapeParams};
use crate::tracking::{lateral_weights, TrackingParams, NEIGHBOURS};
use crate::video::{encode_frame, Baseband, Frame};
use rayon::prelude::*;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

const STREAM_HEAD: u64 = 1;

/// Everything that defines a deck, a tape and how worn they are.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    pub seed: u64,
    pub record: RecordParams,
    pub heads: [HeadParams; 2],
    pub deck: DeckParams,
    pub decode: DecodeParams,
    pub tape: TapeParams,
    pub mech: MechParams,
    pub tracking: TrackingParams,
    /// Tape speed: sets the track pitch, the tape advance and the audio bandwidth.
    pub speed: TapeSpeed,
    /// Composite video stage after the deck's decode (off by default).
    pub composite: CompositeParams,
    /// The audio path of the same deck and tape.
    pub audio: AudioParams,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            seed: 1,
            record: RecordParams::default(),
            heads: [HeadParams::default(), HeadParams::default()],
            deck: DeckParams::default(),
            decode: DecodeParams::default(),
            tape: TapeParams::default(),
            mech: MechParams::default(),
            tracking: TrackingParams::default(),
            speed: TapeSpeed::default(),
            composite: CompositeParams::default(),
            audio: AudioParams::default(),
        }
    }
}

/// Parameters plus the objects built from them.
struct Ctx {
    p: Params,
    mech: Mech,
    tape: Tape,
}

impl Ctx {
    fn new(p: Params) -> Self {
        let (mech, tape) = (Mech::new(p.mech.clone(), p.seed), Tape::new(p.tape.clone(), p.seed));
        Self { p, mech, tape }
    }
}

/// Deck alignment measured against a reference signal (as a technician would
/// with a test tape): fixes the playback level and the burst reference.
#[derive(Clone, Copy, Debug)]
pub struct Calibration {
    pub fm_gain: f64,
    pub burst_nominal: f64,
    /// Chroma-path delay relative to luma, baseband samples.
    pub chroma_delay: f64,
}

struct WindowSource {
    first: i64,
    frames: Vec<Arc<Baseband>>,
    blank: Baseband,
}

impl VideoSource for WindowSource {
    fn fill(&self, bb_start: i64, y: &mut [f32], cu: &mut [f32], cv: &mut [f32]) {
        let n = y.len();
        let mut i = 0;
        while i < n {
            let g = bb_start + i as i64;
            let fr = g.div_euclid(FRAME_BB as i64);
            let off = g.rem_euclid(FRAME_BB as i64) as usize;
            let take = (FRAME_BB - off).min(n - i);
            let rel = fr - self.first;
            let src: &Baseband = if rel >= 0 && (rel as usize) < self.frames.len() { &self.frames[rel as usize] } else { &self.blank };
            y[i..i + take].copy_from_slice(&src.y[off..off + take]);
            cu[i..i + take].copy_from_slice(&src.cu[off..off + take]);
            cv[i..i + take].copy_from_slice(&src.cv[off..off + take]);
            i += take;
        }
    }
}

/// Window (global RF start, length) of the track recorded/played by head `n`.
fn track_window(n: i64) -> (i64, usize) {
    (switch_time(n) - TRACK_MARGIN_RF as i64, FIELD_RF + 2 * TRACK_MARGIN_RF)
}

/// Render the playback-head RF of track `n`: the track itself plus the
/// neighbouring tracks the head partly overlaps (azimuth crosstalk), with
/// timebase warp, head-to-tape spacing and oxide defects applied, then the
/// head amplifier noise and resonance.
fn render_rf(cx: &Ctx, trans: &HashMap<i64, Arc<Vec<Transition>>>, n: i64) -> Vec<f32> {
    let p = &cx.p;
    let (g0, len) = track_window(n);
    let hp = &p.heads[n.rem_euclid(2) as usize];
    let mut ctl = Controls::new(len);
    cx.mech.fill_warp(n, g0, &mut ctl);
    cx.tape.paint(n, g0, hp.width_um, &mut ctl);
    if hp.extra_spacing_um != 0.0 {
        for v in ctl.dd.iter_mut() {
            *v += hp.extra_spacing_um as f32;
        }
    }
    let w = lateral_weights(&p.tracking, hp.width_um, p.seed, n, len, g0);
    let h0 = hp.h0_samples();
    let mut buf = vec![0.0f32; len];
    render_pulses(&trans[&n], 0.0, &ctl, Some(&w[2]), h0, &mut buf);
    if hp.azimuth_error_deg > 0.0 {
        let mean_w = w[2].iter().sum::<f32>() as f64 / w[2].len() as f64;
        azimuth_smooth(&mut buf, hp.width_um * mean_w, hp.azimuth_error_deg);
    }
    // neighbours: opposite azimuth (n +/- 1) is smoothed by the 12 degree sinc
    // law, same azimuth (n +/- 2) is read at full strength (ghost field)
    let mut nb_ctl: Option<Controls> = None;
    for (k, &j) in NEIGHBOURS.iter().enumerate() {
        if j == 0 {
            continue;
        }
        let wmax = w[k].iter().cloned().fold(0.0f32, f32::max);
        let Some(tr) = trans.get(&(n + j)) else { continue };
        if wmax < 0.003 {
            continue;
        }
        let nctl = nb_ctl.get_or_insert_with(|| {
            let mut c = ctl.clone();
            c.mag.iter_mut().for_each(|v| *v = 1.0);
            c
        });
        let mut nb = vec![0.0f32; len];
        render_pulses(tr, -(j as f64) * ADJACENT_TRACK_OFFSET_RF as f64, nctl, Some(&w[k]), h0, &mut nb);
        if j.abs() % 2 == 1 {
            azimuth_smooth(&mut nb, hp.width_um * wmax as f64, 2.0 * AZIMUTH_DEG);
        }
        for (a, b) in buf.iter_mut().zip(nb.iter()) {
            *a += *b;
        }
    }
    let mut rng = Rng::new(p.seed, STREAM_HEAD, n);
    head_front_end(&mut buf, hp, &mut rng);
    buf
}

/// Intermediate signals for analysis.
pub struct DebugField {
    pub doc_fraction: f32,
    pub carrier_level: f32,
    pub g0: i64,
    pub bb_start: i64,
    pub n_transitions: usize,
    pub rf: Vec<f32>,
    pub luma: Vec<f32>,
    pub cunder: Vec<f32>,
}

struct SegMid {
    bb_start: i64,
    luma: Vec<f32>,
    cunder: Vec<f32>,
    /// Fraction of the field during which the dropout compensator was active.
    doc_fraction: f32,
    /// RMS-based carrier amplitude at the limiter input (1.0 = alignment level).
    carrier_level: f32,
}

fn segment(p: &Params, rf: &HashMap<i64, Arc<Vec<f32>>>, n: i64) -> SegMid {
    let w = SEGMENT_WARM_RF as i64;
    let (g_a, g_b) = (switch_time(n), switch_time(n + 1));
    let (s0, s1) = (g_a - w, g_b + w);
    let mut buf = Vec::with_capacity((s1 - s0) as usize);
    for g in s0..s1 {
        let m = if g < g_a { n - 1 } else if g < g_b { n } else { n + 1 };
        let (gr, _) = track_window(m);
        buf.push(rf[&m][(g - gr) as usize]);
    }
    let out = process_segment(&buf, &p.deck, s0);
    let (a, b) = ((w as usize) / OS, (w as usize + FIELD_RF) / OS);
    let doc = out.dropout[a..b].iter().map(|v| *v as f32).sum::<f32>() / (b - a) as f32;
    let mid = &buf[w as usize..w as usize + FIELD_RF];
    let rms = (mid.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / mid.len() as f64).sqrt();
    SegMid {
        bb_start: g_a / OS as i64,
        luma: out.luma[a..b].to_vec(),
        cunder: out.cunder[a..b].to_vec(),
        doc_fraction: doc,
        carrier_level: (rms * 2f64.sqrt() * p.deck.fm_gain) as f32,
    }
}

/// Measure the deck alignment on a clean synthetic signal.
pub fn calibrate(p: &Params) -> Calibration {
    // clean heads, nominal deck, black picture with sync + burst on every line
    let mut clean = p.clone();
    for h in clean.heads.iter_mut() {
        *h = HeadParams { cnr_db: 400.0, media_am_noise: 0.0, ..HeadParams::default() };
    }
    clean.deck = DeckParams::default();
    clean.tape = TapeParams::default();
    clean.mech = MechParams::none();
    clean.tracking = TrackingParams::default();
    let cx = Ctx::new(clean.clone());
    let black = Arc::new(encode_frame(&Frame::new(ACTIVE_W, ACTIVE_H)));
    let src = WindowSource { first: -2, frames: vec![black.clone(); 8], blank: Baseband::blank() };
    let n = 2i64;
    let (g0, len) = track_window(n);
    let tr = record_track(&src, &clean.record, (n % 2) as usize, g0, len);
    let mut trans = HashMap::new();
    trans.insert(n, Arc::new(tr));
    let mut rf = render_rf(&cx, &trans, n);
    // equaliser, then measure the carrier amplitude on the black active area
    let mut x = rf.clone();
    for &(f, q, g) in &clean.deck.eq_peaks {
        crate::dsp::Biquad::peaking(f, q, g, FS_RF).process(&mut x);
    }
    let l0 = (g0 - (g0 - 0)) as usize; // window-relative origin
    let _ = l0;
    // choose lines well inside the active picture of the field
    let line0 = (60 * RF_PER_LINE) as i64 + (switch_time(n) - g0) + SWITCH_LEAD_RF as i64;
    let seg = &x[line0 as usize..line0 as usize + 40 * RF_PER_LINE];
    let rms = (seg.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / seg.len() as f64).sqrt();
    let fm_gain = 1.0 / (rms * 2f64.sqrt());

    // burst magnitude through the whole chain
    let mut deck = clean.deck.clone();
    deck.fm_gain = fm_gain;
    let out = process_segment(&rf, &deck, g0);
    let view = StreamView { bb0: g0 / OS as i64, luma: &out.luma, cunder: &out.cunder };
    // demodulate 30 lines in the middle of the field; find where the burst
    // actually lands (path delay) and how large it is there
    let start = (g0 / OS as i64) + (100 * BB_PER_LINE) as i64;
    let line_start = (start / BB_PER_LINE as i64 + 1) * BB_PER_LINE as i64;
    let dp = DecodeParams::default();
    let (re, im) = demod_chroma(&view, line_start - 200, 30 * BB_PER_LINE + 400, &dp);
    let (mut wsum, mut csum) = (0.0f64, 0.0f64);
    for l in 2..28 {
        for c in 40..200 {
            let i = 200 + l * BB_PER_LINE + c;
            let m2 = (re[i] as f64).powi(2) + (im[i] as f64).powi(2);
            wsum += m2;
            csum += m2 * c as f64;
        }
    }
    let centroid = csum / wsum;
    let chroma_delay = centroid - (BURST_START + BURST_LEN / 2) as f64;
    let (mut acc, mut cnt) = (0.0, 0);
    for l in 2..28 {
        let b0 = 200 + l * BB_PER_LINE + 88 + chroma_delay.round() as usize;
        let (mut br, mut bi) = (0.0f64, 0.0f64);
        for i in 0..20 {
            br += re[b0 + i] as f64;
            bi += im[b0 + i] as f64;
        }
        acc += (br * br + bi * bi).sqrt() / 20.0;
        cnt += 1;
    }
    rf.clear();
    Calibration { fm_gain, burst_nominal: acc / cnt as f64, chroma_delay }
}

/// Streaming session.
pub struct Session {
    pub params: Params,
    cx: Ctx,
    pub cal: Calibration,
    frames: VecDeque<Arc<Baseband>>,
    first_idx: i64,
    pushed: i64,
    finished: bool,
    next_out: i64,
    trans: HashMap<i64, Arc<Vec<Transition>>>,
    rf: HashMap<i64, Arc<Vec<f32>>>,
    seg: HashMap<i64, Arc<SegMid>>,
}

impl Session {
    pub fn new(params: Params) -> Self {
        Self::with_start(params, 0)
    }

    /// Start the session at absolute frame `start` of the timeline. Tape
    /// position, defects and timebase error are functions of the absolute frame
    /// index, so a plugin host can render any part of a clip and get the same
    /// tape every time. Frames pushed are numbered from `start`.
    pub fn with_start(params: Params, start: i64) -> Self {
        let cal = calibrate(&params);
        let mut params = params;
        params.deck.fm_gain = cal.fm_gain;
        params.decode.burst_nominal = cal.burst_nominal;
        params.decode.burst_boost = params.record.burst_boost as f64;
        params.decode.chroma_delay = cal.chroma_delay;
        let cx = Ctx::new(params.clone());
        Self {
            params,
            cx,
            cal,
            frames: VecDeque::new(),
            first_idx: start,
            pushed: start,
            finished: false,
            next_out: start,
            trans: HashMap::new(),
            rf: HashMap::new(),
            seg: HashMap::new(),
        }
    }

    /// Queue an input frame (must be ACTIVE_W x ACTIVE_H RGB).
    pub fn push(&mut self, f: &Frame) {
        self.frames.push_back(Arc::new(encode_frame(f)));
        self.pushed += 1;
    }

    /// Tell the session no more frames will arrive.
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// Number of output frames that can be produced right now.
    pub fn ready(&self) -> i64 {
        let mut k = self.next_out;
        while k < self.pushed && (self.finished || self.pushed >= k + 3) {
            k += 1;
        }
        k - self.next_out
    }

    /// Produce up to `max_frames` output frames.
    pub fn process(&mut self, max_frames: usize) -> Vec<Frame> {
        let count = (self.ready() as usize).min(max_frames) as i64;
        if count == 0 {
            return vec![];
        }
        let (k0, k1) = (self.next_out, self.next_out + count);
        let src = WindowSource {
            first: self.first_idx,
            frames: self.frames.iter().cloned().collect(),
            blank: Baseband::blank(),
        };
        let p = &self.params;
        let cx = &self.cx;

        // 1. record tracks n in [2k0-3, 2k1+3]
        let need: Vec<i64> = (2 * k0 - 3..=2 * k1 + 3).filter(|n| !self.trans.contains_key(n)).collect();
        let made: Vec<(i64, Arc<Vec<Transition>>)> = need
            .par_iter()
            .map(|&n| {
                let (g0, len) = track_window(n);
                (n, Arc::new(record_track(&src, &p.record, n.rem_euclid(2) as usize, g0, len)))
            })
            .collect();
        self.trans.extend(made);

        // 2. playback head RF for n in [2k0-1, 2k1+1]
        let need: Vec<i64> = (2 * k0 - 1..=2 * k1 + 1).filter(|n| !self.rf.contains_key(n)).collect();
        let trans = &self.trans;
        let made: Vec<(i64, Arc<Vec<f32>>)> = need.par_iter().map(|&n| (n, Arc::new(render_rf(cx, trans, n)))).collect();
        self.rf.extend(made);

        // 3. deck segments for n in [2k0, 2k1]
        let need: Vec<i64> = (2 * k0..=2 * k1).filter(|n| !self.seg.contains_key(n)).collect();
        let rfm = &self.rf;
        let made: Vec<(i64, Arc<SegMid>)> = need.par_iter().map(|&n| (n, Arc::new(segment(p, rfm, n)))).collect();
        self.seg.extend(made);

        // 4. decode frames
        let ks: Vec<i64> = (k0..k1).collect();
        let segm = &self.seg;
        let frames: Vec<Frame> = ks
            .par_iter()
            .map(|&k| {
                let parts = [&segm[&(2 * k)], &segm[&(2 * k + 1)], &segm[&(2 * k + 2)]];
                let luma: Vec<f32> = parts.iter().flat_map(|s| s.luma.iter().copied()).collect();
                let cu: Vec<f32> = parts.iter().flat_map(|s| s.cunder.iter().copied()).collect();
                let view = StreamView { bb0: parts[0].bb_start, luma: &luma, cunder: &cu };
                decode_frame(k, &view, &p.decode)
            })
            .collect();

        self.next_out = k1;
        // evict
        let keep_from = 2 * self.next_out - 6;
        self.trans.retain(|&n, _| n >= keep_from - 3);
        self.rf.retain(|&n, _| n >= keep_from - 1);
        self.seg.retain(|&n, _| n >= keep_from);
        while self.first_idx < self.next_out - 3 && !self.frames.is_empty() {
            self.frames.pop_front();
            self.first_idx += 1;
        }
        frames
    }

    /// Intermediate signals of one field, for analysis (requires the clip to be
    /// finished or at least 3 frames beyond the field).
    pub fn debug_field(&mut self, n: i64) -> DebugField {
        let src = WindowSource {
            first: self.first_idx,
            frames: self.frames.iter().cloned().collect(),
            blank: Baseband::blank(),
        };
        let p = &self.params;
        let cx = &self.cx;
        let mut trans = HashMap::new();
        let mut rf = HashMap::new();
        for m in n - 2..=n + 2 {
            let (g0, len) = track_window(m);
            trans.insert(m, Arc::new(record_track(&src, &p.record, m.rem_euclid(2) as usize, g0, len)));
        }
        for m in n - 1..=n + 1 {
            rf.insert(m, Arc::new(render_rf(cx, &trans, m)));
        }
        let seg = segment(p, &rf, n);
        let (g0, _) = track_window(n);
        DebugField { doc_fraction: seg.doc_fraction, carrier_level: seg.carrier_level, g0, n_transitions: trans[&n].len(), rf: rf[&n].to_vec(), luma: seg.luma, cunder: seg.cunder, bb_start: seg.bb_start }
    }

    /// Convenience: run a whole clip.
    pub fn run(params: Params, input: &[Frame]) -> Vec<Frame> {
        let mut s = Session::new(params);
        let mut out = Vec::new();
        for f in input {
            s.push(f);
            while s.ready() > 0 {
                out.extend(s.process(4));
            }
        }
        s.finish();
        while s.ready() > 0 {
            out.extend(s.process(4));
        }
        out
    }
}

/// A chain of decks: each generation re-records the previous generation's playback, the
/// way a dub does ("a copy of a copy"). Stage 0 sees the source; every later stage sees the
/// picture the stage before it played back. One stage is exactly a plain `Session`.
pub struct Chain {
    stages: Vec<Session>,
}

impl Chain {
    pub fn new(stages: Vec<Params>) -> Self {
        Self::with_start(stages, 0)
    }

    /// As `Session::with_start`: the tapes are functions of the absolute frame index.
    pub fn with_start(stages: Vec<Params>, start: i64) -> Self {
        let _ = (&stages, start);
        unimplemented!("generation chain")
    }

    pub fn push(&mut self, f: &Frame) {
        let _ = f;
        unimplemented!("generation chain")
    }

    pub fn finish(&mut self) {
        unimplemented!("generation chain")
    }

    /// Output frames that can be produced right now.
    pub fn ready(&self) -> i64 {
        unimplemented!("generation chain")
    }

    pub fn process(&mut self, max_frames: usize) -> Vec<Frame> {
        let _ = max_frames;
        unimplemented!("generation chain")
    }

    /// Convenience: run a whole clip through every generation.
    pub fn run(stages: Vec<Params>, input: &[Frame]) -> Vec<Frame> {
        let _ = (stages, input);
        unimplemented!("generation chain")
    }
}
