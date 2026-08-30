//! An image that is the whole of its paragraph, and how big it is drawn (MDEDIT-146).
//!
//! Geometry and vocabulary only — no GPU types, no text engine, no window — as [`crate::table`] and
//! [`crate::list`] are, and for the same two reasons: it makes the placement testable without a
//! window, and finding the image needs `&` the document while decoding it needs `&mut` the caches.
//!
//! # A third block that is not a run of lines
//!
//! MDEDIT-126 established the shape. A [`mdedit_text::Layout`] is a `Vec<LayoutLine>` whose glyphs
//! each carry their own position and source span; the linearity is what a *text engine* hands back,
//! not what the type requires. A table lays its cells out apart and places them, and a list
//! (MDEDIT-147) does the same with its items.
//!
//! A picture goes one step further and has **no glyphs at all**. Its chunk lays out as a single
//! empty line whose height is the picture's, so `blocks::stack`, the scrollbar, the anchor and the
//! caret all keep working with no change: an empty line is a shape they already handle, because
//! MDEDIT-81 made a blank page exactly that.
//!
//! # Why the destination is found from segments rather than parsed
//!
//! `view.rs` refused to re-derive where a destination starts, on the grounds that handling angle
//! brackets, titles and reference labels means a second link parser living in the renderer. That
//! holds here too. `mdedit-markers` already reports `![` and `](dest)` as two `Marker::Image`
//! segments with the alt text between them, so [`lone`] reads the boundaries off the segments it is
//! given and classifies bytes within the closing one — the same stance, one level down.
//!
//! **A paragraph is only a picture if it is nothing else.** Any non-whitespace byte outside the
//! construct means it is a sentence with an image in it, which draws its alt text as before. That is
//! also what keeps `[![alt](a.png)](url)` — a link wrapping an image — out of scope rather than
//! half-handled: the link's own brackets are non-whitespace, so it falls through.

use mdedit_document::Document;
use mdedit_language::{Marker, SegmentKind};
use mdedit_offsets::ByteSpan;

/// An image that is the whole of its own paragraph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lone {
    /// The destination as written, ready for `mdedit_image::resolve` — which owns every question
    /// about what it is allowed to name.
    pub destination: ByteSpan,
    /// The whole construct, `![` to `)`. What the caret selects, and what the picture stands for.
    pub span: ByteSpan,
}

/// The image this chunk is entirely made of, if it is made entirely of one.
///
/// `text` is the whole buffer, as segment spans are absolute.
///
/// `None` for a paragraph that merely contains an image, for a paragraph containing two, and for
/// anything wrapped around one. Each of those draws its alt text, which is what the editor did
/// before pictures existed and is never a lie about the document.
#[must_use]
pub fn lone(doc: &Document, chunk: ByteSpan, text: &str) -> Option<Lone> {
    let markers: Vec<ByteSpan> = doc
        .segments()
        .iter()
        .filter(|segment| segment.kind() == SegmentKind::Marker(Marker::Image))
        .map(mdedit_language::Segment::span)
        .filter(|span| span.start() >= chunk.start() && span.end() <= chunk.end())
        .collect();

    // **An image comes back as one segment or as two, and which depends on its alt text.** The
    // locator subtracts a construct's text events from it, so `![alt](cat.png)` arrives as `![` and
    // `](cat.png)` with the alt between them — and `![](cat.png)`, having no text to subtract,
    // arrives whole. Both are one image; neither shape is the general case.
    let span = match markers[..] {
        [whole] if whole.slice(text).starts_with("![") && whole.slice(text).ends_with(')') => whole,
        [opening, closing]
            if opening.slice(text).ends_with('[') && closing.slice(text).starts_with("](") =>
        {
            ByteSpan::new(opening.start(), closing.end())
        }
        _ => return None,
    };

    // **Adjacent markers coalesce, so the shapes above do not tell one image from two.** Segments
    // tile the source, and two segments of the same kind that touch arrive as one — so
    // `![](a.png)![](b.png)` is a *single* marker matching the first arm, and `![a](a.png)![](b.png)`
    // is a pair matching the second. Both looked like one image and both drew the last one, silently
    // swallowing the first. This is the check that actually separates them.
    //
    // A destination or an alt text containing a literal `![` is refused along with them, and draws
    // its alt text. That is the wrong answer for a file called `a![b.png`, and it is the right
    // trade: the failure is a picture that does not appear, against a picture that appears in place
    // of content it is not.
    if span.slice(text).match_indices("![").count() != 1 {
        return None;
    }

    // Nothing else in the paragraph. The blank line `blocks::split` gives a chunk is whitespace and
    // does not count; a single letter beside the image does, and drops the whole thing back to text.
    let before = ByteSpan::new(chunk.start(), span.start());
    let after = ByteSpan::new(span.end(), chunk.end());
    if !before.slice(text).trim().is_empty() || !after.slice(text).trim().is_empty() {
        return None;
    }

    Some(Lone {
        destination: destination_in(span, text),
        span,
    })
}

/// The destination inside a whole `![alt](dest "title")` construct.
///
/// A classification of bytes rather than a parse, for the reason `view::brackets_a_link` gives: the
/// alternative is re-deriving where a destination ends, which means a second link parser in the
/// renderer. Two rules cover what CommonMark allows here — a destination in angle brackets runs to
/// the `>`, and a bare one runs to the first whitespace, which is where a title would begin.
///
/// Found from the **last** `](`, so it works on the whole construct and does not care whether the
/// alt text arrived as its own segment. Alt text cannot contain an unescaped `](`, and a destination
/// that does would have to be bracketed.
///
/// The angle brackets are kept rather than stripped: `mdedit_image::resolve` removes them, and it is
/// the one place that decides what a destination means.
fn destination_in(construct: ByteSpan, text: &str) -> ByteSpan {
    let whole = construct.slice(text);
    let opens = whole.rfind("](").map_or(whole.len(), |at| at + 2);
    // The construct's final `)` is the marker's, not the destination's.
    let body = &whole[opens..whole.len().saturating_sub(1)];

    let length = if body.starts_with('<') {
        body.find('>').map_or(body.len(), |end| end + 1)
    } else {
        body.find(char::is_whitespace).unwrap_or(body.len())
    };

    let start = construct.start().get() + opens;
    ByteSpan::new(
        mdedit_offsets::ByteOffset::new(start),
        mdedit_offsets::ByteOffset::new(start + length),
    )
}

/// How big a picture is drawn, in logical pixels, given the width available to it.
///
/// **Scaled down to fit, never up.** A photograph wider than the window has to be brought in or it
/// is unreadable; a 16-pixel icon blown up to the column width is a worse answer than a 16-pixel
/// icon. That is what every browser and every Markdown preview does, and the asymmetry is the point
/// rather than an omission.
///
/// A picture's pixels are read as **logical** rather than physical, which is also the web's rule: a
/// 100-pixel image is 100 points wide and gets more physical pixels on a denser display, rather than
/// shrinking on one.
#[must_use]
pub fn fit(picture: (u32, u32), column: f32) -> (f32, f32) {
    #[allow(
        clippy::cast_precision_loss,
        reason = "a dimension is capped at 8192 by mdedit_image::MAX_DIMENSION, which f32 holds exactly"
    )]
    let (width, height) = (picture.0 as f32, picture.1 as f32);
    let available = column.max(1.0);
    let scale = (available / width.max(1.0)).min(1.0);
    // Floored at a pixel apiece. A row of no height is a row the caret cannot be put on, and a
    // wide-and-flat picture scaled hard is the case that reaches zero.
    ((width * scale).max(1.0), (height * scale).max(1.0))
}
