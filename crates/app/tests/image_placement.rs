//! Which paragraphs are a picture, and how big the picture is drawn (MDEDIT-146).
//!
//! Placement only — no file is read and nothing is decoded here. What a destination is *allowed* to
//! name belongs to `mdedit-image`, which has its own tests for it; this is about finding the image
//! in the document and sizing the rectangle it gets.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::image::{fit, lone};
use mdedit_document::Document;
use mdedit_offsets::{ByteOffset, ByteSpan};

/// The whole document as one chunk, which is what `blocks::split` gives a lone paragraph.
fn whole(source: &str) -> ByteSpan {
    ByteSpan::new(ByteOffset::new(0), ByteOffset::new(source.len()))
}

/// The destination `lone` found, as text.
fn destination_of(source: &str) -> Option<String> {
    let found = lone(&Document::from(source), whole(source), source)?;
    Some(found.destination.slice(source).to_owned())
}

// ---------------------------------------------------------------------------------------------
// Finding the image.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_paragraph_that_is_only_an_image_is_one() {
    assert_eq!(destination_of("![a cat](cat.png)"), Some("cat.png".into()));
}

#[test]
fn a_trailing_newline_does_not_stop_it_being_one() {
    // `blocks::split` gives a chunk the blank line after its block, so this is the shape the
    // renderer actually passes in — the case that matters more than the one above.
    assert_eq!(
        destination_of("![a cat](cat.png)\n\n"),
        Some("cat.png".into())
    );
}

#[test]
fn an_image_with_no_alt_text_is_still_one() {
    assert_eq!(destination_of("![](cat.png)"), Some("cat.png".into()));
}

#[test]
fn the_span_covers_the_whole_construct_and_no_more() {
    let source = "![a cat](cat.png)\n";
    let found = lone(&Document::from(source), whole(source), source).unwrap();

    assert_eq!(found.span.slice(source), "![a cat](cat.png)");
}

#[test]
fn a_sentence_with_an_image_in_it_is_not_a_picture() {
    // The whole point of "lone". A picture here would swallow the words either side of it.
    assert_eq!(destination_of("Look at ![a cat](cat.png) there."), None);
}

#[test]
fn one_word_beside_the_image_is_enough_to_stop_it() {
    assert_eq!(destination_of("![a cat](cat.png) x"), None);
    assert_eq!(destination_of("x ![a cat](cat.png)"), None);
}

#[test]
fn two_images_in_one_paragraph_are_not_a_picture() {
    assert_eq!(destination_of("![one](a.png)![two](b.png)"), None);
}

#[test]
fn two_images_with_no_alt_text_are_not_a_picture() {
    // **The case no amount of looking at segment shapes catches.** Adjacent markers of the same
    // kind coalesce, so this pair arrives as *one* segment that begins `![` and ends `)` — exactly
    // what a single image with no alt text looks like. Before the `![` count went in, this drew the
    // second picture and swallowed the first without a word.
    assert_eq!(destination_of("![](a.png)![](b.png)"), None);
}

#[test]
fn one_image_with_alt_text_and_one_without_is_not_a_picture() {
    assert_eq!(destination_of("![a](a.png)![](b.png)"), None);
    assert_eq!(destination_of("![](a.png)![b](b.png)"), None);
}

#[test]
fn a_link_wrapping_an_image_is_not_a_picture() {
    // Out of scope rather than half-handled: the link's own brackets are non-whitespace, so it falls
    // through to drawing its alt text. If this ever starts returning a destination, the picture is
    // being drawn with the link silently discarded.
    assert_eq!(
        destination_of("[![a cat](cat.png)](https://example.com)"),
        None
    );
}

#[test]
fn a_paragraph_with_no_image_at_all_is_not_a_picture() {
    assert_eq!(destination_of("Just some ordinary prose."), None);
}

#[test]
fn a_plain_link_is_not_a_picture() {
    assert_eq!(destination_of("[a cat](cat.png)"), None);
}

// ---------------------------------------------------------------------------------------------
// Reading the destination out of the closing marker.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_title_is_not_part_of_the_destination() {
    assert_eq!(
        destination_of("![a cat](cat.png \"a fine cat\")"),
        Some("cat.png".into())
    );
}

#[test]
fn a_destination_in_angle_brackets_keeps_them_for_the_resolver() {
    // Kept rather than stripped here: `mdedit_image::resolve` removes them, and it is the one place
    // that decides what a destination means.
    assert_eq!(
        destination_of("![a cat](<a cat.png>)"),
        Some("<a cat.png>".into())
    );
}

#[test]
fn a_destination_in_angle_brackets_with_a_title_stops_at_the_bracket() {
    assert_eq!(
        destination_of("![a cat](<a cat.png> \"titled\")"),
        Some("<a cat.png>".into())
    );
}

#[test]
fn a_relative_destination_arrives_as_written() {
    assert_eq!(
        destination_of("![](../assets/cat.png)"),
        Some("../assets/cat.png".into())
    );
}

#[test]
fn a_hostile_destination_is_handed_over_unchanged_rather_than_filtered_here() {
    // This module finds destinations; it does not judge them. Doing any of that here would put the
    // security rules in two places, which is how one of them gets forgotten.
    assert_eq!(
        destination_of(r"![](\\evil.example\a\x.png)"),
        Some(r"\\evil.example\a\x.png".into())
    );
}

// ---------------------------------------------------------------------------------------------
// Sizing.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_picture_that_fits_is_drawn_at_its_own_size() {
    assert_eq!(fit((100, 50), 900.0), (100.0, 50.0));
}

#[test]
fn a_picture_narrower_than_the_column_is_never_blown_up() {
    // The asymmetry is the point: a 16-pixel icon stretched to the column width is a worse answer
    // than a 16-pixel icon.
    assert_eq!(fit((16, 16), 900.0), (16.0, 16.0));
}

#[test]
fn a_picture_wider_than_the_column_is_brought_in() {
    assert_eq!(fit((4000, 2000), 900.0), (900.0, 450.0));
}

#[test]
fn scaling_down_keeps_the_aspect_ratio() {
    let (width, height) = fit((1600, 900), 400.0);

    assert_eq!(width, 400.0);
    assert!(
        (height - 225.0).abs() < 0.01,
        "16:9 became {width}x{height}"
    );
}

#[test]
fn a_picture_exactly_the_column_width_is_not_scaled() {
    assert_eq!(fit((900, 300), 900.0), (900.0, 300.0));
}

#[test]
fn a_very_wide_and_flat_picture_still_gets_a_row_to_sit_on() {
    // 8192 x 1 scaled to a 100-pixel column is 0.012 pixels tall. A row of no height is a row the
    // caret cannot be put on, so it is floored rather than allowed to vanish.
    let (_, height) = fit((8192, 1), 100.0);

    assert!(height >= 1.0, "height was {height}");
}

#[test]
fn a_column_of_no_width_does_not_produce_a_nan() {
    let (width, height) = fit((100, 100), 0.0);

    assert!(width.is_finite() && height.is_finite(), "{width}x{height}");
    assert!(width >= 1.0 && height >= 1.0);
}
