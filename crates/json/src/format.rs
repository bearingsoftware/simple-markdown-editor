//! Reformatting a JSON document, and the one place in this editor allowed to rewrite bytes wholesale.
//!
//! # The design decision, because `AGENTS.md` requires one on the ticket
//!
//! The byte-preservation invariant forbids the editor rewriting bytes **the user did not ask it to
//! rewrite**. It does not forbid an explicit, undoable command whose entire purpose is to rewrite
//! them. That is an edit like any other: initiated by the user, applied through `Document::replace`,
//! one undo step, and **never** triggered by a save or by anything else implicit.
//!
//! The line is between "the editor reformatted my file" — forbidden, permanently — and "I asked it
//! to". MDEDIT-48 is where that line was drawn, and it is drawn *once*: nothing else in this codebase
//! gets to reformat anything by citing this.
//!
//! # Only JSON, and the other two are refusals rather than omissions
//!
//! - **JSON: yes.** Pretty-printing is well defined, and the indent is the only choice to make.
//! - **YAML: no.** Every mainstream serializer loses comments, anchors and formatting on a round
//!   trip. A comment-preserving reformatter is a project, not a feature.
//! - **Markdown: no.** "Formatted Markdown" is a style argument — wrap column, emphasis token, list
//!   marker — and this editor's whole architecture exists to avoid holding opinions about the user's
//!   Markdown bytes.
//!
//! # What survives, and how
//!
//! Everything except whitespace, because **the original bytes of every token are what get emitted**.
//! That is not a nicety: re-serializing through a number type loses `1.50`, `1e3` and `-0`, and
//! re-serializing through a string type normalises `A` to `A`. Nothing here parses a value —
//! it walks tokens and decides where the line breaks go. Object key order is preserved for the same
//! reason: there is no map to lose it in.
//!
//! # It refuses rather than guesses
//!
//! Gated on [`validate`](crate::validate) returning nothing. A document with any diagnostic is not
//! formatted at all, and the first diagnostic comes back so the caller can say why. Formatting a
//! malformed document would mean inventing the structure the user has not finished typing.

use mdedit_language::Diagnostic;
use mdedit_offsets::ByteSpan;

use crate::validate::{self, Tok};

/// One level of indent. An option later, if anybody asks; two spaces until then.
const INDENT: &str = "  ";

/// Every byte outside a string that is not JSON whitespace, plus every string verbatim.
///
/// **The oracle for this module**, not a formatting step — nothing here calls it. Comparing it either
/// side of a [`format`](crate::format) is what "only the whitespace moved" means precisely, which is the byte
/// invariant applied to the one operation allowed to break its letter. Whitespace *inside* a string
/// is content and must be kept, which is why this cannot simply strip every space.
///
/// It is here rather than in the test that first needed it because the fuzz target needs the same
/// answer (MDEDIT-111), and an oracle kept in two places is an oracle that drifts.
#[must_use]
pub fn meaningful_bytes(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b' ' | b'\t' | b'\n' | b'\r' => at += 1,
            b'"' => {
                let start = at;
                at += 1;
                while at < bytes.len() {
                    match bytes[at] {
                        b'\\' => at += 2,
                        b'"' => {
                            at += 1;
                            break;
                        }
                        _ => at += 1,
                    }
                }
                // An escape skips two *bytes*, and in a document nobody validated the byte after a
                // backslash can be the first of a multi-byte character — so the walk can stop mid
                // character. Snapping up is what keeps this from panicking on input the fuzzer will
                // certainly produce; an oracle that panics reports itself as the bug.
                at = at.min(source.len());
                while at < source.len() && !source.is_char_boundary(at) {
                    at += 1;
                }
                out.push_str(&source[start..at]);
            }
            _ => {
                let start = at;
                at += 1;
                while at < bytes.len() && !source.is_char_boundary(at) {
                    at += 1;
                }
                out.push_str(&source[start..at]);
            }
        }
    }
    out
}

/// Reformats `source`, or reports the first thing wrong with it.
///
/// `Ok` is the whole document, ready to replace it. It ends with a newline, as every other text file
/// this editor writes does.
///
/// An empty document comes back unchanged rather than as a lone newline: there is nothing to format,
/// and turning an empty file into a one-byte one would be an edit nobody asked for.
///
/// # Errors
///
/// The first [`Diagnostic`] from [`validate`](crate::validate), when the document is not valid JSON.
pub fn format(source: &str) -> Result<String, Diagnostic> {
    if let Some(first) = crate::validate(source).into_iter().next() {
        return Err(first);
    }

    let tokens = validate::tokens(source);
    if tokens.is_empty() {
        return Ok(source.to_owned());
    }

    let mut out = String::with_capacity(source.len() + source.len() / 4);
    let mut depth = 0_usize;

    for (index, (span, tok)) in tokens.iter().enumerate() {
        let text = span.slice(source);
        match tok {
            Tok::Open(_) => {
                out.push_str(text);
                // An empty container stays on one line, which is what every formatter does and what
                // anybody reading `"tags": []` expects. The closer's arm does the same lookup
                // backwards rather than either of them tracking state.
                if !closes_immediately(&tokens, index) {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            Tok::Close(_) => {
                if !opened_immediately(&tokens, index) {
                    depth = depth.saturating_sub(1);
                    newline(&mut out, depth);
                }
                out.push_str(text);
            }
            Tok::Comma => {
                out.push(',');
                newline(&mut out, depth);
            }
            Tok::Colon => out.push_str(": "),
            // The bytes exactly as the user wrote them. See the module docs for why that matters
            // more than it looks like it does.
            //
            // `Garbage` is unreachable here — `validate` returned nothing, and it reports every byte
            // that begins no token — and it is in this arm rather than its own because emitting the
            // bytes is still the right answer if that is ever wrong. Losing them would be worse than
            // an odd-looking line.
            Tok::Str | Tok::Scalar | Tok::Garbage => out.push_str(text),
        }
    }

    out.push('\n');
    Ok(out)
}

/// Whether the token after `index` is the closer for the container opening at it.
fn closes_immediately(tokens: &[(ByteSpan, Tok)], index: usize) -> bool {
    matches!(tokens.get(index + 1), Some((_, Tok::Close(_))))
}

/// Whether the token before `index` is the opener for the container closing at it.
fn opened_immediately(tokens: &[(ByteSpan, Tok)], index: usize) -> bool {
    index > 0 && matches!(tokens.get(index - 1), Some((_, Tok::Open(_))))
}

fn newline(out: &mut String, depth: usize) {
    out.push('\n');
    for _ in 0..depth {
        out.push_str(INDENT);
    }
}
