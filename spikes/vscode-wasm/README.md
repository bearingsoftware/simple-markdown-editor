# vscode-wasm (MDEDIT-206)

A probe, never shipped: the headless crates compiled to wasm, a DOM preview built from the marker
partition, and a minimal `CustomTextEditorProvider` (`*.md`, priority `option`). Findings are in
[`docs/research/08-vscode-webview.md`](../../docs/research/08-vscode-webview.md). Its own Cargo
workspace; the root `Cargo.toml` excludes `spikes/*`, so CI never sees it.

Re-run, from this folder (cargo one command at a time, with a target dir that is not the main one):

```bash
rustup target add wasm32-unknown-unknown
export CARGO_TARGET_DIR='C:\Users\craig\projects\bearingsoftware\sme-target-wasm'
cargo install --locked --root "$PWD/.tools" wasm-bindgen-cli --version 0.2.129   # must equal the wasm-bindgen crate
npm install                          # binaryen (wasm-opt), jsdom, @vscode/test-electron, @vscode/vsce
./build.sh nohl && ./build.sh hl     # pkg/ and pkg-hl/, each with opt.wasm
npm test                             # 16 Node tests: DOM structure (probe 4) and caret mapping (probe 5)
node test/bench.mjs pkg-hl           # probe 3 in Node; args: <pkg dir> [wasm file]
cargo run -p sme-vscode-core --release --example native --no-default-features   # native reference
./stage.sh hl                        # copy a variant into extension/media/core
node test/vscode/run.mjs             # probes 6, 7 in a throwaway VS Code 1.145.3 (.vscode-test/, own profile)
SME_CSP_CONNECT=1 SME_RESULTS_NAME=vscode-e2e-connect-src.json node test/vscode/run.mjs   # CSP comparison
(cd extension && ../node_modules/.bin/vsce package --skip-license --no-dependencies -o ../x.vsix)  # never publish
```

`results/` holds the saved outputs. Timings on this machine swung 2x with other builds running;
the research doc says which runs were quiet.
