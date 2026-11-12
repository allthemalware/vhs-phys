//! `vhsphys`: run a video through the VHS physical model.
//!
//! ffmpeg does the container/codec work and the scaling to and from the active picture of the
//! television standard (754x480 at 29.97 frames per second for NTSC and PAL-60, 740x576 at 25
//! for PAL); this tool does the physics, for the picture and the sound.
//!
//! The model is compiled twice, as the 525-line engine (NTSC, and PAL-60 which is an NTSC tape
//! played into a PAL television) and as the 625-line engine (PAL). `--standard` picks the
//! engine; the run itself is written once, against the `Engine` trait below.

mod audio_io;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;
use vhsphys_core::condition::Condition;
use vhsphys_core::config;
use vhsphys_core::consts::MAINS_HZ_DEFAULT;
use vhsphys_core::frame::Frame;
use vhsphys_core::interference_params::{MagnetKind, PhoneMode, RadioMod};
use vhsphys_core::standard::TvStandard;
use vhsphys_core::{ntsc, pal};

/// Mains frequency the preset listing takes for granted; another one is worth a word.
const NOMINAL_MAINS_HZ: f64 = MAINS_HZ_DEFAULT;

/// Extensions of the still images a run can write (lower case); any other output is H.264 video with AAC sound.
const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "bmp", "tif", "tiff"];

/// Extensions of the containers that hold that H.264 video and AAC sound (lower case); others, such as webm, do not.
const VIDEO_EXTENSIONS: [&str; 4] = ["mp4", "mkv", "mov", "m4v"];

/// Extension of a default output name when the input's is not one a run can write.
const FALLBACK_EXTENSION: &str = "mp4";

/// The `[condition]` table of the config text. `--set condition.KEY=VALUE` items go into it.
const CONDITION_TABLE: &str = "condition";

/// The most generations a chain of dubs may have.
const MAX_GENERATIONS: u8 = 8;

/// What the wizard offers for the camera exposure (stops) and for the generations; Enter takes them.
const WIZARD_IRIS_STOPS: &str = "2.0";
const WIZARD_GENERATIONS: &str = "1";

/// Characters, besides letters and digits, that a POSIX shell takes literally in a word.
const SHELL_SAFE: &str = "_./=:+-,@%";

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
    /// Interactive terminal wizard guiding parameter setup.
    Wizard {
        /// Optional input file to pre-populate.
        input: Option<PathBuf>,
        /// Optional output file to pre-populate.
        output: Option<PathBuf>,
    },
    /// List the condition presets.
    Presets,
    /// Print the full parameter set (TOML) of a preset, as a starting point for a config file.
    Show {
        /// Preset name (see `presets`).
        preset: String,
        /// Random seed, as for `run`.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Print the high-level condition of the preset (the `[condition]` table of a config
        /// file) instead of the full parameter set.
        #[arg(long)]
        condition: bool,
        /// Television standard whose parameters to print (ntsc, pal or pal60).
        #[arg(long, value_enum, default_value_t = StandardArg::Ntsc)]
        standard: StandardArg,
    },
}

/// The television standard: which raster the deck records and which colour system the set decodes.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum StandardArg {
    /// 525 lines at 29.97 frames per second, an NTSC tape into an NTSC television.
    #[value(name = "ntsc")]
    Ntsc,
    /// 625 lines at 25 frames per second, a PAL tape into a PAL television.
    #[value(name = "pal")]
    Pal,
    /// 525 lines at 29.97 frames per second, an NTSC tape into a PAL television.
    #[value(name = "pal60")]
    Pal60,
}

impl StandardArg {
    fn standard(self) -> TvStandard {
        match self {
            StandardArg::Ntsc => TvStandard::Ntsc,
            StandardArg::Pal => TvStandard::Pal,
            StandardArg::Pal60 => TvStandard::Pal60,
        }
    }
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

/// The camera's pickup tubes, ahead of the VHS recorder.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum CameraTubeArg {
    /// No camera stage: the input is the video signal itself.
    Off,
    /// Plumbicon (XQ1020): lag, and comet tails behind highlights the beam cannot take off.
    Plumbicon,
    /// Plumbicon with internal light bias (XQ1410): less lag, the same comet tails.
    PlumbiconBias,
    /// Plumbicon with an anti-comet-tail gun (XQ1520): lag, no comet tails.
    PlumbiconAct,
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
    /// Television standard: ntsc (the default), pal or pal60. Wins over `standard` in the config file.
    #[arg(long, value_enum)]
    standard: Option<StandardArg>,
    /// Random seed: the same seed gives the same tape and deck.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Generations: 1 is a recording of the source, 2 a copy of that tape, and so on.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=MAX_GENERATIONS as i64))]
    generations: u8,
    /// Sound: process (through the simulated tape and deck), copy (untouched) or none.
    #[arg(long, value_enum, default_value_t = AudioMode::Process)]
    audio: AudioMode,
    /// Camera pickup tube ahead of the recorder: lag and comet tails (off, plumbicon, plumbicon-bias, plumbicon-act).
    #[arg(long, value_enum)]
    camera_tube: Option<CameraTubeArg>,
    /// Exposure of the camera tubes in stops: positive overexposes, so highlights leave tails.
    #[arg(long, value_name = "N", allow_negative_numbers = true)]
    iris_stops: Option<f64>,
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
    /// Skip the tape/deck model: only encode to the standard's raster and decode again (framing check).
    #[arg(long)]
    bypass: bool,
    /// JSON, TOML or CSV file with time-varying parameter automation curves (e.g. tracking offset drift, head clogging, iris exposure).
    #[arg(long, value_name = "FILE")]
    automate: Option<PathBuf>,
    /// The ffmpeg program to run.
    #[arg(long, default_value = "ffmpeg")]
    ffmpeg: String,
    /// The ffprobe program to run.
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


/// Seconds as ffmpeg takes them: cut (not rounded) to whole microseconds. A seek that lands a
/// hair after the frame it aims at skips that frame; one a hair before it does not.
fn seek_arg(seconds: f64) -> String {
    micros_arg((seconds.max(0.0) * 1e6).floor() as u64)
}

fn micros_arg(micros: u64) -> String {
    format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000)
}

/// How many frames `seconds` of a clip at `fps` (numerator, denominator) make: the nearest
/// whole number, at least one. A tie goes to the larger count, which is where ffmpeg's own cut
/// puts it (2.5 s at 25 fps is 63 frames), so a `-t` that is not a whole number of frames gives
/// the clip it always gave.
fn frames_in(seconds: f64, (num, den): (u32, u32)) -> u64 {
    ((seconds * num as f64 / den as f64) + 0.5).floor().max(1.0) as u64
}

