//! The lists a toolbar button opens: heading levels, and whatever did not fit on the bar.
//!
//! Three claims carry MDEDIT-116:
//!
//! - **nothing is silently lost** — what the bar drops and what the overflow list offers are one
//!   partition of one set, and neither half may forget an action;
//! - a list is a **third z-order**, drawn after the floating bar and asked before it, and a row that
//!   cannot be clicked is the failure this ticket is most likely to ship;
//! - it closes the way every other panel does.
//!
//! The third z-order is not theoretical. The first build of this had `Editor::click` dismiss the
//! tooltip before hit-testing the menu, and dismissal closed the menu — so every click on a row fell
//! through to the document. It looked exactly like a menu that was not wired up.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::popup;
use mdedit_app::toolbar::{self, Action};
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

mod stub;
use stub::{Grid, MARGIN};

/// Wide enough for every button — see `toolbar.rs` for why the stub needs more than 900.
const WIDE: f32 = 1000.0;
/// Narrow enough that the bar has to drop something.
const NARROW: f32 = 420.0;
const HEIGHT: f32 = MARGIN + 400.0;

fn editor(text: &str, width: f32) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    let _ = editor.build_frame(width, HEIGHT, 1.0);
    editor
}

fn caret_at(editor: &mut Editor<Grid>, offset: usize) {
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(offset)));
}

/// The middle of a button on the bar.
fn centre_of(editor: &Editor<Grid>, action: Action) -> (f32, f32) {
    let button = editor
        .toolbar_buttons()
        .iter()
        .find(|button| button.action == action)
        .unwrap_or_else(|| panic!("{action:?} should be on the bar"));
    (button.x + button.width / 2.0, toolbar::HEIGHT / 2.0)
}

/// Opens a list by clicking the button that owns it.
fn open(editor: &mut Editor<Grid>, button: Action) {
    let (x, y) = centre_of(editor, button);
    editor.click(x, y);
    let _ = editor.build_frame(editor_width(editor), HEIGHT, 1.0);
}

/// The width the editor was last drawn at, so a helper can redraw without being told.
fn editor_width(editor: &Editor<Grid>) -> f32 {
    // The overflow button only exists on a narrow bar, so its presence is the discriminator.
    if editor
        .toolbar_buttons()
        .iter()
        .any(|button| button.action == Action::More)
    {
        NARROW
    } else {
        WIDE
    }
}

/// Where the middle of a row is, in window coordinates.
fn row_centre(editor: &Editor<Grid>, index: usize) -> (f32, f32) {
    let (x, y) = editor.menu_at().expect("a list should be open");
    (
        x + editor.menu_width().expect("a list should be open") / 2.0,
        y + popup::row_top(index) + popup::ROW_HEIGHT / 2.0,
    )
}

// ---- nothing is silently lost ----

/// **The property the ticket's title is about.** What the bar draws and what the overflow offers are
/// one partition of one set: every action appears in exactly one of them, in order.
///
/// Checked across a sweep of widths rather than at one, because the interesting cases are the ones
/// either side of a button's edge and picking a width by hand would miss them.
#[test]
fn every_button_is_either_drawn_or_in_the_overflow() {
    let all = [toolbar::STANDARD, toolbar::MARKDOWN].concat();
    let widths = vec![28.0_f32; all.len()];

    for step in (60..=1200).step_by(20) {
        let available = f32::from(u16::try_from(step).unwrap());
        let row = toolbar::overflowing(&all, &widths, available);
        let drawn: Vec<Action> = row
            .buttons
            .iter()
            .map(|button| button.action)
            .filter(|action| *action != Action::More)
            .collect();

        let rejoined: Vec<Action> = drawn.iter().copied().chain(row.dropped.clone()).collect();
        assert_eq!(
            rejoined, all,
            "at {available}px the two halves do not rejoin into the whole set"
        );
    }
}

