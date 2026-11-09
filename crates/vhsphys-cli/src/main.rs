//! `vhsphys`: run a video through the VHS physical model.
//!
//! ffmpeg does the container/codec work and the scaling to and from the 754x480
//! active picture; this tool does the physics.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;
use vhsphys_core::condition::Condition;
use vhsphys_core::config;
use vhsphys_core::consts::{ACTIVE_H, ACTIVE_W};
use vhsphys_core::pipeline::Chain;
use vhsphys_core::video::{decode_ideal, encode_frame, Frame};

#[derive(Parser)]
#[command(name = "vhsphys", version, about = "Physical model of VHS record/playback, tape wear and deck faults")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Process a video file.
    Run(RunArgs),
    /// List the condition presets.
    Presets,
    /// Print the full parameter set (TOML) of a preset, as a starting point for a config file.
    Show {
        /// Preset name (see `presets`).
        preset: String,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Print the high-level condition of the preset (the `[condition]` table of a config
        /// file) instead of the full parameter set.
        #[arg(long)]
        condition: bool,
    },
}

#[derive(clap::Args)]
struct RunArgs {
    input: PathBuf,
    output: PathBuf,
    /// Condition preset: pristine, good, worn, bad, ruined, or an isolated fault.
    #[arg(short, long, default_value = "worn")]
    preset: String,
    /// TOML file with `[condition]` and/or raw parameter overrides.
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// Override one parameter, e.g. --set tape.defects_per_mm2=0.03 (repeatable).
    #[arg(long = "set", value_name = "KEY=VALUE")]
    sets: Vec<String>,
    /// Random seed: the same seed gives the same tape and deck.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Generations: 1 is a recording of the source, 2 a copy of that tape, and so on.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=8))]
    generations: u8,
    /// Start time in seconds.
    #[arg(long)]
    start: Option<f64>,
    /// Duration in seconds.
    #[arg(short = 't', long)]
    duration: Option<f64>,
    /// Stop after this many frames.
    #[arg(long)]
    frames: Option<usize>,
    /// Output size WxH (default: the input size).
    #[arg(long)]
    size: Option<String>,
    /// x264 quality (lower = better).
    #[arg(long, default_value_t = 18)]
    crf: u32,
    /// Worker threads (default: all cores).
    #[arg(long)]
    threads: Option<usize>,
    /// Frames processed per parallel batch.
    #[arg(long, default_value_t = 8)]
    batch: usize,
    /// Skip the tape/deck model: only encode to NTSC and decode again (framing check).
    #[arg(long)]
    bypass: bool,
    #[arg(long, default_value = "ffmpeg")]
    ffmpeg: String,
    #[arg(long, default_value = "ffprobe")]
    ffprobe: String,
}

fn probe_size(ffprobe: &str, input: &Path) -> Result<(usize, usize)> {
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0:s=x"])
        .arg(input)
        .output()
        .context("running ffprobe")?;
    let s = String::from_utf8_lossy(&out.stdout);
    let (w, h) = s.trim().split_once('x').context("could not read the video size")?;
    Ok((w.parse()?, h.parse()?))
}

fn read_frame(r: &mut impl Read, f: &mut Frame) -> Result<bool> {
    let mut got = 0;
    while got < f.rgb.len() {
        let n = r.read(&mut f.rgb[got..])?;
        if n == 0 {
            if got == 0 {
                return Ok(false);
            }
            bail!("truncated frame from ffmpeg");
        }
        got += n;
    }
    Ok(true)
}