/// `-t` for exactly `n` frame periods. ffmpeg ends a clip at the nearest whole frame, so a
/// whole number of periods is the one length that cannot fall on the wrong side of a frame;
/// it is cut to whole microseconds so that it never reaches the frame after the last.
fn frames_arg(n: u64, (num, den): (u32, u32)) -> String {
    micros_arg((n as u128 * den as u128 * 1_000_000 / num as u128) as u64)
}

/// What a run needs from one of the two compiled engines: the 525-line one (NTSC, and PAL-60
/// which is an NTSC tape) and the 625-line one (PAL). Both have the same API in the core, but
/// their types are different, so the render loop reaches them through this.
trait Engine {
    type Params;
    type Chain: Playback;
    /// Size of the active picture: the frames the engine takes and gives.
    const ACTIVE_W: usize;
    const ACTIVE_H: usize;
    /// Frames per second as the fraction ffmpeg takes (numerator, denominator). It stays
    /// exact in seek times and frame counts, where the float `FRAME_RATE` is a derived value.
    const FPS: (u32, u32);
    /// The engine's own `consts::FRAME_RATE`, which `FPS` must agree with.
    const FRAME_RATE: f64;

    /// The parameters of every generation: preset, config text, then the `--set` overrides.
    fn generations(preset: &str, text: &str, seed: u64, count: usize, sets: &str) -> Result<Vec<Self::Params>, String>;
    fn chain(stages: Vec<Self::Params>, start: i64) -> Self::Chain;
    /// Encode a frame to the television signal and decode it again, with no tape and no deck.
    fn bypass(frame: &Frame) -> Frame;
    /// Play `channels` channels of the input's sound, from `t0` seconds, through the decks.
    fn sound(ffmpeg: &str, input: &Path, t0: f64, duration: Option<f64>, channels: usize, stages: &[Self::Params]) -> Result<SoundFile>;
    /// The full parameter set of a preset, as TOML.
    fn parameters(preset: &str, text: &str, seed: u64) -> Result<String>;
    /// The `[condition]` of a preset, as TOML (the table's contents, without its header).
    fn condition(preset: &str) -> Result<String>;
}

/// A chain of decks as the render loop drives it.
trait Playback {
    fn push_frame(&mut self, f: &Frame);
    fn finish_input(&mut self);
    fn frames_ready(&self) -> i64;
    fn play(&mut self, max_frames: usize) -> Vec<Frame>;
    fn set_automation(&mut self, schedule: Option<vhsphys_core::automation::AutomationSchedule>);
}

/// The processed sound, in a temporary WAV file that goes away with this value.
struct SoundFile(audio_io::TempFile);

impl SoundFile {
    /// Add the sound as the next input of an encoder command.
    fn add_input(&self, enc: &mut Command) {
        enc.arg("-i").arg(self.0.path());
    }
}

/// Implements `Engine` (and `Playback` for its chain) for the engine module `$core`.
macro_rules! engine {
    ($name:ident, $core:ident, $num:literal / $den:literal) => {
        struct $name;

        impl Engine for $name {
            type Params = $core::pipeline::Params;
            type Chain = $core::pipeline::Chain;
            const ACTIVE_W: usize = $core::consts::ACTIVE_W;
            const ACTIVE_H: usize = $core::consts::ACTIVE_H;
            const FPS: (u32, u32) = ($num, $den);
            const FRAME_RATE: f64 = $core::consts::FRAME_RATE;

            fn generations(preset: &str, text: &str, seed: u64, count: usize, sets: &str) -> Result<Vec<Self::Params>, String> {
                $core::config::generation_params(Some(preset), text, seed, count)
                    .and_then(|gens| gens.iter().map(|p| $core::config::apply_overrides(p, sets)).collect::<Result<Vec<_>, _>>())
            }

            fn chain(stages: Vec<Self::Params>, start: i64) -> Self::Chain {
                $core::pipeline::Chain::with_start(stages, start)
            }

            fn bypass(frame: &Frame) -> Frame {
                $core::video::decode_ideal(&$core::video::encode_frame(frame))
            }

            fn sound(ffmpeg: &str, input: &Path, t0: f64, duration: Option<f64>, channels: usize, stages: &[Self::Params]) -> Result<SoundFile> {
                audio_io::render::<$core::audio::AudioProcessor>(ffmpeg, input, t0, duration, channels, stages).map(SoundFile)
            }

            fn parameters(preset: &str, text: &str, seed: u64) -> Result<String> {
                let p = $core::config::build(Some(preset), text, seed).map_err(anyhow::Error::msg)?;
                Ok(toml::to_string_pretty(&p)?)
            }

            fn condition(preset: &str) -> Result<String> {
                let c = $core::condition::Condition::preset(preset)
                    .with_context(|| format!("unknown preset '{preset}' (try: {})", $core::condition::Condition::preset_names().join(", ")))?;
                Ok(toml::to_string_pretty(&c)?)
            }
        }

        impl Playback for $core::pipeline::Chain {
            fn push_frame(&mut self, f: &Frame) {
                self.push(f)
            }
            fn finish_input(&mut self) {
                self.finish()
            }
            fn frames_ready(&self) -> i64 {
                self.ready()
            }
            fn play(&mut self, max_frames: usize) -> Vec<Frame> {
                self.process(max_frames)
            }
            fn set_automation(&mut self, schedule: Option<vhsphys_core::automation::AutomationSchedule>) {
                self.set_automation(schedule);
            }
        }
    };
}

engine!(Ntsc525, ntsc, 30000 / 1001);
engine!(Pal625, pal, 25 / 1);

/// Take the `--set` items for `key` out of `sets`: (those, the rest).
fn take_key(sets: Vec<String>, key: &str) -> (Vec<String>, Vec<String>) {
    sets.into_iter().partition(|s| s.split_once('=').is_some_and(|(k, _)| k.trim() == key))
}

fn parse_standard(value: &toml::Value) -> Result<TvStandard> {
    let list = TvStandard::ALL.map(TvStandard::name).join(", ");
    let name = value.as_str().ok_or_else(|| anyhow!("standard must be a name ({list})"))?;
    TvStandard::ALL.into_iter().find(|s| s.name() == name).ok_or_else(|| anyhow!("unknown television standard '{name}' (use {list})"))
}

/// The television standard of a run: the `--standard` flag; else `--set standard=...`; else the
/// top-level `standard` of the config file; else NTSC. A `--set` wins over the file as it does
/// for every other parameter.
fn pick_standard(flag: Option<StandardArg>, sets: &[String], config_file: &toml::Table) -> Result<TvStandard> {
    if let Some(f) = flag {
        return Ok(f.standard());
    }
    let from_sets: toml::Table = toml::from_str(&config::sets_to_toml(sets).map_err(anyhow::Error::msg)?)?;
    match from_sets.get("standard").or_else(|| config_file.get("standard")) {
        Some(v) => parse_standard(v),
        None => Ok(TvStandard::Ntsc),
    }
}

