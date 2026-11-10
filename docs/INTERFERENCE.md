# Interference: the outside world

A VHS deck and its TV do not live alone. A phone rings next to them, a radio transmitter sits on the
next street, the mains is not quite 60 Hz and not quite the same ground, a dimmer switches on the
same circuit, a magnet gets close to the screen or the head drum. None of these is part of the
circuit, and all of them are physics, so they are modelled as physics: a source has a field at the
equipment (free-space fields, dipole fields, the mains waveform), a coupling turns the field into a
voltage at one node of the circuit (loop pick-up, rectification, induction, the Lorentz force), and
that voltage is added at that node. Nothing is painted over the picture. The existing chain (FM
discriminator, AGC, sync slicer, clamp, colour decoder, audio filters) reacts to what it is given.

With every source off the chain is bit-identical to one without this module (the golden test).

## Where each source enters

| source | field | node | what reacts |
|---|---|---|---|
| radio carrier below 19 MHz | induced emf in the head leads | head signal, before the deck electronics | the FM discriminator, AGC, the colour-under filter and the chroma demodulator |
| dimmer | ringing of the step in the leads | head signal | the same |
| phone, AM radio (any frequency) | rectified envelope (square law) | picture after the TV's clamp; audio before the equaliser | the viewer; the playback equaliser |
| ground loop | voltage between the grounds | picture (gain modulation, a residue of offset); audio after the equaliser | |
| stray mains field | flux through the audio head; Lorentz force on the beam | audio before the equaliser; the picture tube | |
| magnet near the head drum | dipole field | head signal and audio, before the amplifier noise | carrier-to-noise ratio |
| magnet near the tube | dipole field | the electron beam | the picture |

## Sources

### Mobile phone
A GSM handset sends one burst of 577 us in every 4.615 ms frame (216.7 Hz, one slot in eight), with
raised-cosine ramps of 28 us. GPRS uses two adjacent slots (duty 1/4). LTE sends in 1 ms subframes,
each with probability 0.3 (the scheduler), ramped only where it borders a silent one.

The field at distance `d` is the broadside field of a short dipole,
`E = sqrt(30 P G) / d * sqrt(1 - 1/(kd)^2 + 1/(kd)^4)` with `k = 2 pi f / c`: radiated at a few
wavelengths, reactive within about 5 cm at 900 MHz (6.6 times the radiated field at 2 cm). The case shields it by
`shielding_db`. A semiconductor junction rectifies the carrier, so the output follows the *power* envelope:
`v = k_node E^2 (envelope - mean)`, the mean being what AC coupling removes. Twice the distance gives a quarter
of the buzz. The spectrum is that of the pulse train: lines at multiples of 216.7 Hz, with the first null
near the ninth harmonic. In the picture one burst is a band 9 lines tall repeating every 72.6 lines.

### Radio
An AM broadcast, a plain carrier, or single-sideband voice. Below `0.45 * FS_RF` (19 MHz) the carrier is
added to the head signal: the head leads are a loop of 5 cm^2, so the emf is `(2 pi f / c) A E`, 41 uV per V/m
at 3.9 MHz, filtered by the preamplifier (second order, 8 MHz) and compared with a nominal 1 mV head signal.
It then goes through the real FM chain: near the luma carrier (3.4 to 4.4 MHz) it beats with it into the diagonal
herringbone, whose spatial frequency is `|f_i - f_c|` times the active line (52.7 us); near the colour-under
carrier (629 kHz) it beats with that into coloured stripes. The AM envelope is also rectified into the picture
and the sound, as the phone's is. The programme is a synthetic stand-in for speech: ten partials from 180 Hz
upward with a falling tilt.

### Mains, ground and AC timing
The video runs on its own crystal at 59.94 Hz; the mains runs where the grid is (`mains_hz`, 47 to 63 Hz). Hum
is a function of absolute time, so the bars crawl at the difference, as they do in life: 60 Hz mains against
59.94 Hz video crawls at 0.06 Hz; a 50 Hz mains on NTSC video scrolls them fast. The mains is not a sine: a
rectifier-loaded line has a 20 % third and a 10 % fifth harmonic.

* **Ground loop.** The voltage between the grounds of the deck and the TV puts a fraction (0.15) across the video
  input. The TV's clamp removes most of the offset (a tenth stays), but the ground current also moves the bias of the
  amplifiers: a gain modulation of 0.8 % per IRE, the same as supply ripple in the deck. Whites move, black stays:
  hum bars. In the sound the same voltage appears after the equaliser, 0.15 of it.
* **Stray field.** A mains-frequency magnetic field from a transformer or motor induces an emf in the audio head
  (`2 pi N A` volts per uT per Hz, so the harmonics of the mains are emphasised: induction differentiates) and bends
  the beam of the tube (below).
* **Dimmer.** A phase-cut dimmer fires a fixed angle after each zero crossing. The step has the height of the mains
  voltage at that angle (proportional to `sin(angle)`, the sign alternating each half cycle) and rings: 80 kHz with a
  40 us decay in the picture, a 2 MHz burst on the head leads, a click in the sound. Twice per mains cycle.

### Magnet near the head drum
A dipole of moment `m` (fridge sheet 0.3, neodymium disc 1.6, loudspeaker ring 42 A m^2) has `B = mu0 / (4 pi) * 2 m / d^3`
on its axis. Two things follow.

