# CONTRACT
"""The design data of the schematic: complete, on the canvas, readable, disciplined.

These tests gate the designer's files (`layout.json`, `glyphs.json`); the format is in
docs/design/FORMAT.md. `test_the_design_is_rich_enough` fails on the plain skeleton on purpose.
"""
from __future__ import annotations

import colorsys
import json
import math
from pathlib import Path

import pytest

from vhsgui.model import AUDIO_MODES, CONDITION_FIELDS, MAX_GENERATIONS, SIGNALS

DATA = Path(__file__).resolve().parents[1] / "vhsgui" / "schematic"
LAYOUT = json.loads((DATA / "layout.json").read_text())
GLYPHS = json.loads((DATA / "glyphs.json").read_text())["glyphs"]

ROW_H = 18  # a control row: 16 px of text plus its gauge line
ADVANCE = 0.602  # advance of a monospace character in em (DejaVu Sans Mono 0.602; Hack, JetBrains Mono 0.600)
MARGIN = 4
LOOK_IDS = {"preset", "seed", "generations", "signal", "audio"}
BLOCK_OF = {
    "source": ["preset", "seed"],
    "record_deck": ["rec_deck_age_years", "rec_deck_temp_c", "generations"],
    "cassette": ["tape_age_years", "tape_storage", "tape_quality", "tape_passes", "tape_speed", "tape_stretch", "tape_position_m"],
    "head_drum": ["head_wear", "head_clog", "drum_wear"],
    "transport": ["tracking_error_um", "tracking_slope_um", "tension_error"],
    "power": ["deck_age_years", "deck_temp_c", "deck_humidity", "psu_smps", "mains_hz", "mains_sag"],
    "output": ["signal", "audio"],
    "tv": [],
}
PRIMS = {  # kind -> (argument count excluding the optional style, checker)
    "line": 4, "poly": 1, "loop": 1, "rect": 4, "circle": 3, "arc": 5, "dot": 2, "text": 5, "reel": 3, "screen": 4,
}
STYLE_KEYS = {"w": {"hair", "thin", "bold"}, "b": {"faint", "dim", "norm", "hot"}, "fill": {"none", "faint", "dim"}}
W, H = LAYOUT["canvas"]["w"], LAYOUT["canvas"]["h"]
BLOCKS = {b["id"]: b for b in LAYOUT["blocks"]}
CONTROLS = {c["id"]: c for c in LAYOUT["controls"]}


def rect_of(o, h=None):
    return (o["x"], o["y"], o["w"], h if h is not None else o["h"])


def inside(inner, outer, margin=0):
    return inner[0] >= outer[0] + margin and inner[1] >= outer[1] + margin and inner[0] + inner[2] <= outer[0] + outer[2] - margin and inner[1] + inner[3] <= outer[1] + outer[3] - margin


def overlap(a, b, gap=0):
    return not (a[0] + a[2] + gap <= b[0] or b[0] + b[2] + gap <= a[0] or a[1] + a[3] + gap <= b[1] or b[1] + b[3] + gap <= a[1])


def hsv(hexcolor):
    r, g, b = (int(hexcolor[i : i + 2], 16) / 255 for i in (1, 3, 5))
    h, s, v = colorsys.rgb_to_hsv(r, g, b)
    return h * 360, s, v, 0.2126 * r + 0.7152 * g + 0.0722 * b


def test_the_canvas_and_the_version_are_the_documented_ones():
    assert LAYOUT["version"] == 1 and (W, H) == (1180, 620)
    assert json.loads((DATA / "glyphs.json").read_text())["version"] == 1


def test_every_control_is_on_the_schematic_exactly_once():
    ids = [c["id"] for c in LAYOUT["controls"]]
    assert len(ids) == len(set(ids)), "duplicate control ids"
    assert set(ids) == {f.key for f in CONDITION_FIELDS} | LOOK_IDS


