#!/usr/bin/env bash
# Build the browser frontend into web/pkg/.
# Requires: rustup target add wasm32-unknown-unknown
#           cargo install wasm-bindgen-cli --version 0.2.126
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --lib --release --target wasm32-unknown-unknown
wasm-bindgen target/wasm32-unknown-unknown/release/gba.wasm \
  --out-dir web/pkg --target web --no-typescript
echo "built web/pkg"
