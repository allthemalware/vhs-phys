//! Run the test pattern through the full chain and dump frames as PPM.
//! usage: cargo run --release -p vhsphys-core --example e2e -- <outdir> [frames]
use std::io::Write;
use std::time::Instant;
use vhsphys_core::pipeline::{Params, Session};
use vhsphys_core::video::{test_pattern, Frame};

fn write_ppm(path: &str, f: &Frame) {
    let mut o = std::fs::File::create(path).unwrap();
    write!(o, "P6\n{} {}\n255\n", f.w, f.h).unwrap();
    o.write_all(&f.rgb).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).cloned().unwrap_or_else(|| ".".into());
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(6);
    std::fs::create_dir_all(&dir).unwrap();
    let input: Vec<Frame> = (0..n).map(|_| test_pattern()).collect();
    write_ppm(&format!("{dir}/in.ppm"), &input[0]);
    let t = Instant::now();
    let s = Session::new(Params::default());
    eprintln!("calibration: {:?} ({:.2}s)", s.cal, t.elapsed().as_secs_f64());
    let t = Instant::now();
    let out = Session::run(Params::default(), &input);
    eprintln!("{} frames in {:.2}s ({:.2}s/frame)", out.len(), t.elapsed().as_secs_f64(), t.elapsed().as_secs_f64() / out.len() as f64);
    write_ppm(&format!("{dir}/out.ppm"), &out[out.len() / 2]);
}