def test_every_control_lives_in_the_block_of_its_part_of_the_machine():
    for block, ids in BLOCK_OF.items():
        assert block in BLOCKS, f"block {block} is missing"
        assert sorted(c["id"] for c in LAYOUT["controls"] if c["block"] == block) == sorted(ids), block


def test_the_controls_describe_the_same_ranges_as_the_window():
    spec = {f.key: f for f in CONDITION_FIELDS}
    for cid, c in CONTROLS.items():
        assert c["label"] and c["label"] == c["label"].upper() and len(c["label"]) <= 26, cid
        if cid in spec:
            f = spec[cid]
            if f.kind == "float":
                assert c["kind"] == "slider" and (c["min"], c["max"]) == (f.lo, f.hi) and math.isclose(c["step"], f.step), cid
                assert c["unit"] == f.unit and isinstance(c["decimals"], int) and 0 <= c["decimals"] <= 3, cid
            elif f.kind == "bool":
                assert c["kind"] == "toggle", cid
            else:
                assert c["kind"] == "choice" and c["choices"] == list(f.choices), cid
    assert CONTROLS["generations"]["kind"] == "stepper" and (CONTROLS["generations"]["min"], CONTROLS["generations"]["max"]) == (1, MAX_GENERATIONS)
    assert CONTROLS["seed"]["kind"] == "stepper" and CONTROLS["seed"]["min"] == 0 and CONTROLS["seed"]["max"] >= 1_000_000
    assert CONTROLS["signal"]["kind"] == "choice" and CONTROLS["signal"]["choices"] == list(SIGNALS)
    assert CONTROLS["audio"]["kind"] == "choice" and CONTROLS["audio"]["choices"] == list(AUDIO_MODES)
    assert CONTROLS["preset"]["kind"] == "choice" and {"pristine", "worn", "ruined"} <= set(CONTROLS["preset"]["choices"])


def test_everything_fits_on_the_canvas():
    canvas = (0, 0, W, H)
    for b in LAYOUT["blocks"]:
        assert inside(rect_of(b), canvas, MARGIN), b["id"]
    for c in LAYOUT["controls"]:
        assert inside(rect_of(c, ROW_H), canvas, MARGIN), c["id"]
    for w in LAYOUT["wires"]:
        assert all(0 <= x <= W and 0 <= y <= H for x, y in w["points"]), w["id"]
    for t in LAYOUT.get("labels", []):
        assert MARGIN <= t["x"] <= W - MARGIN and MARGIN <= t["y"] <= H - MARGIN, t["text"]
    for i in LAYOUT["instances"]:
        g = GLYPHS[i["glyph"]]
        assert inside((i["x"], i["y"], g["w"] * i["scale"], g["h"] * i["scale"]), canvas, MARGIN), i["glyph"]


def test_blocks_are_titled_and_do_not_touch_each_other():
    ids = [b["id"] for b in LAYOUT["blocks"]]
    assert len(ids) == len(set(ids)) and all(b["title"] for b in LAYOUT["blocks"] if b["frame"] != "none" or b["id"] in BLOCK_OF)
    for i, a in enumerate(LAYOUT["blocks"]):
        for b in LAYOUT["blocks"][i + 1 :]:
            assert not overlap(rect_of(a), rect_of(b), gap=MARGIN), f"{a['id']} touches {b['id']}"


def test_control_rows_sit_inside_their_block_and_never_collide():
    rows = []
    for c in LAYOUT["controls"]:
        block = rect_of(BLOCKS[c["block"]])
        assert inside(rect_of(c, ROW_H), block, MARGIN), f"{c['id']} leaves its block"
        rows.append((c["id"], rect_of(c, ROW_H)))
    for i, (ia, a) in enumerate(rows):
        for ib, b in rows[i + 1 :]:
            assert not overlap(a, b, gap=2), f"rows {ia} and {ib} collide"


