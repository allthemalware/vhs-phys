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
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=8))]
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
}

/// The processed sound, in a temporary file that goes away with this value.
enum SoundFile {
    /// A WAV file: the encoder reads it as it is.
    Wav(audio_io::TempFile),
    /// Raw 16-bit samples with this many channels at the sound's sample rate.
    Raw(pal_sound::RawSound, usize),
}

impl SoundFile {
    /// Add the sound as the next input of an encoder command.
    fn add_input(&self, enc: &mut Command) {
        let path = match self {
            SoundFile::Wav(f) => f.path(),
            SoundFile::Raw(f, channels) => {
                enc.args(["-f", "s16le", "-ar", &audio_io::RATE.to_string(), "-ac", &channels.to_string()]);
                f.path()
            }
        };
        enc.arg("-i").arg(path);
    }
}

fn sound_of_ntsc(ffmpeg: &str, input: &Path, t0: f64, duration: Option<f64>, channels: usize, stages: &[ntsc::pipeline::Params]) -> Result<SoundFile> {
    audio_io::render(ffmpeg, input, t0, duration, channels, stages).map(SoundFile::Wav)
}

fn sound_of_pal(ffmpeg: &str, input: &Path, t0: f64, duration: Option<f64>, channels: usize, stages: &[pal::pipeline::Params]) -> Result<SoundFile> {
    pal_sound::render(ffmpeg, input, t0, duration, channels, stages).map(|raw| SoundFile::Raw(raw, channels))
}

/// Implements `Engine` (and `Playback` for its chain) for the engine module `$core`.
macro_rules! engine {
    ($name:ident, $core:ident, $num:literal / $den:literal, $sound:path) => {
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
                $sound(ffmpeg, input, t0, duration, channels, stages)
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
        }
    };
}

engine!(Ntsc525, ntsc, 30000 / 1001, sound_of_ntsc);
engine!(Pal625, pal, 25 / 1, sound_of_pal);

