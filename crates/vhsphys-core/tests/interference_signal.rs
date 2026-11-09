//! The outside world through the whole chain: the existing signal path (FM discriminator, AGC, sync
//! slicer, clamp, colour decoder, audio filters) reacts to what is added to it as a real set would.
//! Each test measures an effect that follows from the physics (a frequency, a period, a beat, a
//! ratio), not a look.

mod common;

use common::{flat_frame, fnv};
use std::f64::consts::PI;
use vhsphys_core::audio::AudioProcessor;
use vhsphys_core::config;
use vhsphys_core::consts::*;
use vhsphys_core::interference::*;
use vhsphys_core::interference_params::*;
use vhsphys_core::pipeline::{Params, Session};
use vhsphys_core::video::{row_to_line, test_pattern, Frame};

fn pristine(seed: u64) -> Params {
    config::build(Some("pristine"), "", seed).unwrap()
}

fn run(p: Params, frames: usize, grey: u8) -> Vec<Frame> {
    let input: Vec<Frame> = (0..frames).map(|_| flat_frame(grey)).collect();
    Session::run(p, &input)
}

fn luma(f: &Frame, x: usize, y: usize) -> f64 {
    let i = (y * f.w + x) * 3;
    0.299 * f.rgb[i] as f64 + 0.587 * f.rgb[i + 1] as f64 + 0.114 * f.rgb[i + 2] as f64
}

/// Time at which row `row` of frame `k` is scanned.
fn t_row(k: usize, row: usize) -> f64 {
    let (line, _) = row_to_line(row);
    (k as f64 * LINES as f64 + line as f64) / FH
}

/// Mean luma of a row over the middle of the picture.
fn row_mean(f: &Frame, y: usize) -> f64 {
    (60..f.w - 60).map(|x| luma(f, x, y)).sum::<f64>() / (f.w - 120) as f64
}

/// Amplitude and phase (of a sine at global time) of the component at `f` in the row means of `frames`
/// `first..last`, rows 40..440.
fn row_tone(frames: &[Frame], first: usize, last: usize, f: f64) -> (f64, f64) {
    let (mut samples, mut mean) = (Vec::new(), 0.0);
    for k in first..last {
        for y in 40..440 {
            let v = row_mean(&frames[k], y);
            samples.push((t_row(k, y), v));
            mean += v;
        }
    }
    mean /= samples.len() as f64;
    let (mut c, mut s) = (0.0, 0.0);
    for (t, v) in &samples {
        c += (v - mean) * (2.0 * PI * f * t).cos();
        s += (v - mean) * (2.0 * PI * f * t).sin();
    }
    let n = samples.len() as f64 / 2.0;
    ((c * c + s * s).sqrt() / n, s.atan2(c))
}

fn quiet(p: &mut InterferenceParams) {
    p.mains.phase = 0.0;
    p.shielding_db = 10.0;
}

// ------------------------------------------------------------------------------- off is off

#[test]
fn settings_without_a_source_change_nothing_in_picture_or_sound() {
    let mut loud = pristine(5);
    loud.interference.phone = PhoneParams { mode: PhoneMode::Off, distance_m: 0.03, ..PhoneParams::default() };
    loud.interference.radio = RadioParams { modulation: RadioMod::Off, freq_hz: 3.9e6, field_vpm: 10.0, depth: 1.0 };
    loud.interference.crt.magnet = CrtMagnet { kind: MagnetKind::Absent, z_m: 0.3, rho_m: 0.02, angle_deg: 30.0 };
    loud.interference.head_magnet = HeadMagnet { kind: MagnetKind::Absent, distance_m: 0.005, ..HeadMagnet::default() };
    loud.interference.mains.freq_hz = 57.3;
    loud.interference.shielding_db = 0.0;
    loud.interference.approach_s = 9.0;
    assert!(loud.interference.is_off());
    let input: Vec<Frame> = (0..5).map(|_| test_pattern()).collect();
    assert_eq!(fnv(&Session::run(pristine(5), &input)), fnv(&Session::run(loud.clone(), &input)));
    let tone: Vec<f32> = (0..24_000).map(|n| (0.3 * (2.0 * PI * 1000.0 * n as f64 / 48_000.0).sin()) as f32).collect();
    let a = AudioProcessor::new(&pristine(5), 48_000.0, 1).process(&tone, 0);
    let b = AudioProcessor::new(&loud, 48_000.0, 1).process(&tone, 0);
    assert_eq!(a, b);
}

// --------------------------------------------------------------------------- the mains

