#!/usr/bin/env bash
# Render every preset on two synthetic clips and build contact sheets in docs/img.
# usage: scripts/gallery.sh        (needs ffmpeg and the python venv: see README)
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -q
V=target/release/vhsphys
PY=.venv/bin/python
T=$(mktemp -d)
ffmpeg -v error -y -f lavfi -i "testsrc2=size=960x720:rate=30000/1001" -t 4 -pix_fmt yuv420p "$T/bars.mp4"
ffmpeg -v error -y -f lavfi -i "mandelbrot=size=960x720:rate=30000/1001:start_scale=0.4:end_scale=0.05" -t 4 -pix_fmt yuv420p "$T/fractal.mp4"
PRESETS="pristine good worn bad ruined"
for clip in bars fractal; do
  ffmpeg -v error -y -ss 1.8 -i "$T/$clip.mp4" -frames:v 1 "$T/${clip}_source.png"
  items=("source=$T/${clip}_source.png")
  for p in $PRESETS; do
    $V run "$T/$clip.mp4" "$T/${clip}_$p.mp4" --preset "$p" --seed 5 -t 3 2>/dev/null
    ffmpeg -v error -y -ss 1.8 -i "$T/${clip}_$p.mp4" -frames:v 1 "$T/${clip}_$p.png"
    items+=("$p=$T/${clip}_$p.png")
  done
  $PY python/vhslab/sheet.py "docs/img/gallery_$clip.jpg" "${items[@]}"
done
echo "wrote docs/img/gallery_bars.jpg and docs/img/gallery_fractal.jpg"
