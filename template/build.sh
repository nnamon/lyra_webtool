#!/usr/bin/env bash
# Pack this directory into an update.img.
set -euo pipefail
cd "$(dirname "$0")"

OUT="${1:-update.img}"
REPO=..

for f in images/MiniLoaderAll.bin images/uboot.img images/boot.img images/rootfs.img; do
  [ -f "$f" ] || { echo "missing $f; see images/README.md" >&2; exit 1; }
done

# package-file and parameter.txt are packed from this directory, so the copies
# in images/ are what the packer reads.
cp -f package-file parameter.txt images/

cargo run --quiet --manifest-path "$REPO/Cargo.toml" -p rockusb --bin rkimage -- pack . "$OUT"
cargo run --quiet --manifest-path "$REPO/Cargo.toml" -p rockusb --bin rkimage -- info "$OUT"