/// The sound of a 625-line run. `audio_io` plays the sound through the 525-line engine's decks;
/// this is the same chain on the 625-line engine's, whose tape moves at the PAL speed, so the
/// hiss and the loss at high frequencies are those of a PAL tape. It leaves raw 16-bit samples
/// for the encoder, which makes it shorter than a WAV file with its header.
mod pal_sound {
    use super::audio_io::RATE;
    use anyhow::{bail, Context, Result};
    use std::fs::OpenOptions;
    use std::io::{BufWriter, Read, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use vhsphys_core::pal::audio::AudioProcessor;
    use vhsphys_core::pal::pipeline::Params;

    /// Frames (samples per channel) decoded and processed per block, as in `audio_io`.
    const BLOCK: usize = RATE as usize;

    /// A file of raw samples in the temporary directory, removed when this is dropped, so it goes
    /// away on success and on every error path alike.
    pub struct RawSound {
        path: PathBuf,
    }

    impl RawSound {
        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for RawSound {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// One generation of the sound: its processor and the output samples still to be dropped.
    struct Stage {
        proc: AudioProcessor,
        /// Absolute sample index of the next input frame.
        pos: i64,
        /// Output frames still to discard: the first `latency()` frames hold the empty tape.
        skip: usize,
    }

    /// The generations of the sound in series, each one compensated for its own latency, so
    /// every generation receives and returns a stream aligned with the absolute sample index.
    struct SoundChain {
        stages: Vec<Stage>,
        channels: usize,
    }

    impl SoundChain {
        fn new(params: &[Params], channels: usize, start: i64) -> Self {
            let stages = params
                .iter()
                .map(|p| {
                    let proc = AudioProcessor::new(p, RATE as f64, channels);
                    let skip = proc.latency();
                    Stage { proc, pos: start, skip }
                })
                .collect();
            Self { stages, channels }
        }

        /// Run `x` through the generations from `first` on.
        fn run_from(&mut self, first: usize, mut x: Vec<f32>) -> Vec<f32> {
            let ch = self.channels;
            for st in &mut self.stages[first..] {
                let frames = x.len() / ch;
                let y = st.proc.process(&x, st.pos);
                st.pos += frames as i64;
                let drop = st.skip.min(frames);
                st.skip -= drop;
                x = if drop == 0 { y } else { y[drop * ch..].to_vec() };
            }
            x
        }

        fn push(&mut self, block: &[f32]) -> Vec<f32> {
            self.run_from(0, block.to_vec())
        }

        /// The input has ended: feed each generation as many silent frames as its latency, in
        /// order, so that every generation returns exactly as many frames as it was given.
        fn finish(&mut self) -> Vec<f32> {
            let mut out = Vec::new();
            for k in 0..self.stages.len() {
                let tail = vec![0.0; self.stages[k].proc.latency() * self.channels];
                out.extend(self.run_from(k, tail));
            }
            out
        }
    }

    /// Fill `buf` from `r` as far as the stream allows; returns the number of bytes read.
    fn read_full(r: &mut impl Read, buf: &mut [u8]) -> Result<usize> {
        let mut got = 0;
        while got < buf.len() {
            match r.read(&mut buf[got..])? {
                0 => break,
                n => got += n,
            }
        }
        Ok(got)
    }

    fn write_samples(out: &mut impl Write, bytes: &mut Vec<u8>, samples: &[f32]) -> Result<()> {
        bytes.clear();
        for &s in samples {
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        out.write_all(bytes)?;
        Ok(())
    }

    /// As `audio_io::render`, on the 625-line engine: decode `channels` channels of the input's
    /// sound at 48 kHz from `t0` seconds (for at most `duration` seconds), play them through the
    /// tapes and decks of `params` and leave the result in a temporary file of 16-bit samples.
    /// `t0` must be the programme time of the first video frame, so the sound starts in sync with
    /// the picture and the same stretch of tape sounds the same whatever the start.
    pub fn render(ffmpeg: &str, input: &Path, t0: f64, duration: Option<f64>, channels: usize, params: &[Params]) -> Result<RawSound> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let path = std::env::temp_dir().join(format!("vhsphys-sound-{}-{nanos}.raw", std::process::id()));
        let file = OpenOptions::new().write(true).create_new(true).open(&path).with_context(|| format!("creating {}", path.display()))?;
        let tmp = RawSound { path };

        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-v", "error"]);
        if t0 > 0.0 {
            cmd.args(["-ss", &format!("{t0:.6}")]);
        }
        cmd.arg("-i").arg(input);
        if let Some(d) = duration {
            cmd.args(["-t", &format!("{d:.6}")]);
        }
        cmd.args(["-vn", "-ac", &channels.to_string(), "-ar", &RATE.to_string(), "-f", "f32le", "pipe:1"]);
        let mut dec = cmd.stdout(Stdio::piped()).spawn().context("starting ffmpeg (sound decoder)")?;
        let mut pipe = dec.stdout.take().unwrap();

        let result = (|| -> Result<()> {
            let mut out = BufWriter::new(file);
            let mut chain = SoundChain::new(params, channels, (t0 * RATE as f64).round() as i64);
            let frame_bytes = 4 * channels;
            let mut raw = vec![0u8; BLOCK * frame_bytes];
            let mut block = Vec::with_capacity(BLOCK * channels);
            let mut bytes = Vec::new();
            loop {
                let n = read_full(&mut pipe, &mut raw)?;
                // a partial frame at the very end cannot be played
                let n = n - n % frame_bytes;
                if n == 0 {
                    break;
                }
                block.clear();
                block.extend(raw[..n].as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)));
                write_samples(&mut out, &mut bytes, &chain.push(&block))?;
            }
            write_samples(&mut out, &mut bytes, &chain.finish())?;
            out.flush()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = dec.kill();
        }
        let status = dec.wait()?;
        result?;
        if !status.success() {
            bail!("ffmpeg could not decode the sound");
        }
        Ok(tmp)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use vhsphys_core::pal::config;

        /// Splitting the input into blocks must not change the sound, and the latency
        /// compensation must return exactly as many frames as went in, for any number of
        /// generations.
        #[test]
        fn blocks_and_generations_keep_length_and_content() {
            let params = config::generation_params(Some("worn"), "", 3, 2).unwrap();
            let ch = 2;
            let frames = 30_000;
            let x: Vec<f32> = (0..frames * ch).map(|i| 0.3 * ((i / ch) as f32 * 0.05).sin()).collect();

            let mut whole = SoundChain::new(&params, ch, 1234);
            let mut a = whole.push(&x);
            a.extend(whole.finish());

            let mut split = SoundChain::new(&params, ch, 1234);
            let mut b = Vec::new();
            for part in x.chunks(7_001 * ch) {
                b.extend(split.push(part));
            }
            b.extend(split.finish());

            assert_eq!(a.len(), x.len());
            assert_eq!(a, b);
        }
    }
}

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

/// Merge `over` into `base` table by table: a value in `over` replaces the one in `base`.
fn merge_into(base: &mut toml::Table, over: toml::Table) {
    for (key, value) in over {
        if let toml::Value::Table(o) = value {
            if let Some(toml::Value::Table(b)) = base.get_mut(&key) {
                merge_into(b, o);
            } else {
                base.insert(key, toml::Value::Table(o));
            }
        } else {
            base.insert(key, value);
        }
    }
}

/// The `--set` overrides are applied after the engine has built the parameters, and so after its
/// check of what it can play (composite video into a PAL television, EP at 625 lines). Build once
/// more from the config text with the overrides merged in, for the same check to see them too.
fn check_overrides<E: Engine>(preset: &str, text: &str, sets: &str, seed: u64) -> Result<()> {
    let mut table: toml::Table = toml::from_str(text)?;
    let mut over: toml::Table = toml::from_str(sets)?;
    // an override of the `condition` is ignored when the overrides are applied, so it is here
    over.remove("condition");
    merge_into(&mut table, over);
    E::generations(preset, &toml::to_string(&table)?, seed, 1, "").map(drop).map_err(anyhow::Error::msg)
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
    // other `--set` items apply to every generation as they are. So does `--set standard=...`
    // and for the same reason: the standard decides which engine reads the config, and the
    // engine refuses what it cannot play (EP at 625 lines, composite video into a PAL set), which
    // only happens for a standard that is in the text.
    let (seed_sets, other_sets) = take_key(a.sets.clone(), "seed");
    let (standard_sets, mut other_sets) = take_key(other_sets, "standard");
    if let Some(tube) = a.camera_tube {
        let name = match tube {
            CameraTubeArg::Off => "off",
            CameraTubeArg::Plumbicon => "plumbicon",
            CameraTubeArg::PlumbiconBias => "plumbicon-bias",
            CameraTubeArg::PlumbiconAct => "plumbicon-act",
        };
        other_sets.insert(0, format!("camera.tube={name}"));
    }
    if let Some(stops) = a.iris_stops {
        other_sets.insert(0, format!("camera.iris_stops={stops:?}"));
    }
    let mut table: toml::Table = toml::from_str(&file_text).map_err(|e| anyhow!("config: {e}"))?;
    let standard = pick_standard(a.standard, &standard_sets, &table)?;
    let mut edited = false;
    if !seed_sets.is_empty() {
        let seed_set: toml::Table = toml::from_str(&config::sets_to_toml(&seed_sets).map_err(anyhow::Error::msg)?)?;
        table.extend(seed_set);
        edited = true;
    }
    if a.standard.is_some() || !standard_sets.is_empty() {
        table.insert("standard".into(), toml::Value::String(standard.name().into()));
        edited = true;
    }
    let text = if edited { toml::to_string(&table)? } else { file_text };
    let sets = config::sets_to_toml(&other_sets).map_err(anyhow::Error::msg)?;
    if standard.is_625() {
        run_engine::<Pal625>(a, standard, &text, &sets)
    } else {
        run_engine::<Ntsc525>(a, standard, &text, &sets)
    }
}

fn run_engine<E: Engine>(a: RunArgs, standard: TvStandard, text: &str, sets: &str) -> Result<()> {
    debug_assert!((E::FPS.0 as f64 / E::FPS.1 as f64 - E::FRAME_RATE).abs() < 1e-9, "the frame rate fraction does not match the engine");
    let stages = E::generations(&a.preset, text, a.seed, a.generations as usize, sets).map_err(anyhow::Error::msg)?;
    if !sets.is_empty() {
        check_overrides::<E>(&a.preset, text, sets, a.seed)?;
    }

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
    let is_image = matches!(a.output.extension().and_then(|e| e.to_str()), Some("png" | "jpg" | "jpeg" | "bmp" | "tif" | "tiff"));
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
    let mut session = if a.bypass { None } else { Some(E::chain(stages, start_frame)) };
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

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Run(a) => run(a),
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
        assert!(check_overrides::<Pal625>("worn", "standard = \"pal\"\n", &sets("speed=ep"), 1).is_err());
        assert!(check_overrides::<Pal625>("worn", "standard = \"pal\"\n", &sets("composite.enabled=true"), 1).is_err());
        assert!(check_overrides::<Pal625>("worn", "standard = \"pal\"\n", &sets("tape.passes=3"), 1).is_ok());
        assert!(check_overrides::<Ntsc525>("worn", "", &sets("speed=ep"), 1).is_ok());
        assert!(check_overrides::<Ntsc525>("worn", "standard = \"pal60\"\n", &sets("composite.enabled=true"), 1).is_err());
        // an override of a table the file also has adds to it
        let file = "standard = \"pal\"\n[tape]\npasses = 2.0\n";
        assert!(check_overrides::<Pal625>("worn", file, &sets("tape.defects_per_mm2=0.1"), 1).is_ok());
    }

    #[test]
    fn each_engine_refuses_what_it_cannot_play() {
        let ep = "[condition]\ntape_speed = \"ep\"\n";
        assert!(Pal625::generations("worn", &format!("standard = \"pal\"\n{ep}"), 1, 1, "").is_err());
        assert!(Ntsc525::generations("worn", ep, 1, 1, "").is_ok());
        assert!(Pal625::generations("worn", "standard = \"pal\"\n[composite]\nenabled = true\n", 1, 1, "").is_err());
        assert!(Ntsc525::generations("worn", "standard = \"pal60\"\n", 1, 1, "").is_ok());
        assert!(Ntsc525::generations("worn", "standard = \"pal\"\n", 1, 1, "").is_err());
    }
}
