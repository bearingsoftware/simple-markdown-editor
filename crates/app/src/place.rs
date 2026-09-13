//! Moving a laid-out row to where an assembled layout wants it.
//!
//! "Place, don't widen" (MDEDIT-126, MDEDIT-147): a [`Layout`] is not what a text engine hands back,
//! it is what this editor assembles, so a block that needs a row somewhere other than where the
//! engine put it lays the text out on its own and moves the rows. Tables, lists and block quotes all
//! do it. These are the two pieces of arithmetic that are the same every time, kept in one place so
//! the subtle half — that underlines move with their text — is written once.

use mdedit_offsets::LayoutPos;
use mdedit_text::{Layout, LayoutLine};

/// `line`, from a separately laid-out unit, moved right by `indent` and down by `origin`.
///
/// **`origin` is where the unit's layout begins in the assembled one — not this row's own top.**
/// A [`LayoutLine`]'s baseline is measured from the top of the layout it came from, so it already
/// includes every row above it in that unit; adding the row's top as well counts them twice. That
/// is what `list::assemble` did from MDEDIT-147 until #166's review measured it: each continuation
/// row of a wrapped item was a row lower than the one before should have put it, and the third
/// drew below the next item, over its text. No test looked at a row's height, only its indent.
///
/// **Rules move with the text they are under.** A link's underline on a moved row would otherwise
/// stay where the engine drew it while its glyphs moved, which only a document with an underlined
/// link in exactly the right place would ever show.
#[must_use]
pub fn row(line: &LayoutLine, indent: f32, origin: f32) -> LayoutLine {
    let baseline = origin + line.baseline;
    LayoutLine {
        baseline,
        height: line.height,
        glyphs: line
            .glyphs
            .iter()
            .map(|glyph| {
                let mut moved = *glyph;
                moved.pos = LayoutPos::new(glyph.pos.x + indent, baseline);
                moved
            })
            .collect(),
        rules: line
            .rules
            .iter()
            .map(|rule| mdedit_text::Rule {
                x: rule.x + indent,
                ..*rule
            })
            .collect(),
    }
}

/// How many of a separately laid-out unit's rows belong in the assembled layout.
///
/// All of them for the last unit. For any other, its trailing empty line is the newline that ends it
/// rather than a row: every unit ends in a newline, the engine gives it a line with no glyphs, and
/// stacked naively that is a blank row between every pair of units. Only the last keeps it, because
/// that one is the blank line after the block, which `blocks::drops_last_line` expects to find.
#[must_use]
pub fn rows_kept(text: &Layout, last: bool) -> usize {
    if last {
        return text.lines.len();
    }
    let trailing = text
        .lines
        .last()
        .is_some_and(|line| line.glyphs.is_empty() && text.lines.len() > 1);
    text.lines.len() - usize::from(trailing)
}
