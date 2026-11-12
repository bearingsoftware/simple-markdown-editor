// The check a build would make before taking a file into the custom editor: VS Code's TextDocument has
// one EndOfLine and rewrites the rest, so a file with more than one kind of line ending must not open here.

/** Counts of each line-ending kind in `bytes`: CRLF, lone LF, lone CR. */
export function countLineEndings(bytes) {
  const n = { crlf: 0, lf: 0, cr: 0 };
  for (let i = 0; i < bytes.length; i++) {
    if (bytes[i] === 0x0d) {
      if (bytes[i + 1] === 0x0a) { n.crlf++; i++; } else n.cr++;
    } else if (bytes[i] === 0x0a) {
      n.lf++;
    }
  }
  return n;
}

/** True when more than one kind of line ending occurs, however the counts compare (a tie is mixed). */
export function hasMixedLineEndings(bytes) {
  const { crlf, lf, cr } = countLineEndings(bytes);
  return [crlf, lf, cr].filter((c) => c > 0).length > 1;
}