/// Whether a `--set` item sets a key inside `table`: `condition.tape_speed=lp` does for `condition`.
fn is_set_in_table(item: &str, table: &str) -> bool {
    item.split_once('=').is_some_and(|(k, _)| k.trim().strip_prefix(table).is_some_and(|rest| rest.starts_with('.')))
}

/// The config text of a run (the file's, with what the `--set` items put into it), the text of
/// the `--set` items that apply to every generation as they are, and the television standard.
fn assemble_config(file_text: &str, sets: &[String], standard_flag: Option<StandardArg>) -> Result<(String, String, TvStandard)> {
    // `--set seed=N` is the seed of the whole chain, like a `seed` in the config file: it goes
    // into the config text, where every later generation derives its own seed from it. The
    // other `--set` items apply to every generation as they are. So does `--set standard=...`
    // and for the same reason: the standard decides which engine reads the config, and the
    // engine refuses what it cannot play (EP at 625 lines), which only happens for a standard
    // that is in the text. `--set condition.KEY=...` goes into the text too, into its
    // `[condition]` table: the condition is mapped to parameters before the other overrides
    // apply, and an override cannot reach back into that mapping (the tape speed gives the head
    // width, the sub-emphasis, the carrier interleave and the burst boost, and an override of
    // the top-level `speed` changes none of them).
    let (seed_sets, other_sets) = take_key(sets.to_vec(), "seed");
    let (standard_sets, other_sets) = take_key(other_sets, "standard");
    let (condition_sets, other_sets): (Vec<String>, Vec<String>) = other_sets.into_iter().partition(|s| is_set_in_table(s, CONDITION_TABLE));
    let mut table: toml::Table = toml::from_str(file_text).map_err(|e| anyhow!("config: {e}"))?;
    let standard = pick_standard(standard_flag, &standard_sets, &table)?;
    let mut edited = false;
    if !seed_sets.is_empty() {
        let seed_set: toml::Table = toml::from_str(&config::sets_to_toml(&seed_sets).map_err(anyhow::Error::msg)?)?;
        table.extend(seed_set);
        edited = true;
    }
    if standard_flag.is_some() || !standard_sets.is_empty() {
        table.insert("standard".into(), toml::Value::String(standard.name().into()));
        edited = true;
    }
    if !condition_sets.is_empty() {
        let mut from_sets: toml::Table = toml::from_str(&config::sets_to_toml(&condition_sets).map_err(anyhow::Error::msg)?)?;
        if let Some(toml::Value::Table(items)) = from_sets.remove(CONDITION_TABLE) {
            // an item of the same key wins over the file's, the file's other keys stay
            match table.entry(CONDITION_TABLE).or_insert_with(|| toml::Value::Table(toml::Table::new())) {
                toml::Value::Table(condition) => condition.extend(items),
                _ => bail!("config: {CONDITION_TABLE} must be a table"),
            }
            edited = true;
        }
    }
    let text = if edited { toml::to_string(&table)? } else { file_text.to_string() };
    let overrides = config::sets_to_toml(&other_sets).map_err(anyhow::Error::msg)?;
    Ok((text, overrides, standard))
}

fn run(a: RunArgs) -> Result<()> {
    if let Some(t) = a.threads {
        rayon::ThreadPoolBuilder::new().num_threads(t).build_global().ok();
    }
    let file_text = match &a.config {
        Some(p) => std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
        None => String::new(),
    };
    // The flags come first, so that a `--set` of the same key wins over them.
    let mut sets = Vec::new();
    if let Some(stops) = a.iris_stops {
        sets.push(format!("camera.iris_stops={stops:?}"));
    }
    if let Some(tube) = a.camera_tube {
        let name = match tube {
            CameraTubeArg::Off => "off",
            CameraTubeArg::Plumbicon => "plumbicon",
            CameraTubeArg::PlumbiconBias => "plumbicon-bias",
            CameraTubeArg::PlumbiconAct => "plumbicon-act",
        };
        sets.push(format!("camera.tube={name}"));
    }
    sets.extend(a.sets.iter().cloned());
    let (text, sets, standard) = assemble_config(&file_text, &sets, a.standard)?;
    if standard.is_625() {
        run_engine::<Pal625>(a, standard, &text, &sets)
    } else {
        run_engine::<Ntsc525>(a, standard, &text, &sets)
    }
}