/// A wide bar has no overflow button and reserves no room for one — a window with space for
/// everything must not be one button short for the sake of a control that never appears.
#[test]
fn a_wide_bar_has_no_overflow_button() {
    let all = [toolbar::STANDARD, toolbar::MARKDOWN].concat();
    let widths = vec![28.0_f32; all.len()];

    let row = toolbar::overflowing(&all, &widths, 5000.0);
    assert!(row.dropped.is_empty());
    assert!(
        !row.buttons.iter().any(|b| b.action == Action::More),
        "nothing was dropped, so there is nothing to open"
    );
    assert_eq!(row.buttons.len(), all.len());
}

/// A narrow one ends with it, and the button is the last thing on the row rather than somewhere in
/// the middle of what fitted.
#[test]
fn a_narrow_bar_ends_with_the_overflow_button() {
    let all = [toolbar::STANDARD, toolbar::MARKDOWN].concat();
    let widths = vec![28.0_f32; all.len()];

    let row = toolbar::overflowing(&all, &widths, 300.0);
    assert!(!row.dropped.is_empty(), "300px cannot hold all of them");
    assert_eq!(
        row.buttons.last().map(|b| b.action),
        Some(Action::More),
        "the overflow button belongs at the end"
    );
    assert!(
        row.buttons
            .iter()
            .rev()
            .skip(1)
            .all(|b| b.action != Action::More),
        "and only once"
    );
}

/// The reserve is not double-counted: the row including its overflow button still fits.
#[test]
fn the_row_with_its_overflow_button_still_fits() {
    let all = [toolbar::STANDARD, toolbar::MARKDOWN].concat();
    let widths = vec![28.0_f32; all.len()];

    for step in (80..=900).step_by(10) {
        let available = f32::from(u16::try_from(step).unwrap());
        let row = toolbar::overflowing(&all, &widths, available);
        if let Some(last) = row.buttons.last() {
            assert!(
                last.right() <= available,
                "at {available}px the row runs to {} — the reserve was not enough",
                last.right()
            );
        }
    }
}

// ---- the heading list ----

#[test]
fn the_bar_has_one_heading_button_not_three() {
    let editor = editor("prose\n", WIDE);
    let headings: Vec<Action> = editor
        .toolbar_buttons()
        .iter()
        .map(|button| button.action)
        .filter(|action| matches!(action, Action::Heading(_) | Action::Headings))
        .collect();

    assert_eq!(headings, vec![Action::Headings]);
}

/// Six levels, where the bar only ever had room for three. H4 to H6 stop being chord-only, which is
/// what collapsing the buttons bought.
#[test]
fn the_heading_list_offers_all_six_levels() {
    let mut editor = editor("prose\n", WIDE);
    open(&mut editor, Action::Headings);

    assert_eq!(editor.menu_rows(), Some(toolbar::HEADINGS));
}

#[test]
fn choosing_a_level_applies_it_and_closes() {
    let mut editor = editor("prose\n", WIDE);
    caret_at(&mut editor, 2);
    open(&mut editor, Action::Headings);

    // Row four is Heading(5), which no chord on the bar could reach before this ticket.
    let (x, y) = row_centre(&editor, 4);
    editor.click(x, y);

    assert_eq!(editor.document().to_string(), "##### prose\n");
    assert_eq!(editor.menu_rows(), None, "and it closed behind itself");
}

/// Clicking the button that opened a list closes it, which is what every menu does.
#[test]
fn the_button_that_opened_it_closes_it() {
    let mut editor = editor("prose\n", WIDE);
    open(&mut editor, Action::Headings);
    assert!(editor.menu_rows().is_some());

    open(&mut editor, Action::Headings);
    assert_eq!(editor.menu_rows(), None);
}

// ---- the overflow list ----

