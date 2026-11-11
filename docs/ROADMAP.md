# Roadmap

Development status and forward roadmap for `vhs-phys`.

## 1. Currently being worked on

Active tasks in the current development cycle:

- **Composite PAL** (`crates/vhsphys-core/src/composite.rs`): 4.43 MHz PAL composite path (PAL 1H/2H comb filter) for `pal` and `pal60` standards; removes the configuration restriction on composite video into a PAL television set.
- **PAL deck tuning**: Playback equaliser and head resonance scaled to the PAL carrier (vhs-decode RF peak ~4.3 MHz vs 3.9 MHz for NTSC); normalised 58 um track pitch reference for carrier-to-noise ratio.
- **Camera tube limits** (`docs/CAMERA.md`): Addressing low build-up lag, short light-bias tail, target saturation fitting at 50 Hz, and blooming / burn-in models.
- **Engine-generic audio I/O**: Refactoring CLI audio I/O across 525-line and 625-line engines and standard verification checks in configuration overrides.
- **Schematic UI integration**: Television standard, camera tube selection, and conditional spec displays in the PySide6 schematic view.

## 2. Near-term physical modelling roadmap

Physical tape, deck, and circuit features planned for implementation in `vhsphys-core` (physics and circuit theory detailed in `docs/PHYSICAL_MODELLING_RESEARCH.md`):

- **Non-linear noise reduction (HQ decks, SP only)**: High-pass path feeding a back-to-back diode limiter (US4597021A / US3944853) added to the FM signal, with a closed-form inverse filter on playback.
- **Magnetic recording beyond the saturating relay**: Transition broadening via a switching-field distribution / Preisach model ($M(H) = M_s \tanh((H \pm H_c)/\Delta H_c)$), generating high-frequency recording loss and self-demagnetisation directly from physical state.
- **Head pole-tip contour ("head bumps")**: Edge flux coupling on finite 50-100 um pole tips below 1 MHz, affecting colour-under (629 kHz) and linear audio low-end.
- **Closed-loop chroma PLL**: Heterodyning colour-under through an analog phase detector, lag-lead loop filter, and VCO locked to burst to capture unlock hue smear, tape stretch, and head-switch phase jumps.
- **True 59.94 / 50 fps field-sequential input**: Mapping each incoming field to its own helical pass to model true interlace comb filtering on moving video.
- **Hi-Fi depth-multiplex audio**: Dual FM audio carriers (1.3 / 1.7 MHz) recorded by a second pair of rotating heads buried beneath the video layer, including head-switch buzz (59.94 Hz) and tracking-loss fallback muting.

## 3. Long-term goals

Tooling, workflow integration, and usability improvements:

- **Parameter automation in standalone application and plugin preparation**:
  - Time-varying parameter curves (JSON/CSV keyframing or timeline automation) for the standalone app and CLI to model dynamic tracking drift, progressive head clogging, crinkles, and tension changes over time.
  - Per-frame parameter updating in `pipeline::Session` to prepare for host automation lanes.
- **Cleaner user interface (PySide6 GUI and tools)**:
  - Streamlined controls, grouping deck vs. transport vs. tape physical parameters.
  - Dark dock layout for schematic mode, improved preset browsing, and real-time parameter validation.
- **Walkthrough shell version (interactive CLI wizard)**:
  - Interactive terminal wizard (`vhsphys wizard`) stepping users through video input/output, speed (SP/LP/EP), standard (NTSC/PAL/PAL-60), tape condition, camera tube, and generation count to eliminate verbose command lines. *(Completed)*
- **Separate viewing window**:
  - Detachable / floating preview viewport (`DetachedPreviewWindow` with Dock/Pop-out controls) designed for multi-monitor setups. *(Completed)*
- **Ko-fi button and retro easter eggs**:
  - Project funding/support links in the GUI About dialog and documentation.
  - Period-authentic deck easter eggs (e.g. simulated "DEW DETECTED" shutdown, tape eject jam, retro blue screen / OSD counter modes).

## 4. Ultra-long-term goals

Platform expansion, performance architecture, and broader format support:

- **GPU compute architecture**:
  - Porting computationally intensive hot spots (reciprocity integration of ~135k Lorentzian pulses per field, delay-line FM discriminator, and 42.95 MS/s FIR filters) to portable compute shaders (WGPU / Vulkan / Metal / DirectX).
  - Target: Real-time (>=24/30/60 fps) rendering on modern hardware, enabling interactive timeline playback.
- **VFX and NLE host plugins**:
  - OpenFX (OFX) plugin wrapper for DaVinci Resolve, Nuke, Vegas, and Natron.
  - Adobe After Effects and Premiere Pro plugin wrappers.
  - Blender compositor / VSE integration (Python / C FFI).
- **Cross-platform release binaries**:
  - Automated CI release builds for macOS (`x86_64` and `aarch64` Apple Silicon) and Windows (`x86_64`) with bundled static dependencies.
- **Android APK**:
  - Mobile port leveraging the Rust core via NDK / Vulkan compute, scoped for mobile camera emulation and video processing.
- **Expanded media and format coverage**:
  - Analog tape formats: Betamax, Video8, Hi8, and U-matic (adapting helical RF parameters, subcarrier frequencies, and track geometries).
  - Digital tape media: MiniDV / DVCAM model (digital tape failure modes: DCT macroblocking, Reed-Solomon parity errors, concealment blocks, and audio packet drops).
- **Multi-generation bounce simulation**:
  - Heterogeneous generation graphs (e.g. Camera tube -> NTSC VHS SP -> Composite cable dub -> PAL deck -> CRT display) to model authentic compounding analogue generation loss.
- **Real-world tape degradation dataset collection**:
  - Gathering public domain and community RF captures (`vhs-decode`), calibrated test tape runs, and measured hardware deck responses to empirically validate and refine estimated (`[est]`) physical parameters.
