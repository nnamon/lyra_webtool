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

echo "==> building $CRATE ($PROFILE) for wasm32-unknown-unknown"
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

# GitHub Pages runs Jekyll by default, which skips files it does not recognise.
touch www/.nojekyll

echo "==> done"
ls -la "$OUT"
