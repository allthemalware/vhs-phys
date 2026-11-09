#!/usr/bin/env bash
# Render the tape speeds, the generations and the composite decoders on two synthetic
# clips and build contact sheets in docs/img.
# usage: scripts/features.sh        (needs ffmpeg and the python venv: see README)
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -q
V=target/release/vhsphys
PY=.venv/bin/python
T=$(mktemp -d)
ffmpeg -v error -y -f lavfi -i "testsrc2=size=960x720:rate=30000/1001" -t 4 -pix_fmt yuv420p "$T/bars.mp4"
ffmpeg -v error -y -f lavfi -i "mandelbrot=size=960x720:start_scale=0.4:end_scale=0.05" -frames:v 1 "$T/fractal.png"
ffmpeg -v error -y -loop 1 -framerate 30000/1001 -i "$T/fractal.png" -t 4 -pix_fmt yuv420p "$T/fractal.mp4"

# render CLIP NAME ARGS...: a 3 s render, the frame at 1.8 s as a PNG
render() {
  local clip=$1 name=$2; shift 2
  $V run "$T/$clip.mp4" "$T/${clip}_$name.mp4" --seed 5 -t 3 "$@" 2>/dev/null
  ffmpeg -v error -y -ss 1.8 -i "$T/${clip}_$name.mp4" -frames:v 1 "$T/${clip}_$name.png"
}
for clip in bars fractal; do
  ffmpeg -v error -y -ss 1.8 -i "$T/$clip.mp4" -frames:v 1 "$T/${clip}_source.png"
  items=("source=$T/${clip}_source.png")

  for s in sp lp ep; do
    printf '[condition]\ntape_speed = "%s"\ntracking_error_um = 5.0\n' "$s" > "$T/speed_$s.toml"
    render "$clip" "speed_$s" --preset good --config "$T/speed_$s.toml"
    items+=("${s^^}=$T/${clip}_speed_$s.png")
  done
  $PY python/vhslab/sheet.py "docs/img/features_speed_$clip.jpg" "${items[@]}"

  items=("source=$T/${clip}_source.png")
  for g in 1 2 3; do
    render "$clip" "gen_$g" --preset good --generations "$g"
    items+=("gen $g=$T/${clip}_gen_$g.png")
  done
  $PY python/vhslab/sheet.py "docs/img/features_generations_$clip.jpg" "${items[@]}"

  items=()
  render "$clip" svideo --preset good
  items+=("S-Video=$T/${clip}_svideo.png")
  for d in notch comb1h comb2h; do
    render "$clip" "comp_$d" --preset good --set composite.enabled=true --set "composite.decoder=\"$d\""
    items+=("$d=$T/${clip}_comp_$d.png")
  done
  $PY python/vhslab/sheet.py "docs/img/features_composite_$clip.jpg" "${items[@]}"
done
echo "wrote docs/img/features_{speed,generations,composite}_{bars,fractal}.jpg"
