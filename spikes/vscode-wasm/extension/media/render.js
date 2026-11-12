// Marker/content partition -> DOM. Pure DOM calls, no wasm import, so Node tests can drive it with jsdom.
//
// Contract with caret.js: every text node sits alone inside an element carrying data-b0/data-b1, the
// source byte span of exactly that text. Concealed markers produce no DOM text at all; they leave an
// empty hidden element carrying the span so a byte can still be located.

export const K = { CONTENT: 0, MARKER: 1, TOKEN: 2 };
export const M = {
  Emphasis: 1, Strong: 2, Strikethrough: 3, CodeSpan: 4, CodeBlock: 5, Heading: 6, BlockQuote: 7,
  ListItem: 8, ThematicBreak: 9, Link: 10, Image: 11, Escape: 12, HardBreak: 13, Footnote: 14,
  DefinitionList: 15, TaskMarker: 16, Table: 17,
};
export const B = {
  Paragraph: 1, Heading: 2, CodeBlock: 3, BlockQuote: 4, List: 5, ListItem: 6, ThematicBreak: 7,
  Html: 8, Table: 9, TableRow: 10, TableCell: 11,
};
const TOKEN_CLASS = ["", "key", "tag", "string", "number", "literal", "punct", "comment"];
const INLINE_ELEMENT = {
  [M.Emphasis]: "em", [M.Strong]: "strong", [M.Strikethrough]: "del", [M.CodeSpan]: "code", [M.Link]: "a",
};

export function unpack(flat) {
  const ns = flat[0], nb = flat[1];
  const segments = Array.from({ length: ns }, (_, i) => {
    const o = 2 + i * 4;
    return { start: flat[o], end: flat[o + 1], kind: flat[o + 2], sub: flat[o + 3] };
  });
  const blocks = Array.from({ length: nb }, (_, i) => {
    const o = 2 + ns * 4 + i * 6;
    return { id: flat[o], kind: flat[o + 1], sub: flat[o + 2], start: flat[o + 3], end: flat[o + 4], depth: flat[o + 5], children: [] };
  });
  return { segments, blocks };
}

// Blocks arrive in document order, parents first; containment by span rebuilds the tree.
function tree(blocks) {
  const roots = [];
  const stack = [];
  for (const b of blocks) {
    while (stack.length && !(b.start >= stack[stack.length - 1].start && b.end <= stack[stack.length - 1].end)) stack.pop();
    (stack.length ? stack[stack.length - 1].children : roots).push(b);
    stack.push(b);
  }
  return roots;
}

