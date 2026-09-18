//! Tabs that do not fit the strip are still reachable from it (MDEDIT-179).
//!
//! Before this, `tabs::layout` stopped at the edge and said nothing about what it left off: a tab
//! past the edge could not be reached from the strip at all, and because the strip always started at
//! tab zero, the *active* tab could be one of the missing ones. The answer is the formatting bar's
//! (MDEDIT-116): an overflow button listing the rest, in `popup`'s list — plus keeping the active
//! tab in view, which the bar has no equivalent of.
//!
//! The geometry is asserted without a window, and the button and its list through the editor.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::cell::Cell;

use mdedit_app::editor::Editor;
use mdedit_app::file_icon::Icon;
use mdedit_app::popup;
use mdedit_app::tabs::{self, Hit, STRIP_HEIGHT};
use mdedit_app::toolbar;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 400.0;

// ---------------------------------------------------------------------------
// Geometry, without a window.
// ---------------------------------------------------------------------------

/// Every name the same width, so the arithmetic in a failure message is easy to follow.
fn same(_: usize) -> f32 {
    80.0
}

/// Everything fits: no button, nothing hidden, and the tabs are exactly what `layout` gives.
#[test]
fn a_strip_that_fits_has_no_overflow() {
    let strip = tabs::overflowing(3, Some(2), 1000.0, same);
    assert!(strip.hidden.is_empty());
    assert_eq!(strip.more, None);
    assert_eq!(strip.tabs, tabs::layout(&[80.0; 3], 1000.0));
}

/// **Every tab is either drawn or listed, never both and never neither.** The two halves come from
/// one call for exactly this reason — the toolbar's `every_button_is_either_drawn_or_in_the_overflow`
/// is the same rule for the same partition.
#[test]
fn every_tab_is_either_drawn_or_listed() {
    for count in [1, 2, 7, 30] {
        for active in [None, Some(0), Some(count / 2), Some(count - 1)] {
            for width in [50.0_f32, 300.0, 700.0, 5000.0] {
                let strip = tabs::overflowing(count, active, width, same);
                let mut seen: Vec<usize> = strip.tabs.iter().map(|rect| rect.index).collect();
                seen.extend(&strip.hidden);
                seen.sort_unstable();
                assert_eq!(
                    seen,
                    (0..count).collect::<Vec<_>>(),
                    "count {count}, active {active:?}, width {width}",
                );
                assert_eq!(strip.more.is_some(), !strip.hidden.is_empty());
            }
        }
    }
}

/// **The active tab is always drawn**, however far along it is. This is the half of the ticket
/// that was a defect rather than a design choice: switching to a tab past the edge used to light
/// nothing on the strip.
#[test]
fn the_active_tab_is_always_drawn() {
    for active in 0..30 {
        let strip = tabs::overflowing(30, Some(active), 700.0, same);
        assert!(
            strip.tabs.iter().any(|rect| rect.index == active),
            "tab {active} is active and not on the strip: {:?}",
            strip.tabs.iter().map(|rect| rect.index).collect::<Vec<_>>(),
        );
    }
}

/// An active tab that already fits counting from the first does not move the strip. Scrolling it
/// whenever something past the start is active would reshuffle the tabs under the pointer on every
/// click.
#[test]
fn an_active_tab_that_fits_does_not_move_the_strip() {
    let from_the_start = tabs::overflowing(30, None, 700.0, same);
    let last_shown = from_the_start.tabs.last().unwrap().index;
    for active in 0..=last_shown {
        let strip = tabs::overflowing(30, Some(active), 700.0, same);
        assert_eq!(strip.tabs, from_the_start.tabs, "active {active}");
    }
}

