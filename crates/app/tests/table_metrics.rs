//! Where a table row puts its text, against the real font (MDEDIT-154).
//!
//! **The one thing the grid stub cannot answer.** Every other table test drives glyphs that are
//! eight pixels wide with metrics this file exists to measure — an ascent, a descent, and the gap
//! between the ascent and where capitals actually reach. Windows-only, like `icons.rs`, and for the
//! same reason.
//!
//! # What was wrong, and what the ticket got wrong about it
//!
//! Reported from use: text sits low in its row and a `g` or `j` comes close to the bottom rule.
//! MDEDIT-154 blamed the font's internal leading sitting above the ascent. Measured, there is none:
//!
//! ```text
//! line.baseline      = 17.265625     the ascent
//! height - baseline  =  4.015625     the descent
//!                      21.28125      = line.height, exactly
//! ```
//!
//! `ascent + descent == height`, so the line box has no leading in it and the row was already
//! symmetric about the **font** box. What is lopsided is the font box itself: the ascent reserves
//! 5.27 px above the capitals that Latin text never reaches, while the descent is consumed almost
//! exactly by descenders — 0.02 px spare. Centring the font box therefore puts the *ink* low by half
//! the difference.

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::table;
use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_text::{Layout, StyledRun, TextLayout, TextStyle};
use mdedit_text_dwrite::DirectWriteText;

/// Capitals and descenders together, so the ink spans the whole of what Latin text uses.
const SAMPLE: &str = "Agjy";

fn engine() -> DirectWriteText {
    DirectWriteText::new().expect("DirectWrite is available on any supported Windows")
}

/// One cell of [`SAMPLE`], laid out at `size`.
fn cell(text: &mut DirectWriteText, size: f32) -> Layout {
    let span = ByteSpan::new(ByteOffset::new(0), ByteOffset::new(SAMPLE.len()));
    let run = StyledRun::from_buffer(SAMPLE, TextStyle::body(size), span);
    text.layout(&[run], f32::MAX).expect("the sample lays out")
}

/// How far above and below the baseline this line's glyphs actually put ink.
///
/// Rasterised, because that is the only honest answer: a font's ascent is not where its capitals
/// reach, and the whole defect is the difference between those two.
fn ink(text: &mut DirectWriteText, line: &mdedit_text::LayoutLine) -> (f32, f32) {
    let mut top = f32::MAX;
    let mut bottom = f32::MIN;
    for glyph in &line.glyphs {
        if let Ok(Some(image)) = text.rasterize(glyph.key, 1.0) {
            #[allow(
                clippy::cast_precision_loss,
                reason = "a glyph bitmap is tens of pixels"
            )]
            let (image_top, image_height) = (image.top as f32, image.height as f32);
            top = top.min(image_top);
            bottom = bottom.max(image_top + image_height);
        }
    }
    (top, bottom)
}

/// The space above and below the ink of a one-cell table row, in that order.
fn slack_around_the_ink(size: f32) -> (f32, f32) {
    let mut text = engine();
    let laid_out = cell(&mut text, size);
    let (ink_top, ink_bottom) = ink(&mut text, &laid_out.lines[0]);

    let span = ByteSpan::new(ByteOffset::new(0), ByteOffset::new(SAMPLE.len()));
    let (assembled, _) = table::assemble(&[vec![laid_out]], &[vec![span]], false);
    let row = &assembled.lines[0];

    // `row.baseline` is measured from the top of the assembled block, which for the first row is the
    // top of the row itself.
    let above = row.baseline + ink_top;
    let below = row.height - (row.baseline + ink_bottom);
    (above, below)
}

#[test]
fn a_rows_text_is_centred_on_its_ink() {
    // The defect, as the reader sees it: a descender nearly touching the bottom rule while there is
    // room to spare above the capitals.
    let (above, below) = slack_around_the_ink(16.0);

    assert!(
        (above - below).abs() <= 1.0,
        "text sits {:.2}px off centre — {above:.2}px above the ink, {below:.2}px below",
        (above - below).abs() / 2.0
    );
}

#[test]
fn the_correction_holds_at_another_size() {
    // **A fixed pixel nudge would pass the test above and fail here.** The gap between a font's
    // ascent and its capitals scales with the size, so the correction has to as well.
    let (above, below) = slack_around_the_ink(28.0);

    assert!(
        (above - below).abs() <= 1.75,
        "at 28px the text sits {:.2}px off centre — {above:.2} above, {below:.2} below",
        (above - below).abs() / 2.0
    );
}

#[test]
fn the_ink_still_fits_inside_the_row() {
    // Lifting the text must not push it through the top rule. Stated separately because a large
    // enough correction would balance the two numbers above by clipping.
    let (above, below) = slack_around_the_ink(16.0);

    assert!(above > 0.0, "the ink starts {above:.2}px above the row");
    assert!(below > 0.0, "the ink ends {below:.2}px below the row");
}
