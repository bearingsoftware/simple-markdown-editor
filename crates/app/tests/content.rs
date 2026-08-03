//! Where the document may draw, and the one property that makes clicking trustworthy.
//!
//! The reason this type exists is that drawing and hit-testing used to compute the same offset in
//! nine separate places. Now they share one, and the assertion that matters is that the two
//! directions are exact inverses — because when they are not, text draws in one place and clicks
//! land in another, and nothing fails until somebody notices with a mouse.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::content::{ContentRect, Insets, Offset};

const MARGIN: f32 = 24.0;

fn plain() -> ContentRect {
    ContentRect::new(900.0, 700.0, MARGIN, Insets::default())
}

#[test]
fn a_bare_window_leaves_the_margin_on_every_side() {
    let rect = plain();
    assert_eq!(rect.width(), 900.0 - MARGIN * 2.0, "a margin either side");
    assert_eq!(
        rect.height(),
        700.0 - MARGIN,
        "the top margin only; the bottom one is part of the document's own height"
    );
    assert_eq!(
        rect.to_window(0.0, 0.0, Offset::down(0.0)),
        (MARGIN, MARGIN)
    );
}

#[test]
fn chrome_takes_its_share_before_the_margin_does() {
    let rect = ContentRect::new(
        900.0,
        700.0,
        MARGIN,
        Insets {
            top: 30.0,
            bottom: 20.0,
            left: 200.0,
            right: 10.0,
        },
    );

    assert_eq!(
        rect.to_window(0.0, 0.0, Offset::down(0.0)),
        (200.0 + MARGIN, 30.0 + MARGIN),
        "the text is inset from what the chrome left, not from the window"
    );
    assert_eq!(rect.width(), 900.0 - 200.0 - 10.0 - MARGIN * 2.0);
    assert_eq!(rect.height(), 700.0 - 30.0 - 20.0 - MARGIN);
}

/// The whole point. Every offset added to the drawing is subtracted by the hit test, because they
/// are the same offset.
#[test]
fn the_two_directions_are_exact_inverses() {
    let rects = [
        plain(),
        ContentRect::new(
            900.0,
            700.0,
            MARGIN,
            Insets {
                top: 30.0,
                bottom: 22.0,
                left: 180.0,
                right: 8.0,
            },
        ),
        ContentRect::new(320.0, 240.0, 4.0, Insets::default()),
    ];

    for rect in rects {
        for scroll in [0.0, 1.0, 137.5, 10_000.0] {
            for (x, y) in [(0.0, 0.0), (13.0, 47.0), (-5.0, -9.0), (899.0, 699.0)] {
                let (wx, wy) = rect.to_window(x, y, Offset::down(scroll));
                let (dx, dy) = rect.to_document(wx, wy, Offset::down(scroll));
                assert_eq!((dx, dy), (x, y), "round trip at scroll {scroll}");

                let (bx, by) = rect.to_document(x, y, Offset::down(scroll));
                assert_eq!(
                    rect.to_window(bx, by, Offset::down(scroll)),
                    (x, y),
                    "and the other way"
                );
            }
        }
    }
}

#[test]
fn scrolling_moves_the_document_up_past_the_window() {
    let rect = plain();
    let (_, y) = rect.to_window(0.0, 100.0, Offset::down(40.0));
    assert_eq!(y, MARGIN + 60.0, "sixty pixels of it are left above");
}

/// A window dragged to nothing, or chrome claiming more than there is, must not produce a wrapping
/// width the text engine is entitled to reject.
#[test]
fn a_window_with_no_room_left_still_has_a_usable_rectangle() {
    let rect = ContentRect::new(
        10.0,
        10.0,
        MARGIN,
        Insets {
            top: 500.0,
            bottom: 500.0,
            left: 500.0,
            right: 500.0,
        },
    );
    assert!(rect.width() >= 1.0, "got {}", rect.width());
    assert!(rect.height() >= 1.0, "got {}", rect.height());
}

#[test]
fn a_point_over_the_chrome_is_not_in_the_document() {
    let rect = ContentRect::new(
        900.0,
        700.0,
        MARGIN,
        Insets {
            top: 30.0,
            left: 200.0,
            ..Insets::default()
        },
    );

    assert!(!rect.contains(10.0, 400.0), "in the left pane");
    assert!(!rect.contains(400.0, 10.0), "in the strip");
    assert!(rect.contains(400.0, 400.0), "in the text");
    assert!(!rect.contains(899.0, 400.0), "past the right edge");
}

#[test]
fn the_visible_band_is_the_viewport_in_the_documents_own_coordinates() {
    let rect = plain();
    assert_eq!(rect.visible(Offset::down(0.0)), (0.0, rect.height()));
    assert_eq!(
        rect.visible(Offset::down(500.0)),
        (500.0, 500.0 + rect.height())
    );
}
