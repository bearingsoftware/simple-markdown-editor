// The mixed-line-ending detector a build would use as the mitigation for probe 6. Node only.
import test from "node:test";
import assert from "node:assert/strict";
import { countLineEndings, hasMixedLineEndings } from "../extension/media/eol.js";

const b = (s) => new TextEncoder().encode(s);

test("uniform files are not mixed, including CRLF and a file with no line ending", () => {
  assert.equal(hasMixedLineEndings(b("")), false);
  assert.equal(hasMixedLineEndings(b("no newline")), false);
  assert.equal(hasMixedLineEndings(b("a\nb\nc\n")), false);
  assert.equal(hasMixedLineEndings(b("a\r\nb\r\nc\r\n")), false);
  assert.equal(hasMixedLineEndings(b("a\rb\rc\r")), false);
});

test("a CRLF pair counts once and is not also an LF or a CR", () => {
  assert.deepEqual(countLineEndings(b("a\r\nb\r\n")), { crlf: 2, lf: 0, cr: 0 });
});

test("ties are mixed: the case a majority vote would hide", () => {
  assert.equal(hasMixedLineEndings(b("a\r\nb\nc\r\nd\n")), true);
  assert.equal(hasMixedLineEndings(b("a\r\nb\n")), true);
});

test("one stray ending of another kind is mixed", () => {
  assert.equal(hasMixedLineEndings(b("a\nb\nc\nd\ne\r\n")), true);
  assert.equal(hasMixedLineEndings(b("a\r\nb\r\nc\r\nd\n")), true);
});

test("a lone CR is its own kind, alone and beside the others", () => {
  assert.deepEqual(countLineEndings(b("a\rb\n")), { crlf: 0, lf: 1, cr: 1 });
  assert.equal(hasMixedLineEndings(b("a\rb\n")), true);
  assert.equal(hasMixedLineEndings(b("a\r\nb\rc")), true);
  assert.equal(hasMixedLineEndings(b("a\r")), false, "a CR at the very end is still just one CR");
  assert.equal(hasMixedLineEndings(b("a\r\r\nb")), true, "CR then CRLF");
});

test("it flags the probe 6 fixture, which VS Code normalised", () => {
  const fixture = b("# Títle 汉  \r\n\r\nplain *star* and _under_ side by side  \nline with trailing   \r\n\n- item \u{1F600} end\n");
  assert.deepEqual(countLineEndings(fixture), { crlf: 3, lf: 3, cr: 0 });
  assert.equal(hasMixedLineEndings(fixture), true);
});
