# Phosphor schematic design specification

## Intent

The vhs-phys interface replaces a 26-field parameter form with a single 1180 x 620 technical schematic rendered in the style of a monochrome green-phosphor cathode-ray monitor and a 1980s vector explainer diagram. Every physical stage of the VHS signal chain is drawn as a recognizable mechanical or electronic assembly, connected left to right by pulsed signal buses, with each adjustable parameter placed directly below the component it governs as a monospace row (`LABEL ........ VALUE`) underlined by a 1 px proportional gauge. Hierarchy is established through phosphor brightness and stroke weight rather than colour variety or type scale: at a glance, the viewer reads the cassette, the rotating head drum, the signal path, and the television monitor in sequence.

## Palette

All drawing uses a single P1/P31 green phosphor ramp on a near-black CRT faceplate, plus one amber accent reserved exclusively for modified values and fault indicators.

| Token | Hex | Hue (deg) | Saturation | Contrast vs `bg` | Role |
|---|---|---|---|---|---|
| `bg` | `#010804` | 146 | 0.88 | 1.0 : 1 | Unlit CRT glass background |
| `faint` | `#0e4a22` | 140 | 0.81 | 1.9 : 1 | Shape fills, reference axes, leader lines, unlit gauge track (never text) |
| `dim` | `#1c9a45` | 140 | 0.82 | 5.5 : 1 | Secondary chassis outlines, block captions, callouts, and legend lines |
| `phosphor` | `#39ff6e` | 136 | 0.78 | 15.1 : 1 | Primary vector drawings, active control labels, wire labels, lit gauge bar |
| `hot` | `#d8ffe3` | 137 | 0.15 | 18.6 : 1 | Overdriven beam core: head gaps, cathode beam, canvas title, hovered rows |
| `warn` | `#ffb347` | 35 | 0.72 | 11.4 : 1 | Single amber accent: non-default edited values and fault markers |

Body text (`phosphor` on `bg`) achieves 15.1 : 1 WCAG contrast; secondary labels and captions (`dim` on `bg`) achieve 5.5 : 1; hovered or active values (`hot` on `bg`) achieve 18.6 : 1. Because `faint` measures 1.9 : 1 against `bg`, it is restricted to non-text geometry.

## Typography

- **Family stack**: `["JetBrains Mono", "DejaVu Sans Mono", "monospace"]`
- **Base size**: `12 px` for all control rows, block titles, wire labels, and the bottom hint bar; `14 px` for the top canvas title; `11 px` (the strict minimum on the canvas) for block captions, glyph callouts, and technical legend lines.
- **Character advance and tracking**: Monospace advance is `0.602 * size` plus `0.6 px` letter tracking (`7.824 px` per character at `12 px`; `7.222 px` per character at `11 px`). Every control row leaves at least 3 leader dots between its uppercase label and its widest formatted value within `w - 4 px`.
- **Block captions**: Printed at `(x + 8, y + h - 7)` in the `dim` tier at `11 px` (`7.222 px` per character) and must fit within `w - 16 px`: at most `25` characters in a `200 px` block, `28` characters in the `224 px` cassette block, and `30` characters in a `236 px` block (`tv` carries no caption).
- **Brightness tiers in text**:
  - `hot`: canvas title, active or hovered control row, `REC` indicator, and capacitor `C` callout.
  - `norm` (`phosphor`): standard control labels, block titles, wire labels (`VIDEO`, `SOUND`, `DUB`), and primary component identifiers (`COPY N+1`, `T-120 VHS`, `OXIDE`, `+/-6 DEG`, `CTL`, `RIPPLE`, `1H`, `Y/C`).
  - `dim`: block captions, secondary mechanical callouts (`1VPP`, `SYNC`, `FM MOD`, `DUB`, `CREASE`, `180 DEG`, `A`, `B`, `CLOG`, `DX`, `PINCH`, `TENS`, `ESR`, `AC`, `RCA`, `PWR`, `CH 3`, `HOLD`), and technical legend lines.
  - `faint`: never used for text.
- **Static constants vs. controls**: Every numeric or format text in the glyphs (`1VPP`, `T-120 VHS`, `180 DEG`, `+/-6 DEG`, `1H`, `CH 3`, `COPY N+1`) and in the header and TV legend lines (`NTSC 525 / 29.97 FPS`, `LUMA FM 3.4-4.4 MHZ`, `CHROMA 629 KHZ HETERO`, `DRUM 1800 RPM / 62 MM`, `SP 33.35 MM/S // EP 11.12 MM/S`) states a fixed NTSC VHS physical constant or reference table rather than a selectable mode, so no static label can ever disagree with an adjustable control setting.

## Stroke system

Every vector primitive is painted without blur shaders by stacking three concentric glow passes (`w: 9.0, a: 0.05` -> `w: 5.0, a: 0.13` -> `w: 2.6, a: 0.32`) under the crisp core stroke, overlaid with a `3 px` scanline grating (`alpha: 0.16`) and a `0.35` corner vignette.

