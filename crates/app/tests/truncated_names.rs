//! A truncated tab or pane row says its full name on hover (MDEDIT-121).
//!
//! `AGENTS.md` recorded for both of these that they truncate a name and have nowhere to put the full
//! one. MDEDIT-85 built the dwell, the second `ControlFlow` deadline and the panel; this is a hover
//! region and a lookup, and `crate::tooltip` is reused unchanged.
//!
//! **The rule that makes this different from the bar's tooltips** is that a name which *fits* says
//! nothing. Item 1 of MDEDIT-85 was "explain a control", which is true of a button whether or not it
//! is wide; this is "show what did not fit", and whether it did is a measurement the last frame
//! made. So every test here is a pair: a long name and a short one.
//!
//! Timing is driven rather than waited on, as in `tooltip.rs` and for the reason it gives.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::{Editor, Hovered};
use mdedit_app::pane;
use mdedit_app::tabs;
use mdedit_app::toolbar;
use mdedit_app::view::Chrome;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 400.0;

/// A name long enough that no tab or row is ever wide enough for it. The stub's glyphs are 8 px
/// each, so this is over 300 px of text against a tab capped at 200 and a pane row narrower still.
const LONG: &str = "a-really-quite-remarkably-long-file-name.md";
const SHORT: &str = "a.md";

/// `count` distinct names built from `stem`, because **a workspace holds one tab per path** — a
/// fixture that opened the same file six times would open it once, and every `find` below would
/// come back empty.
fn distinct(stem: &str, count: usize) -> Vec<String> {
    let (name, extension) = stem.split_once('.').expect("the fixtures all have one");
    (0..count)
        .map(|index| format!("{name}-{index}.{extension}"))
        .collect()
}

/// An editor with these files open, the last one active, and one frame drawn — the frame matters,
/// because whether a name was truncated is something only drawing it finds out.
fn editor_with<S: AsRef<str>>(names: &[S]) -> (TempDir, Editor<Grid>) {
    let dir = TempDir::new().unwrap();
    let mut workspace = Workspace::new();
    for name in names {
        let path = dir.path().join(name.as_ref());
        std::fs::write(&path, "text\n").unwrap();
        workspace.add(path);
    }
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    (dir, editor)
}

fn in_pane<S: AsRef<str>>(names: &[S]) -> (TempDir, Editor<Grid>) {
    let (dir, mut editor) = editor_with(names);
    editor.toggle_chrome();
    assert_eq!(editor.chrome(), Chrome::Pane);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    (dir, editor)
}

/// The middle of a tab, in window coordinates.
fn on_tab(editor: &Editor<Grid>, index: usize) -> (f32, f32) {
    let rect = editor
        .tab_rects()
        .iter()
        .find(|rect| rect.index == index)
        .unwrap_or_else(|| panic!("tab {index} is on the strip"));
    (
        rect.x + rect.width / 2.0,
        toolbar::HEIGHT + tabs::STRIP_HEIGHT / 2.0,
    )
}

/// The middle of a pane row.
fn on_row(editor: &Editor<Grid>, index: usize) -> (f32, f32) {
    let rect = editor
        .row_rects()
        .iter()
        .find(|rect| rect.index == index)
        .unwrap_or_else(|| panic!("row {index} is in the pane"));
    (
        pane::PANE_WIDTH / 2.0,
        toolbar::HEIGHT + rect.y + pane::ROW_HEIGHT / 2.0,
    )
}

fn rest_at(editor: &mut Editor<Grid>, (x, y): (f32, f32)) {
    editor.hover(x, y);
    editor.on_dwell();
}

// ---- tabs ----

#[test]
fn a_truncated_tab_says_its_full_name() {
    let names = distinct(LONG, 3);
    let (_dir, mut editor) = editor_with(&names);
    let at = on_tab(&editor, 0);
    rest_at(&mut editor, at);

    assert_eq!(editor.tooltip(), Some(Hovered::Tab(0)));
    assert_eq!(
        editor.tooltip_text().as_deref(),
        Some(names[0].as_str()),
        "the whole name, which is the point of it"
    );
}

/// **The rule.** A tab wide enough for its own name has nothing to add, and a panel repeating what
/// is already on screen is noise.
#[test]
fn a_tab_that_fits_says_nothing() {
    let (_dir, mut editor) = editor_with(&[SHORT]);
    let at = on_tab(&editor, 0);
    rest_at(&mut editor, at);

    assert_eq!(editor.tooltip(), None);
    assert_eq!(editor.tooltip_text(), None);
}

/// **Truncation is something a frame finds out, not something a name is.**
///
/// Whether a name fits depends on the width the strip gave that tab and on the glyphs the engine
/// produced for it — neither of which exists until a frame has been drawn. So an editor that has
/// never drawn one knows of no truncated tab, and offers nothing.
///
/// This is the test that fails against the obvious wrong implementation: measuring the name when the
/// tooltip is asked for, which would answer the same before and after.
#[test]
fn nothing_is_truncated_until_a_frame_has_drawn() {
    let dir = TempDir::new().unwrap();
    let mut workspace = Workspace::new();
    for name in distinct(LONG, 3) {
        let path = dir.path().join(&name);
        std::fs::write(&path, "text\n").unwrap();
        workspace.add(path);
    }
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();

    // No frame yet, so no tab rects either — hovering where the strip will be finds nothing at all.
    editor.hover(40.0, toolbar::HEIGHT + tabs::STRIP_HEIGHT / 2.0);
    editor.on_dwell();
    assert_eq!(editor.tooltip(), None, "before the first frame");

    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let at = on_tab(&editor, 0);
    rest_at(&mut editor, at);
    assert_eq!(
        editor.tooltip(),
        Some(Hovered::Tab(0)),
        "and after it, the same name in the same tab"
    );
}

