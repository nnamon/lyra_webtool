#!/usr/bin/env bash
# Build the wasm bundle into www/pkg/.
#
# Needs: rustup target add wasm32-unknown-unknown
#        cargo install wasm-bindgen-cli
set -euo pipefail

cd "$(dirname "$0")"

PROFILE="${PROFILE:-release}"
OUT=www/pkg
CRATE=lyra_webtool

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
DATE=$(date -u +%Y-%m-%d)

# Baked into the binary so Settings can show what is running. build.rs watches
# these, so changing one rebuilds rather than serving a stale version string.
export LYRA_BUILD_COMMIT="$COMMIT"
export LYRA_BUILD_DATE="$DATE"

echo "==> building $CRATE $VERSION ($COMMIT, $PROFILE)"
if [ "$PROFILE" = "release" ]; then
  cargo build -p lyra-webtool --target wasm32-unknown-unknown --release
  WASM=target/wasm32-unknown-unknown/release/$CRATE.wasm
else
  cargo build -p lyra-webtool --target wasm32-unknown-unknown
  WASM=target/wasm32-unknown-unknown/debug/$CRATE.wasm
fi

echo "==> running wasm-bindgen"
rm -rf "$OUT"
wasm-bindgen "$WASM" --out-dir "$OUT" --target web --no-typescript

# What the running app compares itself against to spot a newer deploy.
cat > www/version.json <<JSON
{"version":"$VERSION","commit":"$COMMIT","date":"$DATE"}
JSON

# Settings renders this as Version history.
cp CHANGELOG.md www/CHANGELOG.md

# GitHub Pages runs Jekyll by default, which skips files it does not recognise.
touch www/.nojekyll

echo "==> done"
ls -la "$OUT"