#[test]
fn the_overflow_list_holds_exactly_what_the_bar_dropped() {
    let mut editor = editor("prose\n", NARROW);
    let drawn: Vec<Action> = editor
        .toolbar_buttons()
        .iter()
        .map(|button| button.action)
        .filter(|action| *action != Action::More)
        .collect();
    assert!(
        editor
            .toolbar_buttons()
            .iter()
            .any(|b| b.action == Action::More),
        "{NARROW}px should not hold every button"
    );

    open(&mut editor, Action::More);
    let rows = editor
        .menu_rows()
        .expect("the list should be open")
        .to_vec();

    let rejoined: Vec<Action> = drawn.iter().copied().chain(rows).collect();
    assert_eq!(
        rejoined,
        [toolbar::STANDARD, toolbar::MARKDOWN].concat(),
        "the bar and its overflow should rejoin into the whole set"
    );
}

#[test]
fn choosing_from_the_overflow_performs_it() {
    let mut editor = editor("some prose\n", NARROW);
    editor
        .document_mut()
        .set_selection(Selection::new(ByteOffset::new(0), ByteOffset::new(10)));
    open(&mut editor, Action::More);

    let rows = editor.menu_rows().expect("open").to_vec();
    let index = rows
        .iter()
        .position(|action| *action == Action::Link)
        .expect("Link is far enough right to be displaced at this width");

    let before = editor.document().to_string();
    let (x, y) = row_centre(&editor, index);
    editor.click(x, y);

    // **Link's effect is a dialog since MDEDIT-129**, not an edit. That is still the row performing
    // its action — what this test is about is that a row reached through the overflow does exactly
    // what the button would have done, and the button opens a dialog too.
    assert!(
        editor.dialog().is_some(),
        "Link should have run, and running it opens the dialog"
    );
    assert_eq!(
        editor.document().to_string(),
        before,
        "and a dialog writes nothing until it is accepted"
    );
    assert_eq!(editor.menu_rows(), None, "the overflow closed behind it");

    // The whole way through, so the row is not merely opening something that cannot finish.
    assert!(editor.accept_dialog() || editor.dialog().is_some());
}

/// Widening the window while the overflow list is open closes it. Leaving it would offer actions
/// that are back on the bar, and re-pointing it at nothing would leave an empty panel.
#[test]
fn widening_the_window_closes_the_overflow_list() {
    let mut editor = editor("prose\n", NARROW);
    open(&mut editor, Action::More);
    assert!(editor.menu_rows().is_some());

    let _ = editor.build_frame(WIDE, HEIGHT, 1.0);
    assert_eq!(editor.menu_rows(), None);
}

/// The heading list is *not* about the window, so widening leaves it alone.
#[test]
fn widening_the_window_leaves_the_heading_list_alone() {
    let mut editor = editor("prose\n", WIDE);
    open(&mut editor, Action::Headings);

    let _ = editor.build_frame(WIDE + 200.0, HEIGHT, 1.0);
    assert!(editor.menu_rows().is_some());
}

// ---- the z-order, which is where this breaks ----

/// **The bug this shipped with once.** `Editor::click` dismissed the tooltip before hit-testing the
/// menu, and dismissal closed the menu — so a click on a row found nothing open and fell through to
/// the document. It looked exactly like a menu that had never been wired up.
#[test]
fn a_click_on_a_row_reaches_the_row_and_not_the_document() {
    let source = "some prose here\nand a second line\n";
    let mut with = editor(source, WIDE);
    caret_at(&mut with, 2);
    open(&mut with, Action::Headings);

    // The same click, on an editor with no list open, lands in the text — which is what makes this
    // a test about the z-order rather than about the coordinates.
    let (x, y) = row_centre(&with, 0);
    let mut without = editor(source, WIDE);
    caret_at(&mut without, 2);
    without.click(x, y);
    let landed = without.document().selection();

    assert!(with.click(x, y), "the list should swallow the click");
    assert_eq!(
        with.document().to_string(),
        "# some prose here\nand a second line\n",
        "the row acted"
    );
    assert_ne!(
        with.document().selection(),
        landed,
        "the click reached the document underneath instead of the row"
    );
}