/// The drawn tabs are a contiguous run in order, abutting from the left edge, with the button after
/// the last one and nothing past the strip's right edge.
#[test]
fn an_overflowing_strip_is_a_run_of_whole_tabs_then_the_button() {
    for active in [None, Some(0), Some(15), Some(29)] {
        let strip = tabs::overflowing(30, active, 700.0, same);
        assert_eq!(strip.tabs[0].x, 0.0);
        for pair in strip.tabs.windows(2) {
            assert_eq!(pair[0].index + 1, pair[1].index, "in order, none skipped");
            assert_eq!(pair[0].right(), pair[1].x, "abutting");
        }
        let more = strip.more.expect("thirty tabs overflow 700 px");
        assert_eq!(more, strip.tabs.last().unwrap().right());
        assert!(
            more + tabs::MORE_WIDTH <= 700.0,
            "the button ends at {} in a 700 px strip",
            more + tabs::MORE_WIDTH,
        );
    }
}

/// A strip too narrow for even one tab still draws one, and it is the active one — better a
/// clipped tab than an empty strip that looks broken.
#[test]
fn a_strip_narrower_than_one_tab_still_draws_the_active_one() {
    let strip = tabs::overflowing(5, Some(3), 20.0, same);
    assert_eq!(strip.tabs.len(), 1);
    assert_eq!(strip.tabs[0].index, 3);
    assert_eq!(strip.hidden, vec![0, 1, 2, 4]);
}

/// **Only what might be drawn is measured.** Two hundred tabs in a strip with room for four must
/// not shape two hundred names every frame — the rule `layout` documents and the reason the width
/// is a closure rather than a slice. The bound is the run from the start, which finds whether it
/// overflows at all, and the run around the active tab: each what fits plus the one past the edge.
#[test]
fn only_tabs_that_might_be_drawn_are_measured() {
    for active in [None, Some(0), Some(100), Some(199)] {
        let calls = Cell::new(0);
        let strip = tabs::overflowing(200, active, 700.0, |_| {
            calls.set(calls.get() + 1);
            80.0
        });
        assert!(
            calls.get() <= (strip.tabs.len() + 1) * 2 + 1,
            "{} names measured to draw {} tabs, active {active:?}",
            calls.get(),
            strip.tabs.len(),
        );
    }
}

/// The button is its own target, and the tabs and their close regions are unchanged beside it.
#[test]
fn the_button_is_hit_and_the_tabs_still_are() {
    let strip = tabs::overflowing(30, Some(0), 700.0, same);
    let more = strip.more.unwrap();
    let middle = STRIP_HEIGHT / 2.0;

    assert_eq!(
        strip.hit_test(more + tabs::MORE_WIDTH / 2.0, middle),
        Some(Hit::More)
    );
    assert_eq!(strip.hit_test(more + tabs::MORE_WIDTH + 1.0, middle), None);
    assert_eq!(
        strip.hit_test(more + 1.0, STRIP_HEIGHT),
        None,
        "below the strip"
    );

    let first = strip.tabs[0];
    assert_eq!(strip.hit_test(first.x + 5.0, middle), Some(Hit::Select(0)));
    let (close_x, close_width) = first.close_region();
    assert_eq!(
        strip.hit_test(close_x + close_width / 2.0, middle),
        Some(Hit::Close(0))
    );
}

// ---------------------------------------------------------------------------
// The button and its list, through the editor.
// ---------------------------------------------------------------------------

/// These files open on the strip, the first active, and one frame drawn at [`WIDTH`].
fn editor_with(dir: &TempDir, names: &[String]) -> Editor<Grid> {
    let mut workspace = Workspace::new();
    for name in names {
        let path = dir.path().join(name);
        std::fs::write(&path, "text\n").unwrap();
        workspace.add(path);
    }
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// Which of [`twelve`] is plain text, so its list row can be told from its neighbours by its badge.
const PLAIN: usize = 6;

/// Twelve names of one width, which overflow a 900 px strip at six.
fn twelve() -> Vec<String> {
    (0..12)
        .map(|n| {
            let extension = if n == PLAIN { "txt" } else { "md" };
            format!("file-{n:02}.{extension}")
        })
        .collect()
}

/// Where the strip's overflow button is, in window coordinates.
fn on_more(editor: &Editor<Grid>) -> (f32, f32) {
    let more = editor.strip().more.expect("the strip overflows");
    (
        more + tabs::MORE_WIDTH / 2.0,
        toolbar::HEIGHT + STRIP_HEIGHT / 2.0,
    )
}

/// **The button opens a list of exactly the tabs that did not fit**, under the button.
#[test]
fn the_button_lists_the_hidden_tabs() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with(&dir, &twelve());
    let hidden = editor.strip().hidden.clone();
    assert!(!hidden.is_empty(), "twelve tabs overflow {WIDTH} px");

    let (x, y) = on_more(&editor);
    assert!(editor.click(x, y));
    assert_eq!(editor.menu_kind(), Some(toolbar::Menu::Tabs));
    assert_eq!(editor.menu_tabs(), Some(hidden.as_slice()));
    let (_, top) = editor.menu_at().unwrap();
    assert!(
        top >= toolbar::HEIGHT + STRIP_HEIGHT,
        "the list hangs below the strip"
    );
}

