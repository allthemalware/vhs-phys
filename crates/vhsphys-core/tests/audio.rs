//! The sound of the same tape and deck: wow from the shared transport, hum from the same
//! supply, bandwidth from the tape speed, dropouts from the same defects.

use std::f64::consts::PI;
use vhsphys_core::audio::AudioProcessor;
use vhsphys_core::config;
use vhsphys_core::mech::Mech;
use vhsphys_core::pipeline::Params;

const RATE: f64 = 48_000.0;

fn build(preset: &str, text: &str, seed: u64) -> Params {
    config::build(Some(preset), text, seed).unwrap()
}
fn speed(preset: &str, s: &str, seed: u64) -> Params {
    build(preset, &format!("[condition]\ntape_speed = \"{s}\"\n"), seed)
}
fn run(p: &Params, x: &[f32], channels: usize, start: i64) -> Vec<f32> {
    AudioProcessor::new(p, RATE, channels).process(x, start)
}
fn tone(f: f64, secs: f64, amp: f64) -> Vec<f32> {
    (0..(secs * RATE) as usize).map(|n| (amp * (2.0 * PI * f * n as f64 / RATE).sin()) as f32).collect()
}
fn silence(secs: f64) -> Vec<f32> {
    vec![0.0; (secs * RATE) as usize]
}
fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}
fn db(r: f64) -> f64 {
    20.0 * r.max(1e-12).log10()
}
/// Amplitude of the component at `f` (single-bin DFT over the slice).
fn amp_at(x: &[f32], f: f64) -> f64 {
    let (mut re, mut im) = (0.0, 0.0);
    for (n, v) in x.iter().enumerate() {
        let w = 2.0 * PI * f * n as f64 / RATE;
        re += *v as f64 * w.cos();
        im += *v as f64 * w.sin();
    }
    2.0 * (re * re + im * im).sqrt() / x.len() as f64
}
/// Level of a tone through the processor, dB re the input, from the last half second.
fn tone_gain_db(p: &Params, f: f64) -> f64 {
    let y = run(p, &tone(f, 1.0, 0.1), 1, 0);
    db(amp_at(&y[24_000..], f) / 0.1)
}
/// -3 dB frequency of the response relative to 1 kHz.
fn bandwidth(p: &Params) -> f64 {
    let reference = tone_gain_db(p, 1000.0);
    let freqs = [1000.0, 2000.0, 3000.0, 4000.0, 5000.0, 6000.0, 7000.0, 8000.0, 9000.0, 10_000.0, 12_000.0, 14_000.0, 16_000.0];
    let mut prev = (freqs[0], 0.0);
    for f in freqs {
        let g = tone_gain_db(p, f) - reference;
        if g < -3.0 {
            return prev.0 + (f - prev.0) * (prev.1 + 3.0) / (prev.1 - g);
        }
        prev = (f, g);
    }
    freqs[freqs.len() - 1]
}
/// Residual after removing a sinusoid of frequency `f`.
fn residual_rms(y: &[f32], f: f64) -> f64 {
    let (mut re, mut im) = (0.0, 0.0);
    for (n, v) in y.iter().enumerate() {
        let w = 2.0 * PI * f * n as f64 / RATE;
        re += *v as f64 * w.cos();
        im += *v as f64 * w.sin();
    }
    let (a, b) = (2.0 * re / y.len() as f64, 2.0 * im / y.len() as f64);
    let r: Vec<f32> = y.iter().enumerate().map(|(n, v)| (*v as f64 - a * (2.0 * PI * f * n as f64 / RATE).cos() - b * (2.0 * PI * f * n as f64 / RATE).sin()) as f32).collect();
    rms(&r)
}
/// RMS of the noise left after excluding the tone at `f` and its close-in sidebands (within
/// 20 Hz): how a noise meter with a notch reads a tone that went through a wobbling transport.
/// The wow of a real deck smears the tone into sidebands a few hertz wide; that is wow, not
/// hiss. Hann-windowed, so the skirts of the tone do not leak into the noise bins. Uses the
/// first 2 s of `y` (0.5 Hz resolution).
fn notched_noise_rms(y: &[f32], f: f64) -> f64 {
    let n = 96_000usize;
    assert!(y.len() >= n, "need 2 s of signal");
    let hann = |i: usize| 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos();
    let wy: Vec<f64> = (0..n).map(|i| y[i] as f64 * hann(i)).collect();
    let total: f64 = wy.iter().map(|v| v * v).sum();
    let window_energy: f64 = (0..n).map(|i| hann(i).powi(2)).sum();
    let (k0, span) = ((f * n as f64 / RATE).round() as i64, (20.0 * n as f64 / RATE).round() as i64);
    let mut excluded = 0.0;
    for k in k0 - span..=k0 + span {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in wy.iter().enumerate() {
            let a = 2.0 * PI * ((k as f64 * i as f64 / n as f64) % 1.0);
            re += v * a.cos();
            im -= v * a.sin();
        }
        excluded += 2.0 * (re * re + im * im) / n as f64; // the mirror image at -f counts too
    }
    ((total - excluded).max(0.0) / window_energy).sqrt()
}
fn snr_db(p: &Params) -> f64 {
    let y = run(p, &tone(1000.0, 3.0, 0.1), 1, 0);
    db(0.1 / 2f64.sqrt()) - db(notched_noise_rms(&y[24_000..], 1000.0))
}
/// Delay (seconds) of a pure tone, measured every 10 ms by complex demodulation over 20 ms.
fn delay_series(y: &[f32], f0: f64) -> Vec<(f64, f64)> {
    let (win, hop) = (960usize, 480usize);
    let mut out = vec![];
    let mut unwrapped = 0.0;
    let mut last = None;
    let mut start = 0;
    while start + win <= y.len() {
        let (mut re, mut im) = (0.0, 0.0);
        for k in 0..win {
            let n = start + k;
            let w = 2.0 * PI * f0 * n as f64 / RATE;
            re += y[n] as f64 * w.cos();
            im -= y[n] as f64 * w.sin();
        }
        let phase = im.atan2(re);
        unwrapped += match last {
            None => 0.0,
            Some(prev) => {
                let mut d: f64 = phase - prev;
                while d > PI {
                    d -= 2.0 * PI;
                }
                while d < -PI {
                    d += 2.0 * PI;
                }
                d
            }
        };
        if last.is_none() {
            unwrapped = phase;
        }
        last = Some(phase);
        // y = sin(2 pi f0 (t - d)): the demodulated phase is -2 pi f0 d - pi/2
        let d = (-unwrapped - PI / 2.0) / (2.0 * PI * f0);
        out.push(((start + win / 2) as f64 / RATE, d));
        start += hop;
    }
    out
}
fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let (ma, mb) = (a.iter().sum::<f64>() / a.len() as f64, b.iter().sum::<f64>() / b.len() as f64);
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b.iter()) {
        sab += (x - ma) * (y - mb);
        saa += (x - ma).powi(2);
        sbb += (y - mb).powi(2);
    }
    sab / (saa * sbb).sqrt()
}
fn std(a: &[f64]) -> f64 {
    let m = a.iter().sum::<f64>() / a.len() as f64;
    (a.iter().map(|x| (x - m).powi(2)).sum::<f64>() / a.len() as f64).sqrt()
}

