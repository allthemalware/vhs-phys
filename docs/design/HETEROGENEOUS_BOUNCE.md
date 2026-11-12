# Research Design: Heterogeneous VHS Bouncing

## Status and decision

Steps 1 to 7 of the implementation sequence (section 9) are in the code: `signal.rs`, `aperture.rs`, `convert.rs` and `rack.rs` in `crates/vhsphys-core/src`, the rack config and the mixed-raster run in the command line tool, and `tests/heterogeneous.rs`. What a user configures is in [docs/BOUNCE.md](../BOUNCE.md). Section 11 records what was decided while implementing and what is still open.

This began as a research design for a future implementation. It is not evidence that a particular commercial standards converter, VCR, or cable has been calibrated by this project. The proposed software reference path is deliberately distinguished from a model of any named appliance.

A generation is one VHS record/tape/playback pass. Generations may use different deck and tape parameters. A change in raster timing or colour encoding is allowed only through a declared deck playback mode or an explicit, configured signal link. Every generation and link exchanges timed electrical-signal samples; only the terminal television/CRT path creates an RGB Frame.

The central design decision is to keep four things separate:

1. **Tape recording mode**: the magnetic recording format and mode, including track geometry, tape speed, head arrangement, and the format's record/playback processing.
2. **Scan timing**: the electrical output raster, line and field cadence, interlace phase, and active aperture. A tape mode and a raster are related by a deck profile, not identical concepts.
3. **Colour system and decoder**: PAL, NTSC, or another specified encoding, plus the decoder that recovers components. Colour encoding is not interchangeable with raster timing. The repository's PAL-60 path is a useful example: a 525-line NTSC tape can be decoded by a PAL television without becoming 625/50 video.
4. **Interconnection and any standards converter**: a direct decoded Y/C reference link, composite video over a defined interface, or an explicit converter with a named algorithm/profile.

A multi-format deck can record one mode and emit a different playback signal; the playback conversion must be part of that deck's profile. The graph therefore tags a tape's recorded mode separately from the playback output's scan and colour system.

A mismatched link must fail validation. It must not resize an RGB frame, relabel a sample buffer, or silently choose a conversion method.

## 1. Evidence base and limits of the evidence

The sources support a physically meaningful signal path. The BBC Research Department report supplies a concrete, published four-field aperture design that can anchor an algorithmic reference profile; it does not specify one universal commercial PAL/NTSC converter response.

