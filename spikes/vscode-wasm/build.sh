#!/usr/bin/env bash
# Builds the wasm core: ./build.sh hl | nohl. Output: pkg-hl/ or pkg/, each with opt.wasm (wasm-opt -Oz).
set -euo pipefail
cd "$(dirname "$0")"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-C:\Users\craig\projects\bearingsoftware\sme-target-wasm}"
variant="${1:-hl}"
if [ "$variant" = hl ]; then out=pkg-hl; flags=(); else out=pkg; flags=(--no-default-features); fi
cargo build -p sme-vscode-core --release --target wasm32-unknown-unknown "${flags[@]}"
wasm=$(cygpath -u "$CARGO_TARGET_DIR" 2>/dev/null || echo "$CARGO_TARGET_DIR")/wasm32-unknown-unknown/release/sme_vscode_core.wasm
./.tools/bin/wasm-bindgen --target web --out-dir "$out" "$wasm"
node node_modules/binaryen/bin/wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int \
  --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue \
  -o "$out/opt.wasm" "$out/sme_vscode_core_bg.wasm"
ls -l "$out"/*.wasm