#[test]
fn the_pristine_deck_is_close_to_transparent_in_band() {
    let p = speed("pristine", "sp", 1);
    let g = tone_gain_db(&p, 1000.0);
    assert!(g.abs() < 2.0, "gain at 1 kHz {g:.2} dB");
    assert!(tone_gain_db(&p, 100.0) - g > -6.0, "100 Hz should still be there");
    assert!(snr_db(&p) > 38.0, "pristine SNR {:.1} dB", snr_db(&p));
}

#[test]
fn bandwidth_follows_the_tape_speed() {
    let (sp, lp, ep) = (bandwidth(&speed("pristine", "sp", 1)), bandwidth(&speed("pristine", "lp", 1)), bandwidth(&speed("pristine", "ep", 1)));
    assert!(sp > lp && lp > ep, "-3 dB points {sp:.0} > {lp:.0} > {ep:.0} Hz");
    assert!((7000.0..12_500.0).contains(&sp), "SP bandwidth {sp:.0} Hz (about 10 kHz)");
    assert!((2500.0..5500.0).contains(&ep), "EP bandwidth {ep:.0} Hz (about 4 kHz)");
    assert!(sp / ep > 2.0, "the wavelength on tape sets the bandwidth: {sp:.0} vs {ep:.0}");
}

#[test]
fn worn_tapes_and_slow_speeds_are_hissier() {
    let noise = |preset: &str, s: &str| rms(&run(&speed(preset, s, 2), &silence(4.0), 1, 0)[24_000..]);
    let (pristine, good, worn, ruined) = (noise("pristine", "sp"), noise("good", "sp"), noise("worn", "sp"), noise("ruined", "sp"));
    assert!(pristine < good && good < worn && worn < ruined, "{pristine:.2e} < {good:.2e} < {worn:.2e} < {ruined:.2e}");
    assert!(noise("good", "ep") > 1.2 * good, "a slower tape is hissier");
    let snr = snr_db(&speed("good", "sp", 2));
    assert!((30.0..56.0).contains(&snr), "a good SP tape has about 42 dB SNR, got {snr:.1}");
}

