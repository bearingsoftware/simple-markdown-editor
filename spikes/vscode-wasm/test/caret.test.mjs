// Probe 5: DOM (node, offset) -> source byte -> VS Code Position, asserted against independent references.
// jsdom in Node (not a webview), real wasm analysis.
import test from "node:test";
import assert from "node:assert/strict";
import { core, enc, doc, dec } from "./helpers.mjs";
import { render } from "../extension/media/render.js";
import { byteFromDom, domFromByte, positionFromByte, byteFromPosition } from "../extension/media/caret.js";

function build(src) {
  const bytes = enc.encode(src);
  return { bytes, root: render(doc, bytes, core.analyze(src)) };
}
const textNodes = (root) => {
  const out = [];
  const w = doc.createTreeWalker(root, 4);
  for (let t = w.nextNode(); t; t = w.nextNode()) out.push(t);
  return out;
};
const nodeWith = (root, str) => textNodes(root).find((t) => t.data.includes(str));

// What VS Code does: line breaks are \r\n, \n and \r; character counts UTF-16 units.
function referencePosition(src, byte) {
  const prefix = dec.decode(enc.encode(src).subarray(0, byte));
  let line = 0, start = 0;
  for (const m of prefix.matchAll(/\r\n|\r|\n/g)) { line++; start = m.index + m[0].length; }
  return { line, character: prefix.length - start };
}

test("ASCII", () => {
  const { root } = build("plain text here\n");
  const t = nodeWith(root, "plain");
  assert.equal(byteFromDom(t, 0), 0);
  assert.equal(byteFromDom(t, 5), 5);
  assert.equal(byteFromDom(t, t.data.length), 15);
});

test("multi-byte UTF-8: 2-byte and 3-byte characters", () => {
  const src = "héllo 汉字 end\n";
  const { root } = build(src);
  const t = nodeWith(root, "llo");
  assert.equal(byteFromDom(t, 2), 3, "after the 2-byte e-acute");
  assert.equal(byteFromDom(t, 7), 10, "after the 3-byte han character (units 7, bytes 10)");
  assert.equal(byteFromDom(t, 8), 13);
});

test("astral character: 2 UTF-16 units in the DOM, 4 bytes in the source", () => {
  const src = "a\u{1F600}b\n";
  const { root } = build(src);
  const t = nodeWith(root, "b");
  assert.equal(t.data.length, 4, "surrogate pair is two DOM units");
  assert.equal(byteFromDom(t, 1), 1);
  assert.equal(byteFromDom(t, 3), 5, "after the pair");
  assert.equal(byteFromDom(t, 4), 6);
  assert.deepEqual(positionFromByte(enc.encode(src), 5), { line: 0, character: 3 }, "VS Code counts the pair as 2");
  const back = domFromByte(root, 5);
  assert.equal(back.node, t);
  assert.equal(back.offset, 3);
});

test("right after a concealed marker", () => {
  const src = "x **bold** y\n\n## Head\n";
  const { root } = build(src);
  const bold = nodeWith(root, "bold");
  assert.equal(byteFromDom(bold, 0), 4, "start of the content, past the ** marker");
  const strong = root.querySelector("strong");
  assert.equal(byteFromDom(strong, 0), 4, "element position before the hidden marker resolves to the content side");
  assert.equal(byteFromDom(strong, 1), 4);
  assert.equal(byteFromDom(strong, strong.childNodes.length), 8, "end of content, before the closing marker");
  assert.equal(byteFromDom(nodeWith(root, "Head"), 0), 17, "after the concealed '## '");
  const back = domFromByte(root, 3);
  assert.equal(back.node, bold, "a byte inside the opening marker snaps forward to the next text");
  assert.equal(back.offset, 0);
});

test("CRLF: the line break stays in the DOM text and counts one line", () => {
  const src = "line one\r\nline two\r\n\r\nnext\r\n";
  const { bytes, root } = build(src);
  const t = nodeWith(root, "line one");
  assert.equal(t.data, "line one\r\nline two");
  assert.equal(byteFromDom(t, 10), 10, "after the CRLF");
  assert.deepEqual(positionFromByte(bytes, 10), { line: 1, character: 0 });
  assert.deepEqual(positionFromByte(bytes, 9), { line: 0, character: 8 }, "between CR and LF clamps to before the CR");
  assert.deepEqual(positionFromByte(bytes, 26), { line: 3, character: 4 }, "after next");
  assert.deepEqual(positionFromByte(bytes, 28), { line: 4, character: 0 });
  assert.equal(byteFromPosition(bytes, 1, 0), 10);
  assert.equal(byteFromPosition(bytes, 3, 2), 24);
});

const MIXED = [
  "# Héllo 汉字",
  "",
  "Café **böld** and \u{1F600} *itálic* __under__ `códe`\r",
  "second line\r",
  "",
  "- item \u{1F680} one",
  "- item two  ",
  "",
  "> quote 汉",
  "> more",
  "",
  "tail [línk](http://x.test)",
  "",
].join("\n");

test("exhaustive: every DOM position in a mixed document maps to the right byte, Position and back", () => {
  const { bytes, root } = build(MIXED);
  let checked = 0;
  for (const t of textNodes(root)) {
    const base = Number(t.parentElement.dataset.b0);
    for (let off = 0; off <= t.data.length; off++) {
      // A position inside a surrogate pair is not a position a caret can occupy.
      if (off > 0 && off < t.data.length && /[\ud800-\udbff]/.test(t.data[off - 1]) && /[\udc00-\udfff]/.test(t.data[off])) continue;
      const expected = base + enc.encode(t.data.slice(0, off)).length;
      const byte = byteFromDom(t, off);
      assert.equal(byte, expected, `${JSON.stringify(t.data)} @${off}`);
      // A byte between a CRLF's CR and LF is not a VS Code position; skip the reference there.
      if (!(bytes[byte - 1] === 0x0d && bytes[byte] === 0x0a)) {
        assert.deepEqual(positionFromByte(bytes, byte), referencePosition(MIXED, byte), `position @${byte}`);
      }
      assert.equal(byteFromDom(domFromByte(root, byte).node, domFromByte(root, byte).offset), byte, `round trip @${byte}`);
      checked++;
    }
  }
  assert.ok(checked > 100, `checked ${checked}`);
});

test("byteFromPosition inverts positionFromByte at every character boundary", () => {
  const bytes = enc.encode(MIXED);
  const text = dec.decode(bytes);
  let at = 0;
  for (const ch of text) {
    const here = at;
    at += enc.encode(ch).length;
    if (bytes[here] === 0x0a && bytes[here - 1] === 0x0d) continue;
    const p = positionFromByte(bytes, here);
    assert.equal(byteFromPosition(bytes, p.line, p.character), here, JSON.stringify(p));
  }
});