| Evidence | What it establishes | Limit |
|---|---|---|
| Repository architecture and code, especially [docs/ARCHITECTURE.md](../ARCHITECTURE.md), [docs/RESEARCH.md](../RESEARCH.md), [docs/PAL.md](../PAL.md), [docs/COMPOSITE.md](../COMPOSITE.md), [docs/AUDIO.md](../AUDIO.md), and the core pipeline/video/decode modules | Existing engine timing, VHS record/playback path, baseband representation, same-engine Chain, calibration boundary, and final CRT renderer | This is the simulator's current model and its declared assumptions, not a measurement of every VCR or converter. |
| [SMPTE ST 32M-1998, Video Recording — 1/2-in Type H](https://pub.smpte.org/pub/st32/st0032-1998_stable2004.pdf) | 525/60 Type H recording requirements, field-to-track relationship, luma FM, colour-under recording, pre-emphasis, clipping, and head switching | It does not define the response of a line output, cable, input separator, or external standards converter; it is not a source for PAL 625 VHS parameters. |
| [ITU-R BT.470-7](https://www.itu.int/rec/R-REC-BT.470-7-200502-I/en) and [ITU-R BT.1700-0](https://www.itu.int/rec/R-REC-BT.1700-0-200502-I/en) | Conventional analogue television raster and composite-signal systems | They do not define a digital converter's interpolation kernel or a VCR's proprietary processing. |
| [ITU-R BT.500-15](https://www.itu.int/rec/R-REC-BT.500-15-202305-I/en) | Current ITU methodology for subjective assessment of television image quality | It guides controlled viewing tests; it does not replace waveform-level converter validation or device calibration. |
| Clarke and Tanton, [BBC Research Department Report 1984/20, *Digital Standards Conversion: Interpolation Theory and Aperture Synthesis*](https://downloads.bbc.co.uk/rd/pubs/reports/1984-20.pdf) | Standards conversion is sampling-rate conversion in space and time; interlaced fields occupy offset sampling positions; converter filtering trades detail against aliases, judder, and flicker | It analyses converter theory and implementation choices; it is not a normative profile for a consumer converter. |
| [SMPTE RP 197-2003, Telecine Film Transfer Lists](https://pub.smpte.org/doc/rp197/20031020-pub/rp0197-2003_stable2009.pdf) | Defines film-transfer metadata and documents 3:2 pulldown as a 24-frame/s film to 30-frame/s video transfer relationship | It does not define generic 625/50 to 525/59.94 video standards conversion. |
| [JVC Video Technical Guide VTG82063, section 3](https://decode-orc.github.io/analogue-video-specifications/vhs/JVC-Video-Technical-Guide-VTG82063/section-3/) | A manufacturer's VHS circuit description: colour-under conversion and phase rotation, FM luma playback, AGC, dropout compensation, demodulation, and PAL/NTSC variants | The accessible copy is a reproduced technical guide. Model-specific circuit values must not be generalized to every deck. |
| C. P. Singh, [“Experiment on Video Signal Degradation Due to Generation Loss on VHS Format Recording”](https://bprd.nic.in/uploads/pdf/8484241726-april-june2006.pdf), *The Indian Police Journal*, 2006 | A controlled, same-make copy sequence is evidence that repeated recording/playback can alter measured output levels | The reported setup averages three waveform measurements at selected lines and explicitly notes dependence on the camera and VCR. It is not adequate to fit a universal per-generation loss curve. |

The BBC report explains that the converter must reconstruct picture information at new line and field positions, and warns that simple interpolation can impair moving objects and vertical detail. It treats interlaced input as a two-dimensional space/time sampling lattice: mixing odd and even fields as if they were simultaneous is wrong in moving regions. It also relates passband and stopband errors to blur, vertical aliasing, judder, flicker, and modulation of moving detail. These findings rule out a frame resize plus frame-rate change as an adequate converter model. Its Section 5 gives direction-specific four-field luminance apertures and a separate chroma aperture. Tables 9 and 11 are identified as the preferred general-use compromise in their respective directions: less blur in rapid motion than the more strongly anti-judder alternatives, at the cost of more judder. Table 12 gives the chroma response. Those published responses are a stronger starting point than choosing an unnamed kernel, but still need exact-clock adaptation, coefficient generation, and analytic verification for this simulator.

The standards and manuals establish the system boundaries and the deck mechanisms. They do not supply one authoritative set of passband, stopband, phase, latency, or motion-estimation parameters for an unspecified “PAL/NTSC transcoder.” Those values must belong to a declared mathematical reference profile or to measurements of a named device. Assigning an undocumented Catmull-Rom, windowed-sinc, or motion-interpolation kernel to a commercial converter would imply evidence we do not have.

## 2. Findings from this repository

### 2.1 The homogeneous path is a sound compatibility anchor

The current pipeline::Chain already models successive VHS generations. It sends each upstream Session::process_baseband() result to the next session's push_baseband(). It avoids intermediate RGB quantization and does not send intermediate signals through the CRT. A single-stage chain is intended to match an ordinary Session. This is the correct baseline to preserve.

The two engines compile the shared signal-processing source against separate 525-line and 625-line constants. Their Baseband types are therefore engine-specific even though both carry Y/C planes. Baseband has no stream-level raster tag, field timestamp, connector type, or external-clock metadata. A heterogeneous host needs an explicit typed boundary around these engine-local buffers; it must not infer the signal standard from vector length or sample index.

### 2.2 Current active dimensions are an internal raster choice

The current code declares 754 × 480 samples for its 525-line active picture and 740 × 576 for its 625-line picture. The old draft gave NTSC as 740 × 480. Correct that value if discussing the current implementation. More fundamentally, conventional analogue television standards define scanning and signal timing, not a normative horizontal pixel count. The code's active widths are internal numerical representations, not VHS or ITU pixel specifications. Converter geometry must use active aperture and physical/sample coordinates, with aspect ratio metadata, rather than treating these widths as universal standards values.

### 2.3 Tape mode, raster, and “PAL/NTSC” are not one enum dimension

The code distinguishes the 625-line PAL engine from PAL colour decoding on a 525-line NTSC raster (PAL-60). A heterogeneous design must keep tape recording mode, raster cadence, colour encoding, playback output mode, and display decoder as separate fields. PAL-60 is not a 625/50 standards conversion. A multi-standard deck mode that changes the playback colour output is a different operation from a converter that changes both field cadence and raster line structure. A deck capability profile must declare which recorded modes it accepts, how it records each mode, and the timed electrical signal it emits for each playback mode.

### 2.4 Composite display processing is not yet a complete link model

composite::roundtrip_with_tv() represents composite encode/separation associated with a television connection. It is useful signal-domain machinery, but a future inter-deck link needs its own input and output port semantics, cable/electrical profile, and separator configuration. A terminal monitor's composite setting must not accidentally stand in for every cable and VCR input in a multi-generation rack. Preserve current homogeneous Chain results while separating these concepts.

### 2.5 The current draft's unsupported choices

The prior proposal is a useful motivation document but should not be implemented as written:

- Its StandardsTranscoder::transcode(&Frame) boundary would make RGB frames an intermediate signal and violate the project's signal-before-Frame rule.
- It specifies filter families without the cutoff, ripple, attenuation, phase, aperture, boundary conditions, or coefficients needed to reproduce them.
- Its temporal modes conflate generic standards conversion with cadence patterns that are meaningful only when the source cadence is known.
- It assumes a fixed 48 kHz audio clock and equal frame counts where the physical video field clocks differ.
- It treats 740 × 480 as the NTSC active raster, whereas the current code uses 754 × 480; neither width is a normative analogue-TV quantity.
- It does not carry field parity, half-line phase, source timestamps, or colour-subcarrier phase across a link.

### 2.6 Cross-document review

The wider documentation was reviewed for design constraints and evidence quality. The relevant findings are:

| Documents | Finding for this design |
|---|---|
| [CAMERA.md](../CAMERA.md) | The camera tube is a source-capture process. It belongs before the first recording generation, not between later deck generations. |
| [TAPE_SPEEDS.md](../TAPE_SPEEDS.md), [INTERFERENCE.md](../INTERFERENCE.md) | Tape geometry and external fields are tied to physical stages and absolute coordinates/time. Each deck/link needs stable identity and independent parameters/seeds; one global “generation degradation” slider would discard this causal structure. |
| [PHYSICAL_MODELLING_RESEARCH.md](../PHYSICAL_MODELLING_RESEARCH.md), [RESEARCH.md](../RESEARCH.md), [AUDIO.md](../AUDIO.md) | The project distinguishes standard values, service data, derived values, measurements, and estimates. Preserve those provenance labels for converter and connector parameters; audio has a distinct path and latency. |
| [COMPOSITE.md](../COMPOSITE.md), [PAL.md](../PAL.md) | Composite encoding/separation and PAL phase behavior are signal operations. Keep them separate from raster resampling and from the terminal CRT. PAL-60 is a color-decoder mode on a 525-line raster. |
| [COMPARISON.md](../COMPARISON.md), [PLUGIN.md](../PLUGIN.md), [ROADMAP.md](../ROADMAP.md), [README.md](../../README.md) | These describe current capabilities and future product goals, but some status statements and fixed-raster assumptions disagree with the current code. They are not normative converter specifications. Reconcile them before announcing feature support. |
| [design/FORMAT.md](FORMAT.md), [design/SCHEMATIC.md](SCHEMATIC.md) | These define UI/configuration presentation, not a timed signal-port contract. A rack UI should expose validated model choices rather than define signal semantics. |
| [PERFORMANCE.md](../PERFORMANCE.md), [GPU_RESEARCH.md](../GPU_RESEARCH.md) | Throughput estimates and acceleration plans do not validate physical fidelity or the converter response. |

## 3. Proposed signal graph

The first implementation should remain a serial chain. A general directed acyclic graph can be considered later; it adds branching, synchronization, and validation cases that are not needed to model a dub.

    media source
      -> source decoder and timed source adapter
      -> optional camera model
      -> VHS generation 0: record -> tape -> playback/decode
      -> explicit link 0
      -> VHS generation 1: record -> tape -> playback/decode
      -> ...
      -> explicit link N-2
      -> VHS generation N-1: record -> tape -> playback/decode
      -> terminal television decoder / CRT beam
      -> RGB Frame
      -> final container encoder

A generation consists of the existing record, magnetic tape, head, playback electronics, and deck decode processes. Only the first generation receives source pictures and may run the source camera model. Intermediate generations receive the previous link's electrical signal. Only the final generation feeds a terminal monitor path. Neither an intermediate deck nor a link may call crt::scan_to_frame().

A GUI preview of an intermediate generation is a read-only monitor branch: it observes that generation's signal through its own TV decoder/CRT renderer, and its Frame is never fed back into the chain. This keeps preview behavior consistent with the same physical endpoint rule.

The old Frame type is currently used for both decoded input pictures and rendered output. The implementation should eventually distinguish decoded SourceImage samples from terminal output Frames at the API boundary, so a type signature cannot accidentally make an output Frame the next generation's input.

The source adapter must retain the media PTS/time base, interlace and field-order metadata, active crop, sample/display aspect ratios, primaries, transfer function, matrix, range, and bit depth. With the camera tube enabled, decode the source transfer function to a declared linear-light/exposure domain before the tube model. With the tube disabled, map decoded source values to the declared electrical video-signal levels. Code values are not volts or scene radiance by themselves. If required metadata is missing, a user-selected interpretation must be recorded with the render rather than guessed invisibly.

## 4. Signal and time contract

A signal block is a finite sample window representing a band-limited electrical signal. Samples are not an untyped “continuous float image”: the block must state enough information to reconstruct the represented waveform and align it with other blocks.

Each block or port must carry:

| Field | Required meaning |
|---|---|
| Raster timing | Line count; exact frame and field rates; line period; 2:1 interlace state; field parity and half-line phase; active line and sample aperture |
| Colour coding | Colour system; component axes/matrix and voltage levels; primaries/white point and transfer function at media boundaries; subcarrier frequency and phase convention; burst and line-switch state |
| Interface | Port direction and capabilities; composite-system variant; connector; termination and impedance; signal bandwidth; sync, blanking, and burst levels |
| Signal form | Composite, separate Y/C, or the project's explicit direct decoded-baseband reference port |
| Deck mode | Recorded tape format; record-input signal profile; playback output raster, colour system, and interface; supported internal conversions |
| Sample clock | Exact rational sample rate or exact line-locked samples-per-line relation, sample origin, and reconstruction/anti-alias convention |
| Timeline | Source/program presentation timestamp for the first valid sample or field; source time base, field order and field times; active aperture and sample/display aspect ratios; valid range and end-of-stream padding policy |
| Delay | Physical signal/group delay separately from buffering/lookahead and availability time |
| Provenance | Engine/profile identifier; parameter source and revision; the repository's evidence tag ([std], [svc], [derived], [meas], or [est]); uncertainty/range for fitted or estimated values; and deterministic random-stream identity where noise is generated |

The 525-line engine's baseband grid is line-locked at 4 × its colour-subcarrier frequency (910 samples per line). The 625-line engine also has 910 samples per line, but its grid is 910 × 15,625 = 14.21875 MS/s; it is not 4 × the PAL colour-subcarrier frequency. Therefore, equal sample indices in two engine buffers do not mean equal elapsed time. The converter must map by rational time and line/field coordinates, not by copying indices.

Keep parameter provenance at the parameter level, not only on the profile as a whole. Use the repository's existing [std], [svc], [derived], [meas], and [est] labels. Every [meas] or [est] response should carry an uncertainty or declared range and the conditions under which it applies. A model fit should preserve the raw capture, calibration state, fit procedure, residuals, and held-out validation set; do not relabel a fitted estimate as a standard value.

Use reduced rational timestamps for frame/field scheduling and source PTS. Relevant field periods are exactly 1001/60000 s for 525/59.94 and 1/50 s for 625/50; the corresponding two-field frame periods are 1001/30000 s and 1/25 s. Do not accumulate either clock in binary floating point. Local DSP may use floating-point coefficients and samples, but the sample-to-time map and target event schedule must remain exact.

Separate **signal time** from **availability time**. A symmetric interpolation filter may require future fields; that creates computational lookahead and memory use. It is not automatically a physical delay in the signal. A device's measured group delay is signal behavior and must be represented separately. Audio/video alignment follows signal timestamps and declared path delays, not the order in which buffers become ready.

The program interval, source-frame timestamp policy, output frame/field schedule, and end-of-stream behavior must be explicit. Output count is derived from that timeline policy and the target clock, not assumed equal to source frame count. Variable-frame-rate sources retain their timestamps. Progressive and interlaced source inputs require different source reconstruction; an interlaced frame must be represented as two field samples at their actual times, not silently woven into one simultaneous picture.

## 5. Link types and standards conversion

A link is a physical or explicitly ideal interface between a playback output and the next record input. Its contract owns the connection behavior. The video and audio links may have different routes and delays.

### 5.1 Initial link profiles

1. **Direct Y/C reference link** — the compatibility path corresponding to today's same-standard baseband handoff. It transfers decoded luma/chroma signal samples directly, with no connector, cable, composite cross-effects, or standard conversion. It is a declared numerical reference link, not a claim that ordinary VHS decks have a hidden component connector.
2. **Separate Y/C link** — separate luma/sync and chroma/burst signal paths. Add only connector, impedance, bandwidth, noise, and delay effects that have a stated profile or measurement.
3. **Composite link** — the upstream deck's composite output is formed as a waveform; a declared cable/load profile may alter it; the downstream deck's input separator recovers luma/chroma. The output and input separator are separate model blocks. A PAL/NTSC change is not implied.
4. **Standards-converter link** — a named conversion algorithm/profile between specified input and output raster/colour systems. It may include composite input/output stages if those are the actual modeled connectors. It cannot be a generic Frame transcoder.

The reference link preserves present same-standard behavior. A realistic composite dub is a separate experiment and should be opt-in until its link model has an evidence-backed profile. If no converter is configured for a raster mismatch, graph construction returns a descriptive error.

For any composite or broadcast-style output, name the complete signal system, such as ITU System M, B/G, or I, or identify a named non-ITU variant. “PAL” alone does not determine line timing, blanking, or luminance bandwidth; PAL-60 combines PAL colour encoding with 525-line timing and therefore needs an explicit profile.

### 5.2 Converter signal path

For a composite input, the conceptual converter path is:

    source composite waveform
      -> input termination/cable profile
      -> sync and burst/timing recovery
      -> source colour decoder to component signals
      -> space-time reconstruction and target-raster resampling
      -> target colour encoder and target sync/burst generation
      -> output interface profile
      -> receiving VHS record input

For a direct Y/C input, omit composite encoding/separation while retaining the stated timing and colour conversion. The exact endpoint must be a profile choice. The JVC guide describes VHS colour-under conversion, per-line phase rotation, and separate luma/chroma playback processing; SMPTE ST 32 specifies the luma and chroma recording paths. Neither source supports collapsing a deck output, cable, converter, and receiving deck into one unspecified “transcoder.”

The project's current Baseband U/V axes and IRE conventions are not automatically equivalent to every digital YCbCr matrix or to an arbitrary converter's component voltage convention. Declare the color matrix, reference levels, transfer functions, and chroma-axis mapping at each media/electrical boundary. Never interpolate raw PAL and NTSC modulated chroma as though their subcarrier phase conventions were shared.

### 5.3 Space-time resampling

Standards conversion changes the sampling lattice. For colour component c, a reference reconstruction can be written as

    s_hat_c(x_o, y_o, t_o) = sum over i,j,m of
        s_c(i,j,m) * K_c(x_o-x_i, y_o-y_(j,m), t_o-t_m).

The source sample coordinates y_(j,m) and t_m include field parity and interlace offset. The output samples are then generated on the target system's line/field clock. This equation defines the architecture, not a claimed physical kernel. A converter profile must publish its actual finite-support kernel or coefficients, phase response, passband, transition band, stopband attenuation, edge handling, and total signal delay.

#### Proposed deterministic reference profile

Use the published BBC four-field response design as the mathematical starting point for the software reference converter. Name it as a project-derived algorithmic profile, not as a model of a particular commercial converter. The report's choice is deliberate: it provides the same broad four-field non-separable aperture family in both directions, distinguishes luminance from chroma, and documents the image-quality tradeoff instead of hiding it inside a generic interpolation name.

The following response-point values summarize the report's preferred general-use luminance characteristics and its baseline chroma characteristic ([Clarke and Tanton, Section 5, Tables 9, 11, and 12](https://downloads.bbc.co.uk/rd/pubs/reports/1984-20.pdf)). Rows are vertical frequency in cycles per picture height (c/ph), descending; columns are temporal frequency in Hz, ascending. They are response constraints used to synthesize the aperture, not spatial-temporal convolution coefficients.

625/50 to 525/60 luminance, Table 9:

| c/ph \ Hz | 0 | 12.5 | 25 | 37.5 | 50 |
|---:|---:|---:|---:|---:|---:|
| 312.5 | 0 | 0 | 0 | 0 | 0 |
| 234.375 | 0.02 | 0 | 0 | 0 | 0 |
| 156.25 | 0.50 | 0.15 | 0 | 0 | 0 |
| 78.125 | 0.95 | 0.60 | 0.07 | 0 | 0 |
| 0 | 1.00 | 0.90 | 0.32 | 0 | 0 |

525/60 to 625/50 luminance, Table 11:

| c/ph \ Hz | 0 | 15 | 30 | 45 | 60 |
|---:|---:|---:|---:|---:|---:|
| 262.5 | 0 | 0 | 0 | 0 | 0 |
| 196.875 | 0.10 | 0 | 0 | 0 | 0 |
| 131.25 | 0.50 | 0.15 | 0 | 0 | 0 |
| 65.625 | 0.90 | 0.50 | 0.02 | 0 | 0 |
| 0 | 1.00 | 0.75 | 0.15 | 0 | 0 |

Reference chroma aperture, Table 12:

| c/ph \ Hz | 0 | 12.5 | 25 | 37.5 | 50 |
|---:|---:|---:|---:|---:|---:|
| 312.5 | 0 | 0 | 0 | 0 | 0 |
| 234.375 | 0 | 0 | 0 | 0 | 0 |
| 156.25 | 0.075 | 0 | 0 | 0 | 0 |
| 78.125 | 0.70 | 0.20 | 0 | 0 | 0 |
| 0 | 1.00 | 0.38 | 0 | 0 | 0 |

The report also gives Table 13 as an optional higher chroma-noise and cross-colour suppression characteristic for poor-quality sources, at the cost of more vertical chroma resolution. It should be a separate, named profile, not a hidden adaptive switch. Table 12 is the baseline because it preserves more wanted chroma detail.

Optional high-noise chroma aperture, Table 13:

| c/ph \ Hz | 0 | 12.5 | 25 | 37.5 | 50 |
|---:|---:|---:|---:|---:|---:|
| 312.5 | 0 | 0 | 0 | 0 | 0 |
| 234.375 | 0 | 0 | 0 | 0 | 0 |
| 156.25 | 0 | 0 | 0 | 0 | 0 |
| 78.125 | 0.35 | 0.07 | 0 | 0 | 0 |
| 0 | 1.00 | 0.35 | 0 | 0 | 0 |

These response matrices are not a complete executable coefficient set. To make the profile reproducible, freeze a coefficient-generation specification that follows the report's aperture-synthesis method: form the two-dimensional aperture from the fixed response points, truncate it to the declared finite support, sample it on a declared field/line coordinate grid, apply a unity-sum coefficient quantization rule, and store the resulting coefficients and checksum. The report's aperture-synthesis example uses a four-field-period by eight-picture-line support. Its experimental converter samples at eight positions per field period and sixteen per picture-line interval, and quantizes each 16-coefficient set so its sum remains unity. Treat those as historically documented reference implementation choices, then test whether that quantization is adequate in the project's sample representation. Do not assume old hardware coefficient precision is optimal for floating-point software.

The BBC paper labels its standards 625/50 and 525/60, while this repository's 525 engine uses the exact 1001/60000 field rate. Do not silently equate 60.000 with 60000/1001. For the project-derived profile, interpret the 525-side temporal response coordinates as fractions of the nominal 60 Hz source field clock and scale those coordinates by the exact ratio 1000/1001. Keep the 625/50 source coordinates unchanged; when using the report's chroma aperture for the reverse direction, scale its temporal coordinates by the source-field ratio (60000/1001)/50 = 1200/1001, as the report specifies that the reverse-direction frequency characteristics are scaled. Express the kernel on exact rational field and line coordinates, then evaluate its response against both the published response points and the alias loci of the actual source/target clocks. This is an explicit project derivation, not a claim that the report itself studied 59.94 Hz. Any profile that preserves a nominal 60 Hz input must be separately named and must not be presented as the exact NTSC engine path.

Operational requirements for the reference profile:

- Decode the source colour encoding to a common, declared component basis before resampling; apply the documented matrix, range, and chroma scaling at the boundary. Do not blend PAL and NTSC subcarrier phases as if they were the same component values.
- Work from fields and their true timestamps. Preserve the interlaced sampling geometry. Do not resize a woven frame, blend fields captured at different times without an explicit motion model, or discard parity metadata.
- Treat the BBC response points as vertical-temporal constraints; they do not specify the horizontal line-domain response. Separately declare the horizontal active-video interval, aspect-ratio mapping, source/target video bandwidth, horizontal resampling filter, and sync/blanking regeneration. Resample the electrical line waveform in physical time and preserve the declared picture geometry; never infer horizontal bandwidth from the engine's 754- or 740-sample width.
- Implement deterministic finite-support convolution over the source field/line lattice, with a separately specified within-line resampler. Store all fixed response points, source/target clock rationals, aperture support, sampling grid, quantizer, coefficient precision, phase convention, edge rule, version, and coefficient checksum with the profile.
- Define image-boundary and programme-start/end behavior. Filter warm-up, lookahead, and padding must not invent undocumented picture history.
- Make conversion direction, source and target colour system, active-aperture mapping, sample clocks, algorithm version, latency, and provenance part of configuration and render metadata.

A zero-order field-hold profile can be retained as a diagnostic baseline. It is deterministic but has cadence judder and is not a model of a high-quality standards converter. Motion-compensated conversion must be a separately named algorithm with a specified estimator, confidence/fallback behavior, and validation set; it must not be described as a generic analog property. Film cadence extraction/pulldown likewise requires a declared film-origin cadence and detection policy. SMPTE RP 197 documents 3:2 pulldown for film-to-video transfer, so it is not a generic 625/50 ↔ 525/59.94 video conversion rule.

The acceptance criteria must evaluate the frequency response and temporal behavior as a space-time system. A still frame resize test alone cannot validate field conversion. The BBC report's analysis specifically shows that converter-filter errors can appear as vertical aliasing, motion judder, detail flicker, and moving-edge modulation.

## 6. Audio and timing

Audio is a separately clocked signal path. Video standards determine field and line rates; they do not by themselves require audio to change sample rate or playback pitch.

- Preserve decoded audio PTS and the selected sample clock. If an internal processing rate is used (the current CLI path uses 48 kHz), resampling must be explicit, band-limited, and timestamp preserving.
- Give each generation an audio route and mode: linear track, modeled Hi-Fi, bypass, or mute. Carry each generation's wow/flutter and tape/deck effects through its own processor.
- A video-only standards converter leaves the audio program timeline unchanged unless its declared device profile also retimes or delays audio. Do not change audio speed to force its sample count to match a video frame count.
- Track processor group delay, stream buffering, and any converter audio delay independently; align at final mux from PTS and path-delay metadata.
- Preserve the current audio chain's latency compensation behavior for same-rate homogeneous runs, then add rational timestamp support before heterogeneous audio rates or converter delays are enabled.

The present audio renderer is scheduled separately from the video renderer and uses a fixed 48 kHz processing rate. That is a practical current implementation, not a physical consequence of PAL or NTSC. Heterogeneous A/V synchronization needs an explicit shared programme timeline.

## 7. Configuration and runtime contract

Configuration should describe a sequence of generation nodes and one link between each adjacent pair. A conceptual configuration shape is:

    program:
      start_time: rational
      end_time: rational
      source_timebase: rational
      source_scan: progressive | interlaced
      source_reconstruction: declared_profile

    generation:
      - name: first dub
        record_mode: vhs_525_5994 | vhs_625_50 | ...
        input_signal: raster_and_colour_profile
        tape_speed: ...
        deck_profile: ...
        playback_mode: native | declared_conversion_profile
        output_signal: raster_and_colour_profile
        seed: ...
      - name: second dub
        record_mode: ...
        input_signal: ...
        tape_speed: ...
        deck_profile: ...
        playback_mode: ...
        output_signal: ...
        seed: ...

    link:
      - name: link from first to second dub
        type: direct_yc_reference | separate_yc | composite | standards_converter
        input_port: ...
        output_port: ...
        conversion_profile: optional declared profile
        audio_route: ...
        seed: ...

This is a schema sketch, not a committed TOML API. Validation runs before media processing and checks every adjacent port pair, the deck's record/playback capabilities, tape speed, raster, colour system, exact clock relationship, connector, conversion profile, audio route, and parameter range. Error messages should identify the edge and the incompatible fields. A standard mismatch must never be “fixed” by resizing or relabeling.

The scheduler should be streaming and bounded-memory:

- Each source, deck, converter, and output exposes a timestamped chunk interface with push, ready, pull, and finish semantics.
- Each stage declares temporal support/lookahead, group delay, and required history. The scheduler propagates watermarks and applies back-pressure; it does not accumulate the entire programme in a vector of Frames.
- At end of stream, flush each stage exactly once with its declared boundary policy, then propagate end-of-stream downstream. Do not pad with an arbitrary blank frame to satisfy a frame-count assumption.
- Derive deterministic seeds from the root seed and stable stage/link identity. Noise remains a pure function of seed plus absolute time/track coordinates so that chunk size, scheduling, seeking, and thread count do not change results.
- Calibrate each deck independently with its own reference conditions. Do not calibrate a later generation against a degraded previous output; that would hide accumulated loss.

A useful host-side API is a tagged signal port and a dispatcher over the two monomorphized engines. Keep each engine's inner processing statically typed. The boundary adapter validates metadata and makes the engine transition explicit. A trait that accepts/returns untagged Frame objects is the wrong abstraction.

## 8. Validation plan

Validation separates mathematical correctness, software compatibility, and empirical physical accuracy. Passing one level does not establish the next.

| Level | Required evidence |
|---|---|
| Existing-path regression | One-generation output is identical to the existing Session; homogeneous multi-generation Chain output is identical when every link selects the direct Y/C reference profile. Preserve current deterministic and arbitrary-start behavior. |
| Port and clock invariants | Reject incompatible links; prove target field PTS, parity, half-line phase, burst phase, sample origin, and end-of-stream count against rational arithmetic. Verify chunk-size, thread-count, and seek invariance. |
| Signal-domain invariants | No intermediate edge carries Frame; a preview monitor branch cannot feed the chain; only the terminal TV/CRT path creates output RGB. |
| Converter analytic tests | Constant Y/C levels; component impulse and step responses; horizontal/vertical/temporal multiburst; zone plates; moving diagonals and point targets; alternating-field motion; burst-phase sweeps; PAL line alternation; source and target Nyquist/alias tests. Measure passband ripple, stopband rejection, phase/group delay, edge response, motion judder, and flicker against the declared profile. |
| Generation tests | Track luma/chroma transfer, noise, sync/burst levels, dropout compensation, and timing across each generation. Verify that every generation adds only its configured physical/link effects; do not assume a universal monotonic scalar “quality” metric. |
| A/V synchronization | Use a video flash/field marker and audio impulse at the same rational PTS. Measure end-to-end signal delay, sample-rate conversion, wow/flutter, channel latency, and mux timestamps through same-standard and converted chains. |
| Device calibration | For a named deck/converter, retain service-manual revision, model/revision, test equipment, calibration state, source pattern, capture tap point, raw captures, uncertainty, fitting procedure, and held-out validation clips. Compare waveform-level quantities before judging rendered images. |
| Subjective claims | If claiming a perceptual preference between response profiles, use a controlled, randomized viewing protocol with fixed display conditions, screened observers, representative static and moving clips, and the current ITU-R BT.500 method. Keep perceptual results separate from analytical response and physical calibration claims. |

Test content should include both static and moving material. A chart suite needs grayscale and color bars, multiburst, zone plate, slanted edges, fine vertical detail, moving diagonal/vertical edges, and isolated flashes/impulses. A moving pattern is essential because still-image conversion can conceal incorrect inter-field mixing. Compare component and waveform signals before the CRT renderer; final RGB perceptual scores are secondary.

The small Singh copy-sequence study is consistent with generation-dependent changes in one specific setup, but it measures only a few video/sync voltage levels and acknowledges equipment dependence. It supports measuring a real chain; it does not provide a response curve to encode into this simulator. Until named hardware captures exist, report the converter as an algorithmic reference and mark its physical transfer characteristics as uncalibrated.

## 9. Implementation sequence

1. **Evidence and contract freeze** — define raster, colour, connector, source timestamp, field, signal-unit, and programme-end types. Reconcile stale documentation before claiming support.
2. **Compatibility adapter** — wrap the current same-engine Baseband transfer in timestamped, tagged signal chunks. Prove identity against Session and Chain; keep the CRT out of intermediate stages.
3. **Explicit signal links** — separate direct Y/C reference, composite output/input separation, cable profile, terminal TV decode, and audio route. Preserve existing defaults and golden output.
4. **Rational field scheduler** — add timestamped field input and output, parity-aware source reconstruction, exact rational clock mapping, bounded queues, and end-of-stream propagation.
5. **Reference standards converter** — implement a project-derived four-field profile from the BBC report's direction-specific luminance Tables 9/11 and chroma Table 12; adapt it explicitly to exact engine clocks, publish the aperture synthesis and coefficient-generation procedure, and test measured response, delay, and edge behavior before connecting it to a VHS deck.
6. **Heterogeneous dispatcher and config** — construct PAL or NTSC sessions per generation, validate every edge, and reject unsupported combinations before rendering.
7. **Audio alignment** — use the same program timestamps and explicit audio-path delays; keep raster conversion independent of audio unless a device profile says otherwise.
8. **Measured hardware profiles and UI** — only after calibrated captures exist, add named deck/converter profiles and intermediate signal-monitor previews.

## 10. Research conclusions and open decisions

The architecture is now sufficiently specified to prevent the major category errors: frame-space conversion between generations, implicit PAL/NTSC changes, frame-index clocks shared across unlike engines, and conflating software lookahead with physical delay. The recommended software baseline is the report's four-field non-separable family with its preferred general-use responses in each direction and its distinct chroma aperture. It makes the target response and its motion tradeoff inspectable.

The vertical-temporal converter baseline is now research-backed, but the report does not set the horizontal line-domain response, the project's complete composite-system variants, or a ready-made coefficient file for this repository's 60000/1001 field clock. The exact-clock scaling rule is defined above; generate and verify its coefficients analytically. Before implementation, also freeze the horizontal active-video/bandwidth profile and component-level conventions using the selected ITU system or a named reference interface. Before labeling any profile as a particular commercial device, obtain that device's service documentation and/or calibrated input/output captures. The mathematical reference profile must not be presented as empirical appliance fidelity.

Other repository documentation is out of step with current code: the PAL-composite support status differs between the README, PAL/composite notes, and current config/tests; field-sequential input and Hi-Fi status also differ between roadmap, comparison, and implementation. This proposal follows the current code for its architecture observations and flags the doc drift for a separate reconciliation. It does not silently treat the inconsistent pages as additional specifications.

## 11. Implementation record

Decisions taken while implementing, where the design left a choice or the work showed something the design had not settled.

**Verified against the source.** Tables 9, 11, 12 and 13 above were checked against the scanned report and are as printed. The report's section 3.4.1 confirms the synthesis: the aperture is a sum of `cos t . cos y` terms weighted by the fixed points, truncated to four field periods by eight picture lines.

**Coefficient generation.** The aperture is expressed in source field periods and source picture lines, which makes the exact-clock rule of section 5.3 automatic: the temporal points of a 525-line source are multiples of a quarter of 60000/1001 Hz. Coefficients are computed in `f64` at the exact rational phase of each output line and field and each set of sixteen is scaled to unit sum. The report's 8 by 16 phase grid and 128th quantisation are not used for conversion. They are not needed: the reciprocal lattice of interlaced scanning, (1/2 cycle per picture line, 1/2 cycle per field), (1 cycle per line, 0) and (0, 1 cycle per field), lies on the grid of fixed points at zeros of all four tables, so the unscaled sets already add up to one at every phase (tested to 1e-9). The report's grid is kept only as the sampling for the recorded checksum of each aperture.

**Output count across several conversions.** "An output frame exists when it starts inside the source" lets the count grow from one conversion to the next, because a converted stream's last frame starts inside the programme and ends outside it (8 frames of 525 became 7 of 625 and then 9 of 525). The rule is now: an output frame exists when it starts before the end of its source frames and before the end of the original programme, which the rack passes to every link at end of stream.

**Start of each generation.** Each generation starts at its first frame at or after the start of the generation before it, not at or after the programme start, so no converter is asked for a picture from before its first source field.

**Edges.** Outside the active picture the signal is taken at black, horizontally and vertically, as on the wire. Before the first and after the last field of the programme the nearest field of the same parity is held. Both are declared in the profile record.

**Field difference is flicker.** The validation plan lists alternating-field patterns. One field white and the other black, standing still, is the same set of samples as a flat field flickering at the frame rate; the lattice cannot tell them apart. With table 9 it converts to a flat field per output field with gain 2 x 0.32 at 25 Hz (both images pass) and no line structure. The test asserts that, not silence.

**Colour on Y/C links.** Separate Y/C still carries chroma on a subcarrier, so `separate_yc` requires matching colour systems as `composite` does. PAL-60 output is accepted by an NTSC recorder only over the direct reference link (decoded components).

**Composite between decks.** A rack built from config keeps `composite.enabled` on the last generation only; `Chain` and `--generations` are unchanged and still apply it at every generation, so existing output is identical.

**Scheduler.** Each earlier generation plays whole batches into its link as they become ready and everything at end of stream; each link is flushed exactly once. Memory is bounded by a batch plus the converter's history (about three source frames).

**Sound.** The sound runs through every generation's processor on the one 48 kHz programme clock and is cut to start at the first output frame's time. Links carry it unchanged with no delay; a per-link audio delay is a field of the record but is always zero.

Still open:

- Step 8: named measured profiles and previews of intermediate generations (a monitor branch that cannot feed the chain).
- Cable, connector and termination profiles for any link; a converter with composite input and output stages.
- The `SourceImage` / output `Frame` type split of section 3 and the timed source adapter (source PTS, interlaced and variable-rate input, transfer function and matrix metadata). The input is still resampled to the first generation's frame rate by ffmpeg.
- Chunk interface for sources and outputs; branching graphs.
- The converter's response is tested at the fixed points, for flat fields, still vertical detail, whole-field temporal change, field difference and a moving edge. Zone plates, multiburst through the horizontal filter in MHz, burst-phase sweeps and a measured A/V marker test of section 8 are not written.
- The documentation drift noted in section 10 was not reconciled beyond the pages this work touched.
- Found while testing, not changed: the 625-line engine plays colour bars about 11 % more saturated than recorded, and again for each further 625-line generation (75 % magenta 191 -> 205 -> 223). The converter itself is exact on the same bars, so the excess in a mixed rack is the 625-line decks'.

## References

### Repository design and implementation

- [Architecture](../ARCHITECTURE.md) — exact engine clocks, engine separation, field pipeline, and signal-before-Frame invariant.
- [Research notes](../RESEARCH.md) — VHS model parameters and provenance tags.
- [PAL model](../PAL.md) — PAL 625 timing and PAL-60 distinction.
- [Composite model](../COMPOSITE.md) — current Y/C composite round-trip.
- [Audio model](../AUDIO.md) — tape/deck audio path, processing rate, and latency.
- [Physical modelling research](../PHYSICAL_MODELLING_RESEARCH.md) — declared model limits and future physics.

### Standards and engineering literature

- SMPTE, [ST 32M-1998, Video Recording — 1/2-in Type H — Cassette, Tape and Records](https://pub.smpte.org/pub/st32/st0032-1998_stable2004.pdf), especially sections 3.6, 3.8, 3.9, 7.5 and annex A.
- ITU-R, [Recommendation BT.470-7, Conventional analogue television systems](https://www.itu.int/rec/R-REC-BT.470-7-200502-I/en).
- ITU-R, [Recommendation BT.1700-0, Characteristics of composite video signals for conventional analogue television systems](https://www.itu.int/rec/R-REC-BT.1700-0-200502-I/en).
- ITU-R, [Recommendation BT.500-15, Methodologies for the subjective assessment of the quality of television images](https://www.itu.int/rec/R-REC-BT.500-15-202305-I/en).
- C. K. P. Clarke and N. E. Tanton, [Digital Standards Conversion: Interpolation Theory and Aperture Synthesis](https://downloads.bbc.co.uk/rd/pubs/reports/1984-20.pdf), BBC Research Department Report 1984/20, December 1984.
- SMPTE, [RP 197-2003, Telecine Film Transfer Lists](https://pub.smpte.org/doc/rp197/20031020-pub/rp0197-2003_stable2009.pdf), especially section 7.5.3.
- E. R. Rout and R. E. Davies, [Electronic Standards Conversion for Transatlantic Color Television](https://journal.smpte.org/periodicals/Journal%20of%20the%20SMPTE/77/1/5/), *Journal of the SMPTE*, vol. 77, no. 1, 1968, DOI 10.5594/J05865.
- JVC, [Video Technical Guide VTG82063, section 3: Video Circuit](https://decode-orc.github.io/analogue-video-specifications/vhs/JVC-Video-Technical-Guide-VTG82063/section-3/), reproduced engineering guide.
- C. P. Singh, [Experiment on Video Signal Degradation Due to Generation Loss on VHS Format Recording](https://bprd.nic.in/uploads/pdf/8484241726-april-june2006.pdf), *The Indian Police Journal*, vol. LIII, no. 2, 2006.