#[test]
fn a_ground_loop_puts_hum_bars_on_the_picture_at_the_mains_frequency() {
    let hum = |mv: f64, mains: f64| {
        let mut p = pristine(1);
        quiet(&mut p.interference);
        p.interference.mains.ground_loop_mv = mv;
        p.interference.mains.freq_hz = mains;
        run(p, 20, 180)
    };
    let strong = hum(400.0, 60.0);
    let at_mains = row_tone(&strong, 4, 16, 60.0).0;
    // 400 mV: 8.4 IRE of hum at the input, a gain modulation of 9.3 % at the fundamental: some 15 levels on a 70 IRE picture
    assert!((6.0..45.0).contains(&at_mains), "amplitude {at_mains:.2} levels");
    for f in [45.0, 52.0, 68.0, 75.0] {
        assert!(at_mains > 6.0 * row_tone(&strong, 4, 16, f).0, "{f} Hz: {:.2} vs {at_mains:.2}", row_tone(&strong, 4, 16, f).0);
    }
    // linear in the voltage between the grounds
    let half = row_tone(&hum(200.0, 60.0), 4, 16, 60.0).0;
    assert!((half / at_mains - 0.5).abs() < 0.12, "{}", half / at_mains);
    // a 50 Hz mains puts its bars at 50 Hz
    let fifty = hum(400.0, 50.0);
    assert!(row_tone(&fifty, 4, 16, 50.0).0 > 6.0 * row_tone(&fifty, 4, 16, 60.0).0);
}

#[test]
fn a_mains_off_nominal_makes_the_bars_crawl_up_or_down_the_picture() {
    // the video runs on its own crystal at 59.94 Hz; the bars sit at a fixed place in the frame only if the mains is a multiple of the frame rate
    let phases = |mains: f64| -> Vec<f64> {
        let mut p = pristine(1);
        quiet(&mut p.interference);
        p.interference.mains.ground_loop_mv = 500.0;
        p.interference.mains.freq_hz = mains;
        let frames = run(p, 14, 180);
        // the phase of the pattern within each frame (time measured from the start of the frame)
        (3..11)
            .map(|k| {
                let (mut c, mut s, mut mean, mut n) = (0.0, 0.0, 0.0, 0.0);
                for y in 40..440 {
                    mean += row_mean(&frames[k], y);
                    n += 1.0;
                }
                mean /= n;
                for y in 40..440 {
                    let t = t_row(k, y) - k as f64 * LINES as f64 / FH;
                    let v = row_mean(&frames[k], y) - mean;
                    c += v * (2.0 * PI * mains * t).cos();
                    s += v * (2.0 * PI * mains * t).sin();
                }
                s.atan2(c)
            })
            .collect()
    };
    let wrap = |a: f64| (a + PI).rem_euclid(2.0 * PI) - PI;
    for (mains, expect) in [(58.5, 2.0 * PI * 58.5 / FRAME_RATE), (61.5, 2.0 * PI * 61.5 / FRAME_RATE)] {
        let ph = phases(mains);
        // each frame the hum has advanced by f / frame rate cycles: the pattern moves by that much, modulo a whole cycle
        let want = wrap(-expect);
        for w in ph.windows(2) {
            let d = wrap(w[1] - w[0]);
            assert!((wrap(d - want)).abs() < 0.12, "mains {mains}: moved {d:.3} rad per frame, expected {want:.3}");
        }
    }
    // the two directions are opposite
    let (a, b) = (phases(58.5), phases(61.5));
    assert!(wrap(a[1] - a[0]) * wrap(b[1] - b[0]) < 0.0);
}

#[test]
fn a_dimmer_adds_pulses_to_the_picture_at_twice_the_mains_frequency() {
    let mut p = pristine(1);
    quiet(&mut p.interference);
    p.interference.mains.dimmer_angle_deg = 90.0;
    p.interference.mains.freq_hz = 60.0;
    let frames = run(p, 14, 140);
    // the rows hit by a ring are noisy: take the spread along each row as the signal
    let mut samples = Vec::new();
    for k in 3..11 {
        for y in 40..440 {
            let m = row_mean(&frames[k], y);
            let var = (60..frames[k].w - 60).map(|x| (luma(&frames[k], x, y) - m).powi(2)).sum::<f64>() / (frames[k].w - 120) as f64;
            samples.push((t_row(k, y), var.sqrt()));
        }
    }
    let mean = samples.iter().map(|s| s.1).sum::<f64>() / samples.len() as f64;
    let tone = |f: f64| {
        let (mut c, mut s) = (0.0, 0.0);
        for (t, v) in &samples {
            c += (v - mean) * (2.0 * PI * f * t).cos();
            s += (v - mean) * (2.0 * PI * f * t).sin();
        }
        (c * c + s * s).sqrt() / (samples.len() as f64 / 2.0)
    };
    let at_120 = tone(120.0);
    assert!(at_120 > 0.3, "{at_120}");
    for f in [60.0, 90.0, 150.0, 180.0] {
        assert!(at_120 > 3.0 * tone(f), "{f} Hz: {} vs {at_120}", tone(f));
    }
}

