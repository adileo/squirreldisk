#!/usr/bin/env bash
# Records the README demo GIFs with fake data (MARKETING_DEMO) and frames them
# in a macOS-style window on a gradient background.
# Requires: ffmpeg, rsvg-convert. Usage: scripts/make-gifs.sh [scripts...]
set -euo pipefail
BIN="${BIN:-target/release/squirreldisk}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
for f in background mask overlay; do
  rsvg-convert "packaging/gif/$f.svg" -o "$WORK/$f.png"
done
mkdir -p screenshots
for script in "${@:-hero collect themes}"; do
  for s in $script; do
    rm -rf "$WORK/$s"
    SQUIRRELDISK_LANG=en MARKETING_DEMO=1 MARKETING_DEMO_SCRIPT="$s" MARKETING_DEMO_RECORD="$WORK/$s" "$BIN"
    ffmpeg -loglevel error -y -framerate 15 -i "$WORK/$s/frame-%05d.png" \
      -i "$WORK/background.png" -i "$WORK/mask.png" -i "$WORK/overlay.png" \
      -filter_complex "[0]scale=1180:760:flags=lanczos,format=rgba[f];[2]format=gray[m];[f][m]alphamerge[w];[1][w]overlay=56:56:format=auto[b];[b][3]overlay=0:0,scale=1120:-1:flags=lanczos,split[a][c];[a]palettegen=max_colors=256:stats_mode=full[p];[c][p]paletteuse=dither=sierra2_4a:diff_mode=rectangle" \
      "screenshots/$s.gif"
    echo "screenshots/$s.gif"
  done
done