export function render(doc, bytes, flat, opts = {}) {
  const { segments, blocks } = unpack(flat);
  const dec = new TextDecoder();
  const text = (a, b) => dec.decode(bytes.subarray(a, b));

  // First segment whose end is past `at`.
  const firstSeg = (at) => {
    let lo = 0, hi = segments.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (segments[mid].end <= at) lo = mid + 1; else hi = mid;
    }
    return lo;
  };

  const span = (tag, a, b, cls) => {
    const el = doc.createElement(tag);
    el.dataset.b0 = String(a);
    el.dataset.b1 = String(b);
    if (cls) el.className = cls;
    return el;
  };
  const leafText = (parent, a, b, cls) => {
    if (a >= b) return;
    const el = span("span", a, b, cls);
    el.appendChild(doc.createTextNode(text(a, b)));
    parent.appendChild(el);
  };
  const concealed = (parent, a, b, sub) => {
    const el = span("span", a, b, "marker");
    el.hidden = true;
    el.dataset.marker = String(sub);
    parent.appendChild(el);
  };

  function inline(parent, from, to) {
    const open = [];
    let cur = parent;
    for (let i = firstSeg(from); i < segments.length && segments[i].start < to; i++) {
      const s = segments[i];
      const a = Math.max(s.start, from), b = Math.min(s.end, to);
      if (s.kind === K.MARKER) {
        const el = INLINE_ELEMENT[s.sub];
        if (s.sub === M.Image || s.sub === M.Footnote) {
          leafText(cur, a, b, "source");
        } else if (s.sub === M.HardBreak) {
          concealed(cur, a, b, s.sub);
          cur.appendChild(doc.createElement("br"));
        } else if (el) {
          const at = open.findLastIndex((o) => o.sub === s.sub);
          if (at >= 0) {
            // Close the matching opener, and anything left open inside it.
            concealed(open[at].el, a, b, s.sub);
            open[at].el.dataset.b1 = String(b);
            open.length = at;
            cur = open.length ? open[open.length - 1].el : parent;
          } else {
            const node = span(el, a, b);
            concealed(node, a, b, s.sub);
            cur.appendChild(node);
            open.push({ sub: s.sub, el: node });
            cur = node;
          }
        } else {
          concealed(cur, a, b, s.sub);
        }
      } else if (s.kind === K.CONTENT) {
        leafText(cur, a, b);
      }
    }
    for (const o of open) o.el.dataset.b1 = String(to);
  }

  const trimEol = (a, b) => {
    while (b > a && (bytes[b - 1] === 0x0a || bytes[b - 1] === 0x0d)) b--;
    return b;
  };

  function source(parent, b) {
    const el = doc.createElement("pre");
    el.className = "source";
    leafText(el, b.start, trimEol(b.start, b.end));
    parent.appendChild(el);
  }

  function codeBlock(parent, b) {
    const pre = doc.createElement("pre");
    const code = doc.createElement("code");
    let info = "";
    const body = [];
    for (let i = firstSeg(b.start); i < segments.length && segments[i].start < b.end; i++) {
      const s = segments[i];
      if (s.kind === K.MARKER) {
        concealed(code, s.start, s.end, s.sub);
        if (!info) info = text(s.start, s.end).replace(/^[`~]+\s*/, "").trim().split(/\s+/)[0] ?? "";
      } else {
        body.push(s);
      }
    }
    if (info) code.className = `language-${info}`;
    for (const s of body) {
      // Drop the line break that ends the fence line, so the first code line is not led by a blank one.
      let a = Math.max(s.start, b.start);
      const e = Math.min(s.end, b.end);
      if (body[0] === s && info) {
        if (bytes[a] === 0x0d && bytes[a + 1] === 0x0a) a += 2; else if (bytes[a] === 0x0a) a += 1;
      }
      const tokens = opts.highlight && info ? opts.highlight(info, text(a, e)) : null;
      if (tokens && tokens.length) {
        // Token spans are byte offsets into the code string, so they index `bytes` directly.
        let at = a;
        for (let t = 0; t < tokens.length; t += 3) {
          const ts = a + tokens[t], te = a + tokens[t + 1];
          leafText(code, at, ts);
          leafText(code, ts, te, `tok-${TOKEN_CLASS[tokens[t + 2]] ?? "x"}`);
          at = te;
        }
        leafText(code, at, e);
      } else {
        leafText(code, a, e);
      }
    }
    pre.appendChild(code);
    parent.appendChild(pre);
  }

  function list(parent, b) {
    const first = segments[firstSeg(b.start)];
    const marker = first && first.kind === K.MARKER && first.sub === M.ListItem ? text(first.start, first.end) : "";
    const ordered = /^\s*\d+[.)]/.test(marker);
    const el = doc.createElement(ordered ? "ol" : "ul");
    if (ordered) {
      const n = Number.parseInt(marker.trim(), 10);
      if (n !== 1) el.start = n;
    }
    for (const item of b.children) {
      const li = span("li", item.start, item.end);
      // The bullet or number is substituted by the list element itself, never drawn from the source bytes.
      let at = item.start;
      for (const child of item.children) {
        inline(li, at, trimEol(at, child.start));
        blocks_([child], li);
        at = child.end;
      }
      inline(li, at, trimEol(at, item.end));
      el.appendChild(li);
    }
    parent.appendChild(el);
  }

  function blocks_(bs, parent) {
    for (const b of bs) {
      switch (b.kind) {
        case B.Heading: {
          const h = span(`h${Math.min(6, Math.max(1, b.sub))}`, b.start, b.end);
          inline(h, b.start, trimEol(b.start, b.end));
          parent.appendChild(h);
          break;
        }
        case B.Paragraph: {
          const p = span("p", b.start, b.end);
          inline(p, b.start, trimEol(b.start, b.end));
          parent.appendChild(p);
          break;
        }
        case B.CodeBlock: codeBlock(parent, b); break;
        case B.BlockQuote: {
          const q = span("blockquote", b.start, b.end);
          blocks_(b.children, q);
          parent.appendChild(q);
          break;
        }
        case B.List: list(parent, b); break;
        case B.ThematicBreak: parent.appendChild(span("hr", b.start, b.end)); break;
        default: source(parent, b); // html, tables and anything unclassified: as source, bytes intact
      }
    }
  }

  const root = doc.createElement("div");
  root.className = "doc";
  blocks_(tree(blocks), root);
  return root;
}
