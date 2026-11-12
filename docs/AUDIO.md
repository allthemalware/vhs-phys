# Linear audio

The sound of an old tape is the same machine as the picture. `audio::AudioProcessor` reads the
audio track of the tape and deck that `Params` describes, and each artefact is derived from a
parameter the picture model already has. Nothing is painted on. Estimated numbers are tagged
`[est]`; numbers that come from the format sheets or from the picture model are named as such.

## The chain

One processor per `Params`, `channels` interleaved channels at `rate` Hz. For each channel:

1. **Record.** A level control (peak envelope, attack 10 ms, release 500 ms, 4:1 above an
   envelope of 0.5, `[est]`) and tape saturation `tanh(x)`. The record deck's `drive_offset` (the
   bias drift that distorts its picture carrier) shifts the saturation and adds even harmonics.
2. **Wow and flutter.** A variable delay (4-point Lagrange) of
   `k_wow * Mech::wow(t) + k_flutter * Mech::flutter(t)`, with `Mech::new(params.mech, params.seed)`
   at `t = sample / rate`. Both channels share it: there is one capstan. The transport model is
   evaluated every 16 samples and interpolated linearly (flutter above about 1.5 kHz is lost; it is
   a few microseconds and well below audibility there).
3. **Spacing loss and dropouts.** Spacing `d` costs `exp(-2 pi f d / v)` (`54.6 d / lambda` dB), with
   `v = speed.tape_speed_m_s()`. A symmetric 49-tap filter per loss value approximates that
   Lorentzian response; a bank of them, spaced by a factor 1.6 in `d`, is crossfaded by the
   instantaneous spacing. The permanent part is the worn head: `h0_um` above new plus
   `extra_spacing_um` (mean of both heads) and 0.8 `rough_um` of the oxide. The varying part is the
   tape's debris: `tape.defects_in_band` returns the defects whose centre lies in the track's band
   and the stretch of tape under the head, at position `position_m * 1e6 + v * t` um. A defect is a disc of radius `r`; at each instant its
   lateral chord overlaps the track's band by a fraction `c` of the band width (as `Tape::paint` does
   with the head width). The head reads the whole width, so in the amplitude domain the output is
   `(1 - c)` at the permanent loss plus `c` at the extra loss: debris lifts the tape by `strength` um
   with the envelope `(1 - x^2)^2` along its footprint, but only over the covered fraction; a void
   removes `strength * c` of the level. Overlapping defects take the coverage-weighted mean lift. A
   small particle on the 1000 um mono track therefore does almost nothing, while a particle of 200 um
   radius on a 350 um stereo track blanks the highs. A clean tape never leaves the permanent filter. Mono reads the whole 1000 um band at the tape edge, stereo two 350 um tracks at its
   ends, so the channels see different defects.
4. **Head response and equaliser.** Net response: a 2nd-order high-pass at 80 Hz (`[est]`) and a
   2nd-order low-pass at `10.5 kHz * speed ratio` (10.5 / 5.25 / 3.5 kHz at SP / LP / EP; the format
   gives about 10 kHz at SP and about 4 kHz at EP). The corner is proportional to the tape speed
   because the equaliser is aligned for a fixed wavelength: it boosts the nominal spacing loss
   (0.36 um, `[est]`) and gap loss (1 um gap, `[est]`) up to the corner, 7.6 dB there at every speed.
   Wear and dirt, which the equaliser does not know about, lower the response below it through step 3.
5. **Hiss** is white noise added *before* the equaliser, so the equaliser's boost shapes it (a gentle
   rise to the corner, then the low-pass) and a slower tape, whose head voltage is proportional to
   the speed, hisses more: `sigma = sqrt(s_head^2 + s_tape^2) / speed_ratio`, with
   `s_head = hiss_head * 10^((40 - cnr_db) / 20)` and `s_tape = hiss_tape * media_am_noise / 0.02`
   (mean over the two heads). Both are the picture's noise figures, which `condition.rs` already
   derives from the deck's age and the tape's age, storage and quality. `hiss_head` and `hiss_tape`
   are tuned so that a good SP tape measures about 42 dB SNR (format sheet), against a -20 dBFS tone.
6. **Hum.** `deck.supply.hum_ire` (the ripple that makes hum bars in the picture) is added at twice
   the mains frequency with amplitude `hum_per_ire * hum_ire` (`[est]`, 1.3e-4 per IRE p-p), the mains
   fundamental at 0.4 of that (`[est]`) and, for peaky ripple (`ripple_peakiness`), the 4th and 6th
   harmonics. Same on all channels. Dry capacitors (30 IRE) give -48 dBFS.
