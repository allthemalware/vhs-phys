# Schematic data format (version 1)

The schematic is drawn from two JSON files in `python/vhsgui/schematic/`: `layout.json`
(what is where) and `glyphs.json` (the drawings). The designer owns both; the engine only
reads them. Coordinates are canvas pixels, origin top left, y down; the canvas is 1180 x 620
and the view scales it to the widget. Everything is monochrome phosphor: brightness is the
only colour.

## glyphs.json

```json
{"version": 1, "glyphs": {"drum": {"w": 80, "h": 80, "prims": [ ["circle", 40, 40, 34], ... ]}}}
```

A glyph is drawn in its own box (`w` x `h`, origin at its top left); `prims` is a list of
primitives, each a JSON array whose first item is the kind. The last item of any primitive may
be a style object `{"w": "hair|thin|bold", "b": "faint|dim|norm|hot", "dash": [on, off], "fill": "none|faint|dim"}`;
the default is `{"w": "thin", "b": "norm", "fill": "none"}`.

| primitive | arguments |
|---|---|
| `line` | `x1, y1, x2, y2` |
| `poly` | `[[x, y], ...]` open polyline, at least 2 points |
| `loop` | `[[x, y], ...]` closed polygon, at least 3 points |
| `rect` | `x, y, w, h` |
| `circle` | `cx, cy, r` |
| `arc` | `cx, cy, r, start_deg, sweep_deg`: 0 deg is east, positive sweeps run clockwise on screen |
| `dot` | `x, y`: a filled junction dot |
| `text` | `x, y, "STRING", size, align` with `align` one of `l`, `c`, `r` (the baseline is at `y`) |
| `reel` | `cx, cy, r`: a cassette reel; the engine draws its spokes turning |
| `screen` | `x, y, w, h`: the window in which the engine shows the live preview (at most one per glyph) |

## layout.json

```json
{
  "version": 1,
  "canvas": {"w": 1180, "h": 620},
  "theme": {"bg": "#010804", "faint": "#0e4a22", "dim": "#1c9a45", "phosphor": "#39ff6e", "hot": "#d8ffe3", "warn": "#ffb347",
            "line": {"hair": 1.0, "thin": 1.6, "bold": 2.6},
            "glow": [{"w": 9, "a": 0.05}, {"w": 5, "a": 0.13}, {"w": 2.6, "a": 0.32}],
            "scanline": {"period": 3, "alpha": 0.16}, "vignette": 0.35,
            "font": {"family": ["JetBrains Mono", "DejaVu Sans Mono", "monospace"], "size": 12, "tracking": 0.6}},
  "title": {"text": "...", "x": 20, "y": 14, "size": 14},
  "blocks": [{"id": "head_drum", "title": "HEAD DRUM", "x": 500, "y": 20, "w": 220, "h": 110, "frame": "box", "caption": ""}],
  "instances": [{"glyph": "drum", "x": 520, "y": 40, "scale": 1.0, "block": "head_drum"}],
  "wires": [{"id": "w1", "kind": "video", "points": [[250, 60], [300, 60]], "label": "", "label_at": [0, 0], "arrow": true}],
  "labels": [{"text": "NTSC 525", "x": 20, "y": 600, "size": 11, "align": "l", "b": "dim"}],
  "hint": {"x": 20, "y": 606, "w": 1140, "size": 12, "default": "HOVER A LINE. DRAG TO SET. WHEEL TO STEP. DOUBLE CLICK TO RESET."},
  "controls": [{"id": "head_wear", "block": "head_drum", "label": "HEAD WEAR", "kind": "slider",
                "x": 512, "y": 60, "w": 196, "unit": "", "min": 0, "max": 1, "step": 0.01, "decimals": 2}],
  "animation": {"pulse_speed": 90, "pulse_spacing": 70, "reel_rpm": 12, "flicker": 0.03, "boot_ms": 1800, "refresh_flash_ms": 250}
}
```

- `blocks.frame`: `box` (outlined rectangle), `bracket` (corner brackets only) or `none`.
- `wires.kind`: `video`, `audio`, `dub` (the loop that feeds a playback back into the record deck: the
  GENERATIONS control sits on it) or `power`. The engine sends small pulses along every wire.
- **Controls are lines.** A control occupies a row `x, y, w` by 16 px: its label on the left, a
  leader of dots, its value on the right, and under the row a one-pixel *gauge line* whose bright
  part is as long as the value is large (a toggle shows `ON` / `OFF`, a choice shows its current
  word). `kind` is `slider` (a number between `min` and `max`, snapped to `step`, shown with `decimals`
  places and `unit`), `stepper` (an integer between `min` and `max`), `toggle` (on/off) or `choice`
  (one of `choices`, in order).
- Every control of the window has exactly one row, in the block that holds its part of the machine:

| block | controls |
|---|---|
| `source` | `preset` (choice), `seed` (stepper) |
| `record_deck` | `rec_deck_age_years`, `rec_deck_temp_c`, `generations` (stepper 1..8) |
| `cassette` | `tape_age_years`, `tape_storage`, `tape_quality`, `tape_passes`, `tape_speed` (choice sp, lp, ep), `tape_stretch`, `tape_position_m` |
| `head_drum` | `head_wear`, `head_clog`, `drum_wear` |
| `transport` | `tracking_error_um`, `tracking_slope_um`, `tension_error` |
| `power` | `deck_age_years`, `deck_temp_c`, `deck_humidity`, `psu_smps` (toggle), `mains_hz`, `mains_sag` |
| `output` | `signal` (choice svideo, notch, comb1h, comb2h), `audio` (choice process, copy, none) |
| `tv` | none: the TV shows the picture (a glyph with a `screen`) |

  The numeric ranges, steps and choices of the condition controls are those of
  `python/vhsgui/model.py` (`CONDITION_FIELDS`); a test keeps the two in step.
- `hint`: one line of text (baseline at `y`, at most `w` wide) in which the engine prints the explanation of the
  line under the pointer, or `default` when there is none. Nothing else may be drawn in that band.
- `font.family` is one name or a list of names tried in order (the last should be the generic `monospace`);
  `tracking` is extra space between letters in pixels. Text is monospace, so a row holds about
  `w / (0.602 * size + tracking)` characters.
- The TV glyph's `screen` is the window of the live preview. It must be at least 200 x 150 and 4:3; the engine
  enlarges it over the whole schematic when the user clicks it.
- **Text placement.** The engine prints a block's `title` at (x + 8, y + 15) in the norm tier at the theme size and its
  `caption` at (x + 8, y + h - 7) in the dim tier one pixel smaller (so the theme size is at least 12 and the caption
  must fit in `w - 16` px); a wire's `label` with its left edge and baseline at `label_at`, norm tier, theme size.
  No text may overlap other text, a control row or a wire, none may straddle a block frame (it sits wholly inside a
  block or wholly clear of it), none is `faint`, and none is smaller than 11 px after the instance's `scale`.
- `animation`: `pulse_speed` (px per second), `pulse_spacing` (px between pulses), `reel_rpm`,
  `flicker` (fraction of brightness), `boot_ms` (the screen draws itself line by line), `refresh_flash_ms`
  (a value brightens for this long after it changes).

## What the engine guarantees

Hover brightens a row; dragging along a row sets its value from the position along the row;
the wheel and the arrow keys step it; a double click asks for a reset; an edited value (one that
differs from the preset's) is marked. All of it is drawn with layered strokes (no blur filters):
each stroke is painted once per `glow` entry from the widest to the narrowest, then at full
brightness.
