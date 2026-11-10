#!/usr/bin/env bash
# Render each outside-world preset on two synthetic clips and build contact sheets in docs/img.
# usage: scripts/interference.sh        (needs ffmpeg and the python venv: see README)
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -q
V=target/release/vhsphys
PY=.venv/bin/python
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
ffmpeg -v error -y -f lavfi -i "testsrc2=size=960x720:rate=30000/1001" -t 4 -pix_fmt yuv420p "$T/bars.mp4"
ffmpeg -v error -y -f lavfi -i "mandelbrot=size=960x720:start_scale=0.4:end_scale=0.05" -frames:v 1 "$T/fractal.png"
ffmpeg -v error -y -loop 1 -framerate 30000/1001 -i "$T/fractal.png" -t 4 -pix_fmt yuv420p "$T/fractal.mp4"

# render CLIP PRESET: a 3 s render, the frame at 1.8 s as a PNG
render() {
  local clip=$1 preset=$2
  $V run "$T/$clip.mp4" "$T/${clip}_$preset.mp4" --seed 5 -t 3 --audio none --preset "$preset" 2>/dev/null
  ffmpeg -v error -y -ss 1.8 -i "$T/${clip}_$preset.mp4" -frames:v 1 "$T/${clip}_$preset.png"
}
for clip in bars fractal; do
  items=()
  for preset in pristine phone-buzz ham-radio am-station ground-loop dimmer magnet-head magnet-screen; do
    render "$clip" "$preset"
    items+=("$preset=$T/${clip}_$preset.png")
  done
  $PY python/vhslab/sheet.py "docs/img/features_interference_$clip.jpg" "${items[@]}"
done
echo "wrote docs/img/features_interference_{bars,fractal}.jpg"