fn run(a: RunArgs) -> Result<()> {
    if let Some(t) = a.threads {
        rayon::ThreadPoolBuilder::new().num_threads(t).build_global().ok();
    }
    let file_text = match &a.config {
        Some(p) => std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
        None => String::new(),
    };
    // `--set seed=N` is the seed of the whole chain, like a `seed` in the config file: it goes
    // into the config text, where every later generation derives its own seed from it. The
    // other `--set` items apply to every generation as they are.
    let (seed_sets, other_sets): (Vec<String>, Vec<String>) =
        a.sets.iter().cloned().partition(|s| s.split_once('=').is_some_and(|(k, _)| k.trim() == "seed"));
    let mut text = file_text;
    if !seed_sets.is_empty() {
        let seed_set: toml::Table = toml::from_str(&config::sets_to_toml(&seed_sets).map_err(anyhow::Error::msg)?)?;
        let mut table: toml::Table = toml::from_str(&text).context("reading the config")?;
        table.extend(seed_set);
        text = toml::to_string(&table)?;
    }
    let sets = config::sets_to_toml(&other_sets).map_err(anyhow::Error::msg)?;
    let stages = config::generation_params(Some(&a.preset), &text, a.seed, a.generations as usize)
        .and_then(|gens| gens.iter().map(|p| config::apply_overrides(p, &sets)).collect::<Result<Vec<_>, _>>())
        .map_err(anyhow::Error::msg)?;

    let (iw, ih) = probe_size(&a.ffprobe, &a.input)?;
    let (ow, oh) = match &a.size {
        Some(s) => {
            let (w, h) = s.split_once('x').context("--size must look like 1280x720")?;
            (w.parse()?, h.parse()?)
        }
        None => (iw, ih),
    };
    let fps = "30000/1001";

    // ---- decoder ----
    let mut dec = Command::new(&a.ffmpeg);
    dec.args(["-v", "error"]);
    if let Some(s) = a.start {
        dec.args(["-ss", &s.to_string()]);
    }
    dec.arg("-i").arg(&a.input);
    if let Some(d) = a.duration {
        dec.args(["-t", &d.to_string()]);
    }
    dec.args(["-an", "-vf", &format!("fps={fps},scale={ACTIVE_W}:{ACTIVE_H}:flags=lanczos,format=rgb24"), "-f", "rawvideo", "pipe:1"]);
    let mut dec = dec.stdout(Stdio::piped()).spawn().context("starting ffmpeg (decoder)")?;
    let mut dec_out = dec.stdout.take().unwrap();

    // ---- encoder ----
    let is_image = matches!(a.output.extension().and_then(|e| e.to_str()), Some("png" | "jpg" | "jpeg" | "bmp" | "tif" | "tiff"));
    let mut enc = Command::new(&a.ffmpeg);
    enc.args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", &format!("{ACTIVE_W}x{ACTIVE_H}"), "-r", fps, "-i", "pipe:0"]);
    if !is_image {
        if let Some(s) = a.start {
            enc.args(["-ss", &s.to_string()]);
        }
        enc.arg("-i").arg(&a.input);
        if let Some(d) = a.duration {
            enc.args(["-t", &d.to_string()]);
        }
        enc.args(["-map", "0:v", "-map", "1:a?", "-c:a", "aac", "-shortest"]);
    }
    // stills stay RGB (no 4:2:0 chroma subsampling); video is encoded as yuv420p
    enc.args(["-vf", &format!("scale={ow}:{oh}:flags=bicubic,format={}", if is_image { "rgb24" } else { "yuv420p" })]);
    if !is_image {
        enc.args(["-c:v", "libx264", "-crf", &a.crf.to_string(), "-preset", "medium"]);
    } else if !a.output.to_string_lossy().contains('%') {
        // a single still: keep only the last frame written
        enc.args(["-update", "1"]);
    }
    enc.arg(&a.output);
    let mut enc = enc.stdin(Stdio::piped()).spawn().context("starting ffmpeg (encoder)")?;
    let mut enc_in = enc.stdin.take().unwrap();

    eprintln!("preset '{}' seed {}  {}x{} -> {}x{}  ({})", a.preset, a.seed, iw, ih, ow, oh, a.output.display());
    // The tape, its defects and the timebase error are functions of the absolute frame index,
    // so rendering from `--start` gives the same picture as rendering the whole file.
    let start_frame = a.start.map_or(0, |s| (s * 30000.0 / 1001.0).round() as i64);
    let mut session = if a.bypass { None } else { Some(Chain::with_start(stages, start_frame)) };
    let t0 = Instant::now();
    let (mut n_in, mut n_out) = (0usize, 0usize);
    let limit = a.frames.unwrap_or(usize::MAX);
    let mut frame = Frame::new(ACTIVE_W, ACTIVE_H);
    let mut write = |f: &Frame, n_out: &mut usize| -> Result<()> {
        enc_in.write_all(&f.rgb)?;
        *n_out += 1;
        if *n_out % 15 == 0 {
            eprint!("\r  {} frames  {:.2} fps   ", n_out, *n_out as f64 / t0.elapsed().as_secs_f64());
        }
        Ok(())
    };
    while n_in < limit && read_frame(&mut dec_out, &mut frame)? {
        n_in += 1;
        match session.as_mut() {
            Some(s) => {
                s.push(&frame);
                while s.ready() >= a.batch as i64 {
                    for f in s.process(a.batch) {
                        write(&f, &mut n_out)?;
                    }
                }
            }
            None => write(&decode_ideal(&encode_frame(&frame)), &mut n_out)?,
        }
    }
    if let Some(s) = session.as_mut() {
        s.finish();
        while s.ready() > 0 {
            for f in s.process(a.batch) {
                write(&f, &mut n_out)?;
            }
        }
    }
    drop(enc_in);
    let _ = dec.kill();
    let st = enc.wait()?;
    eprintln!("\r  {} frames in {:.1}s ({:.2} fps)          ", n_out, t0.elapsed().as_secs_f64(), n_out as f64 / t0.elapsed().as_secs_f64());
    if !st.success() {
        bail!("ffmpeg encoder failed");
    }
    Ok(())
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Run(a) => run(a),
        Cmd::Presets => {
            println!("Condition presets (use with --preset):\n");
            for n in Condition::preset_names() {
                let c = Condition::preset(n).unwrap();
                println!(
                    "  {n:12}  tape {:>2.0} y (quality {:.2}, stretch {:.2})  heads wear {:.2} clog {:.2}  deck {:>2.0} y at {:.0} C  tracking {:.0} um",
                    c.tape_age_years, c.tape_quality, c.tape_stretch, c.head_wear, c.head_clog, c.deck_age_years, c.deck_temp_c, c.tracking_error_um
                );
            }
            Ok(())
        }
        Cmd::Show { preset, seed, condition } => {
            if condition {
                let c = Condition::preset(&preset)
                    .with_context(|| format!("unknown preset '{preset}' (try: {})", Condition::preset_names().join(", ")))?;
                print!("{}", toml::to_string_pretty(&c)?);
            } else {
                let p = config::build(Some(&preset), "", seed).map_err(anyhow::Error::msg)?;
                print!("{}", toml::to_string_pretty(&p)?);
            }
            Ok(())
        }
    }
}
