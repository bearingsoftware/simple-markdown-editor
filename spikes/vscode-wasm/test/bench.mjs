// Probe 3 in Node (V8, not a webview). Usage: node test/bench.mjs <pkg-dir> [wasm-file-name]
import { readFileSync } from "node:fs";
import { performance } from "node:perf_hooks";
import { pathToFileURL } from "node:url";
import { resolve } from "node:path";

const dir = resolve(process.argv[2] ?? "pkg");
const wasmName = process.argv[3] ?? "sme_vscode_core_bg.wasm";
const glue = await import(pathToFileURL(resolve(dir, "sme_vscode_core.js")).href);
const bytes = readFileSync(resolve(dir, wasmName));

function stats(xs) {
  const s = [...xs].sort((a, b) => a - b);
  const q = (p) => s[Math.min(s.length - 1, Math.ceil(p * s.length) - 1)];
  return { n: s.length, median: +q(0.5).toFixed(3), p99: +q(0.99).toFixed(3), max: +s[s.length - 1].toFixed(3) };
}
function time(n, f, warm = 3) {
  for (let i = 0; i < warm; i++) f();
  const out = [];
  for (let i = 0; i < n; i++) {
    const t = performance.now();
    f();
    out.push(performance.now() - t);
  }
  return stats(out);
}

const result = { wasm: wasmName, bytes: bytes.length };

// Instantiate: compile + instantiate, 10 fresh modules. The first is reported apart (cold).
const inst = [];
for (let i = 0; i < 10; i++) {
  const t = performance.now();
  const m = new WebAssembly.Module(bytes);
  void m;
  inst.push(performance.now() - t);
}
result.compile_ms = stats(inst);
const t0 = performance.now();
glue.initSync({ module: bytes });
result.initSync_first_ms = +(performance.now() - t0).toFixed(3);

for (const [label, size, runs] of [["100KB", 100 * 1024, 100], ["1MB", 1024 * 1024, 40]]) {
  const text = glue.fixture(size);
  const r = { bytes: text.length };
  { const t = performance.now(); glue.analyze_count(text); r.first_analyze_cold_ms = +(performance.now() - t).toFixed(3); }
  r.analyze_count = time(runs, () => glue.analyze_count(text));
  r.analyze_flat = time(runs, () => glue.analyze(text));
  r.session_new = time(runs, () => new glue.Session(text).free());

  const s = new glue.Session(text);
  const mid = text.indexOf("Some prose", text.length >> 1) + 5;
  const ins = [], rec = [], flat = [];
  for (let i = 0; i < runs + 3; i++) {
    let t = performance.now();
    s.replace(mid, mid, "x");
    const a = performance.now() - t;
    t = performance.now();
    s.flat();
    const f = performance.now() - t;
    t = performance.now();
    s.reconcile();
    const b = performance.now() - t;
    s.undo();
    if (i >= 3) { ins.push(a); rec.push(b); flat.push(f); }
  }
  r.edit_replace = stats(ins);
  r.edit_marshal_after = stats(flat);
  r.edit_reconcile = stats(rec);
  r.scoped_after_edit = (s.replace(mid, mid, "x"), s.is_scoped());
  result[label] = r;
}

if (typeof glue.highlight === "function") {
  const code = "fn example() -> u32 { 42 }\n".repeat(20);
  const t = performance.now();
  const spans = glue.highlight("rust", code);
  result.highlight_first_call_ms = +(performance.now() - t).toFixed(3);
  result.highlight_spans = spans.length / 3;
  result.highlight_warm = time(50, () => glue.highlight("rust", code));
}
console.log(JSON.stringify(result, null, 1));
