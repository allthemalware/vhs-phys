# Physical Modelling of VHS Recording and Playback: Architectural Analysis, Competitive Landscape, and Roadmap for Electrical Accuracy

## Executive Summary

The modern landscape of analog video simulation is dominated by digital post-processing filter chains designed for visual aesthetics: pixel shaders, chromatic displacement, synthetic scanlines, and arbitrary noise overlays. These tools produce a stylized impression of "retro video" for motion graphics, game development, and music videos.

**vhs-phys** operates on an entirely distinct paradigm: **forward physical simulation from first principles**. Instead of layering digital image effects to emulate symptoms, it simulates the underlying electromagnetic, mechanical, and circuit physics of a helical-scan video tape recorder (VCR) and cathode-ray tube (CRT) display. Visual and acoustic phenomena—including FM discriminator noise, dropouts, head-switching glitches, colour-under crosstalk, tape flagging, capacitor hum bars, and radio frequency interference—**emerge naturally** from physical equations rather than heuristic artifice.

This report evaluates:
1. The unique theoretical and architectural foundations of [vhs-phys](file:///home/whyte/git/vhs-phys/README.md).
2. A comprehensive comparative assessment against competing tools and projects: [ntsc-rs](https://github.com/valadaptive/ntsc-rs), [vhs-decode](https://github.com/oyvindln/vhs-decode), [Chow Tape Model](https://github.com/jatinchowdhury18/AnalogTapeModel), and commercial digital effects suites.
3. Where competing approaches lead in performance, ecosystem integration, and signal tracking.
4. Actionable engineering paths to advance the physical and electrical accuracy of `vhs-phys`.

```mermaid
flowchart TD
    subgraph AestheticApproach["Aesthetic / Heuristic Approach (e.g. ntsc-rs, Shaders, Commercial Plugins)"]
        In1["Source Video Frame"] --> Filter1["1D Horizontal Blurs (Chroma Bleed)"]
        Filter1 --> Filter2["Composite YIQ Subcarrier Synthesis"]
        Filter2 --> Filter3["Additive Synthetic Noise & Jitter Offsets"]
        Filter3 --> Filter4["Tracking Bar & Edge Wave Overlays"]
        Filter4 --> Out1["Stylized Video Output"]
    end

    subgraph PhysicalApproach["vhs-phys Physical Simulation Pipeline"]
        In2["Source Video Frame"] --> Baseband["NTSC Baseband Rasterization"]
        Baseband --> FM["FM Luma Modulator + Colour-Under Carrier (AM Duty Cycle)"]
        FM --> MagRec["Saturating Magnetic Recording Head (Coercive Relay Model)"]
        MagRec --> TapeMed["2D Helical Tape Coordinates (Poisson Defect Field, Creases)"]
        TapeMed --> HeadRep["Karlqvist Reciprocity Readback (Lorentzian Pulse Summation)"]
        HeadRep --> MechWarp["Multi-band Mechanical Timebase Warp e(t)"]
        MechWarp --> DeckElec["Deck RF Electronics: Q-Resonance, AGC, 1H DOC, Limiter/Discriminator"]
        DeckElec --> TVDec["TV Receiver: Tip-Referenced Slicer, AFC Flywheel, APC/ACC Comb"]
        TVDec --> Out2["Emergent Physical Video Output"]
    end
```

---

## 1. What Makes vhs-phys Unique

### 1.1 The Principle of Physical Emergence
In standard digital video filters, an engineer designs algorithms targeted at specific visual defects (e.g., "draw a noisy horizontal bar across the bottom to simulate head switching"). In `vhs-phys`, no code paints head-switching noise. Instead:
- In [`consts.rs`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/consts.rs), head switching occurs $6.5\text{ H}$ before vertical sync when the rotating drum switches active read heads.
- In [`mech.rs`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/mech.rs), switching introduces an instantaneous mechanical displacement step (`switch_us`) and an exponential recovery (`switch_lines`).
- In [`electronics.rs`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/electronics.rs), the FM signal at the head transition undergoes RF filtering, limiting, and delay-line discrimination.
- In [`decode.rs`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/decode.rs), the TV's horizontal automatic frequency control (AFC) flywheel fails to instantly track the rapid displacement step, yielding line displacement across the bottom of the active frame.

A single physical root cause yields multiple coupled symptoms across domains:
* **Head Clog:** Spacing between tape and head expands via Wallace's spacing law ($54.6\,d/\lambda\text{ dB}$). The RF carrier drops, tripping the dropout compensator (DOC) into recirculating the previous line, while the Automatic Chroma Control (ACC) starves and the colour killer mutes chrominance on that field.
* **Oxide Creases:** A longitudinal crease on the tape surface at coordinate $v$ is swept by diagonal helical passes, producing signal drops at a constant vertical height on every field.
* **Point Defects:** A microscopic particle of debris on tape advances by $1.5\text{ H}$ ($556.4\,\mu\text{m}$) per field relative to the head path at SP speed, causing the dropout glitch to step downward across consecutive fields.

### 1.2 Continuous-Time RF Domain and Exact Demodulation
Digital video operates at low pixel sampling rates ($13.5\text{--}14.32\text{ MHz}$). Simulating physical frequency modulation (FM) at low rates introduces catastrophic edge-quantization jitter and harmonic aliasing. 

`vhs-phys` establishes a line-locked RF domain running at $12 \times f_{sc} = 42.95\text{ MS/s}$ ($2730\text{ samples/line}$). As detailed in [`electronics.rs`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/electronics.rs#L295-L379) and [`RESEARCH.md`](file:///home/whyte/git/vhs-phys/docs/RESEARCH.md#L250-L284):
1. **Continuous Sub-Sample Crossings:** Tape magnetization reversals ([`Transition`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/record.rs#L20-L27)) are evaluated at continuous fractional positions. Quantizing transition edges to a discrete sample grid would cap the carrier-to-noise ratio (CNR) at $\sim 15\text{ dB}$.
2. **Piecewise Exact Limiter and Area Integration:** The FM limiter is modeled as a Schmitt trigger with hysteresis. Rather than point-sampling a hard-clipped digital wave, the product of the wave and its delayed copy is integrated analytically across sample intervals, completely eliminating aliased limiter harmonics.
3. **Chroma as FM Bias:** Chroma is heterodyned down to colour-under ($629.371\text{ kHz}$) and summed with the FM luma current. The saturating magnetic record head behaves as a coercive relay; the low-frequency duty cycle of the resulting square wave is modulated by the chroma signal, matching SMPTE ST 32-1998 specifications.

### 1.3 Reciprocity and Lorentzian Pulse Summation
In [`head.rs`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/head.rs#L53-L89), playback does not apply digital frequency-domain attenuation filters. Based on the Karlqvist head field and reciprocity theory, each magnetic reversal on tape reads back as a unit-area Lorentzian voltage pulse:
$$e(x) = \frac{1}{\pi} \frac{h}{h^2 + x^2}$$
where effective half-width is:
$$h = \sqrt{(g/2)^2 + (d+a)(d+a+\delta)}$$
Because Lorentzian convolutions compose, variations in physical head-to-tape spacing $d(t)$ add linearly to $h$. Wallace's classical spacing loss law ($54.6\,d/\lambda\text{ dB}$) emerges analytically from the pulse summation without explicit digital filter banks.

### 1.4 Electrical Ageing Laws (Arrhenius and Moisture Models)
In [`condition.rs`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/condition.rs#L144-L206), physical parameters are derived from operational lifespan and environmental exposure:
* **Electrolytics:** Capacitance $C$ decreases and equivalent series resistance (ESR) increases according to Arrhenius's rule (life halves per $+10^\circ\text{C}$ rise). When regulator headroom collapses ($V_{in} < 14\text{ V}$), 120 Hz full-wave rectified supply ripple couples into video amplifier stages, causing gain-modulated rolling hum bars.
* **Resistors:** Moisture absorption causes carbon-film resistors to drift upward proportionally to $\sqrt{t}$, detuning pre-emphasis and de-emphasis time constants ($\tau = R C$) and shifting discriminator center frequencies.

### 1.5 Shared Physics with Linear Audio
Unlike video-only effects, [`audio.rs`](file:///home/whyte/git/vhs-phys/docs/AUDIO.md) couples linear audio to the exact same physical parameters:
- **Shared Transport Wow and Flutter:** Audio reads the identical mechanical speed deviations generated by `Mech::wow` and `Mech::flutter`.
- **Shared Debris Map:** The physical oxide debris discs mapped in [`tape.rs`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/tape.rs) pass across the audio head band, dynamically modulating spacing loss and producing momentary treble attenuation.
- **Supply Ripple Hum:** Power supply ripple that causes video hum bars simultaneously injects 120 Hz and harmonic hum into the audio pre-amplifier.

### 1.6 External Physical Interference
As documented in [`INTERFERENCE.md`](file:///home/whyte/git/vhs-phys/docs/INTERFERENCE.md), external fields couple to physical circuit nodes:
* **GSM Handsets:** 217 Hz TDMA burst frames rectify across amplifier PN junctions, creating acoustic buzz and periodic video banding.
* **RF Pick-up:** Stray electromagnetic carriers induce voltage loops in the head leads, beating against the FM luma carrier into fine diagonal herringbone patterns.
* **Lorentz Force on CRT Beams:** Stray magnetic dipole fields deflect accelerated electron beams according to $\mathbf{F} = q(\mathbf{E} + \mathbf{v} \times \mathbf{B})$, shifting beam landing across phosphor stripes, inducing chromatic purity errors, and warping the raster.

---

## 2. Competitive Landscape Analysis

| Dimension | `vhs-phys` | `ntsc-rs` (valadaptive) | `vhs-decode` (oyvindln) | `Chow Tape Model` (Chowdhury) | Commercial FX (Red Giant / Boris FX) |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Primary Domain** | Forward physical & electrical simulation | Baseband signal & video effect processing | Reverse RF archival decoding & TBC | Audio magnetic hysteresis ODE modeling | 2D image post-processing & motion graphics |
| **RF / Carrier Domain** | Explicit continuous-time FM at $42.95\text{ MS/s}$ | None (operates on YIQ baseband fields) | Captured physical RF digitization ($20\text{--}40\text{ MS/s}$) | Explicit continuous bias / audio saturation | None (8/16-bit RGB/YUV pixel manipulation) |
| **Magnetic Media Model** | Discrete transitions via coercive relay + Lorentzian reciprocity | None | None (processes physical tape signal) | Nonlinear Jiles-Atherton hysteresis ODE solver | None |
| **Audio Modeling** | Physically coupled linear track (shared transport/defects) | None (recommends external Chow Tape plugin) | Separate Hi-Fi / linear decoding suites | Pure audio tape (reel-to-reel/cassette) | Independent synthetic hum/crackle tracks |
| **Component Level Modeling** | Arrhenius capacitor wear, ESR, resistor drift, PSU dropout | None (direct visual parameter sliders) | Emulates IC-specific analog filters (Sony/Panasonic) | Op-amp clipping, transformer saturation | None |
| **Speed / Throughput** | $12\text{--}14\text{ fps}$ on 8 CPU cores (SD 480p) | Real-time ($60\text{+ fps}$) at HD/4K resolutions | Near real-time decoding on modern CPUs | Real-time audio DSP ($>100\times$ real-time) | Real-time GPU compute (100+ fps via OpenCL/CUDA) |
| **Host Integration** | Standalone CLI + PySide6 desktop GUI | Standalone, WebAssembly, OFX, Premiere, After Effects | Standalone CLI & GUI tools | VST3, AU, LV2, CLAP DAW plugins | Native After Effects, Premiere, Resolve, Final Cut |
| **Standard Coverage** | NTSC VHS (SP, LP, EP) | NTSC (composite + stylized VHS mode) | NTSC, PAL, SECAM, S-VHS, Video8, Hi8, LaserDisc | Audio format agnostic | Format agnostic visual filters |

---

## 3. Deep Dive: Where Competitors Excel and How

```mermaid
flowchart LR
    subgraph CompetitorStrengths["Where Competitors Excel"]
        ntscRS["ntsc-rs: Real-Time Performance & Plugin Ecosystem"]
        vhsDecode["vhs-decode: Hardware-Calibrated RF Demodulation & Standards"]
        chowTape["Chow Tape: Continuous-Time Hysteresis & Saturation"]
    end

    subgraph ArchitecturalSolutions["Underlying Engineering Mechanisms"]
        SIMD["1D Separable IIR/FIR Filters at Baseband Rates"]
        EmpiricalIC["Empirical Matching of Dedicated VCR Chipsets"]
        StateSpace["Discretized Jiles-Atherton ODE Solvers"]
    end

    ntscRS --> SIMD
    vhsDecode --> EmpiricalIC
    chowTape --> StateSpace
```

### 3.1 ntsc-rs: Throughput, Architecture, and Usability
[ntsc-rs](file:///home/whyte/git/vhs-phys/docs/COMPARISON.md) dominates in creative video workflows due to four design choices:
1. **Baseband Pixel-Rate Processing:** By operating exclusively on discrete YIQ horizontal lines rather than a $43\text{ MS/s}$ RF carrier, computational complexity is reduced by over $90\%$.
2. **Separable 1D Horizontal Filtering:** Artifacts such as chroma low-pass delay, notch filters, and edge ringing are implemented using low-order biquad IIR or boxcar filters that run horizontally across scanlines, enabling vectorization via AVX2/NEON.
3. **Comprehensive Plugin Host Integration:** Packaged as native OpenFX (DaVinci Resolve), Adobe After Effects, and Premiere Pro plugins with full timeline scrubbing and keyframing support.
4. **Flexible Field and Aspect Handling:** Supports interlaced input/output (upper field, lower field, interleaved) and arbitrary resolutions, whereas `vhs-phys` enforces a fixed $754 \times 480$ raster.

### 3.2 vhs-decode: Hardware Calibrations and Standard Coverage
`vhs-decode` is the gold standard for software decoding of physical VHS RF signals. It leads in areas where `vhs-phys` currently relies on estimates:
1. **Empirical Demodulation Hardware Profiles:** `vhs-decode` models the exact filtering and deemphasis response of specific analog signal-processing ICs (e.g., Sony CXA1201, Panasonic AN3248, JVC VC5036).
2. **Phase-Locked Loop (PLL) Demodulation:** Uses continuous phase-locked loop subcarrier regeneration and tracking for colour-under chroma recovery, handling wild phase errors that break simpler burst-sampling loops.
3. **Broad Standard Coverage:** Implements complete decoding specifications for PAL (625 lines, 25 fps, $3.8\text{--}4.8\text{ MHz}$ FM band, $626.953\text{ kHz}$ colour-under with line alternation, 8-field subcarrier sequence) and S-VHS ($5.4\text{--}7.0\text{ MHz}$ FM band).

### 3.3 Chow Tape Model: Deep Magnetic Hysteresis
Jatin Chowdhury’s work (DAFx-2019) models the physics of magnetic tape using the **Jiles-Atherton differential equation**:
$$\frac{dM}{dH} = \frac{M_{an} - M}{k \cdot \text{sgn}\left(\frac{dH}{dt}\right) - \alpha (M_{an} - M)} + c \frac{dM_{an}}{dH}$$
This formulation captures:
- Dynamic magnetic domain wall motion (reversible vs. irreversible magnetization).
- Minor B-H hysteresis loops and magnetic memory.
- Harmonic distortion and saturation compression as continuous nonlinear state-space responses.
By contrast, `vhs-phys` currently utilizes a simplified **saturating relay model** (coercive threshold switching), trading fine hysteresis dynamics for computational feasibility in the video RF domain.

---

## 4. Engineering Roadmap: Advancing Physical and Electrical Accuracy

To advance `vhs-phys` strictly as a physical and electrical model of video recording hardware, the following enhancements should be implemented:

```mermaid
flowchart TD
    subgraph Priority1["1. Core Electromagnetic & Signal Physics"]
        A1["Non-linear Level-Dependent Emphasis Network"]
        A2["Preisach / Algebraic Hysteresis Formulation"]
        A3["Head Pole-Tip Contour Effect (Head Bumps)"]
    end

    subgraph Priority2["2. Circuit Architecture & Demodulation"]
        B1["Closed-Loop 4.21 MHz APC / PLL Heterodyne Circuit"]
        B2["True 59.94 Field-Sequential Temporal Processing"]
        B3["Hi-Fi Depth Multiplex FM Stereo Audio"]
    end

    subgraph Priority3["3. Formats & High-Performance Compute"]
        C1["PAL 625/50 & S-VHS Formats"]
        C2["GPU Accelerated Continuous Pulse Rendering"]
    end

    Priority1 --> Priority2 --> Priority3
```

### 4.1 Non-linear (Level-Dependent) Emphasis and De-emphasis Networks
> [!IMPORTANT]
> The single largest discrepancy between the current `vhs-phys` record chain and SMPTE ST 32-1998 is the absence of **non-linear pre-emphasis**.

* **Circuit Physics:** VHS uses two cascaded emphasis networks on luminance:
  1. *Main linear pre-emphasis:* A high-frequency shelf network ($1.3\,\mu\text{s}$ time constant, $+14\text{ dB}$ HF boost) modeled in [`record.rs`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/record.rs#L35-L36).
  2. *Non-linear dynamic pre-emphasis:* A high-pass network loaded by a back-to-back diode limiter. Small-amplitude high frequencies pass through and receive up to $+10\text{ dB}$ additional boost. Large-amplitude transients (sharp contrast edges) forward-bias the diodes, clamping the boost to prevent over-deviating the FM carrier past the white/dark clipping limits.
* **Playback Circuit Physics:** An inverse non-linear de-emphasis circuit expands small high frequencies while compressing large ones, achieving significant high-frequency FM noise reduction.
* **Proposed Implementation:**
  In `record.rs`, insert a stateful diode-clamp circuit:
  $$i_{\text{diode}} = I_s \left(e^{\frac{v}{n V_t}} - e^{-\frac{v}{n V_t}}\right) = 2 I_s \sinh\left(\frac{v}{n V_t}\right)$$
  In `electronics.rs`, add matching complementary non-linear feedback de-emphasis. This will eliminate edge over-clipping and reproduce genuine VHS edge response and texture noise reduction.

### 4.2 Upgrading Magnetic Recording Beyond the Saturating Relay
While the current coercive relay model accurately modulates chroma onto the FM duty cycle, it treats transitions as instantaneous steps.
* **Proposed Implementation:** Replace the hard-threshold relay in [`record.rs`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/record.rs#L50) with an algebraic Switching Field Distribution (SFD) formulation:
  $$M(H) = M_s \tanh\left(\frac{H \pm H_c}{\Delta H_c}\right)$$
  where $\Delta H_c$ is the switching field spread (typically $0.2\text{--}0.3\,H_c$ for cobalt-doped gamma ferric oxide). This introduces physical transition width broadening ($a$-parameter) as a direct consequence of recording current amplitude and tape formulation, naturally capturing high-frequency recording losses and partial self-demagnetization.

### 4.3 Head Pole-Tip Contour Effect ("Head Bumps")
* **Circuit Physics:** The Karlqvist equation assumes an infinite planar pole face. In real video and audio heads, the pole tips have finite physical dimensions ($50\text{--}100\,\mu\text{m}$). Long-wavelength magnetic flux couples into the outer edges of the head core, producing secondary interference ripples ("head bumps") in the frequency response below $1\text{ MHz}$.
* **Proposed Implementation:** In [`head.rs`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/head.rs) and [`audio.rs`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/audio.rs), modulate readback sensitivity with the finite pole-piece factor:
  $$K_{\text{contour}}(f) = 1 + \sum_{m} \alpha_m \cos\left(\frac{2 \pi f L_m}{v}\right) e^{-\beta_m f}$$
  This directly refines colour-under carrier behavior ($629\text{ kHz}$) and linear audio low-frequency response.

### 4.4 Closed-Loop 4.21 MHz APC / PLL Heterodyne Circuit
* **Circuit Physics:** Real decks convert $629.371\text{ kHz}$ colour-under to $3.58\text{ MHz}$ baseband by heterodyning with a $4.21\text{ MHz}$ carrier generated by a voltage-controlled oscillator (VCO) locked to the horizontal sync and burst phase via an Automatic Phase Control (APC) loop.
* **Current Limitation:** In [`decode.rs`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/decode.rs#L136-L163), chroma is demodulated open-loop against a fixed trigonometric carrier table, with phase corrections smoothed across lines.
* **Proposed Implementation:** Model the closed-loop analog phase detector (Gilbert cell multiplier) and low-pass lag-lead loop filter. This ensures that rapid phase jumps during tape stretch or head switching cause true transient color unlocking, phase hunting, and hue smearing.

### 4.5 True 59.94 Field-Sequential Processing
* **Physical Reality:** VHS helical tracks physically correspond to discrete fields recorded at $59.94\text{ fields/second}$, each capturing a unique instant of motion.
* **Current Limitation:** [`pipeline.rs`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/pipeline.rs) treats input frames as progressive $29.97\text{ fps}$, duplicating frame content across fields $2k$ and $2k+1$.
* **Proposed Implementation:** Refactor [`Session`](file:///home/whyte/git/vhs-phys/crates/vhsphys-core/src/pipeline.rs#L24) to accept field-rate input ($59.94\text{ fps}$ progressive or interlaced fields), mapping each field to a unique helical pass. This eliminates inter-field temporal duplication and allows authentic vertical-temporal interlace comb filtering.

### 4.6 Hi-Fi Stereo Depth-Multiplex FM Audio
* **Physical Reality:** VHS Hi-Fi records high-fidelity audio by frequency-modulating stereo carriers at $1.3\text{ MHz}$ (Left) and $1.7\text{ MHz}$ (Right). Separate rotating audio heads with steep azimuth angles ($\pm 30^\circ$) record audio deep into the oxide before the video heads overwrite the surface layer with video FM.
* **Degradation Physics:** When tracking is misaligned or tape is damaged, the FM audio carrier drops below the limiter threshold, causing sharp $59.94\text{ Hz}$ switching buzz, threshold clicks, and automatic muting fallback to the noisy linear track.
* **Proposed Implementation:** Model depth-multiplexed dual-layer recording in [`record.rs`] and [`head.rs`], providing a second pair of rotating FM heads and an FM audio demodulator block in [`audio.rs`].

---

## 5. Summary and Architectural Conclusion

| Goal | Architectural Approach | Expected Physical Result |
| :--- | :--- | :--- |
| **Edge Sharpness & Noise Fidelity** | Add non-linear diode-limiter emphasis and de-emphasis | Authentic edge transient response without artificial clipping; true SNR behavior on fine textures |
| **Magnetic Saturation Dynamics** | Upgrade from relay to algebraic switching field distribution | Natural high-frequency self-demagnetization and record-current saturation |
| **Chroma Stability Limits** | Replace open-loop burst table with closed-loop 4.21 MHz APC PLL | Accurate phase hunting, burst recovery delay, and hue distortion under stress |
| **Temporal Accuracy** | Move from progressive frame duplication to true 59.94 field pipeline | Accurate field-rate motion judder, head-switch line positioning, and 3D comb behavior |
| **Complete Audio Fidelity** | Implement depth-multiplexed Hi-Fi FM audio with linear fallback | Authentic head-switching buzz, FM dropouts, and audio track switching mechanics |

By remaining committed to rigorous physical modeling over heuristic shortcuts, `vhs-phys` establishes a new scientific benchmark in analog media emulation.