- **Weights**:
  - `hair` (`1.0 px`): leader lines from part to control row, dimension ticks, internal reference axes, reel tape-pack rings, and spring zigzags.
  - `thin` (`1.6 px`): standard component outlines, cassette windows, reels, signal waveforms, and connector barrels.
  - `bold` (`2.6 px`): primary silhouettes (cassette shell, rotating drum cylinder, ferrite head tips, capstan and pinch roller, capacitor plates, transformer core, CRT cabinet).
- **Brightness tiers**: `faint`, `dim`, `norm`, and `hot` scale beam intensity from background structure up to active contact points (the tape wrap arc, ferrite gaps, and capacitor positive plate).
- **Dash patterns**:
  - `[5, 3]` and `[4, 3]`: mechanical hinge seams and cassette throat guides.
  - `[4, 2]` and `[3, 2]`: internal rotor bridges, mistracked helical scan paths, and copy-loop feedback paths.
  - `[3, 3]` and `[2, 2]`: zero-volt signal baselines and vertical azimuth reference axes.

## Grid

The canvas is fixed at `1180 x 620 px` with a `4 px` outer margin and five vertical columns separated by `20 px` gutters:

| Column | X (px) | Width (px) | Blocks (`y .. y+h`) | Control row `x, w` |
|---|---|---|---|---|
| 1 | 20 | 200 | `source` (`40..236`), `record_deck` (`252..520`) | `x: 26, w: 188` |
| 2 | 240 | 224 | `cassette` (`40..520`) | `x: 246, w: 212` |
| 3 | 484 | 200 | `head_drum` (`40..284`), `transport` (`300..520`) | `x: 490, w: 188` |
| 4 | 704 | 200 | `power` (`40..328`), `output` (`344..520`) | `x: 710, w: 188` |
| 5 | 924 | 236 | `tv` (`40..400`) + `VIDEO` bus (`424`) + legend (`450..530`) | none (preview screen) |

- **Vertical rhythm**: Stacked blocks in columns 1, 3, and 4 are separated by a `16 px` horizontal gutter. Every block reserves its top `28 px` for the title header and at least `32 px` at the bottom for its caption.
- **Control rows**: Each row is `18 px` tall (`16 px` text plus `1 px` gauge line) on a `22 px` vertical pitch (`4 px` clear between rows). Every row within a block aligns on a single left edge (`block.x + 6`) and right edge (`block.x + block.w - 6`).
- **Bottom bands**: `y = 520..580` is the horizontal wire raceway for the `SOUND` and `DUB` buses; `y = 594..611` is the exclusive band for the `hint` explanation line (baseline `y = 606`).

## Symbol vocabulary

Every glyph in `glyphs.json` depicts a concrete physical part and is instantiated in `layout.json` directly above the control rows it governs:

| Glyph | Physical part depicted | Block | Adjacent controls |
|---|---|---|---|
| `source_gate` | 35 mm 4:3 film aperture with sprocket holes, lens reticle, and 1 Vpp sync step waveform | `source` | `PRESET`, `SEED` |
| `rec_deck` | Top view of VCR cassette throat, `REC` counter window, `FM MOD` modulator, and record head amp | `record_deck` | `REC AGE`, `REC TEMP` |
| `dub_loop` | Coaxial dubbing input jack and `COPY N+1` cascade feedback loop (`DUB`) | `record_deck` | `GENERATIONS` |
| `cassette` | 188 x 104 mm VHS shell (`1.86:1`) with turning supply/take-up reels, window, write-protect tab, and `T-120 VHS` label | `cassette` | `SPEED`, `PLAYS`, `POSITION` |
| `tape_strip` | Magnified 12.7 mm tape ribbon cross-section showing `OXIDE` layer, Mylar base, and edge `CREASE` | `cassette` | `TAPE AGE`, `STORAGE`, `QUALITY`, `STRETCH` |
| `drum` | 62 mm rotating upper head drum seen from above with `180 DEG` M-wrap tape path and two heads | `head_drum` | `HEAD WEAR`, `DRUM WEAR` |
| `azimuth_pair` | Opposed ferrite head gaps (`A`, `B`) tilted at `+/-6 DEG` azimuth with gap `CLOG` debris | `head_drum` | `HEAD CLOG` |
| `ctl_head` | Stationary control-track sync head (`CTL`) and skewed helical scan track offset (`DX`) | `transport` | `TRACK ERROR`, `TRACK SLOPE` |
| `capstan` | Spring-loaded tension arm (`TENS`), guide posts, steel capstan shaft, and rubber pinch roller (`PINCH`) | `transport` | `TENSION` |
| `capacitor` | Polarized electrolytic capacitor marked `C` and series `ESR` with thermometer and heat wave | `power` | `DECK AGE`, `CAP TEMP`, `HUMIDITY` |
| `transformer` | Two-prong `AC` mains plug, iron-core transformer windings, and supply `RIPPLE` waveform | `power` | `SMPS PSU`, `MAINS`, `SAG` |
| `comb_filter` | 1H delay line (`1H`), summer node, and notch/comb passband dip curve | `output` | `SIGNAL` |
| `av_socket` | 4-pin mini-DIN S-Video socket (`Y/C`) and coaxial RCA composite/audio jack (`RCA`) | `output` | `SOUND` |
| `tv` | CRT monitor cabinet with chamfered bezel, `204 x 153` (4:3) `screen`, speaker grille, `PWR` switch, and `CH 3`/`HOLD` knobs | `tv` | live preview window |

