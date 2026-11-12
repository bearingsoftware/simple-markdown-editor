// Probe 4: marker/content partition -> DOM, asserted structurally and dumped to results/dom-dump.txt.
// Runs under jsdom in Node (not a webview); the wasm is the real core.
import test from "node:test";
import assert from "node:assert/strict";
import { writeFileSync } from "node:fs";
import { core, enc, doc, dec } from "./helpers.mjs";
import { render } from "../extension/media/render.js";

const SOURCE = [
  "# Title with *emphasis*",
  "",
  "Some **strong** and `code` and a [link](http://example.com \"t\") plus ~~gone~~.",
  "",
  "- first",
  "- second",
  "",
  "3. three",
  "4. four",
  "",
  "> quoted",
  "> more",
  "",
  "```rust",
  "fn x() {}",
  "```",
  "",
  "| a | b |",
  "|---|---|",
  "| 1 | 2 |",
  "",
  "An image ![alt](p.png) inline.",
  "",
  "---",
  "",
].join("\n");

function build(src, highlight) {
  const bytes = enc.encode(src);
  return { bytes, root: render(doc, bytes, core.analyze(src), { highlight }) };
}
const visible = (root) => {
  const clone = root.cloneNode(true);
  for (const m of clone.querySelectorAll(".marker")) m.remove();
  return clone;
};

test("headings, emphasis, strong, code, links, strikethrough render as elements", () => {
  const { root } = build(SOURCE);
  assert.equal(root.querySelector("h1").textContent, "Title with emphasis");
  assert.equal(root.querySelector("h1 em").textContent, "emphasis");
  assert.equal(root.querySelector("strong").textContent, "strong");
  assert.equal(root.querySelector("p code").textContent, "code");
  assert.equal(root.querySelector("a").textContent, "link");
  assert.equal(root.querySelector("del").textContent, "gone");
});

test("markers are concealed: no marker byte appears in rendered text, and each is a hidden empty element", () => {
  const { root } = build(SOURCE);
  const markers = [...root.querySelectorAll(".marker")];
  assert.ok(markers.length > 10, "markers are present in the DOM as hidden spans");
  for (const m of markers) {
    assert.equal(m.hidden, true);
    assert.equal(m.textContent, "");
  }
  const text = visible(root).textContent;
  for (const leaked of ["# ", "**", "~~", "](http", "```", "> ", "- ", "3. "]) {
    assert.ok(!text.includes(leaked), `leaked ${JSON.stringify(leaked)}`);
  }
});

test("lists: bullets and numbers are substituted by the list element, not drawn from the source", () => {
  const { root } = build(SOURCE);
  const ul = root.querySelector("ul");
  assert.deepEqual([...ul.children].map((li) => li.textContent), ["first", "second"]);
  const ol = root.querySelector("ol");
  assert.equal(ol.start, 3);
  assert.deepEqual([...ol.children].map((li) => li.textContent), ["three", "four"]);
});

test("block quote and fenced code", () => {
  const { root } = build(SOURCE);
  assert.equal(root.querySelector("blockquote").textContent.replace(/\s+/g, " ").trim(), "quoted more");
  const code = root.querySelector("pre > code");
  assert.equal(code.className, "language-rust");
  assert.equal(code.textContent, "fn x() {}\n");
});

test("tables, images and a thematic break come out as source or a rule", () => {
  const { root } = build(SOURCE);
  const table = root.querySelector("pre.source");
  assert.equal(table.textContent, "| a | b |\n|---|---|\n| 1 | 2 |");
  assert.match(root.textContent, /!\[alt\]\(p\.png\)/);
  assert.ok(root.querySelector("hr"));
});

test("a highlighter hook colours fence text without losing a byte", () => {
  const { root } = build(SOURCE, (info, code) => core.highlight(info, code));
  const toks = root.querySelectorAll("code [class^=tok-]");
  assert.ok(toks.length > 0, "tokens rendered");
  assert.equal(root.querySelector("pre > code").textContent, "fn x() {}\n");
});

test("every text node carries exactly its own source bytes", () => {
  const { bytes, root } = build(SOURCE);
  const walker = doc.createTreeWalker(root, 4);
  let n = 0;
  for (let t = walker.nextNode(); t; t = walker.nextNode(), n++) {
    const el = t.parentElement;
    const a = Number(el.dataset.b0), b = Number(el.dataset.b1);
    assert.equal(dec.decode(bytes.subarray(a, b)), t.data);
  }
  assert.ok(n > 20);
});

test("dump", () => {
  const { root } = build(SOURCE, (i, c) => core.highlight(i, c));
  const pretty = root.outerHTML.replace(/></g, ">\n<");
  writeFileSync(new URL("../results/dom-dump.html", import.meta.url), pretty + "\n");
});
