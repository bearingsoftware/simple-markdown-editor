//! What the status bar says.
//!
//! Text only — no geometry, no GPU types, no text engine. The renderer asks for three strings and
//! places them; everything about *what* they say is decided here, where a test can read it without a
//! window.
//!
//! # Two halves with different costs
//!
//! The breadcrumb, the caret position, the encoding and the line-ending style are all free: the
//! first two are a scan of the block list and a rope lookup, and the last two are what
//! `mdedit-file` already knows.
//!
//! **The word count is not.** Counting words is a walk of the whole buffer, and doing it per frame
//! on a 245 KB document would undo what MDEDIT-17 and MDEDIT-18 bought. So it is computed on the
//! same idle timer that already runs the whole-document re-parse once typing stops, and the last
//! known value is shown in between. That is why [`details`] takes a count rather than a
//! document: the thing that produces it is deliberately not on this path.

use std::fmt::Write as _;

use mdedit_file::Newline;

/// Height of the bar.
pub const BAR_HEIGHT: f32 = 24.0;

/// Padding at each end of the bar.
pub const PADDING: f32 = 10.0;

/// Between the breadcrumb's container and the right-hand fields', so a full one never touches a
/// full other.
pub const GAP: f32 = 16.0;

/// The most of the bar the right-hand fields may take, as a share of its width.
///
/// They are short and bounded and the breadcrumb is neither, so the fields ask first — but not
/// without limit, or a long path and a wide window would leave the breadcrumb nothing. Above this
/// they truncate, and because the fields read left to right with the least important last, what goes
/// first is the word count, then the line endings, then the encoding, leaving line and column — the
/// priority order MDEDIT-38 asked for, without needing to encode it.
const RIGHT_SHARE: f32 = 0.6;

/// How much room the right-hand fields may take in a bar `width` wide.
#[must_use]
pub fn right_share(width: f32) -> f32 {
    (width * RIGHT_SHARE - PADDING).max(0.0)
}

/// Between breadcrumb steps.
///
/// A single glyph rather than `->`, because the breadcrumb is the widest thing in the bar and every
/// character costs room a heading could have used.
const STEP: &str = " › ";

/// Between the fields on the right.
const FIELD: &str = "   ";

/// Between the diagnostic count and the fields beside it.
///
/// A measured gap rather than the [`FIELD`] spaces, because the count is a separate label — it has
/// its own colour, and a colour is the whole reason it is not simply more text in [`details`].
pub const COUNT_GAP: f32 = 12.0;

/// How many problems the document has, or nothing at all.
///
/// **Absence of errors is silence, not "0 errors".** A permanent zero in the corner is a thing the
/// eye learns to stop reading, which costs exactly the attention the count exists to buy.
#[must_use]
pub fn errors(count: usize) -> Option<String> {
    (count > 0).then(|| {
        let plural = if count == 1 { "" } else { "s" };
        format!("{count} error{plural}")
    })
}

/// The heading trail to the caret, as one line.
///
/// Empty when nothing encloses the caret — a position above the document's first heading has no
/// trail, and saying nothing is better than inventing a root for it.
#[must_use]
pub fn breadcrumb(steps: &[String]) -> String {
    steps.join(STEP)
}

/// Where the caret is, counting from one.
///
/// **Characters, not bytes and not display columns.** Bytes would say "Col 4" in the middle of an
/// emoji; display columns would be answering a question about a terminal, since a tab is one
/// character here and this editor does not render it as eight. `Document::line_and_column` returns
/// the offsets; the `+ 1` is here because it is a presentation decision, not a document one.
#[must_use]
pub fn position(line: usize, column: usize) -> String {
    format!("Ln {}, Col {}", line + 1, column + 1)
}

/// Encoding, line-ending style, and a word count that may be a moment behind.
///
/// `words` is `None` before the first count has finished, which is the state a just-opened document
/// is in for as long as it takes to walk it once. Showing nothing there is better than showing a
/// zero that is about to change.
#[must_use]
pub fn details(encoding: &str, newline: Newline, words: Option<usize>) -> String {
    let newline = match newline {
        Newline::Lf => "LF",
        Newline::CrLf => "CRLF",
        Newline::Mixed => "Mixed",
    };
    let mut text = format!("{encoding}{FIELD}{newline}");
    if let Some(words) = words {
        let plural = if words == 1 { "" } else { "s" };
        let _ = write!(text, "{FIELD}{words} word{plural}");
    }
    text
}

/// How many words a buffer has.
///
/// Whitespace-separated runs, which is what every editor means by it and what nobody can define
/// precisely. Deliberately not clever about Markdown: `**bold**` is one word, a table row is as many
/// words as it has cells' worth of text, and a fence's contents count. Anything else needs a rule
/// for every construct and would still disagree with whatever the user's word processor says.
///
/// **Not to be called on the keystroke path.** It walks the buffer; that is the whole reason the
/// count is kept on a timer.
#[must_use]
pub fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}
