//! Dump intermediate signals of one field to raw f32 files for the Python lab.
//! usage: dump <outdir> [field] [clean]
use std::io::Write;
use vhsphys_core::pipeline::{Params, Session};
use vhsphys_core::video::{test_pattern, Frame, encode_frame};
use vhsphys_core::consts::*;

fn save(path: &str, v: &[f32]) {
    let mut o = std::fs::File::create(path).unwrap();
    for x in v { o.write_all(&x.to_le_bytes()).unwrap(); }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = &a[1];
    let field: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
    let mode = a.get(3).map(|s| s.as_str()).unwrap_or("pattern");
    std::fs::create_dir_all(dir).unwrap();
    let mut p = match std::env::var("PRESET") {
        Ok(n) => vhsphys_core::config::build(Some(&n), &std::env::var("TOML").unwrap_or_default(), 7).unwrap(),
        Err(_) => Params::default(),
    };
    if mode == "clean" {
        for h in p.heads.iter_mut() { h.cnr_db = 400.0; h.media_am_noise = 0.0; }
    }
    let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f64>().ok());
    if let Some(v) = env("CHROMA_LEVEL") { p.record.chroma_level = v; }
    if let Some(v) = env("DOC") { p.deck.doc_enabled = v > 0.0; }
    if let Some(v) = env("EQ") { if v == 0.0 { p.deck.eq_peaks.clear(); } }
    if let Some(v) = env("QGAIN") { for h in p.heads.iter_mut() { h.q_gain_db = v; } }
    if let Some(v) = env("SHARP") { p.deck.sharp_db = v; }
    if let Some(v) = env("LPF") { p.deck.demod_lpf_hz = v; }
    if let Some(v) = env("EMPH") { p.record.emph_ratio = v; p.deck.deemph_ratio_scale = v / 5.0; }
    let mut s = Session::new(p);
    let f: Frame = if mode == "black" { Frame::new(ACTIVE_W, ACTIVE_H) } else { test_pattern() };
    for _ in 0..8 { s.push(&f); }
    s.finish();
    let d = s.debug_field(field);
    save(&format!("{dir}/rf.f32"), &d.rf);
    save(&format!("{dir}/luma.f32"), &d.luma);
    save(&format!("{dir}/cunder.f32"), &d.cunder);
    let meta = format!("{{\"g0\":{},\"bb_start\":{},\"n_transitions\":{},\"fs_rf\":{},\"fs_bb\":{},\"cal_fm_gain\":{},\"cal_burst\":{}}}",
        d.g0, d.bb_start, d.n_transitions, FS_RF, FS_BB, s.cal.fm_gain, s.cal.burst_nominal);
    std::fs::write(format!("{dir}/meta.json"), meta).unwrap();
    let _ = encode_frame;
    eprintln!("dumped field {field}: {} transitions, carrier level {:.3} (1.0 = alignment), DOC active {:.1}% of the field", d.n_transitions, d.carrier_level, d.doc_fraction * 100.0);
}