#[test]
fn wow_is_the_capstan_error_the_picture_also_suffers() {
    let p = build("worn", "", 4);
    let mech = Mech::new(p.mech.clone(), p.seed);
    let y = run(&p, &tone(3000.0, 30.0, 0.2), 1, 0);
    let series = delay_series(&y[24_000..], 3000.0);
    let d: Vec<f64> = series.iter().map(|(_, d)| *d).collect();
    let w: Vec<f64> = series.iter().map(|(t, _)| mech.wow(t + 0.5)).collect();
    let r = pearson(&d, &w);
    assert!(r > 0.8, "audio delay should follow Mech::wow (the same capstan): r = {r:.2}");
    let s = std(&d);
    assert!((5e-6..2e-3).contains(&s), "a worn deck wows by tens to hundreds of microseconds: {:.1} us rms", s * 1e6);
    let calm = delay_series(&run(&build("pristine", "", 4), &tone(3000.0, 30.0, 0.2), 1, 0)[24_000..], 3000.0);
    assert!(std(&calm.iter().map(|(_, d)| *d).collect::<Vec<_>>()) < s, "a pristine deck wows less");
}

#[test]
fn both_channels_share_the_transport_but_not_the_hiss() {
    let p = build("worn", "", 6);
    let mono = tone(3000.0, 20.0, 0.2);
    let stereo: Vec<f32> = mono.iter().flat_map(|v| [*v, *v]).collect();
    let y = run(&p, &stereo, 2, 0);
    let (l, r): (Vec<f32>, Vec<f32>) = (y.iter().step_by(2).cloned().collect(), y.iter().skip(1).step_by(2).cloned().collect());
    let dl: Vec<f64> = delay_series(&l[24_000..], 3000.0).iter().map(|v| v.1).collect();
    let dr: Vec<f64> = delay_series(&r[24_000..], 3000.0).iter().map(|v| v.1).collect();
    assert!(pearson(&dl, &dr) > 0.95, "one capstan moves both tracks: r = {:.2}", pearson(&dl, &dr));
    let hiss = run(&p, &vec![0.0; 2 * 96_000], 2, 0);
    let (hl, hr): (Vec<f64>, Vec<f64>) = (hiss.iter().step_by(2).map(|v| *v as f64).collect(), hiss.iter().skip(1).step_by(2).map(|v| *v as f64).collect());
    assert!(pearson(&hl, &hr).abs() < 0.2, "the two tracks have their own hiss: r = {:.2}", pearson(&hl, &hr));
}