def test_the_text_is_legible_and_fits_its_row():
    font = LAYOUT["theme"]["font"]["size"]
    assert font >= 11
    sizes = [LAYOUT["title"]["size"]] + [t["size"] for t in LAYOUT.get("labels", [])] + [p[4] for g in GLYPHS.values() for p in g["prims"] if p[0] == "text"]
    assert all(s >= 11 for s in sizes), "text under 11 px is not readable on a screen like this"
    for cid, c in CONTROLS.items():
        if c["kind"] == "slider":
            value = max(len(f"{c['max']:.{c['decimals']}f}"), len(f"{c['min']:.{c['decimals']}f}")) + (len(c["unit"]) + 1 if c["unit"] else 0)
        elif c["kind"] == "choice":
            value = max(len(x) for x in c["choices"])
        elif c["kind"] == "toggle":
            value = 3
        else:
            value = len(str(c["max"]))
        per_char = ADVANCE * font + LAYOUT["theme"]["font"]["tracking"]
        assert (len(c["label"]) + 3 + value) * per_char <= c["w"] - 4, f"{cid}: label and value do not fit in {c['w']} px"


def test_glyph_primitives_are_well_formed():
    assert len(GLYPHS) >= 3
    for name, g in GLYPHS.items():
        assert g["w"] > 0 and g["h"] > 0 and g["prims"], name
        screens = 0
        for p in g["prims"]:
            kind, args = p[0], p[1:]
            assert kind in PRIMS, f"{name}: unknown primitive {kind}"
            style = args[-1] if args and isinstance(args[-1], dict) else None
            args = args[:-1] if style is not None else args
            assert len(args) == PRIMS[kind], f"{name}: {kind} takes {PRIMS[kind]} arguments"
            if style:
                assert set(style) <= set(STYLE_KEYS) | {"dash"}, f"{name}: style {style}"
                assert all(style[k] in STYLE_KEYS[k] for k in style if k in STYLE_KEYS), f"{name}: style {style}"
            if kind in ("poly", "loop"):
                assert len(args[0]) >= (2 if kind == "poly" else 3) and all(len(pt) == 2 for pt in args[0]), name
                pts = args[0]
            elif kind in ("line",):
                pts = [(args[0], args[1]), (args[2], args[3])]
            elif kind in ("circle", "reel"):
                pts = [(args[0] - args[2], args[1] - args[2]), (args[0] + args[2], args[1] + args[2])]
            elif kind == "arc":
                pts = [(args[0] - args[2], args[1] - args[2]), (args[0] + args[2], args[1] + args[2])]
            elif kind in ("rect", "screen"):
                pts = [(args[0], args[1]), (args[0] + args[2], args[1] + args[3])]
            else:
                pts = [(args[0], args[1])]
            screens += kind == "screen"
            assert all(-2 <= x <= g["w"] + 2 and -2 <= y <= g["h"] + 2 for x, y in pts), f"{name}: {kind} leaves its {g['w']}x{g['h']} box"
        assert screens <= 1, f"{name}: more than one screen"


def test_instances_use_glyphs_that_exist_and_the_tv_has_a_screen():
    for i in LAYOUT["instances"]:
        assert i["glyph"] in GLYPHS and i["block"] in BLOCKS and i["scale"] > 0, i
    tv = [i for i in LAYOUT["instances"] if i["block"] == "tv"]
    assert any(any(p[0] == "screen" for p in GLYPHS[i["glyph"]]["prims"]) for i in tv), "the TV block needs a glyph with a screen"


def _near_a_block(pt, tol=8):
    for b in LAYOUT["blocks"]:
        x, y, w, h = rect_of(b)
        dx = max(x - pt[0], 0, pt[0] - (x + w))
        dy = max(y - pt[1], 0, pt[1] - (y + h))
        if math.hypot(dx, dy) <= tol:
            return True
    return False