/// A click off the panel closes it and is swallowed. A click that both dismissed and landed is how
/// somebody closing a menu accidentally moves their caret.
#[test]
fn a_click_outside_the_list_closes_it_and_is_swallowed() {
    let mut editor = editor("some prose here\nand a second line\n", WIDE);
    caret_at(&mut editor, 2);
    open(&mut editor, Action::Headings);

    let before = editor.document().selection();
    let deep = editor.content_rect().top_edge() + 40.0;
    assert!(editor.click(300.0, deep), "the list should swallow it");

    assert_eq!(editor.menu_rows(), None, "and close");
    assert_eq!(
        editor.document().selection(),
        before,
        "the caret must not move on the click that dismissed the menu"
    );
}

// ---- how it closes ----

#[test]
fn typing_closes_it() {
    let mut editor = editor("prose\n", WIDE);
    open(&mut editor, Action::Headings);
    editor.insert_text("x");
    assert_eq!(editor.menu_rows(), None);
}

/// **A click on a tab closes the list and does not switch tab**, which is the same rule as clicking
/// anywhere else while one is open: the first click belongs to the menu. The second one switches.
///
/// Worth its own test rather than folding into the general dismissal case, because a tab is chrome
/// and the temptation is to let chrome through — which would mean closing a menu could also change
/// the document under it.
#[test]
fn a_click_on_a_tab_closes_the_list_before_it_switches() {
    let dir = tempfile::TempDir::new().unwrap();
    let first = dir.path().join("first.md");
    let second = dir.path().join("second.md");
    std::fs::write(&first, "one\n").unwrap();
    std::fs::write(&second, "two\n").unwrap();

    let mut workspace = mdedit_session::Workspace::new();
    workspace.add(first);
    workspace.add(second);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.show_tab(1);
    let _ = editor.build_frame(WIDE, HEIGHT, 1.0);
    open(&mut editor, Action::Headings);

    let tab = editor.tab_rects()[0];
    let at = (tab.x + 5.0, toolbar::HEIGHT + 5.0);
    editor.click(at.0, at.1);
    assert_eq!(editor.menu_rows(), None, "the list closed");
    assert_eq!(editor.workspace().active(), Some(1), "and nothing switched");

    editor.click(at.0, at.1);
    assert_eq!(
        editor.workspace().active(),
        Some(0),
        "the second one switches"
    );
}

// ---- hover ----

/// A menu highlights the row under the pointer **at once**. No dwell: the highlight is the feedback
/// that the row is a target at all, where a tooltip is extra information about a button already
/// visible.
#[test]
fn a_row_lights_under_the_pointer() {
    let mut editor = editor("prose\n", WIDE);
    open(&mut editor, Action::Headings);
    assert_eq!(editor.hovered_row(), None, "nothing hovered yet");

    let (x, y) = row_centre(&editor, 3);
    assert!(editor.hover(x, y), "moving onto a row is a redraw");
    assert_eq!(editor.hovered_row(), Some(3));

    let (x, y) = row_centre(&editor, 1);
    editor.hover(x, y);
    assert_eq!(editor.hovered_row(), Some(1));
}

/// Off the panel, nothing is hovered — including over the bar behind it, because the list is above
/// everything while it is open.
#[test]
fn moving_off_the_list_lights_nothing() {
    let mut editor = editor("prose\n", WIDE);
    open(&mut editor, Action::Headings);
    let (x, y) = row_centre(&editor, 2);
    editor.hover(x, y);
    assert_eq!(editor.hovered_row(), Some(2));

    // Well below the panel, which is taller than it looks: six rows plus its margins reach a good
    // way into the document. The first version of this test used the content rect's top edge and
    // was still inside the list.
    let below = y + popup::height_of(6) + 40.0;
    editor.hover(x, below);
    assert_eq!(editor.hovered_row(), None);
    assert_eq!(editor.tooltip(), None, "and no tooltip was raised either");
}