#[test]
fn hum_is_the_supplys_ripple() {
    let hum_dbfs = |p: &Params| {
        let y = run(p, &silence(8.0), 1, 0);
        let m = p.deck.supply.mains_hz;
        db(amp_at(&y[24_000..], m).max(amp_at(&y[24_000..], 2.0 * m)))
    };
    let dry = build("dry-caps", "", 1);
    assert!(dry.deck.supply.hum_ire > 1.0, "the dry-caps preset hums in the picture");
    let (hum, calm) = (hum_dbfs(&dry), hum_dbfs(&build("pristine", "", 1)));
    assert!(hum > -60.0, "dry capacitors are audible in the sound too: {hum:.1} dBFS");
    assert!(calm < -75.0, "a healthy supply is silent: {calm:.1} dBFS");
    let strong = build("dry-caps", &format!("[deck.supply]\nhum_ire = {}\n", dry.deck.supply.hum_ire * 3.0), 1);
    assert!(hum_dbfs(&strong) > hum + 6.0, "more ripple, more hum");
}

#[test]
fn dirt_on_the_tape_dulls_the_high_frequencies_first() {
    // 1 kHz and 8 kHz together on a stereo programme, read from the left track (350 um wide).
    // A defect only covers part of the track width, so a 1 mm mono track shrugs off all but the
    // biggest debris while a narrow stereo track loses most of its width to a particle of
    // 150 um: count the 5 ms windows where only the 8 kHz drops.
    let both: Vec<f32> = tone(1000.0, 60.0, 0.1).iter().zip(tone(8000.0, 60.0, 0.1).iter()).map(|(a, b)| a + b).collect();
    let stereo: Vec<f32> = both.iter().flat_map(|v| [*v, *v]).collect();
    let events = |p: &Params| {
        let y: Vec<f32> = run(p, &stereo, 2, 0).iter().step_by(2).cloned().collect();
        let win = 240;
        let levels: Vec<(f64, f64)> = y[48_000..].chunks_exact(win).map(|c| (db(amp_at(c, 1000.0)), db(amp_at(c, 8000.0)))).collect();
        let mut hf: Vec<f64> = levels.iter().map(|l| l.1).collect();
        let mut lf: Vec<f64> = levels.iter().map(|l| l.0).collect();
        hf.sort_by(|a, b| a.partial_cmp(b).unwrap());
        lf.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let (m_hf, m_lf) = (hf[hf.len() / 2], lf[lf.len() / 2]);
        let mut count = 0;
        let mut inside = false;
        for (lo, hi) in &levels {
            let hit = *hi < m_hf - 10.0 && *lo > m_lf - 4.0;
            if hit && !inside {
                count += 1;
            }
            inside = hit;
        }
        count
    };
    let dirty = events(&build("good", "[tape]\ndefects_per_mm2 = 200.0\n", 5));
    let clean = events(&build("pristine", "[tape]\ndefects_per_mm2 = 0.0\n", 5));
    // over seeds 1 to 8 a tape this dirty gives 15 to 30 such windows a minute; a clean one none
    assert!(dirty >= 5, "a dirty tape should dull the highs now and then: {dirty} events");
    assert_eq!(clean, 0, "a clean tape should not");
}