// ----------------------------------------------------------------------------- the phone

#[test]
fn a_gsm_phone_buzzes_the_picture_at_217_hz() {
    let buzz = |mode: PhoneMode, d: f64, shield: f64| {
        let mut p = pristine(1);
        quiet(&mut p.interference);
        p.interference.shielding_db = shield;
        p.interference.phone = PhoneParams { mode, distance_m: d, ..PhoneParams::default() };
        run(p, 14, 150)
    };
    let f1 = 1.0 / GSM_FRAME_S;
    let frames = buzz(PhoneMode::Gsm, 0.25, 10.0);
    let a1 = row_tone(&frames, 3, 11, f1).0;
    // 0.25 m, 10 dB of shielding: 9.8 V/m at the case, 10 IRE pulses (25 levels), a fundamental of a few levels
    assert!((1.5..25.0).contains(&a1), "{a1}");
    for f in [180.0, 250.0] {
        assert!(a1 > 5.0 * row_tone(&frames, 3, 11, f).0, "{f} Hz");
    }
    // the pulse train has harmonics at multiples of the frame rate
    let a2 = row_tone(&frames, 3, 11, 2.0 * f1).0;
    assert!(a2 > 2.5 * row_tone(&frames, 3, 11, 2.0 * f1 + 35.0).0, "second harmonic {a2}");
    // a phone three metres away behind 30 dB of shielding is below the noise
    let far = buzz(PhoneMode::Gsm, 3.0, 30.0);
    assert!(row_tone(&far, 3, 11, f1).0 < 0.4, "{}", row_tone(&far, 3, 11, f1).0);
    // closer is stronger by the square of the field: halving the distance quadruples the pulse
    let closer = buzz(PhoneMode::Gsm, 0.125, 10.0);
    let ratio = row_tone(&closer, 3, 11, f1).0 / a1;
    assert!((2.8..5.5).contains(&ratio), "{ratio}");
}

// ----------------------------------------------------------------------------- the radio

/// Magnitude spectrum along the rows (averaged over rows 100..400), by cycles per 754 pixels.
fn row_spectrum(f: &Frame) -> Vec<f64> {
    let (x0, x1) = (60usize, f.w - 60);
    let n = x1 - x0;
    let mut acc = vec![0.0f64; 90];
    for y in (100..400).step_by(3) {
        let v: Vec<f64> = (x0..x1).map(|x| luma(f, x, y)).collect();
        let mean = v.iter().sum::<f64>() / n as f64;
        for (c, a) in acc.iter_mut().enumerate().skip(1) {
            let cycles_in_window = c as f64 * n as f64 / f.w as f64;
            let (mut re, mut im) = (0.0, 0.0);
            for (i, s) in v.iter().enumerate() {
                let hann = 0.5 - 0.5 * (2.0 * PI * (i as f64 + 0.5) / n as f64).cos();
                let w = 2.0 * PI * cycles_in_window * i as f64 / n as f64;
                re += (s - mean) * hann * w.cos();
                im += (s - mean) * hann * w.sin();
            }
            *a += (re * re + im * im).sqrt() / 300.0 * 100.0 / 100.0;
        }
    }
    acc
}

fn peak(spec: &[f64], lo: usize, hi: usize) -> (usize, f64) {
    (lo..hi).map(|c| (c, spec[c])).fold((lo, 0.0), |a, b| if b.1 > a.1 { b } else { a })
}