## Wire language

All wires run strictly along horizontal and vertical segments outside block interiors. Short inter-block jumpers inside the `16 px` and `20 px` gutters carry directional arrows and pulses without text labels, while the three long horizontal buses carry `12 px` `norm` labels positioned `6 px` above their horizontal segments in open space:

- `video`: Primary picture and sync signal path flowing left to right from `source` (`w_source`) through `record_deck` (`w_record`), `cassette` (`w_tape`, `w_ctl`), `head_drum`/`transport` (`w_servo`, `w_heads`), and exiting `output` (`w_out`, labelled `VIDEO`) horizontally into the bottom center of `tv`.
- `audio` (`SOUND`): Linear edge-track audio path (`w_audio`) exiting the bottom of `cassette` (`x = 352`), running along the `y = 546` raceway, and entering the bottom of `output` (`x = 760`).
- `dub` (`DUB`): Multi-generation copy loop (`w_dub`) exiting the bottom of `output` (`x = 848`), running left along `y = 574` outside the audio bus, and re-entering the bottom of `record_deck` (`x = 120`) directly below `GENERATIONS`.
- `power`: Supply rails (`w_mech`, `w_power_out`) carrying regulated DC and capacitor ripple from `power` left into `head_drum` (`y = 100`) and down into `output` (`x = 804`).

## Motion language

Motion is calm and continuous so the schematic can remain open indefinitely without visual fatigue:

- **What moves**: Signal pulses (`90 px/s`, `70 px` spacing) travel along wires in the direction of signal flow; the two cassette `reel` primitives rotate at `12 rpm`; the CRT phosphor exhibits a subtle `0.03` brightness flicker; at start-up the schematic sweeps onto the tube over `1800 ms`; a changed control flashes `hot` for `250 ms`.
- **What never moves**: Block frames, control rows, callout text, head drum geometry, and wire geometry never translate, bounce, or scale during normal operation. Keeping all structural lines stationary preserves reading precision while dragging sliders.

## Rules

- **Do**:
  - Keep every control row within a block aligned to the same `x` and `w`.
  - Place each control row directly below the glyph depicting the part it modifies, linked by a `hair` `faint` leader line that stops clear of all text.
  - Keep block captions (`11 px`, `dim`, at `(x + 8, y + h - 7)`) within `w - 16 px`: at most `25` characters in `200 px` blocks, `28` in the `224 px` cassette block, and `30` in a `236 px` block.
  - Use `bold` strokes only for primary physical silhouettes and active contact points.
- **Do not**:
  - Set any piece of text in the `faint` tier (`1.9 : 1` contrast) or smaller than `11 px`.
  - Allow any two pieces of text, control rows, or the hint band to overlap, or allow any text to straddle a block frame.
  - Allow any wire or glyph stroke to run through or touch any text.
  - Introduce any colour outside the six-token green phosphor and single amber (`warn`) palette, or place any element within `4 px` of the canvas border or inside the `y = 594..611` hint band.

## Rejected ideas

1. **Decorative PCB trace background**: Considered filling gutters with 45-degree circuit traces and via dots; rejected because fake traces compete with the real signal wires and obscure the left-to-right flow.
2. **Third track-footprint glyph in the cassette block**: Considered adding a separate `tape_tracks` glyph at the bottom of `cassette`; rejected because `ctl_head` in `transport` already draws the helical slant tracks and control track 40 px to the right.
3. **Side-elevation CRT electron-gun diagram below the TV**: Considered filling the lower right column with a second large `crt_neck` box; rejected because in the 60% squint test it made the TV column look like two stacked boxes instead of an unmistakable television monitor.
4. **Internal bounding boxes around sub-glyphs**: Considered wrapping `rec_deck`, `dub_loop`, and `azimuth_pair` in their own rectangular frames; rejected to avoid nested boxes inside block frames.
5. **Split-width control rows beside narrow drawings**: Considered narrowing control rows to `110 px` to sit horizontally next to tall glyphs; rejected because `12 px` monospace labels and units would truncate or break the single left/right alignment edge per block.
6. **High-speed 45 rpm reels and rapid wire pulses**: Considered faster reel rotation and `250 px/s` wire pulses; rejected as distracting during long editing sessions in favour of a calm `12 rpm` and `90 px/s`.
7. **Gutter wire labels**: Considered placing labels (`MASTER`, `FM REC`, `RF PB`, `CTL`, `SERVO`, `Y/C`, `DC`) on short inter-block wires inside the `20 px` vertical and `16 px` horizontal gutters; rejected because a `20 px` gutter fits at most two `12 px` characters, causing labels to straddle block frames, collide with the wires themselves, overprint adjacent control rows, and duplicate callouts (`CTL`, `Y/C`) already inside the glyphs.
