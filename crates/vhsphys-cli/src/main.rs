//! `vhsphys`: run a video through the VHS physical model.
//!
//! ffmpeg does the container/codec work and the scaling to and from the 754x480
//! active picture; this tool does the physics, for the picture and the sound.

mod audio_io;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;
use vhsphys_core::condition::Condition;
use vhsphys_core::config;
use vhsphys_core::consts::{ACTIVE_H, ACTIVE_W};
use vhsphys_core::interference_params::{MagnetKind, PhoneMode, RadioMod};
use vhsphys_core::pipeline::Chain;
use vhsphys_core::video::{decode_ideal, encode_frame, Frame};

/// Mains frequency the preset listing takes for granted; another one is worth a word.
const NOMINAL_MAINS_HZ: f64 = 60.0;

/// A number as a person would write it: 6, 59.4, 0.9 (no trailing zeros).
fn plain(x: f64) -> String {
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn phone_name(m: PhoneMode) -> &'static str {
    match m {
        PhoneMode::Off => "no",
        PhoneMode::Gsm => "gsm",
        PhoneMode::Gprs => "gprs",
        PhoneMode::Lte => "lte",
    }
}

fn radio_name(m: RadioMod) -> &'static str {
    match m {
        RadioMod::Off => "no",
        RadioMod::Am => "am",
        RadioMod::Cw => "cw",
        RadioMod::Ssb => "ssb",
    }
}

fn magnet_name(k: MagnetKind) -> &'static str {
    match k {
        MagnetKind::Absent => "no",
        MagnetKind::Fridge => "fridge",
        MagnetKind::Neodymium => "neodymium",
        MagnetKind::Speaker => "speaker",
    }
}

/// What a preset has switched on outside the circuit, in plain words; empty when nothing is.
/// The on/off decisions are those of the parameters themselves, the figures those of the condition.
fn outside_description(c: &Condition) -> String {
    let itf = c.to_params(1).interference;
    if itf.is_off() {
        return String::new();
    }
    let mut parts = Vec::new();
    if itf.phone_on() {
        parts.push(format!("{} phone at {} cm", phone_name(c.phone_mode), plain(c.phone_distance_cm)));
    }
    if itf.radio_on() {
        parts.push(format!("{} radio at {} MHz, {} V/m", radio_name(c.radio_mod), plain(c.radio_freq_mhz), plain(c.radio_field_vpm)));
    }
    if itf.ground_on() {
        parts.push(format!("ground loop {} mV", plain(c.ground_loop_mv)));
    }
    if itf.stray_on() {
        parts.push(format!("stray field {} uT", plain(c.stray_field_ut)));
    }
    if itf.dimmer_on() {
        parts.push(format!("dimmer firing at {} degrees", plain(c.dimmer_angle_deg)));
    }
    if itf.crt_magnet_on() {
        parts.push(format!("{} magnet at the tube", magnet_name(c.crt_magnet)));
    }
    if itf.head_magnet_on() {
        parts.push(format!("{} magnet at the head, {} cm", magnet_name(c.head_magnet), plain(c.head_magnet_cm)));
    }
    let on_the_mains = itf.ground_on() || itf.stray_on() || itf.dimmer_on();
    if on_the_mains && c.mains_hz != NOMINAL_MAINS_HZ {
        parts.push(format!("mains {} Hz", plain(c.mains_hz)));
    }
    parts.join(", ")
}

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

/// What happens to the input's sound.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum AudioMode {
    /// Play it through the same tape and deck as the picture.
    Process,
    /// Keep the original sound.
    Copy,
    /// No sound in the output.
    None,
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
    /// Sound: process (through the simulated tape and deck), copy (untouched) or none.
    #[arg(long, value_enum, default_value_t = AudioMode::Process)]
    audio: AudioMode,
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
    let is_image = matches!(a.output.extension().and_then(|e| e.to_str()), Some("png" | "jpg" | "jpeg" | "bmp" | "tif" | "tiff"));
    // The tape, its defects and the timebase error are functions of the absolute frame index,
    // so rendering from `--start` gives the same picture as rendering the whole file.
    let start_frame = a.start.map_or(0, |s| (s * 30000.0 / 1001.0).round() as i64);

    // ---- sound ----
    // Processed before the picture, into a temporary file the encoder reads as its sound.
    // It starts at the first video frame, so it is in sync and the same stretch of tape
    // sounds the same whatever `--start` is.
    // Stills have no sound, and a clip without sound gets none whatever the mode.
    let input_sound = if is_image || a.audio == AudioMode::None { None } else { audio_io::probe(&a.ffprobe, &a.input)? };
    let audio = if input_sound.is_some() { a.audio } else { AudioMode::None };
    let sound = match input_sound {
        Some((channels, _)) if audio == AudioMode::Process => {
            let channels = channels.min(2);
            let t0 = start_frame as f64 * 1001.0 / 30000.0;
            // no need to process sound past the last frame the picture will have
            let frames_s = a.frames.map(|n| n as f64 * 1001.0 / 30000.0);
            let duration = match (a.duration, frames_s) {
                (Some(d), Some(f)) => Some(d.min(f)),
                (d, f) => d.or(f),
            };
            let stages_sound = if a.bypass { &[][..] } else { &stages[..] };
            let wav = audio_io::render(&a.ffmpeg, &a.input, t0, duration, channels, stages_sound)?;
            eprintln!("sound: {channels} ch, {} Hz, processed", audio_io::RATE);
            Some(wav)
        }
        _ => None,
    };

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
    let mut enc = Command::new(&a.ffmpeg);
    enc.args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", &format!("{ACTIVE_W}x{ACTIVE_H}"), "-r", fps, "-i", "pipe:0"]);
    match (&sound, audio) {
        (Some(wav), _) => {
            enc.arg("-i").arg(wav.path());
            enc.args(["-map", "0:v", "-map", "1:a", "-c:a", "aac", "-shortest"]);
        }
        (None, AudioMode::Copy) => {
            if let Some(s) = a.start {
                enc.args(["-ss", &s.to_string()]);
            }
            enc.arg("-i").arg(&a.input);
            if let Some(d) = a.duration {
                enc.args(["-t", &d.to_string()]);
            }
            enc.args(["-map", "0:v", "-map", "1:a?", "-c:a", "aac", "-shortest"]);
        }
        _ => {}
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
                let outside = outside_description(&c);
                let outside = if outside.is_empty() { outside } else { format!("  outside: {outside}") };
                println!(
                    "  {n:12}  tape {:>2.0} y (quality {:.2}, stretch {:.2})  heads wear {:.2} clog {:.2}  deck {:>2.0} y at {:.0} C  tracking {:.0} um{outside}",
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
