# Racks: dubbing between decks of different standards

A plain run with `--generations N` dubs through N decks of one television standard. A *rack*
is the general case: each generation is its own deck with its own standard, and every pair of
neighbours is joined by a named link. A 625-line PAL tape can be copied to a 525-line NTSC
deck and back, through a standards converter, without the picture ever leaving the signal
domain: only the last deck's output reaches the picture tube and becomes a frame.

The design and its sources are in `design/HETEROGENEOUS_BOUNCE.md`. The code is in
`crates/vhsphys-core/src`: `signal.rs` (ports and exact time), `aperture.rs` (the converter's
interpolation apertures), `convert.rs` (the converter), `rack.rs` (decks, links, config).

## Configuration

A config file describes a rack when it has `[[generation]]` tables. Everything else at the top
level is common to every generation; each `[[generation]]` table is merged over it.

```toml
# common to every generation
[condition]
tape_age_years = 5.0

[[generation]]
name = "PAL master"
standard = "pal"        # ntsc (default), pal or pal60: picks the engine
preset = "good"         # default: the run's --preset

[[generation]]
name = "NTSC copy"
standard = "ntsc"
seed = 77               # default: derived from the run's seed, as --generations does
[generation.condition]
tape_speed = "lp"
[generation.tape]       # any raw parameter override
passes = 12.0

[[link]]
name = "converter"
type = "standards_converter"
profile = "bbc_four_field"
```

```sh
vhsphys run in.mp4 out.mp4 --config bounce.toml --record bounce.json
```

* The input is decoded at the first generation's raster and frame rate, the output encoded at
  the last one's.
* One `[[link]]` per pair of neighbours, in order. With no `[[link]]` tables every link is
  `direct_yc_reference`.
* `--set KEY=VALUE` applies to every generation. `--generations` and `--bypass` do not go with
  a rack.
* `composite.enabled` is the connection to the television at the end, so it is kept on the
  last generation only. A composite hop between two decks is a `composite` link.
* `--record FILE` writes what was built: every generation's standard, seed and first frame,
  every link's ports and timing, and for a converter its tables, checksums and edge rules.
* Automation keyframes are in frames of the generation that reads them: the camera track in
  frames of the first generation, the deck and picture-tube tracks in frames of the last.

## Links

| `type` | What it is | Requires |
|---|---|---|
| `direct_yc_reference` | Decoded luma and colour difference handed over unchanged. A numerical reference, not a connector of a real deck. What `--generations` does. | the same raster |
| `separate_yc` | Separate luma and chroma conductors: no cross-effects. No cable or connector loss is modelled (none has a stated profile yet), so the samples pass unchanged. | the same raster and colour system |
| `composite` | The signal is encoded on the upstream deck's subcarrier and separated at the downstream input; `separator = "notch"` (default), `"comb1h"` or `"comb2h"`. No cable loss is modelled. | the same raster and colour system |
| `standards_converter` | A change of raster by a named `profile`. There is no default profile. | different rasters |

The rack is checked before anything is decoded. An edge that does not fit is an error that
names the link, the two generations and the two signals; nothing is resized or relabelled to
make it fit. PAL-60 is PAL colour on the 525-line raster: its decoded components pass to an
NTSC recorder over the reference link, its composite or S-Video signal does not, and it is
never a 625-line signal.

## Converter profiles

| `profile` | Luminance | Colour difference |
|---|---|---|
| `bbc_four_field` | four-field aperture, table 9 (625 to 525) or table 11 (525 to 625) | four-field aperture, table 12 |
| `bbc_four_field_noisy_chroma` | as above | table 13: more noise and cross-colour suppression, less vertical colour resolution |
| `field_hold` | the latest source field repeated, lines interpolated linearly | the same |

The tables are the fixed frequency-response points of Clarke and Tanton, *Digital Standards
Conversion: Interpolation Theory and Aperture Synthesis*, BBC Research Department Report
1984/20, section 5. The aperture is synthesised from them the way the report describes (a sum
of `cos t . cos y` terms weighted by the points, truncated to four field periods by eight
picture lines), in units of the source standard's field period and picture line. For the
525-line source that puts the temporal points at multiples of 59.94/4 Hz, the report's 15 Hz
scaled by 1000/1001 for this project's exact clock [derived].

Where the report's hardware stored the aperture at 8 positions per field and 16 per line and
quantised each set of 16 coefficients to 128ths, the converter here computes each set at the
exact phase of the output line and field, in `f64`, scaled to unit sum. Interlaced scanning's
alias points fall on zeros of every table, so the sets already add up to one at any phase and
a flat field converts flat; the scaling removes only rounding.

Horizontally each line is resampled from the source active line to the target active line by a
32-tap Kaiser-windowed sinc, with luminance limited to the narrower of the two systems'
bandwidths (4.2 MHz, System M) and colour difference to 1.3 MHz. Luminance and colour
difference are converted as components: black level and span are rescaled (7.5 IRE of setup
at 525 lines, none at 625), U and V keep their scaling, and the subcarrier is never
interpolated. Sync, blanking and burst are generated new for the target.

`field_hold` is a diagnostic baseline. It judders and is not a model of a good converter.

These profiles are mathematical references. None is a model of a commercial converter and none
has been calibrated against hardware; the record of a run says so.

## Time

Every time on the programme timeline is an exact fraction of a second: a 525-line field is
1001/60000 s, a 625-line field 1/50 s. Frame counts are not shared between generations.

* The first generation starts at the source frame nearest `--start`. Each later generation
  starts at the first frame of its own raster at or after the start of the one before.
* An output frame of a converter exists when it starts inside the programme: at or after the
  first source field and before the end of the source frames that were pushed. One second of
  625/50 (25 frames) converts to 30 frames of 525/59.94; the count follows from the clocks.
* A symmetric aperture needs two source fields that come after the output field. That is
  lookahead (the converter waits for them), not delay: the converted picture has the programme
  time of its source.
* At the start and end of the programme the first and last fields are held. Outside the
  active picture the signal is at black, as it is on the wire, so the picture's outermost
  lines and samples are softened.

## Sound

The sound is on one clock (48 kHz samples of the programme) whatever the raster of the
picture. It plays through every generation's deck in order and a change of raster does not
change its speed or pitch. It is cut to start at the first output frame, which can lie a
fraction of a frame after the first source frame.

## Not modelled yet

* Cable, connector and termination losses of any link.
* A converter with composite input and output stages; the converter joins the reference ports.
* Motion-compensated conversion and film cadence (3:2) handling.
* Named, measured hardware profiles of decks or converters, and previews of intermediate
  generations in the window.
* Interlaced or variable-frame-rate source metadata: the input is still resampled to the
  first generation's frame rate by ffmpeg before it is recorded.
