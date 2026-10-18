#!/bin/sh
# builds the web app into app/web/dist: the wasm, its bindings, the page and the icon.
# Serve dist where the hub answers /app/ws (the hub's app API), e.g. beside it.
set -e
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
target=${CARGO_TARGET_DIR:-$root/target}
cargo build -p anend-app --target wasm32-unknown-unknown --no-default-features --features web --release
mkdir -p "$here/dist"
wasm-bindgen --target web --no-typescript --out-dir "$here/dist" "$target/wasm32-unknown-unknown/release/anend-app.wasm"
cp "$here/index.html" "$here/dist/index.html"
cp "$root/assets/icon.svg" "$here/dist/icon.svg"
echo "built $here/dist"