def test_wires_are_schematic_lines_that_join_blocks():
    kinds = {w["kind"] for w in LAYOUT["wires"]}
    assert {"video", "audio", "dub"} <= kinds <= {"video", "audio", "dub", "power"}
    for w in LAYOUT["wires"]:
        pts = w["points"]
        assert len(pts) >= 2, w["id"]
        for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
            dx, dy = abs(x1 - x0), abs(y1 - y0)
            assert dx == 0 or dy == 0 or dx == dy, f"{w['id']}: segments are horizontal, vertical or 45 degrees"
        assert _near_a_block(pts[0]) and _near_a_block(pts[-1]), f"{w['id']} ends in empty space"


def test_the_signal_flow_reads_from_left_to_right():
    cx = {k: b["x"] + b["w"] / 2 for k, b in BLOCKS.items()}
    assert cx["source"] < cx["cassette"] and cx["record_deck"] < cx["cassette"] < cx["head_drum"] <= cx["power"] < cx["tv"]
    assert cx["output"] < cx["tv"] and all(cx[k] < cx["tv"] for k in cx if k != "tv")


def _segment_points(a, b, step=2.0):
    n = max(1, int(math.hypot(b[0] - a[0], b[1] - a[1]) / step))
    return [(a[0] + (b[0] - a[0]) * i / n, a[1] + (b[1] - a[1]) * i / n) for i in range(n + 1)]


def test_wires_never_strike_through_a_line_of_text():
    rows = [rect_of(c, ROW_H) for c in LAYOUT["controls"]]
    for w in LAYOUT["wires"]:
        for a, b in zip(w["points"], w["points"][1:]):
            for x, y in _segment_points(a, b):
                for r in rows:
                    assert not (r[0] - 2 <= x <= r[0] + r[2] + 2 and r[1] - 2 <= y <= r[1] + r[3] + 2), f"{w['id']} runs through a control row at ({x:.0f}, {y:.0f})"


def test_the_hint_line_has_a_band_of_its_own():
    h = LAYOUT["hint"]
    assert h["size"] >= 11 and h["w"] >= 700 and h["default"] and len(h["default"]) * (ADVANCE * h["size"] + LAYOUT["theme"]["font"]["tracking"]) <= h["w"]
    band = (h["x"], h["y"] - h["size"], h["w"], h["size"] + 5)
    assert inside(band, (0, 0, W, H), MARGIN)
    for b in LAYOUT["blocks"]:
        assert not overlap(band, rect_of(b), gap=2), f"the hint line runs into {b['id']}"
    for c in LAYOUT["controls"]:
        assert not overlap(band, rect_of(c, ROW_H), gap=2), f"the hint line runs into {c['id']}"
    for t in LAYOUT.get("labels", []):
        assert not (band[0] <= t["x"] <= band[0] + band[2] and band[1] - 4 <= t["y"] <= band[1] + band[3] + 4), f"the label {t['text']!r} is in the hint band"
    for w in LAYOUT["wires"]:
        assert all(not (band[1] - 2 <= y <= band[1] + band[3] + 2) for _, y in w["points"]), f"{w['id']} runs into the hint band"


def _box(x, baseline, s, size, align="l"):
    """The area a piece of text covers: monospace advance plus tracking, cap height above the baseline."""
    width = len(s) * (ADVANCE * size + LAYOUT["theme"]["font"]["tracking"])
    left = x - width / 2 if align == "c" else x - width if align == "r" else x
    return (left, baseline - 0.8 * size, width, size)