7. Clip to +-1.

## Determinism and latency

Everything random or time-dependent is a pure function of the absolute sample index: noise is drawn
per block of 1024 samples from `Rng::new(seed, stream + channel, block)`, hum, transport and defects
are functions of time. Any split of the input gives the same output and a processor started at
`start` reproduces the same tape once its filters have settled (the level control only acts above
0.5, so it carries no history in normal programme).

The output is delayed by `latency()` samples (1.5 ms of room for the wow excursion plus the 24-sample
half length of the loss filter: 98 samples at 48 kHz); the equaliser adds under 0.3 ms. The first
`latency()` output samples are the empty tape. `AudioParams.enabled = false` returns the input
unchanged.

## Parameters

`AudioParams` (TOML table `audio`): `enabled`, `alc_*`, `sat_drive`, `bias_coupling`, `corner_sp_hz`,
`low_cut_hz`, `gap_um`, `nominal_spacing_um`, `hiss_head`, `hiss_tape`, `wow_gain`, `flutter_gain`,
`max_delay_ms`, `transport_step`, `hum_per_ire`, `hum_fundamental`. All the other couplings read
`Params` directly (`heads`, `tape`, `deck.supply`, `mech`, `record`, `speed`), so a change to the
picture model changes the sound with it.

`condition.rs` sets `wow_gain` and `flutter_gain`. `wow_gain` (65) and `flutter_gain` (4) are
constants (`[est]`): the picture sees only what the drum servo leaves of the capstan's error, the
audio head sees it directly, and the deck's wear already enters through the `Mech` parameters that
`condition.rs` derives for the picture. They are chosen so that the rms speed deviation of a tone
is 0.03 % for a pristine deck, 0.06 % good, 0.14 % worn and 0.22 % ruined (real VHS linear audio is
specified around 0.1 to 0.2 %).

## Measured (48 kHz)

| quantity | value |
|---|---|
| -3 dB bandwidth, pristine SP / LP / EP | 8.3 / 4.8 / 3.3 kHz (10.1 / 5.0 / 3.3 with a steady transport: wow smears a high tone over several bins of a single-bin meter) |
| SNR of a -20 dBFS 1 kHz tone with a notch (tone and sidebands within 20 Hz excluded), pristine / good SP | 43.6 / 42.3 dB |
| EP hiss against SP | about 1.85 times the rms |
| speed deviation, 3 kHz tone, pristine / good / worn / bad / ruined | 0.026 / 0.057 / 0.139 / 0.177 / 0.221 % |
| delay rms, same decks | 13 / 30 / 72 / 92 / 115 us |
| hum, `dry-caps` / `pristine` | -48 / -111 dBFS |
| speed (one thread) | 300 s of stereo in 2.5 to 3.8 s, 80 to 120 times real time |

## Hi-Fi

Hi-Fi audio is modelled as an option, off by default: `--set audio.hifi=true` (config: `[audio]` / `hifi = true`). It is a simplified model, not a carrier model: with Hi-Fi on, the sound takes 5 % of the capstan's wow and flutter (the drum servo follows the capstan), 5 % of the linear track's hiss and 10 % of the supply hum; a head-switch buzz at the field rate (59.94 or 50 Hz) is added, amplitude `audio.head_switch_buzz` (default 0.002, -54 dB), decaying in 0.4 ms; when the worst displacement of the head from the track centre exceeds `audio.hifi_tracking_threshold_um` (default 18 um at SP, scaled with the track pitch at LP and EP) the deck falls back to the linear track. The carrier level per field from the tracking weights and the tape's defects is not modelled yet.

| parameter | default | meaning |
|---|---|---|
| `audio.hifi` | `false` | Enable the simplified Hi-Fi audio model |
| `audio.head_switch_buzz` | `0.002` | Amplitude of the field-rate head-switch buzz (-54 dB, decays in 0.4 ms) |
| `audio.hifi_tracking_threshold_um` | `18.0` | Mistracking displacement threshold to fall back to the linear track (SP, scaled at LP/EP) |

## Limits

Defects in the tape's video area (creases, edge curl) do not reach the audio band. Dropouts on the wide mono track are shallow: a disc of at most 800 um across covers at
most 0.8 of the 1000 um band, so the highs fall by about 4 dB at best. The loss filter is a 49-tap approximation of the exponential: within about 1.5 dB for lifts up to 1.4 um at SP, understating the low-frequency loss of deeper ones. The SMPS ripple (38 kHz) is
above the audio band and is not coupled.
