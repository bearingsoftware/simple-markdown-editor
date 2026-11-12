// DOM position <-> source byte offset <-> VS Code position (line, UTF-16 character).

/** UTF-8 bytes of the first `units` UTF-16 code units of `str`. A split surrogate pair counts as the whole pair. */
export function utf8Prefix(str, units) {
  let n = 0;
  for (let i = 0; i < units && i < str.length; i++) {
    const c = str.charCodeAt(i);
    if (c < 0x80) n += 1;
    else if (c < 0x800) n += 2;
    else if (c >= 0xd800 && c <= 0xdbff && i + 1 < str.length) { n += 4; i++; }
    else n += 3;
  }
  return n;
}

const spanOf = (el) => (el && el.dataset && el.dataset.b0 !== undefined ? [Number(el.dataset.b0), Number(el.dataset.b1)] : null);

const isMarker = (n) => n.nodeType === 1 && n.classList.contains("marker");

// Content-side start of a node: descend first, skipping concealed markers, then fall back to its own span.
function firstStart(node) {
  if (isMarker(node)) return null;
  for (const c of node.childNodes ?? []) {
    const r = firstStart(c);
    if (r !== null) return r;
  }
  const own = node.nodeType === 1 ? spanOf(node) : null;
  return own ? own[0] : null;
}

function lastEnd(node) {
  if (isMarker(node)) return null;
  const kids = node.childNodes ?? [];
  for (let i = kids.length - 1; i >= 0; i--) {
    const r = lastEnd(kids[i]);
    if (r !== null) return r;
  }
  const own = node.nodeType === 1 ? spanOf(node) : null;
  return own ? own[1] : null;
}

/**
 * Source byte offset for a DOM position, the shape `Selection.anchorNode/anchorOffset` gives.
 * A position inside a concealed marker cannot exist (the marker has no text), so a caret after one is
 * the start of the content that follows it.
 */
export function byteFromDom(node, offset) {
  if (node.nodeType === 3) {
    const owner = node.parentElement;
    const span = spanOf(owner);
    if (!span) throw new Error("text node without a source span");
    return span[0] + utf8Prefix(node.data, offset);
  }
  const kids = node.childNodes;
  for (let i = offset; i < kids.length; i++) {
    const r = firstStart(kids[i]);
    if (r !== null) return r;
  }
  for (let i = Math.min(offset, kids.length) - 1; i >= 0; i--) {
    const r = lastEnd(kids[i]);
    if (r !== null) return r;
  }
  const own = spanOf(node);
  if (own) return offset === 0 ? own[0] : own[1];
  return firstStart(node) ?? 0;
}

/** The inverse: a DOM position for a byte. A byte inside a concealed marker snaps forward to the next text. */
export function domFromByte(root, byte) {
  const walker = root.ownerDocument.createTreeWalker(root, 4 /* SHOW_TEXT */);
  let prev = null;
  for (let t = walker.nextNode(); t; t = walker.nextNode()) {
    const [a, b] = spanOf(t.parentElement);
    if (byte < a) return { node: t, offset: 0 };
    if (byte <= b) {
      const str = t.data;
      let units = 0, bytes = 0;
      while (bytes < byte - a && units < str.length) {
        const c = str.codePointAt(units);
        bytes += c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;
        units += c >= 0x10000 ? 2 : 1;
      }
      return { node: t, offset: units };
    }
    prev = t;
  }
  return prev ? { node: prev, offset: prev.data.length } : null;
}

/**
 * VS Code Position for a byte in `bytes`. Line breaks are \n, \r\n and a lone \r, as VS Code counts them.
 * A byte between the \r and \n of a CRLF is clamped to before the \r, which is what
 * `TextDocument.validatePosition` does with a column past the line's end.
 */
export function positionFromByte(bytes, byte) {
  let line = 0, lineStart = 0;
  for (let i = 0; i < byte; i++) {
    const c = bytes[i];
    if (c === 0x0a) { line++; lineStart = i + 1; }
    else if (c === 0x0d) {
      if (bytes[i + 1] === 0x0a) { if (i + 1 >= byte) { byte = i; break; } i++; }
      line++; lineStart = i + 1;
    }
  }
  const character = new TextDecoder().decode(bytes.subarray(lineStart, byte)).length;
  return { line, character };
}

/** Inverse of positionFromByte, for turning a VS Code Range back into byte offsets. */
export function byteFromPosition(bytes, line, character) {
  let at = 0;
  for (let l = 0; l < line; l++) {
    while (at < bytes.length && bytes[at] !== 0x0a && bytes[at] !== 0x0d) at++;
    if (at >= bytes.length) return bytes.length;
    at += bytes[at] === 0x0d && bytes[at + 1] === 0x0a ? 2 : 1;
  }
  let units = 0;
  while (at < bytes.length && units < character && bytes[at] !== 0x0a && bytes[at] !== 0x0d) {
    const lead = bytes[at];
    const len = lead < 0x80 ? 1 : lead < 0xe0 ? 2 : lead < 0xf0 ? 3 : 4;
    units += len === 4 ? 2 : 1;
    at += len;
  }
  return at;
}