#[test]
fn streaming_and_windowing_are_deterministic() {
    let p = build("worn", "", 4);
    let x: Vec<f32> = tone(1000.0, 6.0, 0.15).iter().zip(tone(3000.0, 6.0, 0.15).iter()).map(|(a, b)| a + b).collect();
    let one = run(&p, &x, 1, 0);
    let n = x.len() / 2;
    let mut pr = AudioProcessor::new(&p, RATE, 1);
    let mut split = pr.process(&x[..n], 0);
    split.extend(pr.process(&x[n..], n as i64));
    let worst = one.iter().zip(split.iter()).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(worst < 1e-6, "continuing a stream must equal one pass: {worst}");
    let fresh = run(&p, &x[n..], 1, n as i64);
    let w = RATE as usize;
    let worst = one[n + w..].iter().zip(fresh[w..].iter()).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(worst < 5e-3, "a processor started mid-programme reproduces the same tape: {worst}");
    assert_ne!(run(&build("worn", "", 5), &x, 1, 0), one, "another seed is another tape");
}

#[test]
fn it_can_be_switched_off_and_it_stays_in_bounds() {
    let mut p = build("worn", "", 1);
    let x = tone(1000.0, 1.0, 0.5);
    p.audio.enabled = false;
    assert_eq!(run(&p, &x, 1, 0), x);
    p.audio.enabled = true;
    let loud: Vec<f32> = tone(200.0, 2.0, 0.99).iter().zip(tone(900.0, 2.0, 0.99).iter()).map(|(a, b)| a + b).collect();
    for (ch, input) in [(1usize, loud.clone()), (2, loud.iter().flat_map(|v| [*v, -*v]).collect())] {
        let y = run(&p, &input, ch, 0);
        assert_eq!(y.len(), input.len());
        assert!(y.iter().all(|v| v.is_finite() && v.abs() <= 1.0 + 1e-6), "output must be finite and within full scale");
    }
}

#[test]
fn the_measurement_helpers_are_sound() {
    // a 3 kHz tone delayed by a known 2 Hz, 50 us wobble: the demodulator must recover it
    let wobble = |t: f64| 50e-6 * (2.0 * PI * 2.0 * t).sin();
    let y: Vec<f32> = (0..(10.0 * RATE) as usize).map(|n| { let t = n as f64 / RATE; (0.2 * (2.0 * PI * 3000.0 * (t - wobble(t))).sin()) as f32 }).collect();
    let series = delay_series(&y, 3000.0);
    let measured: Vec<f64> = series.iter().map(|v| v.1).collect();
    let truth: Vec<f64> = series.iter().map(|v| wobble(v.0)).collect();
    assert!(pearson(&measured, &truth) > 0.99, "delay_series recovers a known wobble: r = {:.3}", pearson(&measured, &truth));
    assert!((std(&measured) / std(&truth) - 1.0).abs() < 0.1, "with the right size: {:.1} vs {:.1} us", std(&measured) * 1e6, std(&truth) * 1e6);
    // a pure tone leaves no residual, and its amplitude is read back
    let tone = tone(1000.0, 3.0, 0.1);
    assert!(residual_rms(&tone, 1000.0) < 1e-4);
    assert!((amp_at(&tone, 1000.0) - 0.1).abs() < 1e-3);
    assert!(pearson(&[1.0, 2.0, 3.0, 5.0], &[2.0, 4.1, 6.0, 9.9]) > 0.99);
    // the notched noise meter does not count the wobble sidebands as noise, the plain one does
    let mut hissy = y.clone();
    for (n, v) in hissy.iter_mut().enumerate() {
        *v += 0.002 * ((n as f64 * 12.9898).sin() * 43758.5453).fract() as f32; // deterministic white-ish noise
    }
    assert!(notched_noise_rms(&y[24_000..], 3000.0) < 1e-3, "a wobbling pure tone has no noise: {}", notched_noise_rms(&y[24_000..], 3000.0));
    assert!(residual_rms(&y[24_000..], 3000.0) > 0.01, "the plain projection counts the wobble as noise");
    assert!(notched_noise_rms(&hissy[24_000..], 3000.0) > 5e-4, "but the meter still reads real noise");
}
