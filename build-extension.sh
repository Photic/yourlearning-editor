#!/usr/bin/env bash
set -euo pipefail

# Builds the Dioxus popup and the background service worker, and assembles
# them with the extension package (manifest.json, background.js, content.js)
# into dist/public, ready to be loaded unpacked via chrome://extensions.

dx bundle --release

# The background worker isn't a Dioxus "app" (no UI, no asset_dir usage), so
# it's built as a plain wasm binary via `dx build --bin` rather than
# `dx bundle`, which skips the HTML-app packaging we don't need here.
# Old hashed outputs aren't cleaned between runs, so wipe them first —
# otherwise the glob below can pair a stale .js with a fresh .wasm (or vice
# versa) from a previous build.
rm -rf target/dx/background
dx build --bin background --platform web --release --inject-loading-scripts false

BG_ASSETS="target/dx/background/release/web/public/assets"
BG_JS=$(find "$BG_ASSETS" -name 'background-*.js' | head -1)
BG_WASM=$(find "$BG_ASSETS" -name 'background_bg-*.wasm' | head -1)

if [[ -z "$BG_JS" || -z "$BG_WASM" ]]; then
  echo "Could not find built background worker assets under $BG_ASSETS" >&2
  exit 1
fi

# The glue JS hardcodes the hashed wasm filename it was built with; rename
# both to stable names and patch that one reference so background.js (which
# is static, checked into extension/) can import them without knowing the hash.
cp "$BG_WASM" dist/public/background_bg.wasm
sed "s#/\\./assets/$(basename "$BG_WASM")#./background_bg.wasm#" "$BG_JS" > dist/public/background_wasm.js

cp extension/manifest.json extension/background.js extension/content.js dist/public/

echo ""
echo "✓ Extension assembled at dist/public"
echo "  Load it via chrome://extensions → Developer mode → Load unpacked → dist/public"