/// It sits below the strip, not under the pointer — [`tooltip::place`]'s rule, and the pointer is on
/// its way somewhere.
#[test]
fn a_tabs_panel_hangs_below_the_strip() {
    let (_dir, mut editor) = editor_with(&distinct(LONG, 3));
    let before = editor.build_frame(WIDTH, HEIGHT, 1.0).len();

    let at = on_tab(&editor, 0);
    rest_at(&mut editor, at);
    let after = editor.build_frame(WIDTH, HEIGHT, 1.0).len();

    assert!(
        after > before,
        "the panel and its text should have been drawn: {before} then {after}"
    );
}

// ---- the pane ----

#[test]
fn a_truncated_pane_row_says_its_full_name() {
    let (_dir, mut editor) = in_pane(&[LONG]);
    let at = on_row(&editor, 0);
    rest_at(&mut editor, at);

    assert_eq!(editor.tooltip(), Some(Hovered::Row(0)));
    assert_eq!(editor.tooltip_text().as_deref(), Some(LONG));
}

#[test]
fn a_pane_row_that_fits_says_nothing() {
    let (_dir, mut editor) = in_pane(&[SHORT]);
    let at = on_row(&editor, 0);
    rest_at(&mut editor, at);

    assert_eq!(editor.tooltip(), None);
}

/// The pane is as tall as the window, so a panel below *it* would point at nothing. It goes below
/// the row.
#[test]
fn a_rows_panel_hangs_below_that_row() {
    let (_dir, mut editor) = in_pane(&[SHORT, LONG]);
    let row = editor
        .row_rects()
        .iter()
        .find(|rect| rect.index == 1)
        .copied()
        .expect("the second row");

    let at = on_row(&editor, 1);
    rest_at(&mut editor, at);
    assert_eq!(editor.tooltip(), Some(Hovered::Row(1)));
    assert!(
        row.y > 0.0,
        "the second row is not at the pane's top, which is what makes this test say anything"
    );
}

// ---- the shared machinery ----

/// Moving off dismisses it, as it does for a button. The panel belongs to what the pointer left.
#[test]
fn moving_off_a_tab_takes_the_panel_away() {
    let (_dir, mut editor) = editor_with(&distinct(LONG, 3));
    let at = on_tab(&editor, 0);
    rest_at(&mut editor, at);
    assert!(editor.tooltip().is_some());

    editor.hover(WIDTH / 2.0, HEIGHT - 4.0);
    assert_eq!(editor.tooltip(), None);
}

/// Crossing the strip shows nothing — the dwell, which is MDEDIT-85's and is reused rather than
/// reimplemented. Without it a row of tabs flickers a panel per tab as the pointer travels it.
#[test]
fn crossing_the_strip_shows_nothing() {
    let (_dir, mut editor) = editor_with(&distinct(LONG, 3));

    for index in 0..3 {
        let (x, y) = on_tab(&editor, index);
        editor.hover(x, y);
    }

    assert_eq!(
        editor.tooltip(),
        None,
        "no dwell elapsed, so nothing should have appeared"
    );
}

/// Resting on a tab arms the dwell, which is what `next_wake` is then asked about — the same
/// deadline machinery the bar uses, reached from a second place.
#[test]
fn resting_on_a_truncated_tab_arms_the_dwell() {
    let (_dir, mut editor) = editor_with(&distinct(LONG, 3));
    let (x, y) = on_tab(&editor, 0);
    editor.hover(x, y);

    assert!(
        editor.next_wake().is_some(),
        "a hover with something to say has to wake the loop, or the panel never appears"
    );
}

/// And resting on one that fits does **not**, which is the rule showing up in the loop rather than
/// only in the panel: an editor whose every tab fits should sleep.
#[test]
fn resting_on_a_tab_that_fits_arms_nothing() {
    let (_dir, mut editor) = editor_with(&[SHORT]);
    let (x, y) = on_tab(&editor, 0);
    editor.hover(x, y);

    assert_eq!(editor.next_wake(), None);
}

/// **A tab's panel hangs below the strip, centred on the tab** — measured, not merely counted.
/// `cargo-mutants` on #173 showed `tooltip_anchor` could return any point at all and
/// `a_tabs_panel_hangs_below_the_strip` would still see a panel drawn.
#[test]
fn a_tabs_panel_is_placed_under_its_tab() {
    let (_dir, mut editor) = editor_with(&distinct(LONG, 3));
    let before = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let rect = editor.tab_rects()[1];

    let at = on_tab(&editor, 1);
    rest_at(&mut editor, at);
    let after = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let panel: Vec<_> = after
        .iter()
        .filter(|quad| quad.uv == [0.0, 0.0, 1.0, 1.0] && !before.contains(quad))
        .collect();
    assert!(!panel.is_empty(), "a panel was drawn");

    let strip_bottom = toolbar::HEIGHT + tabs::STRIP_HEIGHT;
    let left = panel
        .iter()
        .map(|quad| quad.rect[0])
        .fold(f32::MAX, f32::min);
    let right = panel
        .iter()
        .map(|quad| quad.rect[0] + quad.rect[2])
        .fold(f32::MIN, f32::max);
    let top = panel
        .iter()
        .map(|quad| quad.rect[1])
        .fold(f32::MAX, f32::min);
    assert!(
        top >= strip_bottom - 1.0,
        "the panel starts at {top}, above the strip's bottom at {strip_bottom}"
    );
    let centre = rect.x + rect.width / 2.0;
    assert!(
        (f32::midpoint(left, right) - centre).abs() <= 1.0,
        "the panel spans {left}..{right}, centred on {} rather than the tab's {centre}",
        f32::midpoint(left, right),
    );
}