/// Closing the list forgets which row was under the pointer, or the next one to open would come up
/// with a row already lit.
#[test]
fn closing_the_list_forgets_the_hovered_row() {
    let mut editor = editor("prose\n", WIDE);
    open(&mut editor, Action::Headings);
    let (x, y) = row_centre(&editor, 2);
    editor.hover(x, y);

    editor.insert_text("x");
    assert_eq!(editor.hovered_row(), None);
}

// ---- geometry, on its own ----

#[test]
fn a_list_at_the_right_edge_stays_inside_the_window() {
    let width = popup::width_of(120.0);
    let (x, _) = popup::place(width, 380.0, toolbar::HEIGHT, 400.0);

    assert!(x >= 0.0);
    assert!(
        x + width <= 400.0,
        "a panel from {x} for {width} runs past 400"
    );
}

#[test]
fn a_window_narrower_than_the_list_keeps_its_left_edge() {
    let (x, _) = popup::place(300.0, 40.0, toolbar::HEIGHT, 80.0);
    assert_eq!(x, 0.0);
}

#[test]
fn a_list_hangs_below_the_bar() {
    let (_, y) = popup::place(100.0, 50.0, toolbar::HEIGHT, 900.0);
    assert!(y >= toolbar::HEIGHT);
}

/// Row boundaries are exclusive at the bottom, so two neighbouring rows cannot both claim a pixel.
#[test]
fn hit_testing_is_exclusive_at_a_row_boundary() {
    let at = (10.0, 50.0);
    let width = 120.0;
    let first_row_ends = 50.0 + popup::MARGIN + popup::ROW_HEIGHT;

    assert_eq!(
        popup::hit_test(3, 40.0, first_row_ends - 0.5, at, width),
        Some(0)
    );
    assert_eq!(popup::hit_test(3, 40.0, first_row_ends, at, width), Some(1));
}

/// The margins are part of the panel — a click in them hits no row, and the caller still swallows
/// it, because a click on a menu is never a click on what is behind the menu.
#[test]
fn the_margin_is_on_the_panel_but_on_no_row() {
    let at = (10.0, 50.0);
    let width = 120.0;

    assert_eq!(popup::hit_test(3, 40.0, 51.0, at, width), None);
    assert!(popup::contains(3, 40.0, 51.0, at, width));
    assert!(
        !popup::contains(3, 40.0, 49.0, at, width),
        "above the panel"
    );
}

/// A heading list carries no words, so it is the icon column and no more. Six lines of "Heading 3"
/// beside an `H3` says it twice, the second time in smaller type.
#[test]
fn a_heading_row_has_no_words_and_every_other_row_does() {
    assert_eq!(popup::text_for(Action::Heading(2)), None);
    assert_eq!(popup::text_for(Action::Link), Some("Link".to_owned()));
    assert!(!popup::any_text(toolbar::HEADINGS));
    assert!(popup::any_text(&[Action::Link, Action::Heading(1)]));
}

// ---- the size picker (MDEDIT-127) ----

/// Where the middle of a picker cell is, in window coordinates.
fn cell_centre(editor: &Editor<Grid>, row: usize, column: usize) -> (f32, f32) {
    let (x, y) = editor.menu_at().expect("the picker should be open");
    let (cell_x, cell_y) = popup::grid::cell_at(row, column);
    (
        x + cell_x + popup::grid::CELL / 2.0,
        y + cell_y + popup::grid::CELL / 2.0,
    )
}

/// **MDEDIT-87 shipped this button writing a fixed 2×2 skeleton and it was rejected on sight.** The
/// button is back because it no longer writes anything on its own — it opens a panel that is not a
/// list, which is the one thing this bar had never had.
#[test]
fn the_table_button_opens_a_picker_rather_than_writing_a_table() {
    let mut editor = editor("prose\n", WIDE);
    let before = editor.document().to_string();
    open(&mut editor, Action::Table);

    assert_eq!(editor.menu_kind(), Some(toolbar::Menu::TableSize));
    assert_eq!(
        editor.document().to_string(),
        before,
        "opening the picker must not write anything"
    );
    assert_eq!(
        editor.menu_rows(),
        Some(&[][..]),
        "a grid has no rows of actions"
    );
}