#[test]
fn a_carrier_in_the_luma_band_beats_with_the_fm_carrier_at_the_offset_frequency() {
    let beat = |f_hz: f64, field: f64| {
        let mut p = pristine(1);
        quiet(&mut p.interference);
        p.interference.radio = RadioParams { modulation: RadioMod::Cw, freq_hz: f_hz, field_vpm: field, depth: 0.8 };
        let out = run(p, 8, 128);
        row_spectrum(&out[4])
    };
    // the FM carrier of a flat grey sits somewhere in 3.4 - 4.4 MHz; two interferers above it, 200 kHz apart,
    // make beats whose spatial frequencies differ by 0.2 MHz x the 52.7 us of the active line = 10.5 cycles
    let (s1, s2) = (beat(4.9e6, 8.0), beat(5.1e6, 8.0));
    let (c1, a1) = peak(&s1, 4, 80);
    let (c2, a2) = peak(&s2, 4, 80);
    let median = |s: &[f64]| {
        let mut v: Vec<f64> = s[4..80].to_vec();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    assert!(a1 > 5.0 * median(&s1) && a2 > 5.0 * median(&s2), "no clear tone: {a1} {a2} over {} {}", median(&s1), median(&s2));
    let dc = c2 as f64 - c1 as f64;
    assert!((dc - 10.5).abs() < 1.6, "the beat moved {dc} cycles per line ({c1} -> {c2}), expected 10.5");
    // a weaker field makes a proportionally weaker beat (small-signal linearity)
    let (_, a_half) = peak(&beat(4.9e6, 4.0), 4, 80);
    assert!((a_half / a1 - 0.5).abs() < 0.2, "{}", a_half / a1);
}

#[test]
fn an_am_carrier_in_the_colour_under_band_beats_with_the_colour_carrier() {
    // flat grey has no colour; an interferer near the 629 kHz colour-under carrier beats with it into a colour stripe pattern
    // whose spatial frequency is |f - 629 kHz| times the active line
    let beat = |f_hz: f64| {
        let mut p = pristine(1);
        quiet(&mut p.interference);
        p.interference.radio = RadioParams { modulation: RadioMod::Cw, freq_hz: f_hz, field_vpm: 8.0, depth: 0.8 };
        let out = run(p, 8, 128);
        // the colour error: red minus blue along the rows
        let f = &out[4];
        let mut tmp = f.clone();
        for y in 0..f.h {
            for x in 0..f.w {
                let i = (y * f.w + x) * 3;
                let d = (f.rgb[i] as i32 - f.rgb[i + 2] as i32).clamp(-128, 127) + 128;
                tmp.rgb[i] = d as u8;
                tmp.rgb[i + 1] = d as u8;
                tmp.rgb[i + 2] = d as u8;
            }
        }
        row_spectrum(&tmp)
    };
    let (s1, s2) = (beat(0.9e6), beat(1.1e6));
    let (c1, a1) = peak(&s1, 3, 80);
    let (c2, a2) = peak(&s2, 3, 80);
    let want1 = (0.9e6 - F_CU) * ACTIVE_W as f64 / FS_BB;
    let want2 = (1.1e6 - F_CU) * ACTIVE_W as f64 / FS_BB;
    assert!(a1 > 4.0 * s1[3..80].iter().sum::<f64>() / 77.0 && a2 > 4.0 * s2[3..80].iter().sum::<f64>() / 77.0, "no clear colour tone: {a1} {a2}");
    assert!((c1 as f64 - want1).abs() < 2.0, "colour beat at {c1} cycles per line, expected {want1:.1}");
    assert!((c2 as f64 - want2).abs() < 2.0, "colour beat at {c2} cycles per line, expected {want2:.1}");
}

// ---------------------------------------------------------------------------- the magnet

#[test]
fn a_magnet_near_the_head_dims_the_carrier_by_the_core_saturation_and_the_erasure() {
    let carrier = |kind: MagnetKind, d: f64| {
        let mut p = pristine(1);
        quiet(&mut p.interference);
        p.interference.head_magnet = HeadMagnet { kind, distance_m: d, ..HeadMagnet::default() };
        let mut s = Session::new(p);
        for _ in 0..16 {
            s.push(&flat_frame(128));
        }
        s.finish();
        s.debug_field(10).carrier_level as f64
    };
    let clean = carrier(MagnetKind::Absent, 0.05);
    assert!((0.8..1.25).contains(&clean), "{clean}");
    let mut p = InterferenceParams::default();
    p.head_magnet = HeadMagnet { kind: MagnetKind::Neodymium, distance_m: 0.06, ..HeadMagnet::default() };
    let want = head_signal_gain(&p, 0.0);
    let got = carrier(MagnetKind::Neodymium, 0.06) / clean;
    assert!((got / want - 1.0).abs() < 0.12, "{got:.3} vs the physics {want:.3}");
    assert!(carrier(MagnetKind::Neodymium, 0.03) / clean < 0.25);
    assert!((carrier(MagnetKind::Speaker, 0.30) / clean - 1.0).abs() < 0.06);
}

// ------------------------------------------------------------------------------- the sound

const RATE: f64 = 48_000.0;

fn amp_at(x: &[f32], f: f64) -> f64 {
    let (mut re, mut im) = (0.0, 0.0);
    for (n, v) in x.iter().enumerate() {
        let w = 2.0 * PI * f * n as f64 / RATE;
        re += *v as f64 * w.cos();
        im += *v as f64 * w.sin();
    }
    2.0 * (re * re + im * im).sqrt() / x.len() as f64
}

fn sound(p: &Params, x: &[f32]) -> Vec<f32> {
    AudioProcessor::new(p, RATE, 1).process(x, 0)
}

#[test]
fn a_gsm_phone_buzzes_the_sound_with_the_spectrum_of_a_pulse_train() {
    let mut p = pristine(1);
    quiet(&mut p.interference);
    p.interference.phone = PhoneParams { mode: PhoneMode::Gsm, distance_m: 0.3, ..PhoneParams::default() };
    let y = sound(&p, &vec![0.0; 96_000]);
    let y = &y[24_000..];
    let f1 = 1.0 / GSM_FRAME_S;
    let a1 = amp_at(y, f1);
    // 0.3 m, 10 dB of shielding: 8.2 V/m at the case, 53 mFS pulses, a fundamental near 12 mFS
    assert!((0.005..0.03).contains(&a1), "{a1}");
    for f in [180.0, 250.0, 300.0] {
        assert!(a1 > 8.0 * amp_at(y, f), "{f} Hz");
    }
    // the spectrum of a pulse train of duty 0.119 has its first null between the 8th and 9th harmonic
    assert!(amp_at(y, 8.0 * f1) < 0.2 * a1, "8th harmonic {}", amp_at(y, 8.0 * f1));
    assert!(amp_at(y, 2.0 * f1) > 0.5 * a1, "2nd harmonic {}", amp_at(y, 2.0 * f1));
}

#[test]
fn the_mains_hums_in_the_sound_through_the_ground_and_the_stray_field() {
    let hum = |f: &dyn Fn(&mut InterferenceParams)| {
        let mut p = pristine(1);
        quiet(&mut p.interference);
        f(&mut p.interference);
        let y = sound(&p, &vec![0.0; 96_000]);
        y[24_000..].to_vec()
    };
    let ground = hum(&|i| i.mains.ground_loop_mv = 200.0);
    // 30 mV rms after the equaliser: 41 mFS at the fundamental
    assert!((0.03..0.06).contains(&amp_at(&ground, 60.0)), "{}", amp_at(&ground, 60.0));
    assert!(amp_at(&ground, 60.0) > 8.0 * amp_at(&ground, 70.0));
    assert!(amp_at(&ground, 180.0) > 0.15 * amp_at(&ground, 60.0), "the third harmonic is part of the mains");
    let fifty = hum(&|i| {
        i.mains.ground_loop_mv = 200.0;
        i.mains.freq_hz = 50.0;
    });
    assert!(amp_at(&fifty, 50.0) > 8.0 * amp_at(&fifty, 60.0));
    // a stray field induces the hum in the head: stronger in the harmonics (it is the derivative of the flux)
    let stray = hum(&|i| i.mains.stray_field_ut = 2.0);
    assert!(amp_at(&stray, 60.0) > 0.02, "{}", amp_at(&stray, 60.0));
    assert!(amp_at(&stray, 60.0) > 8.0 * amp_at(&stray, 70.0));
    assert!(amp_at(&stray, 180.0) / amp_at(&stray, 60.0) > amp_at(&ground, 180.0) / amp_at(&ground, 60.0), "induction emphasises the harmonics");
}

#[test]
fn a_magnet_near_the_audio_head_lowers_the_signal_by_the_same_physics() {
    let level = |kind: MagnetKind, d: f64| {
        let mut p = pristine(1);
        quiet(&mut p.interference);
        p.interference.head_magnet = HeadMagnet { kind, distance_m: d, ..HeadMagnet::default() };
        let tone: Vec<f32> = (0..96_000).map(|n| (0.2 * (2.0 * PI * 1000.0 * n as f64 / RATE).sin()) as f32).collect();
        amp_at(&sound(&p, &tone)[24_000..], 1000.0)
    };
    let clean = level(MagnetKind::Absent, 0.05);
    let mut p = InterferenceParams::default();
    p.head_magnet = HeadMagnet { kind: MagnetKind::Neodymium, distance_m: 0.06, ..HeadMagnet::default() };
    let want = audio_gain(&p, 0.0);
    assert!((level(MagnetKind::Neodymium, 0.06) / clean / want - 1.0).abs() < 0.08);
    assert!(level(MagnetKind::Neodymium, 0.03) / clean < 0.06);
}
