//! The sound of a run: probe the input's audio, play it through the same tapes and decks as
//! the picture, and leave the result in a temporary WAV file for the encoder.
//!
//! The sound is decoded and processed in blocks of about one second, so the memory does not
//! grow with the length of the programme.

use anyhow::{bail, Context, Result};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use vhsphys_core::audio::AudioProcessor;
use vhsphys_core::pipeline::Params;

/// Sample rate of the processed sound.
pub const RATE: u32 = 48_000;
/// Frames (samples per channel) decoded and processed per block.
const BLOCK: usize = RATE as usize;
const WAV_HEADER: u64 = 44;

/// Channel count and sample rate of the input's first audio stream, or `None` without one.
pub fn probe(ffprobe: &str, input: &Path) -> Result<Option<(usize, u32)>> {
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-select_streams", "a:0", "-show_entries", "stream=channels,sample_rate", "-of", "default=nw=1"])
        .arg(input)
        .output()
        .context("running ffprobe")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (mut channels, mut rate) = (None, None);
    for line in text.lines() {
        match line.trim().split_once('=') {
            Some(("channels", v)) => channels = v.parse().ok(),
            Some(("sample_rate", v)) => rate = v.parse().ok(),
            _ => {}
        }
    }
    Ok(match (channels, rate) {
        (Some(c), Some(r)) if c > 0 => Some((c, r)),
        _ => None,
    })
}

/// A file in the temporary directory that is removed when this value is dropped, so it
/// goes away on success and on every error path alike.
pub struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn create(stem: &str, ext: &str) -> Result<(Self, File)> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let path = std::env::temp_dir().join(format!("{stem}-{}-{nanos}.{ext}", std::process::id()));
        let file = OpenOptions::new().write(true).create_new(true).open(&path).with_context(|| format!("creating {}", path.display()))?;
        Ok((Self { path }, file))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
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

/// 16-bit PCM WAV writer; the sizes in the header are filled in by `finish`.
struct WavWriter {
    out: BufWriter<File>,
    channels: u16,
    data_bytes: u64,
    buf: Vec<u8>,
}

impl WavWriter {
    fn new(file: File, channels: usize) -> Result<Self> {
        let mut w = Self { out: BufWriter::new(file), channels: channels as u16, data_bytes: 0, buf: Vec::new() };
        w.header(0)?;
        Ok(w)
    }

    fn header(&mut self, data_bytes: u32) -> Result<()> {
        let ch = self.channels;
        let align = ch * 2;
        let mut h = Vec::with_capacity(WAV_HEADER as usize);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&(36u32.wrapping_add(data_bytes)).to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&16u32.to_le_bytes());
        h.extend_from_slice(&1u16.to_le_bytes()); // integer PCM
        h.extend_from_slice(&ch.to_le_bytes());
        h.extend_from_slice(&RATE.to_le_bytes());
        h.extend_from_slice(&(RATE * align as u32).to_le_bytes());
        h.extend_from_slice(&align.to_le_bytes());
        h.extend_from_slice(&16u16.to_le_bytes());
        h.extend_from_slice(b"data");
        h.extend_from_slice(&data_bytes.to_le_bytes());
        self.out.write_all(&h)?;
        Ok(())
    }

    fn write(&mut self, samples: &[f32]) -> Result<()> {
        self.buf.clear();
        for &s in samples {
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            self.buf.extend_from_slice(&v.to_le_bytes());
        }
        self.data_bytes += self.buf.len() as u64;
        // the sizes in a RIFF header are 32-bit
        if self.data_bytes + WAV_HEADER > u32::MAX as u64 {
            bail!("the programme is too long for the processed sound; use --audio copy");
        }
        self.out.write_all(&self.buf)?;
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        let n = self.data_bytes as u32;
        self.out.seek(SeekFrom::Start(0))?;
        self.header(n)?;
        self.out.flush()?;
        Ok(())
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

/// Decode `channels` channels of the input's sound at 48 kHz from `t0` seconds (for at most
/// `duration` seconds), play them through the tapes and decks of `params` (one per
/// generation) and write the result to a temporary WAV file. `t0` must be the programme
/// time of the first video frame, so the sound starts in sync with the picture and the same
/// stretch of tape sounds the same whatever the start.
pub fn render(ffmpeg: &str, input: &Path, t0: f64, duration: Option<f64>, channels: usize, params: &[Params]) -> Result<TempFile> {
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

    let (tmp, file) = TempFile::create("vhsphys-sound", "wav")?;
    let result = (|| -> Result<()> {
        let mut wav = WavWriter::new(file, channels)?;
        let mut chain = SoundChain::new(params, channels, (t0 * RATE as f64).round() as i64);
        let frame_bytes = 4 * channels;
        let mut raw = vec![0u8; BLOCK * frame_bytes];
        let mut block = Vec::with_capacity(BLOCK * channels);
        loop {
            let n = read_full(&mut pipe, &mut raw)?;
            // a partial frame at the very end cannot be played
            let n = n - n % frame_bytes;
            if n == 0 {
                break;
            }
            block.clear();
            block.extend(raw[..n].as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)));
            wav.write(&chain.push(&block))?;
        }
        wav.write(&chain.finish())?;
        wav.finish()
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
    use vhsphys_core::config;

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

    /// Starting later in the programme plays the same stretch of tape: once the filters have
    /// settled, the sound matches the sound of a run from the beginning.
    #[test]
    fn a_later_start_sounds_the_same() {
        let params = config::generation_params(Some("worn"), "", 3, 1).unwrap();
        let ch = 2;
        let (frames, k) = (60_000, 20_000);
        let x: Vec<f32> = (0..frames * ch).map(|i| 0.3 * ((i / ch) as f32 * 0.05).sin()).collect();

        let mut whole = SoundChain::new(&params, ch, 0);
        let mut a = whole.push(&x);
        a.extend(whole.finish());
        let mut late = SoundChain::new(&params, ch, k as i64);
        let mut b = late.push(&x[k * ch..]);
        b.extend(late.finish());

        assert_eq!(b.len(), (frames - k) * ch);
        let settle = 4_800 * ch;
        let (a, b) = (&a[k * ch + settle..], &b[settle..]);
        let err = a.iter().zip(b).map(|(p, q)| (p - q).powi(2)).sum::<f32>() / a.iter().map(|p| p * p).sum::<f32>();
        assert!(err < 1e-6, "relative error {err}");
    }
}