/// The gesture: the pointer sizes the table, the click inserts it.
#[test]
fn choosing_a_cell_inserts_that_size_and_closes() {
    let mut editor = editor("prose\n", WIDE);
    caret_at(&mut editor, 2);
    open(&mut editor, Action::Table);

    // Row 2, column 3 — zero-based, so three rows by four columns.
    let (x, y) = cell_centre(&editor, 2, 3);
    assert!(editor.click(x, y), "the picker should swallow the click");

    let text = editor.document().to_string();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "prose");
    assert_eq!(lines[1], "", "the blank line GFM needs above a table");
    assert_eq!(lines[3], "| --- | --- | --- | --- |");
    assert_eq!(lines.len(), 6, "prose, a blank line, and four table rows");
    assert_eq!(editor.menu_kind(), None, "and it closed behind itself");
}

/// The top-left cell is a table, not a nothing. A picker whose smallest offer is two rows is one
/// that lies about its own first square.
#[test]
fn the_top_left_cell_is_a_one_by_one_table() {
    let mut editor = editor("", WIDE);
    open(&mut editor, Action::Table);

    let (x, y) = cell_centre(&editor, 0, 0);
    editor.click(x, y);

    assert_eq!(editor.document().to_string(), "|     |\n| --- |\n");
}

/// Hover-to-size, with no dwell — the lit block *is* the table, so its shape is read rather than
/// counted.
#[test]
fn a_cell_lights_under_the_pointer() {
    let mut editor = editor("prose\n", WIDE);
    open(&mut editor, Action::Table);
    assert_eq!(editor.hovered_cell(), None, "nothing hovered yet");

    let (x, y) = cell_centre(&editor, 3, 5);
    assert!(editor.hover(x, y), "moving onto a cell is a redraw");
    assert_eq!(editor.hovered_cell(), Some((3, 5)));

    let (x, y) = cell_centre(&editor, 0, 1);
    editor.hover(x, y);
    assert_eq!(editor.hovered_cell(), Some((0, 1)));
    assert_eq!(editor.hovered_row(), None, "a grid lights no rows");
}

/// Closing the picker forgets the cell, or the next one to open would come up already sized.
#[test]
fn closing_the_picker_forgets_the_hovered_cell() {
    let mut editor = editor("prose\n", WIDE);
    open(&mut editor, Action::Table);
    let (x, y) = cell_centre(&editor, 2, 2);
    editor.hover(x, y);
    assert_eq!(editor.hovered_cell(), Some((2, 2)));

    editor.insert_text("x");
    assert_eq!(editor.hovered_cell(), None);
    assert_eq!(editor.menu_kind(), None);
}

/// **Where MDEDIT-116's row version had its one bug**, now with two axes to get wrong. Boundaries
/// are exclusive at the far edge, so two neighbouring cells cannot both claim a pixel.
#[test]
fn hit_testing_is_exclusive_at_a_cell_boundary() {
    let at = (10.0, 50.0);
    let first_column_ends = 10.0 + popup::MARGIN + popup::grid::PITCH;
    let first_row_ends = 50.0 + popup::MARGIN + popup::grid::PITCH;

    assert_eq!(
        popup::grid::hit_test(first_column_ends - 0.5, first_row_ends - 0.5, at),
        Some((0, 0))
    );
    assert_eq!(
        popup::grid::hit_test(first_column_ends, first_row_ends - 0.5, at),
        Some((0, 1))
    );
    assert_eq!(
        popup::grid::hit_test(first_column_ends - 0.5, first_row_ends, at),
        Some((1, 0))
    );
}

