#!/usr/bin/env bash
set -euo pipefail

# Builds the Dioxus popup and the background service worker, and assembles
# them with the extension package (manifest.json, background.js, content.js,
# panel.js) into dist/public, ready to be loaded unpacked via
# chrome://extensions.

# Start both the output directory and the popup's build directory from empty.
#
# dx names its assets by content hash and never removes the ones a previous
# build left behind, and `dx bundle` copies that whole directory into
# dist/public — so without the second wipe the bundle accumulates every
# stylesheet and wasm blob this project has ever produced (164 files / 133MB
# at the time of writing, against 3 files / 4.7MB for one build's worth). Only
# the hashes the current wasm references are ever loaded; the rest is dead
# weight that still ships in the unpacked extension, and makes it needlessly
# hard to tell which stylesheet is the live one when debugging. Clearing it is
# not even a cost — copying the stale pile took longer than rebuilding.
#
# Wiping dist as well means it only ever holds what this run produced: when a
# step below fails, the result is a visibly missing file rather than the last
# build's copy sitting there looking current.
rm -rf dist target/dx/owls-ui

dx bundle --release

# dx's generated index.html preloads the module script with `crossorigin`,
# but the <script type="module"> tag that actually loads it doesn't carry
# that attribute — on a chrome-extension:// page the mismatch makes Chrome
# log a "cross-world extension resource mismatch" warning and re-fetch the
# script instead of reusing the preload. It's just a perf hint, so drop it.
# (Writing to a temp file instead of using `sed -i` keeps this portable
# between BSD sed on macOS and GNU sed on Linux CI runners.)
sed -E 's#<link rel="preload" as="script" href="[^"]*" crossorigin>##' dist/public/index.html > dist/public/index.html.tmp
mv dist/public/index.html.tmp dist/public/index.html

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

cp extension/manifest.json extension/background.js extension/content.js extension/panel.js dist/public/
cp extension/icon16.png extension/icon32.png extension/icon48.png extension/icon128.png dist/public/

echo ""
echo "✓ Extension assembled at dist/public"
echo "  Load it via chrome://extensions → Developer mode → Load unpacked → dist/public"