def _text_items():
    """Everything the engine prints, as (name, box, brightness tier, effective size). The engine puts a block's
    title at (x + 8, y + 15) at the theme size in the norm tier, its caption at (x + 8, y + h - 7) one pixel
    smaller in the dim tier, and a wire's label at `label_at` at the theme size in the norm tier."""
    fs = LAYOUT["theme"]["font"]["size"]
    t = LAYOUT["title"]
    items = [("title", _box(t["x"], t["y"], t["text"], t["size"]), "hot", t["size"])]
    for b in LAYOUT["blocks"]:
        if b["frame"] == "none":
            continue
        items.append((f"{b['id']} title", _box(b["x"] + 8, b["y"] + 15, b["title"], fs), "norm", fs))
        if b["caption"]:
            items.append((f"{b['id']} caption", _box(b["x"] + 8, b["y"] + b["h"] - 7, b["caption"], fs - 1), "dim", fs - 1))
    for w in LAYOUT["wires"]:
        if w["label"]:
            items.append((f"wire {w['id']} label {w['label']!r}", _box(w["label_at"][0], w["label_at"][1], w["label"], fs), "norm", fs))
    for t in LAYOUT.get("labels", []):
        items.append((f"label {t['text']!r}", _box(t["x"], t["y"], t["text"], t["size"], t.get("align", "l")), t.get("b", "norm"), t["size"]))
    for i in LAYOUT["instances"]:
        for p in GLYPHS[i["glyph"]]["prims"]:
            if p[0] == "text":
                style = p[-1] if isinstance(p[-1], dict) else {}
                size = p[4] * i["scale"]
                items.append((f"{i['glyph']} text {p[3]!r}", _box(i["x"] + p[1] * i["scale"], i["y"] + p[2] * i["scale"], p[3], size, p[5]), style.get("b", "norm"), size))
    return items


def test_every_piece_of_text_is_readable_and_never_faint():
    assert LAYOUT["theme"]["font"]["size"] >= 12, "captions are one pixel smaller than the theme size and must stay at 11 or more"
    problems = []
    for name, box, tier, size in _text_items():
        if size < 11:
            problems.append(f"{name}: {size:.1f} px is too small")
        if tier == "faint":
            problems.append(f"{name}: faint text (about 2 : 1 against the background) cannot be read")
        if not inside(box, (0, 0, W, H), MARGIN):
            problems.append(f"{name} leaves the canvas")
    assert not problems, "\n".join(problems)


