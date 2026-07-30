//! Where in the document's headings a position sits.
//!
//! The trail to a caret is what a status bar shows as a breadcrumb, and it is a question about the
//! document rather than about drawing — which is why it is here and not in the renderer.
//!
//! # It is a scan, not a tree
//!
//! `AGENTS.md` is explicit that blocks are extents and not a tree: nothing walks parent links,
//! because nothing has needed to. Headings are no exception, and they are the case where a tree
//! would be least help anyway — Markdown headings do not nest structurally. `### C` after `# A` is a
//! sibling of it in the block list; that it is *under* A is a convention readers apply to the levels,
//! not something the parser records.
//!
//! So the trail is computed the way a reader computes it: walk backwards from the position, and take
//! each heading that is shallower than the shallowest one taken so far.

use mdedit_markers::{Block, BlockKind};
use mdedit_offsets::{ByteOffset, ByteSpan};

/// One step of a heading trail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heading {
    level: u8,
    span: ByteSpan,
}

impl Heading {
    /// The heading's level, 1–6.
    #[must_use]
    pub fn level(self) -> u8 {
        self.level
    }

    /// The bytes of the whole heading block, markers included.
    ///
    /// Markers included because this is the block's own span, and stripping them is the caller's
    /// job — [`Document::heading_text`](crate::Document::heading_text) does it from the segments,
    /// which is the only place that knows which bytes are syntax.
    #[must_use]
    pub fn span(self) -> ByteSpan {
        self.span
    }
}

/// The heading trail enclosing `at`, outermost first.
///
/// Empty when nothing encloses it, which is the honest answer for a position above the document's
/// first heading — a breadcrumb there has nothing to say, and inventing a root for it would be
/// worse than saying nothing.
///
/// A position *inside* a heading counts as being under it: putting the caret in `## Costs` shows
/// that heading as the last step, not the one before it.
///
/// Levels that skip are handled by construction rather than by a case: `# A` then `### C` gives both,
/// because each is shallower than the shallowest taken so far. Nothing here assumes the levels are
/// contiguous, because CommonMark does not.
#[must_use]
pub fn heading_trail(blocks: &[Block], at: ByteOffset) -> Vec<Heading> {
    let mut trail = Vec::new();
    // Nothing has been taken yet, so the first heading found qualifies whatever its level.
    let mut shallowest = u8::MAX;

    // A block always starts at or after the one before it — a parent opens before its children — so
    // the blocks that could enclose `at` are a prefix, and it can be found rather than walked to.
    // The alternative is skipping over every block below the caret on the way back, which on a large
    // document is most of it, on every frame.
    let end = blocks.partition_point(|block| block.span().start() <= at);

    for block in blocks[..end].iter().rev() {
        let BlockKind::Heading(level) = block.kind() else {
            continue;
        };
        if level >= shallowest {
            continue;
        }
        shallowest = level;
        trail.push(Heading {
            level,
            span: block.span(),
        });
        if level == 1 {
            break;
        }
    }

    trail.reverse();
    trail
}
