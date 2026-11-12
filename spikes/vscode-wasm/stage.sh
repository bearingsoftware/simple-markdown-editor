#!/usr/bin/env bash
# Copies a built variant into the extension: ./stage.sh hl | nohl. The optimised wasm replaces the raw one.
set -euo pipefail
cd "$(dirname "$0")"
variant="${1:-hl}"
if [ "$variant" = hl ]; then src=pkg-hl; else src=pkg; fi
rm -rf extension/media/core && mkdir -p extension/media/core
cp "$src/sme_vscode_core.js" extension/media/core/
cp "$src/opt.wasm" extension/media/core/sme_vscode_core_bg.wasm