/// **Choosing a row shows that tab, and the strip moves to include it.** That is what makes the
/// list a way to the tab rather than a second place to switch from.
#[test]
fn choosing_a_hidden_tab_shows_it_on_the_strip() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with(&dir, &twelve());
    let (x, y) = on_more(&editor);
    editor.click(x, y);
    let chosen = editor.menu_tabs().unwrap()[2];

    let (panel_x, panel_y) = editor.menu_at().unwrap();
    assert!(editor.click(
        panel_x + popup::ICON_COLUMN,
        panel_y + popup::row_top(2) + popup::ROW_HEIGHT / 2.0,
    ));

    assert_eq!(editor.menu_kind(), None, "choosing closes the list");
    assert_eq!(editor.workspace().active(), Some(chosen));
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(
        editor.tab_rects().iter().any(|rect| rect.index == chosen),
        "tab {chosen} is active and should be on the strip",
    );
    assert!(!editor.strip().hidden.contains(&chosen));
}

/// A click anywhere while the list is open belongs to it, the button included: it closes, as every
/// menu does, rather than opening a second one.
#[test]
fn clicking_the_button_again_closes_the_list() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with(&dir, &twelve());
    let (x, y) = on_more(&editor);
    editor.click(x, y);
    assert!(editor.menu_tabs().is_some());
    editor.click(x, y);
    assert_eq!(editor.menu_kind(), None);
}

/// **A list that no longer matches what is hidden closes.** Widened until everything fits, it would
/// otherwise offer tabs that are on the strip beside it.
#[test]
fn widening_the_window_closes_a_stale_list() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with(&dir, &twelve());
    let (x, y) = on_more(&editor);
    editor.click(x, y);
    assert!(editor.menu_tabs().is_some());

    let _ = editor.build_frame(WIDTH * 3.0, HEIGHT, 1.0);
    assert!(editor.strip().hidden.is_empty(), "everything fits now");
    assert_eq!(editor.menu_kind(), None);
}

/// **Each row draws its file's icon**, the same one its tab would. Without the icon font that is
/// its badge, counted by letters so the plain-text row's TXT is told from the Markdown rows' MD.
#[test]
fn each_row_draws_its_files_icon() {
    let dir = TempDir::new().unwrap();
    let names = twelve();
    let mut editor = editor_with(&dir, &names);
    let (x, y) = on_more(&editor);
    editor.click(x, y);
    let listed = editor.menu_tabs().unwrap().to_vec();
    let (panel_x, panel_y) = editor.menu_at().unwrap();
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);

    for (row, index) in listed.iter().enumerate() {
        let top = panel_y + popup::row_top(row);
        let in_icon_column = quads
            .iter()
            .filter(|quad| {
                quad.uv != [0.0, 0.0, 1.0, 1.0]
                    && quad.rect[0] >= panel_x
                    && quad.rect[0] < panel_x + popup::ICON_COLUMN
                    && quad.rect[1] >= top
                    && quad.rect[1] < top + popup::ROW_HEIGHT
            })
            .count();
        let want = if *index == PLAIN {
            Icon::Text
        } else {
            Icon::Note
        };
        assert_eq!(
            in_icon_column,
            want.badge().chars().count(),
            "row {row} ({}) should hold {:?}",
            names[*index],
            want.badge(),
        );
    }
}
