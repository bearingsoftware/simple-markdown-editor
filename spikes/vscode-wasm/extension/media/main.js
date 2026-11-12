// Webview app: wasm core + DOM renderer + caret mapping. Spike code; it handles single-character
// inserts through `beforeinput` and re-renders the whole document, which is what the virtualisation
// section of docs/research/08-vscode-webview.md says a build must not do.
import init, { Session, analyze, fixture, highlight } from "./core/sme_vscode_core.js";
import { render } from "./render.js";
import { byteFromDom, domFromByte } from "./caret.js";

const vscode = acquireVsCodeApi();
const root = document.getElementById("root");
const enc = new TextEncoder();
let session = null;
let bytes = new Uint8Array();
let nextId = 1;
const pending = new Map();
const boot = {};

const fnv = (u8) => {
  let h = 0x811c9dc5;
  for (let i = 0; i < u8.length; i++) h = Math.imul(h ^ u8[i], 0x01000193) >>> 0;
  return h;
};

function post(m) {
  vscode.postMessage(m);
}

function draw() {
  const t = performance.now();
  bytes = enc.encode(session.text());
  const flat = session.flat();
  const tree = render(document, bytes, flat, { highlight: typeof highlight === "function" ? highlight : undefined });
  root.replaceChildren(tree);
  return performance.now() - t;
}

function report(kind, extra = {}) {
  post({ type: "state", kind, len: bytes.length, hash: fnv(bytes), domChars: root.textContent.length, ...extra });
}

function fromHost(type) {
  return new Promise((resolve) => pending.set(type, resolve));
}

async function loadWasm() {
  const url = document.body.dataset.wasm;
  const t0 = performance.now();
  let wasm;
  // The strict CSP has no connect-src, so this fetch is expected to be refused (probe 7 / CSP note).
  try {
    const r = await fetch(url);
    wasm = await r.arrayBuffer();
    boot.fetch = "ok";
    boot.fetchMs = performance.now() - t0;
  } catch (e) {
    boot.fetch = `blocked: ${e && e.message}`;
    boot.fetchRefusedMs = performance.now() - t0;
    const t = performance.now();
    const reply = fromHost("wasm");
    post({ type: "wasm" });
    wasm = (await reply).bytes;
    boot.hostWasmMs = performance.now() - t;
  }
  const t1 = performance.now();
  await init({ module_or_path: wasm });
  boot.instantiateMs = performance.now() - t1;
  boot.wasmBytes = wasm.byteLength ?? wasm.length;
}

function applyText(text) {
  session?.free();
  session = new Session(text);
  const ms = draw();
  report("text", { drawMs: ms });
}

function insertAtSelection(data) {
  const sel = getSelection();
  if (!sel.anchorNode) return;
  const a = byteFromDom(sel.anchorNode, sel.anchorOffset);
  const b = byteFromDom(sel.focusNode, sel.focusOffset);
  const start = Math.min(a, b), end = Math.max(a, b);
  const from = session.position(start), to = session.position(end);
  const id = nextId++;
  session.replace(start, end, data);
  const ms = draw();
  const caret = domFromByte(root, start + enc.encode(data).length);
  if (caret) getSelection().collapse(caret.node, caret.offset);
  post({ type: "edit", id, bytes: { start, end }, range: { sl: from[0], sc: from[1], el: to[0], ec: to[1] }, text: data });
  report("local-edit", { id, drawMs: ms });
}

root.addEventListener("beforeinput", (e) => {
  if (e.inputType !== "insertText" || !e.data) return;
  e.preventDefault();
  insertAtSelection(e.data);
});

function stats(xs) {
  const s = [...xs].sort((x, y) => x - y);
  const q = (p) => s[Math.min(s.length - 1, Math.ceil(p * s.length) - 1)];
  return { n: s.length, median: +q(0.5).toFixed(3), p99: +q(0.99).toFixed(3), max: +s[s.length - 1].toFixed(3) };
}
function time(n, f) {
  for (let i = 0; i < 3; i++) f();
  const out = [];
  for (let i = 0; i < n; i++) {
    const t = performance.now();
    f();
    out.push(performance.now() - t);
  }
  return stats(out);
}

// Probe 3 and 7 inside the real webview: analysis timings and a 1 MB postMessage each way.
async function bench() {
  const result = { boot };
  for (const [label, size, runs] of [["100KB", 100 * 1024, 100], ["1MB", 1024 * 1024, 40]]) {
    const text = fixture(size);
    const r = { bytes: text.length };
    r.analyze_count = time(runs, () => analyze(text).length);
    r.session_new = time(runs, () => new Session(text).free());
    const s = new Session(text);
    const mid = text.indexOf("Some prose", text.length >> 1) + 5;
    const ins = [], rec = [];
    for (let i = 0; i < runs + 3; i++) {
      let t = performance.now();
      s.replace(mid, mid, "x");
      const a = performance.now() - t;
      t = performance.now();
      s.reconcile();
      const b = performance.now() - t;
      s.undo();
      if (i >= 3) { ins.push(a); rec.push(b); }
    }
    r.edit_replace = stats(ins);
    r.edit_reconcile = stats(rec);
    s.free();
    result[label] = r;
  }
  // 1 MB each way. One-way uses the wall clock, shared with the host process; the round trip needs no clock.
  const payload = "x".repeat(1024 * 1024);
  const rtt = [];
  for (let i = 0; i < 12; i++) {
    const id = nextId++;
    const done = new Promise((resolve) => pending.set(`echo${id}`, resolve));
    const t = performance.now();
    post({ type: "echo", id, payload, sentAt: performance.timeOrigin + performance.now() });
    await done;
    if (i >= 2) rtt.push(performance.now() - t);
  }
  result.postMessage_1MB_roundtrip_ms = stats(rtt);
  post({ type: "bench", result });
}

window.addEventListener("message", (ev) => {
  const m = ev.data;
  if (m.type === "wasm") {
    pending.get("wasm")?.(m);
  } else if (m.type === "text") {
    applyText(m.text);
  } else if (m.type === "ack") {
    post({ type: "acked", id: m.id, ok: m.ok, version: m.version });
  } else if (m.type === "echo") {
    pending.get(`echo${m.id}`)?.(m);
  } else if (m.type === "sim-insert") {
    // The test cannot type into a webview; this dispatches the same beforeinput a keystroke would.
    const p = domFromByte(root, m.byte);
    getSelection().collapse(p.node, p.offset);
    root.dispatchEvent(new InputEvent("beforeinput", { inputType: "insertText", data: m.data, bubbles: true, cancelable: true }));
  } else if (m.type === "bench") {
    bench();
  } else if (m.type === "report") {
    report("requested");
  }
});

(async () => {
  try {
    await loadWasm();
    post({ type: "booted", boot });
    post({ type: "ready" });
  } catch (e) {
    post({ type: "error", message: String(e && e.stack ? e.stack : e) });
  }
})();