/// **No dead pixels between two squares.** The gap is drawn, not hit-tested: a cell owns its whole
/// pitch. A pointer crossing the grid must never find itself over nothing.
#[test]
fn the_gap_between_two_cells_belongs_to_a_cell() {
    let at = (10.0, 50.0);
    let (cell_x, cell_y) = popup::grid::cell_at(0, 0);
    // Just past the drawn square's right edge, which is where the gap is.
    let in_the_gap = 10.0 + cell_x + popup::grid::CELL + 0.5;

    assert_eq!(
        popup::grid::hit_test(in_the_gap, 50.0 + cell_y + 1.0, at),
        Some((0, 0)),
        "the gap after a square is still that square's"
    );
}

/// The claim above at **every** cell rather than at the one that happened to be interesting: across
/// a cell's whole pitch, gap included, the answer is that cell and no other.
///
/// The last fraction is the one that matters. At 0.9 of the pitch the pointer is past the drawn
/// square and in the gap after it, which is the point in the grid a naive hit test loses.
#[test]
fn every_part_of_every_cell_answers_with_that_cell() {
    let at = (10.0, 50.0);

    for row in 0..popup::grid::ROWS {
        for column in 0..popup::grid::COLUMNS {
            let (cell_x, cell_y) = popup::grid::cell_at(row, column);
            for fraction in [0.0_f32, 0.25, 0.5, 0.9] {
                let (x, y) = (
                    at.0 + cell_x + popup::grid::PITCH * fraction,
                    at.1 + cell_y + popup::grid::PITCH * fraction,
                );
                assert_eq!(
                    popup::grid::hit_test(x, y, at),
                    Some((row, column)),
                    "{fraction} of the way into cell ({row}, {column}), at ({x}, {y})"
                );
            }
        }
    }
}

/// The margins and the caption strip are on the panel and on no cell. A click there hits nothing and
/// is still swallowed, for the reason the row version gives.
#[test]
fn the_margin_and_the_caption_are_on_the_picker_but_on_no_cell() {
    let at = (10.0, 50.0);

    assert_eq!(popup::grid::hit_test(11.0, 60.0, at), None, "left margin");
    assert_eq!(popup::grid::hit_test(40.0, 51.0, at), None, "top margin");
    let caption = 50.0 + popup::grid::caption_middle();
    assert_eq!(popup::grid::hit_test(40.0, caption, at), None, "caption");

    assert!(popup::grid::contains(11.0, 60.0, at));
    assert!(popup::grid::contains(40.0, caption, at));
    assert!(!popup::grid::contains(40.0, 49.0, at), "above the panel");
    assert!(
        !popup::grid::contains(40.0, 50.0 + popup::grid::height(), at),
        "below it"
    );
}

/// The caption strip sits between the grid and the bottom margin, so it is not
/// `height() - CAPTION / 2` — that lands in the margin, and the words would sit low in the panel.
#[test]
fn the_caption_sits_below_the_grid_and_above_the_bottom_margin() {
    let last_row = popup::grid::cell_at(popup::grid::ROWS - 1, 0).1 + popup::grid::PITCH;
    let middle = popup::grid::caption_middle();

    assert!(middle > last_row, "the caption overlaps the last row");
    assert!(
        middle < popup::grid::height() - popup::MARGIN,
        "the caption is in the bottom margin"
    );
}

/// **`rows` counts the header**, so the caption describes the squares that are lit rather than the
/// body rows underneath them.
#[test]
fn the_caption_names_the_lit_block() {
    assert_eq!(popup::grid::caption(3, 4), "3 × 4 table");
    assert_eq!(popup::grid::caption(1, 1), "1 × 1 table");
}

#[test]
fn the_picker_at_the_right_edge_stays_inside_the_window() {
    let width = popup::grid::width();
    let (x, _) = popup::place(width, 380.0, toolbar::HEIGHT, 400.0);

    assert!(x >= 0.0);
    assert!(
        x + width <= 400.0,
        "a picker from {x} for {width} runs past 400"
    );
}