def test_captions_fit_inside_their_block():
    problems = []
    for b in LAYOUT["blocks"]:
        if b["caption"]:
            width = _box(b["x"] + 8, b["y"] + b["h"] - 7, b["caption"], LAYOUT["theme"]["font"]["size"] - 1)[2]
            room = b["w"] - 16
            if width > room:
                fits = int(room // (ADVANCE * 11 + LAYOUT["theme"]["font"]["tracking"]))
                problems.append(f"{b['id']}: the caption is {width:.0f} px wide and the block leaves {room} px ({len(b['caption'])} characters, at most {fits} fit)")
    assert not problems, "\n".join(problems)


def test_no_two_pieces_of_text_overlap():
    items = [(n, b) for n, b, _, _ in _text_items()] + [(f"the row of {c['id']}", rect_of(c, ROW_H)) for c in LAYOUT["controls"]]
    h = LAYOUT["hint"]
    items.append(("the hint band", (h["x"], h["y"] - h["size"], h["w"], h["size"] + 5)))
    problems = [f"{na} overlaps {nb}" for i, (na, a) in enumerate(items) for nb, b in items[i + 1 :] if overlap(a, b)]
    assert not problems, "\n".join(problems)


def test_text_never_straddles_a_block_frame():
    problems = []
    for name, box, _, _ in _text_items():
        for b in LAYOUT["blocks"]:
            if b["frame"] == "none" or name.startswith(f"{b['id']} "):
                continue
            if not (inside(box, rect_of(b), 2) or not overlap(box, rect_of(b), gap=2)):
                problems.append(f"{name} runs across the frame of {b['id']}: keep it wholly inside the block or wholly clear of it")
    assert not problems, "\n".join(problems)


def test_wires_do_not_run_through_text():
    boxes = [(n, bx) for n, bx, _, _ in _text_items()]
    problems = set()
    for w in LAYOUT["wires"]:
        for a, b in zip(w["points"], w["points"][1:]):
            for x, y in _segment_points(a, b):
                for name, r in boxes:
                    if r[0] - 2 <= x <= r[0] + r[2] + 2 and r[1] - 2 <= y <= r[1] + r[3] + 2:
                        problems.add(f"{w['id']} runs through {name}")
    assert not problems, "\n".join(sorted(problems))


def test_the_palette_is_one_phosphor():
    t = LAYOUT["theme"]
    h = {k: hsv(t[k]) for k in ("bg", "faint", "dim", "phosphor", "hot", "warn")}
    assert h["bg"][3] <= 0.05, "the screen is dark"
    assert h["faint"][3] < h["dim"][3] < h["phosphor"][3] < h["hot"][3], "brightness tiers must climb"
    assert 90 <= h["phosphor"][0] <= 160 and h["phosphor"][1] >= 0.5, "the phosphor is green"
    assert all(abs(h[k][0] - h["phosphor"][0]) <= 25 for k in ("faint", "dim")), "the dim tiers keep the phosphor's hue"
    assert h["hot"][1] <= 0.35, "the hottest tier is nearly white"
    assert 15 <= h["warn"][0] <= 55, "the one warning colour is amber"
    lw = t["line"]
    assert 0 < lw["hair"] < lw["thin"] < lw["bold"] <= 4
    glow = t["glow"]
    assert len(glow) >= 3 and all(a["w"] > b["w"] and a["a"] < b["a"] for a, b in zip(glow, glow[1:])), "glow: wide and faint first, narrow and strong last"
    assert 2 <= t["scanline"]["period"] <= 6 and 0.10 <= t["scanline"]["alpha"] <= 0.3 and 0.1 <= t["vignette"] <= 0.6
    family = t["font"]["family"]
    assert isinstance(family, str) or (isinstance(family, list) and family and all(isinstance(f, str) for f in family) and family[-1] == "monospace"), "font.family: a name, or a list ending in monospace"
    assert 0 <= t["font"]["tracking"] <= 1.5


def test_the_animation_is_calm():
    a = LAYOUT["animation"]
    assert 20 <= a["pulse_speed"] <= 300 and 30 <= a["pulse_spacing"] <= 300 and 3 <= a["reel_rpm"] <= 60
    assert 0 <= a["flicker"] <= 0.08 and 600 <= a["boot_ms"] <= 4000 and 100 <= a["refresh_flash_ms"] <= 800


def test_the_design_is_rich_enough():
    """Every part of the machine is drawn, not just labelled (red on the plain skeleton)."""
    drawn = {i["block"] for i in LAYOUT["instances"]}
    assert set(BLOCK_OF) <= drawn, f"blocks without a drawing: {sorted(set(BLOCK_OF) - drawn)}"
    assert len({i["glyph"] for i in LAYOUT["instances"]}) >= 8, "the vocabulary of symbols is too small"
    assert len(LAYOUT["instances"]) >= 12
    assert sum(len(g["prims"]) for g in GLYPHS.values()) >= 120, "the drawings are too thin"
    assert any(p[0] == "reel" for g in GLYPHS.values() for p in g["prims"]), "the cassette needs its reels"
    texts = [p for g in GLYPHS.values() for p in g["prims"] if p[0] == "text"] + LAYOUT.get("labels", [])
    assert len(texts) >= 8, "callouts and legends are part of the look"
    assert all(b["caption"] for b in LAYOUT["blocks"] if b["id"] != "tv"), "every block carries a one-line caption of what it does to the picture"
    assert len([w for w in LAYOUT["wires"] if w["label"]]) >= 3, "label the signal paths"
    tv = next(i for i in LAYOUT["instances"] if i["block"] == "tv" and any(p[0] == "screen" for p in GLYPHS[i["glyph"]]["prims"]))
    screen = next(p for p in GLYPHS[tv["glyph"]]["prims"] if p[0] == "screen")
    w, h = screen[3] * tv["scale"], screen[4] * tv["scale"]
    assert w >= 200 and h >= 150 and abs(w / h - 4 / 3) <= 0.12, f"the TV screen is {w:.0f} x {h:.0f}: it must be a 4:3 window of at least 200 x 150"