* The Mn-Zn ferrite core saturates: `B_core = B_sat tanh(mu_i mu0 H / B_sat)`, so the incremental permeability, and the
  output, fall as `sech^2(mu_i k B / B_sat)`. A neodymium disc biases the core by 1.5 mT at 6 cm and leaves 79 % of
  the signal; at 3 cm the core is deep in saturation (0.15 %). The fraction `k = 0.1` of the field that enters the
  core path is an estimate.
* The tape is erased. VHS oxide has a coercivity of about 52 kA/m (650 Oe) and the particles a spread of switching fields
  of 30 %, so the fraction switched is the normal distribution function of `(H - Hc) / (0.3 Hc)`. A neodymium disc erases
  the tape within about 2 cm.

Both weaken the *signal*; the head amplifier's noise is unchanged, so the carrier-to-noise ratio falls and the picture
turns to snow before it goes. The audio head, next to the drum, loses signal in the same way.

### Magnet near the picture tube, and the stray field
Electrons accelerated through 25 kV have a momentum of `p = sqrt((eV)^2 + 2 eV m c^2) / c = 8.65e-23 kg m/s`
(1.2 % above the classical value). In a field `B` the beam turns as `dv/dl = -(e/p) (v x B)`. A field that fills the tube
moves the central beam on the screen by `(e/p) B D^2 / 2` (D = 0.35 m): 5.7 mm for 50 uT, which is why every set shows
the earth's field. A magnet is a dipole; the field is integrated along the path of each beam, from the gun along the axis
to the deflection point and straight to the screen point. Consequences:

* a magnet near the neck moves or rotates the whole raster, one near the screen warps it locally (the lever is the distance
  to the screen);
* the displacement falls as the cube of the distance: a neodymium disc 20 cm from the beam gives about the earth's field (a few
  millimetres), one at 5 cm is violent;
* beams that crowd together are brighter: the picture is scaled by the inverse of the local area change;
* when a beam lands on the neighbouring phosphor stripe the colour changes: the displacement in stripe widths mixes
  red into green into blue (the matrix `(1 - f) P^n + f P^(n+1)`, no light lost); a displacement that changes by more than
  a stripe inside one pixel gives the even mix, a grey stain, as on a real set.

The stray mains field bends the beam too, but its displacement follows the mains waveform *line by line*, so the lines of
a field wave sideways and up and down at the mains frequency, crawling against the 59.94 Hz of the frame.

## Time

Everything is a function of absolute time (`index / rate`), so a render from any start gives the same picture, and
previews equal the render. A phone or a magnet that comes closer does so from the start of the video: `approach_s`
seconds from four times its distance to its distance.

## The controls

All are conditions (`[condition]` in a config file), off by default and in every older preset.

| key | meaning | range |
|---|---|---|
| `phone_mode`, `phone_distance_cm` | `off`, `gsm`, `gprs`, `lte`; distance | 2 to 200 cm |
| `radio_mod`, `radio_freq_mhz`, `radio_field_vpm` | `off`, `am`, `cw`, `ssb`; carrier; field at the equipment | 0.5 to 19 MHz, 0 to 10 V/m |
| `mains_hz`, `mains_sag` | the real mains frequency; fractional sag | 47 to 63 Hz, 0 to 0.2 |
| `ground_loop_mv`, `stray_field_ut`, `dimmer_angle_deg` | voltage between the grounds; stray field; firing angle (0 = none) | 0 to 500 mV, 0 to 200 uT, 0 to 170 deg |
| `crt_magnet`, `crt_magnet_z_cm`, `crt_magnet_cm`, `crt_magnet_angle_deg` | `none`, `fridge`, `neodymium`, `speaker`; place along the tube from the gun; distance from the axis; angle around it | 0 to 35 cm, 0 to 40 cm, 0 to 360 deg |
| `head_magnet`, `head_magnet_cm` | magnet near the head drum, distance | 0.5 to 30 cm |
| `shielding_db`, `approach_s` | shielding of case and cables; approach time | 0 to 40 dB, 0 to 60 s |

Presets that isolate one mechanism: `phone-buzz`, `ham-radio`, `am-station`, `ground-loop` (with a mains at 59.4 Hz),
`magnet-screen`, `magnet-head`, `dimmer`.

## What is estimated

The field physics is closed form. How much of a field a real circuit picks up depends on the equipment, and those coupling
coefficients are engineering estimates, tagged `[est]` in `interference_params.rs`: the rectification coefficients of the video
and audio stages, the head-lead loop area (5 cm^2), the nominal head signal (1 mV), the fraction of the ground voltage across
the video input and what the clamp leaves of it, the ring parameters of a dimmer, the fraction of an external flux that enters the head
core, the shape of the tube (depth 0.35 m, yoke at 0.27 m, 0.7 mm triad pitch). They are meant to be tuned against real
equipment, and they are the first thing to change when a measured set disagrees.

Not modelled: the radio-frequency modulator and channel of a VCR (the output is baseband), radio signals above 19 MHz in the
head leads (above what the preamplifier passes; they still act through rectification), persistent magnetisation of the
shadow mask (the stain disappears with the magnet), lightning and electrostatic discharge, interference on the capstan servo.