fn run_engine<E: Engine>(a: RunArgs, standard: TvStandard, text: &str, sets: &str) -> Result<()> {
    debug_assert!((E::FPS.0 as f64 / E::FPS.1 as f64 - E::FRAME_RATE).abs() < 1e-9, "the frame rate fraction does not match the engine");
    // Read before the sound is processed and before the encoder starts: it overwrites the
    // output, which a missing or malformed file must not cost.
    let automation = match &a.automate {
        Some(path) => Some(vhsphys_core::automation::AutomationSchedule::from_file(path).map_err(|e| anyhow::anyhow!("loading automation file '{}': {e}", path.display()))?),
        None => None,
    };
    let stages = E::generations(&a.preset, text, a.seed, a.generations as usize, sets).map_err(anyhow::Error::msg)?;

    let (iw, ih) = probe_size(&a.ffprobe, &a.input)?;
    let (ow, oh) = match &a.size {
        Some(s) => {
            let (w, h) = s.split_once('x').context("--size must look like 1280x720")?;
            (w.parse()?, h.parse()?)
        }
        None => (iw, ih),
    };
    let (fps_num, fps_den) = E::FPS;
    let fps = format!("{fps_num}/{fps_den}");
    let is_image = has_extension(&a.output, &IMAGE_EXTENSIONS);
    // The tape, its defects and the timebase error are functions of the absolute frame index,
    // so rendering from `--start` gives the same picture as rendering the whole file.
    let start_frame = a.start.map_or(0, |s| (s * fps_num as f64 / fps_den as f64).round() as i64);
    // The length of the picture in frames, which is what the decoder is asked for.
    let picture_frames = match a.duration {
        Some(d) if d.is_finite() && d > 0.0 => Some(frames_in(d, E::FPS)),
        Some(_) => bail!("the duration must be more than zero seconds"),
        None => None,
    };

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
            let t0 = start_frame as f64 * fps_den as f64 / fps_num as f64;
            // no need to process sound past the last frame the picture will have
            let frames_s = a.frames.map(|n| n as f64 * fps_den as f64 / fps_num as f64);
            let duration = match (a.duration, frames_s) {
                (Some(d), Some(f)) => Some(d.min(f)),
                (d, f) => d.or(f),
            };
            let stages_sound = if a.bypass { &[][..] } else { &stages[..] };
            let sound = E::sound(&a.ffmpeg, &a.input, t0, duration, channels, stages_sound)?;
            eprintln!("sound: {channels} ch, {} Hz, processed", audio_io::RATE);
            Some(sound)
        }
        _ => None,
    };

    // ---- decoder ----
    let mut dec = Command::new(&a.ffmpeg);
    dec.args(["-v", "error"]);
    if let Some(s) = a.start {
        dec.args(["-ss", &seek_arg(s)]);
    }
    dec.arg("-i").arg(&a.input);
    if let Some(n) = picture_frames {
        dec.args(["-t", &frames_arg(n, E::FPS)]);
    }
    dec.args(["-an", "-vf", &format!("fps={fps},scale={}:{}:flags=lanczos,format=rgb24", E::ACTIVE_W, E::ACTIVE_H), "-f", "rawvideo", "pipe:1"]);
    let mut dec = dec.stdout(Stdio::piped()).spawn().context("starting ffmpeg (decoder)")?;
    let mut dec_out = dec.stdout.take().unwrap();

    // ---- encoder ----
    let mut enc = Command::new(&a.ffmpeg);
    enc.args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", &format!("{}x{}", E::ACTIVE_W, E::ACTIVE_H), "-r", &fps, "-i", "pipe:0"]);
    match (&sound, audio) {
        (Some(sound), _) => {
            sound.add_input(&mut enc);
            enc.args(["-map", "0:v", "-map", "1:a", "-c:a", "aac", "-shortest"]);
        }
        (None, AudioMode::Copy) => {
            if let Some(s) = a.start {
                enc.args(["-ss", &seek_arg(s)]);
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

    let standard_note = if standard == TvStandard::Ntsc { String::new() } else { format!(" standard {}", standard.name()) };
    eprintln!("preset '{}' seed {}{}  {}x{} -> {}x{}  ({})", a.preset, a.seed, standard_note, iw, ih, ow, oh, a.output.display());
    let mut session = if a.bypass {
        None
    } else {
        let mut chain = E::chain(stages, start_frame);
        if automation.is_some() {
            chain.set_automation(automation);
        }
        Some(chain)
    };
    let t0 = Instant::now();
    let (mut n_in, mut n_out) = (0usize, 0usize);
    let limit = a.frames.unwrap_or(usize::MAX);
    let mut frame = Frame::new(E::ACTIVE_W, E::ACTIVE_H);
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
                s.push_frame(&frame);
                while s.frames_ready() >= a.batch as i64 {
                    for f in s.play(a.batch) {
                        write(&f, &mut n_out)?;
                    }
                }
            }
            None => write(&E::bypass(&frame), &mut n_out)?,
        }
    }
    if let Some(s) = session.as_mut() {
        s.finish_input();
        while s.frames_ready() > 0 {
            for f in s.play(a.batch) {
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

/// Print the parameters, or with `condition` the `[condition]` table, of a preset on the
/// engine that plays `standard`.
fn show<E: Engine>(standard: TvStandard, preset: &str, seed: u64, condition: bool) -> Result<()> {
    if condition {
        let c = E::condition(preset)?;
        // A standard other than NTSC goes above the table, so that the output is a config file
        // that plays on the same engine as it stands (the condition maps to another set of
        // parameters at 625 lines).
        if standard != TvStandard::Ntsc {
            println!("standard = \"{}\"", standard.name());
        }
        // under its table header, so the output is a config file as it stands
        print!("[condition]\n{c}");
    } else {
        print!("{}", E::parameters(preset, &format!("standard = \"{}\"\n", standard.name()), seed)?);
    }
    Ok(())
}

/// Whether the path's extension is one of `list` (which is lower case), whatever its case.
fn has_extension(path: &Path, list: &[&str]) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| list.iter().any(|l| e.eq_ignore_ascii_case(l)))
}

/// The wizard's default output for an input: its name with `_vhs` added. It keeps the input's
/// extension only if a run can write it (a still image, or a container that holds H.264 video
/// with AAC sound); the encoder fails or refuses the sound in others, such as webm.
fn default_output_name(input: &Path) -> String {
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let writable = has_extension(input, &IMAGE_EXTENSIONS) || has_extension(input, &VIDEO_EXTENSIONS);
    let ext = input.extension().and_then(|e| e.to_str()).filter(|_| writable).unwrap_or(FALLBACK_EXTENSION);
    format!("{stem}_vhs.{ext}")
}

/// One argument of a command line as a POSIX shell reads it back: bare if it is made of safe
/// characters only, else in single quotes (a single quote inside is closed, escaped and reopened).
fn shell_quote(arg: &str) -> String {
    if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || SHELL_SAFE.contains(c)) {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// Whether two paths name the same file. The encoder overwrites its output, so naming the input
/// as the output would destroy the source while it is read. An existing output is compared by
/// canonical path (links and `..` resolved); one that does not exist yet by its plain path.
fn same_file(input: &Path, output: &Path) -> bool {
    if output.exists() {
        if let (Ok(i), Ok(o)) = (input.canonicalize(), output.canonicalize()) {
            return i == o;
        }
    }
    input == output
}

/// A camera exposure in stops: any finite number.
fn parse_iris_stops(answer: &str) -> Option<f64> {
    answer.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A number of generations: a whole number from 1 to `MAX_GENERATIONS`.
fn parse_generations(answer: &str) -> Option<u8> {
    answer.parse::<u8>().ok().filter(|n| (1..=MAX_GENERATIONS).contains(n))
}

fn ask_line(prompt_text: &str, default_val: Option<&str>) -> Result<String> {
    print!("{prompt_text}");
    if let Some(d) = default_val {
        print!(" [{d}]");
    }
    print!(": ");
    std::io::stdout().flush()?;
    let mut buf = String::new();
    let n = std::io::stdin().read_line(&mut buf)?;
    if n == 0 {
        if let Some(d) = default_val {
            return Ok(d.to_string());
        }
        bail!("Interactive input ended");
    }
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        if let Some(d) = default_val {
            return Ok(d.to_string());
        }
    }
    Ok(trimmed.to_string())
}

fn ask_choice(prompt_text: &str, choices: &[(&str, &str)], default_idx: usize) -> Result<usize> {
    println!("\n{prompt_text}");
    for (i, (name, desc)) in choices.iter().enumerate() {
        if desc.is_empty() {
            println!("  {}) {}", i + 1, name);
        } else {
            println!("  {}) {:<16} - {}", i + 1, name, desc);
        }
    }
    let default_str = (default_idx + 1).to_string();
    loop {
        let answer = ask_line(&format!("Select (1-{})", choices.len()), Some(&default_str))?;
        if let Ok(idx) = answer.parse::<usize>() {
            if idx >= 1 && idx <= choices.len() {
                return Ok(idx - 1);
            }
        }
        println!("Invalid selection. Please enter a number between 1 and {}.", choices.len());
    }
}

fn ask_iris_stops() -> Result<Option<f64>> {
    loop {
        let answer = ask_line("Camera exposure (iris stops: 0 = normal, +2 = overexposed highlights leave tails)", Some(WIZARD_IRIS_STOPS))?;
        match parse_iris_stops(&answer) {
            Some(stops) => return Ok(Some(stops)),
            None => println!("'{answer}' is not a number of stops. Please enter a number such as 2 or -1.5."),
        }
    }
}

fn ask_generations() -> Result<u8> {
    loop {
        let answer = ask_line("Tape dub generations (1 = first recording, 2 = copy of tape)", Some(WIZARD_GENERATIONS))?;
        match parse_generations(&answer) {
            Some(n) => return Ok(n),
            None => println!("'{answer}' is not a number of generations. Please enter a whole number from 1 to {MAX_GENERATIONS}."),
        }
    }
}

fn wizard(input_opt: Option<PathBuf>, output_opt: Option<PathBuf>) -> Result<()> {
    println!("=== vhsphys interactive wizard ===");
    println!("Press Enter at any prompt to accept the [default] value.\n");

    let input_path = loop {
        let default_in = input_opt.as_ref().map(|p| p.to_string_lossy().to_string());
        let p = ask_line("Input video file", default_in.as_deref())?;
        if p.is_empty() {
            println!("Input file is required.");
            continue;
        }
        let path = PathBuf::from(&p);
        if path.exists() {
            break path;
        }
        println!("File '{}' not found. Please enter a valid path.", path.display());
    };

    let default_out = output_opt.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| default_output_name(&input_path));
    let output_path = loop {
        // A default that is the input itself is not offered, so that Enter (or the end of the input) cannot take it.
        let default = (!same_file(&input_path, Path::new(&default_out))).then_some(default_out.as_str());
        let answer = ask_line("Output video file", default)?;
        if answer.is_empty() {
            println!("Output file is required.");
            continue;
        }
        let path = PathBuf::from(&answer);
        if same_file(&input_path, &path) {
            println!("'{}' is the input file, which the run would overwrite while reading it. Please name another output file.", path.display());
            continue;
        }
        break path;
    };

    let std_choices = [
        ("NTSC", "525 lines @ 29.97 fps (North America, Japan)"),
        ("PAL", "625 lines @ 25.00 fps (Europe, Australia)"),
        ("PAL-60", "NTSC tape played into a PAL television (60 Hz colour)"),
    ];
    let std_idx = ask_choice("Television standard:", &std_choices, 0)?;
    let standard_arg = match std_idx {
        0 => StandardArg::Ntsc,
        1 => StandardArg::Pal,
        _ => StandardArg::Pal60,
    };
    let is_pal625 = matches!(standard_arg, StandardArg::Pal);

    let speed_choices: Vec<(&str, &str)> = if is_pal625 {
        vec![
            ("SP", "Standard Play (49 um track, SP playback head)"),
            ("LP", "Long Play (half speed, 24.5 um track, higher noise/crosstalk)"),
        ]
    } else {
        vec![
            ("SP", "Standard Play (58 um track pitch, standard speed)"),
            ("LP", "Long Play (29 um track pitch, slow-speed sub-emphasis)"),
            ("EP", "Extended Play / SLP (19 um track pitch, highest noise)"),
        ]
    };
    let speed_idx = ask_choice("Tape speed:", &speed_choices, 0)?;
    let speed_name = speed_choices[speed_idx].0.to_lowercase();

    let preset_choices = [
        ("worn", "Typical home recording (slight wear, head flutter, mild dropouts)"),
        ("pristine", "Brand new tape and calibrated studio VCR (clean alignment)"),
        ("good", "Well-maintained home deck and good quality tape"),
        ("bad", "Heavily worn tape, clogged head, frequent dropouts, jitter"),
        ("ruined", "Severe degradation, aged binder, high dropout rate"),
        ("other", "Choose from full list of environmental and circuit presets"),
    ];
    let preset_idx = ask_choice("Condition preset:", &preset_choices, 0)?;
    let preset_name = if preset_idx == 5 {
        println!("\nAvailable presets: {}", Condition::preset_names().join(", "));
        loop {
            let name = ask_line("Enter preset name", Some("ground-loop"))?;
            if Condition::preset(name.trim()).is_some() {
                break name.trim().to_string();
            }
            println!("Unknown preset '{}'. Try one of: {}", name, Condition::preset_names().join(", "));
        }
    } else {
        preset_choices[preset_idx].0.to_string()
    };

    let tube_choices = [
        ("Off", "Direct video signal (no camera pickup tube stage)"),
        ("Plumbicon", "Standard tube (XQ1020): target lag and highlight comet tails"),
        ("Plumbicon+Bias", "Light-bias tube (XQ1410): reduced lag in shadows"),
        ("Plumbicon+ACT", "Anti-comet-tail gun (XQ1520): lag without persistent tails"),
    ];
    let tube_idx = ask_choice("Camera pickup tube simulation:", &tube_choices, 0)?;
    let (camera_tube_arg, iris_stops) = match tube_idx {
        1 => (Some(CameraTubeArg::Plumbicon), ask_iris_stops()?),
        2 => (Some(CameraTubeArg::PlumbiconBias), ask_iris_stops()?),
        3 => (Some(CameraTubeArg::PlumbiconAct), ask_iris_stops()?),
        _ => (None, None),
    };

    let audio_choices = [
        ("Process", "Simulate linear audio track through tape and deck electronics"),
        ("Copy", "Copy original audio stream untouched"),
        ("None", "Mute audio in the output video"),
    ];
    let audio_idx = ask_choice("Audio mode:", &audio_choices, 0)?;
    let audio_arg = match audio_idx {
        0 => AudioMode::Process,
        1 => AudioMode::Copy,
        _ => AudioMode::None,
    };

    let generations = ask_generations()?;

    let mut sets = Vec::new();
    if speed_name != "sp" {
        // through the condition, which derives the head width and the sub-emphasis from the speed
        sets.push(format!("{CONDITION_TABLE}.tape_speed={speed_name}"));
    }

    let mut cmd_parts = vec!["vhsphys".to_string(), "run".to_string()];
    cmd_parts.push(input_path.display().to_string());
    cmd_parts.push(output_path.display().to_string());
    if standard_arg != StandardArg::Ntsc {
        cmd_parts.push("--standard".to_string());
        cmd_parts.push(
            match standard_arg {
                StandardArg::Ntsc => "ntsc",
                StandardArg::Pal => "pal",
                StandardArg::Pal60 => "pal60",
            }
            .to_string(),
        );
    }
    if preset_name != "worn" {
        cmd_parts.push("--preset".to_string());
        cmd_parts.push(preset_name.clone());
    }
    for s in &sets {
        cmd_parts.push("--set".to_string());
        cmd_parts.push(s.clone());
    }
    if let Some(tube) = camera_tube_arg {
        let t_str = match tube {
            CameraTubeArg::Off => "off",
            CameraTubeArg::Plumbicon => "plumbicon",
            CameraTubeArg::PlumbiconBias => "plumbicon-bias",
            CameraTubeArg::PlumbiconAct => "plumbicon-act",
        };
        cmd_parts.push("--camera-tube".to_string());
        cmd_parts.push(t_str.to_string());
    }
    if let Some(stops) = iris_stops {
        cmd_parts.push("--iris-stops".to_string());
        cmd_parts.push(stops.to_string());
    }
    if audio_arg != AudioMode::Process {
        let a_str = match audio_arg {
            AudioMode::Process => "process",
            AudioMode::Copy => "copy",
            AudioMode::None => "none",
        };
        cmd_parts.push("--audio".to_string());
        cmd_parts.push(a_str.to_string());
    }
    if generations > 1 {
        cmd_parts.push("--generations".to_string());
        cmd_parts.push(generations.to_string());
    }

    let full_cmd = cmd_parts.iter().map(|part| shell_quote(part)).collect::<Vec<_>>().join(" ");
    println!("\n{}", "=".repeat(60));
    println!("Generated command:\n  {}", full_cmd);
    println!("{}\n", "=".repeat(60));

    let proceed = ask_line("Run this command now? (y/n)", Some("y"))?;
    if proceed.eq_ignore_ascii_case("y") || proceed.eq_ignore_ascii_case("yes") {
        println!("\nStarting run...\n");
        let run_args = RunArgs {
            input: input_path,
            output: output_path,
            preset: preset_name,
            config: None,
            sets,
            standard: Some(standard_arg),
            seed: 1,
            generations,
            audio: audio_arg,
            camera_tube: camera_tube_arg,
            iris_stops,
            start: None,
            duration: None,
            frames: None,
            size: None,
            crf: 18,
            threads: None,
            batch: 8,
            bypass: false,
            automate: None,
            ffmpeg: "ffmpeg".to_string(),
            ffprobe: "ffprobe".to_string(),
        };
        run(run_args)?;
    } else {
        println!("Setup completed. You can run the generated command above when ready.");
    }
    Ok(())
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Run(a) => run(a),
        Cmd::Wizard { input, output } => wizard(input, output),
        Cmd::Presets => {
            println!("Condition presets (use with --preset):\n");
            let width = Condition::preset_names().iter().map(|n| n.len()).max().unwrap_or(0);
            for n in Condition::preset_names() {
                let c = Condition::preset(n).unwrap();
                let outside = outside_description(&c);
                let outside = if outside.is_empty() { outside } else { format!("  outside: {outside}") };
                println!(
                    "  {n:width$}  tape {:>2.0} y (quality {:.2}, stretch {:.2})  heads wear {:.2} clog {:.2}  deck {:>2.0} y at {:.0} C  tracking {:.0} um{outside}",
                    c.tape_age_years, c.tape_quality, c.tape_stretch, c.head_wear, c.head_clog, c.deck_age_years, c.deck_temp_c, c.tracking_error_um
                );
            }
            Ok(())
        }
        Cmd::Show { preset, seed, condition, standard } => {
            let standard = standard.standard();
            if standard.is_625() {
                show::<Pal625>(standard, &preset, seed, condition)
            } else {
                show::<Ntsc525>(standard, &preset, seed, condition)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_engine_runs_at_its_own_frame_rate() {
        assert_eq!(Ntsc525::FPS, (30000, 1001));
        assert_eq!(Pal625::FPS, (25, 1));
        assert_eq!((Ntsc525::ACTIVE_W, Ntsc525::ACTIVE_H), (754, 480));
        assert_eq!((Pal625::ACTIVE_W, Pal625::ACTIVE_H), (740, 576));
        for (num, den, rate) in [(Ntsc525::FPS.0, Ntsc525::FPS.1, Ntsc525::FRAME_RATE), (Pal625::FPS.0, Pal625::FPS.1, Pal625::FRAME_RATE)] {
            assert!((num as f64 / den as f64 - rate).abs() < 1e-9, "{num}/{den} against {rate}");
        }
    }

    #[test]
    fn a_whole_number_of_periods_gives_that_many_frames() {
        for fps in [Ntsc525::FPS, Pal625::FPS] {
            let (num, den) = fps;
            for n in 1..3000u64 {
                // what a caller computes in floating point from the frame count
                let seconds = n as f64 * den as f64 / num as f64;
                assert_eq!(frames_in(seconds, fps), n, "{n} frames at {num}/{den}");
                // what ffmpeg is asked for: at most n periods, and more than n - 1 of them
                let asked: f64 = frames_arg(n, fps).parse().unwrap();
                let periods = asked * num as f64 / den as f64;
                assert!(periods <= n as f64 + 1e-9 && periods > n as f64 - 1.0, "{n}: {periods}");
            }
        }
        // a plain duration goes to the nearest whole frame, and a tie to the larger count
        assert_eq!(frames_in(1.0, Pal625::FPS), 25);
        assert_eq!(frames_in(3.0, Ntsc525::FPS), 90);
        assert_eq!(frames_in(3.01, Ntsc525::FPS), 90);
        assert_eq!(frames_in(3.02, Ntsc525::FPS), 91);
        assert_eq!(frames_in(1.5, Pal625::FPS), 38);
        assert_eq!(frames_in(1.49, Pal625::FPS), 37);
        assert_eq!(frames_in(0.001, Pal625::FPS), 1);
        assert_eq!(frames_arg(25, Pal625::FPS), "1.000000");
        assert_eq!(frames_arg(1, Ntsc525::FPS), "0.033366");
    }

    #[test]
    fn a_seek_is_cut_not_rounded() {
        assert_eq!(seek_arg(1.0), "1.000000");
        assert_eq!(seek_arg(0.0), "0.000000");
        assert_eq!(seek_arg(-1.0), "0.000000");
        assert_eq!(seek_arg(1.0000009), "1.000000");
        assert_eq!(seek_arg(12.5), "12.500000");
        for k in 0..2000u32 {
            let t = k as f64 * 1001.0 / 30000.0;
            assert!(seek_arg(t).parse::<f64>().unwrap() <= t, "{t}");
        }
    }

    #[test]
    fn the_standard_comes_from_the_flag_then_set_then_the_file() {
        let file = |s: &str| toml::from_str::<toml::Table>(s).unwrap();
        let none: Vec<String> = Vec::new();
        let set = vec!["standard=pal60".to_string()];
        assert!(pick_standard(None, &none, &file("")).unwrap() == TvStandard::Ntsc);
        assert!(pick_standard(None, &none, &file("standard = \"pal\"")).unwrap() == TvStandard::Pal);
        assert!(pick_standard(None, &set, &file("standard = \"pal\"")).unwrap() == TvStandard::Pal60);
        assert!(pick_standard(Some(StandardArg::Ntsc), &set, &file("standard = \"pal\"")).unwrap() == TvStandard::Ntsc);
        assert!(pick_standard(None, &none, &file("standard = \"secam\"")).is_err());
        assert!(pick_standard(None, &none, &file("standard = 625")).is_err());
        assert!(pick_standard(None, &["standard=secam".to_string()], &file("")).is_err());
    }

    #[test]
    fn overrides_are_checked_like_the_config() {
        let sets = |s: &str| config::sets_to_toml(&[s.to_string()]).unwrap();
        assert!(Pal625::generations("worn", "standard = \"pal\"\n", 1, 1, &sets("speed=ep")).is_err());
        assert!(Pal625::generations("worn", "standard = \"pal\"\n", 1, 1, &sets("composite.enabled=true")).is_ok());
        assert!(Pal625::generations("worn", "standard = \"pal\"\n", 1, 1, &sets("tape.passes=3")).is_ok());
        assert!(Ntsc525::generations("worn", "", 1, 1, &sets("speed=ep")).is_ok());
        assert!(Ntsc525::generations("worn", "standard = \"pal60\"\n", 1, 1, &sets("composite.enabled=true")).is_ok());
        // an override of a table the file also has adds to it
        let file = "standard = \"pal\"\n[tape]\npasses = 2.0\n";
        assert!(Pal625::generations("worn", file, 1, 1, &sets("tape.defects_per_mm2=0.1")).is_ok());
    }

    #[test]
    fn each_engine_refuses_what_it_cannot_play() {
        let ep = "[condition]\ntape_speed = \"ep\"\n";
        assert!(Pal625::generations("worn", &format!("standard = \"pal\"\n{ep}"), 1, 1, "").is_err());
        assert!(Ntsc525::generations("worn", ep, 1, 1, "").is_ok());
        assert!(Pal625::generations("worn", "standard = \"pal\"\n[composite]\nenabled = true\n", 1, 1, "").is_ok());
        assert!(Ntsc525::generations("worn", "standard = \"pal60\"\n", 1, 1, "").is_ok());
        assert!(Ntsc525::generations("worn", "standard = \"pal\"\n", 1, 1, "").is_err());
    }

    #[test]
    fn wizard_subcommand_parses_arguments() {
        let cli = Cli::try_parse_from(["vhsphys", "wizard"]).unwrap();
        match cli.cmd {
            Cmd::Wizard { input, output } => {
                assert!(input.is_none());
                assert!(output.is_none());
            }
            _ => panic!("expected Wizard command"),
        }
        let cli2 = Cli::try_parse_from(["vhsphys", "wizard", "in.mp4", "out.mp4"]).unwrap();
        match cli2.cmd {
            Cmd::Wizard { input, output } => {
                assert_eq!(input, Some(PathBuf::from("in.mp4")));
                assert_eq!(output, Some(PathBuf::from("out.mp4")));
            }
            _ => panic!("expected Wizard command"),
        }
    }

    /// The parameters of generation 0 of a 525-line run with this config file and these `--set` items.
    fn run_params(file: &str, sets: &[&str]) -> <Ntsc525 as Engine>::Params {
        let sets: Vec<String> = sets.iter().map(|s| s.to_string()).collect();
        let (text, overrides, _) = assemble_config(file, &sets, None).unwrap();
        Ntsc525::generations("worn", &text, 1, 1, &overrides).unwrap().remove(0)
    }

    fn condition_of(text: &str) -> toml::Table {
        let table: toml::Table = toml::from_str(text).unwrap();
        table["condition"].as_table().unwrap().clone()
    }

    #[test]
    fn a_condition_set_gives_the_parameters_of_the_same_condition_in_a_file() {
        let sp = run_params("", &[]);
        for speed in ["lp", "ep"] {
            let via_set = run_params("", &[&format!("condition.tape_speed={speed}")]);
            let via_file = run_params(&format!("[condition]\ntape_speed = \"{speed}\"\n"), &[]);
            assert!(via_set.speed != sp.speed, "{speed}");
            assert!(via_set.speed == via_file.speed, "{speed}");
            assert_eq!(via_set.heads[0].width_um, via_file.heads[0].width_um, "{speed}");
            assert!(via_set.heads[0].width_um < sp.heads[0].width_um, "{speed}");
            assert!(via_set.record.sub_emphasis.is_some() && sp.record.sub_emphasis.is_none(), "{speed}");
            assert_eq!(via_set.record.sub_emphasis.is_some(), via_file.record.sub_emphasis.is_some(), "{speed}");
            assert_eq!(via_set.record.carrier_interleave_hz, via_file.record.carrier_interleave_hz, "{speed}");
            assert!(via_set.record.carrier_interleave_hz != sp.record.carrier_interleave_hz, "{speed}");
            assert_eq!(via_set.record.burst_boost, via_file.record.burst_boost, "{speed}");
        }
        // the top-level `speed` alone leaves what the condition derives from the speed at SP
        let only_speed = run_params("", &["speed=lp"]);
        assert!(only_speed.speed != sp.speed && only_speed.heads[0].width_um == sp.heads[0].width_um);
    }

    #[test]
    fn a_condition_set_is_merged_into_the_condition_table_of_the_file() {
        let file = "[condition]\ntape_speed = \"ep\"\nhead_wear = 0.9\n[tape]\npasses = 2.0\n";
        let sets = vec!["condition.tape_speed=lp".to_string(), "condition.deck_age_years=30".to_string(), "tape.defects_per_mm2=0.1".to_string()];
        let (text, overrides, _) = assemble_config(file, &sets, None).unwrap();
        let condition = condition_of(&text);
        assert_eq!(condition["tape_speed"].as_str(), Some("lp"), "a --set wins over the file");
        assert_eq!(condition["head_wear"].as_float(), Some(0.9), "the file's other keys stay");
        assert_eq!(condition["deck_age_years"].as_integer(), Some(30));
        let table: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(table["tape"]["passes"].as_float(), Some(2.0));
        // only the items that are not for the condition are left as overrides
        assert_eq!(overrides, "tape.defects_per_mm2 = 0.1\n");
        // without a `[condition]` in the file the table is created
        let (text, overrides, _) = assemble_config("[tape]\npasses = 2.0\n", &["condition.tape_speed=ep".to_string()], None).unwrap();
        assert_eq!(condition_of(&text)["tape_speed"].as_str(), Some("ep"));
        assert_eq!(overrides, "");
        // a `condition` that is not a table cannot take the items
        assert!(assemble_config("condition = 5\n", &["condition.tape_speed=lp".to_string()], None).is_err());
    }

    #[test]
    fn seed_and_standard_sets_go_into_the_text_and_the_rest_stay_overrides() {
        let file = "[tape]\ndefects_per_mm2 = 0.1\n";
        let sets = vec!["seed=7".to_string(), "standard=pal60".to_string(), "tape.passes=3".to_string()];
        let (text, overrides, standard) = assemble_config(file, &sets, None).unwrap();
        assert!(standard == TvStandard::Pal60);
        let table: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(table["seed"].as_integer(), Some(7));
        assert_eq!(table["standard"].as_str(), Some("pal60"));
        assert_eq!(table["tape"]["defects_per_mm2"].as_float(), Some(0.1));
        assert!(!table.contains_key(CONDITION_TABLE));
        assert_eq!(overrides, "tape.passes = 3\n");
        // the flag wins over a `--set standard`, and the text says so
        let (text, _, standard) = assemble_config("", &["standard=pal60".to_string()], Some(StandardArg::Ntsc)).unwrap();
        assert!(standard == TvStandard::Ntsc);
        assert_eq!(toml::from_str::<toml::Table>(&text).unwrap()["standard"].as_str(), Some("ntsc"));
        // nothing for the text: the file is passed on as it is, comments and all
        let file = "# my tape\n[tape]\npasses = 2.0\n";
        let (text, overrides, standard) = assemble_config(file, &["tape.defects_per_mm2=0.2".to_string()], None).unwrap();
        assert_eq!((text.as_str(), overrides.as_str()), (file, "tape.defects_per_mm2 = 0.2\n"));
        assert!(standard == TvStandard::Ntsc);
        assert!(assemble_config("", &["standard=secam".to_string()], None).is_err());
    }

    #[test]
    fn a_missing_automation_file_is_reported_before_anything_is_started() {
        // the programs do not exist: only a check that comes before probing and encoding reports the file
        let cli = Cli::try_parse_from([
            "vhsphys",
            "run",
            "in.mp4",
            "out.mp4",
            "--automate",
            "/nonexistent/automation.json",
            "--ffmpeg",
            "/nonexistent/ffmpeg",
            "--ffprobe",
            "/nonexistent/ffprobe",
        ])
        .unwrap();
        let Cmd::Run(a) = cli.cmd else { panic!("expected Run command") };
        let err = run_engine::<Ntsc525>(a, TvStandard::Ntsc, "", "").unwrap_err().to_string();
        assert!(err.starts_with("loading automation file '/nonexistent/automation.json'"), "{err}");
    }

    #[test]
    fn printed_arguments_are_quoted_for_a_shell() {
        assert_eq!(shell_quote("clip_vhs.mp4"), "clip_vhs.mp4");
        assert_eq!(shell_quote("condition.tape_speed=lp"), "condition.tape_speed=lp");
        assert_eq!(shell_quote("/mnt/tapes/a-b,c@d%e:f+g.mp4"), "/mnt/tapes/a-b,c@d%e:f+g.mp4");
        assert_eq!(shell_quote("/tmp/my clip.webm"), "'/tmp/my clip.webm'");
        assert_eq!(shell_quote("it's.mp4"), "'it'\\''s.mp4'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a;b$c"), "'a;b$c'");
        // what a shell makes of the quoted word is the word
        #[cfg(unix)]
        for arg in ["clip_vhs.mp4", "/tmp/my clip.webm", "it's.mp4", "a;b$c \"d\" `e`", ""] {
            let out = Command::new("sh").arg("-c").arg(format!("printf %s {}", shell_quote(arg))).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), arg);
        }
    }

    #[test]
    fn the_default_output_keeps_only_an_extension_the_run_can_write() {
        for (input, output) in [
            ("clip.mp4", "clip_vhs.mp4"),
            ("clip.mkv", "clip_vhs.mkv"),
            ("clip.mov", "clip_vhs.mov"),
            ("clip.m4v", "clip_vhs.m4v"),
            ("clip.MKV", "clip_vhs.MKV"),
            ("photo.png", "photo_vhs.png"),
            ("photo.JPEG", "photo_vhs.JPEG"),
            ("photo.tif", "photo_vhs.tif"),
            ("clip.webm", "clip_vhs.mp4"),
            ("clip.avi", "clip_vhs.mp4"),
            ("clip.WEBM", "clip_vhs.mp4"),
            ("clip", "clip_vhs.mp4"),
            ("some/dir/clip.ogv", "clip_vhs.mp4"),
            ("some/dir/clip.mov", "clip_vhs.mov"),
        ] {
            assert_eq!(default_output_name(Path::new(input)), output, "{input}");
        }
        assert!(has_extension(Path::new("a.PNG"), &IMAGE_EXTENSIONS) && !has_extension(Path::new("a.mp4"), &IMAGE_EXTENSIONS));
        assert!(!has_extension(Path::new("png"), &IMAGE_EXTENSIONS));
    }

    #[test]
    fn the_input_is_not_taken_for_a_different_output() {
        let dir = std::env::temp_dir().join(format!("vhsphys-cli-same-file-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let input = dir.join("in.mp4");
        let other = dir.join("other.mp4");
        std::fs::write(&input, b"x").unwrap();
        std::fs::write(&other, b"y").unwrap();
        assert!(same_file(&input, &input));
        // the same file under another spelling
        assert!(same_file(&input, &dir.join("sub").join("..").join("in.mp4")));
        assert!(!same_file(&input, &other));
        // an output that does not exist is not the input
        assert!(!same_file(&input, &dir.join("new.mp4")));
        #[cfg(unix)]
        {
            let link = dir.join("link.mp4");
            std::os::unix::fs::symlink(&input, &link).unwrap();
            assert!(same_file(&input, &link));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wizard_numbers_must_be_numbers() {
        assert_eq!(parse_iris_stops("2.0"), Some(2.0));
        assert_eq!(parse_iris_stops("-1.5"), Some(-1.5));
        assert_eq!(parse_iris_stops("0"), Some(0.0));
        for bad in ["", "two", "1,5", "inf", "-inf", "NaN"] {
            assert_eq!(parse_iris_stops(bad), None, "{bad:?}");
        }
        assert_eq!(parse_generations("1"), Some(1));
        assert_eq!(parse_generations(&MAX_GENERATIONS.to_string()), Some(MAX_GENERATIONS));
        assert_eq!(parse_generations(WIZARD_GENERATIONS), Some(1));
        for bad in ["", "0", "-1", "2.5", "three", "256", &(MAX_GENERATIONS as u32 + 1).to_string()] {
            assert_eq!(parse_generations(bad), None, "{bad:?}");
        }
        assert_eq!(parse_iris_stops(WIZARD_IRIS_STOPS), Some(2.0));
    }
}
