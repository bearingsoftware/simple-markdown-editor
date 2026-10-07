//! A tab or pane row shows its file's full path on hover (MDEDIT-173).
//!
//! Was MDEDIT-121, which showed the full *name* and only when the name had been truncated — these
//! are its tests, kept and turned round rather than replaced, which is why the history of this file
//! starts under `truncated_names.rs`.
//!
//! **The rule that went.** MDEDIT-121's one rule was that a name which *fits* says nothing, on the
//! ground that a panel repeating what is already on screen is noise. A path is not a repeat: it adds
//! where the file is, and it subsumes the truncated case because a path ends in the name. So the
//! paired tests here — a long name and a short one — now assert the *same* answer for both, and the
//! measurement of what fitted is gone from the editor along with the two lists that carried it.
//!
//! **What takes its place** is having a file at all: an untitled buffer has no path and gets nothing.
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
use mdedit_app::tooltip;
use mdedit_app::view::{self, Chrome};
use mdedit_session::Workspace;
use std::path::PathBuf;
use tempfile::TempDir;

use crate::stub;
use stub::{ADVANCE, Grid, MARGIN};

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
fn a_truncated_tab_says_its_full_path() {
    let names = distinct(LONG, 3);
    let (dir, mut editor) = editor_with(&names);
    let at = on_tab(&editor, 0);
    rest_at(&mut editor, at);

    assert_eq!(editor.tooltip(), Some(Hovered::Tab(0)));
    assert_eq!(
        editor.tooltip_text(),
        Some(dir.path().join(&names[0]).display().to_string()),
        "the whole path, which ends in the name the tab could not fit"
    );
}

/// **The rule MDEDIT-173 retired.** A tab wide enough for its own name used to say nothing; it now
/// says where the file is, which the tab cannot show however wide it is.
#[test]
fn a_tab_that_fits_still_says_its_path() {
    let (dir, mut editor) = editor_with(&[SHORT]);
    let at = on_tab(&editor, 0);
    rest_at(&mut editor, at);

    assert_eq!(editor.tooltip(), Some(Hovered::Tab(0)));
    assert_eq!(
        editor.tooltip_text(),
        Some(dir.path().join(SHORT).display().to_string()),
    );
}

/// **The case the whole ticket is for.** Two files with the same name in different folders are told
/// apart without opening either.
#[test]
fn two_files_with_one_name_give_two_different_tooltips() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    let mut workspace = Workspace::new();
    for dir in [&first, &second] {
        let path = dir.path().join("README.md");
        std::fs::write(&path, "text\n").unwrap();
        workspace.add(path);
    }
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let said: Vec<String> = (0..2)
        .map(|index| {
            let at = on_tab(&editor, index);
            rest_at(&mut editor, at);
            editor.tooltip_text().expect("both tabs have a path")
        })
        .collect();

    assert_ne!(said[0], said[1], "the two paths must differ: {said:?}");
    assert!(
        said.iter().all(|path| path.ends_with("README.md")),
        "and both still end in the name: {said:?}",
    );
}

/// An untitled buffer has no path, so it gets no panel. The alternative — one saying it has not been
/// saved — tells the reader what the tab already says.
#[test]
fn an_untitled_buffer_says_nothing() {
    let mut workspace = Workspace::new();
    workspace.add_untitled();
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let at = on_tab(&editor, 0);
    rest_at(&mut editor, at);
    assert_eq!(editor.tooltip(), None);
    assert_eq!(editor.tooltip_text(), None);
}

/// A file deleted under an open tab still shows the path it was opened from, which is the useful
/// fact when the file is missing — and a hover must not be asking the filesystem anything.
#[test]
fn a_tab_whose_file_has_gone_still_says_where_it_was() {
    let (dir, mut editor) = editor_with(&[SHORT]);
    let path = dir.path().join(SHORT);
    std::fs::remove_file(&path).unwrap();

    let at = on_tab(&editor, 0);
    rest_at(&mut editor, at);
    assert_eq!(
        editor.tooltip_text(),
        Some(path.display().to_string()),
        "the path it was opened from",
    );
}

/// The close button is part of the tab for this purpose, as it was under MDEDIT-121: resting there
/// still names the file rather than going blank on the way to clicking it.
#[test]
fn the_close_button_says_the_same_as_the_rest_of_the_tab() {
    let (dir, mut editor) = editor_with(&distinct(LONG, 3));
    let rect = editor
        .tab_rects()
        .iter()
        .find(|rect| rect.index == 0)
        .copied()
        .expect("the first tab");

    // The middle of the close region, asked of the rect rather than worked out here.
    let (close_x, close_width) = rect.close_region();
    let x = close_x + close_width / 2.0;
    rest_at(&mut editor, (x, toolbar::HEIGHT + tabs::STRIP_HEIGHT / 2.0));
    assert_eq!(editor.tooltip(), Some(Hovered::Tab(0)));
    assert!(
        editor
            .tooltip_text()
            .is_some_and(|said| said.starts_with(&dir.path().display().to_string())),
        "the same path as anywhere else on the tab",
    );
}

/// **A tab is not hoverable until a frame has drawn**, which survives MDEDIT-173 for a different
/// reason than it was written for.
///
/// It used to be that whether a name *fitted* was a measurement only drawing could make. That rule
/// is gone — but where a tab *is* is also something only a drawn frame knows, since the strip's
/// rects come from laying it out. So an editor that has never drawn one has nothing under the
/// pointer, and the panel still cannot appear.
#[test]
fn nothing_is_hoverable_until_a_frame_has_drawn() {
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
fn a_truncated_pane_row_says_its_full_path() {
    let (dir, mut editor) = in_pane(&[LONG]);
    let at = on_row(&editor, 0);
    rest_at(&mut editor, at);

    assert_eq!(editor.tooltip(), Some(Hovered::Row(0)));
    assert_eq!(
        editor.tooltip_text(),
        Some(dir.path().join(LONG).display().to_string()),
    );
}

/// The pane's half of the retired rule: a row whose name fitted now says its path too.
#[test]
fn a_pane_row_that_fits_still_says_its_path() {
    let (dir, mut editor) = in_pane(&[SHORT]);
    let at = on_row(&editor, 0);
    rest_at(&mut editor, at);

    assert_eq!(editor.tooltip(), Some(Hovered::Row(0)));
    assert_eq!(
        editor.tooltip_text(),
        Some(dir.path().join(SHORT).display().to_string()),
    );
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

/// And resting on a tab with **no file behind it** arms nothing, which is the rule showing up in the
/// loop rather than only in the panel: an editor with nothing to say should sleep.
///
/// This was `resting_on_a_tab_that_fits_arms_nothing` until MDEDIT-173. A tab that fits now has
/// something to say — where its file is — so the tab that says nothing is the untitled one.
#[test]
fn resting_on_a_tab_with_no_file_arms_nothing() {
    let mut workspace = Workspace::new();
    workspace.add_untitled();
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let (x, y) = on_tab(&editor, 0);
    editor.hover(x, y);

    assert_eq!(editor.next_wake(), None);
}

/// A tab **with** a file arms it, so the pair above is a pair.
#[test]
fn resting_on_a_tab_with_a_file_arms_the_dwell() {
    let (_dir, mut editor) = editor_with(&[SHORT]);
    let (x, y) = on_tab(&editor, 0);
    editor.hover(x, y);

    assert!(editor.next_wake().is_some());
}

/// **A tab's panel hangs below the strip, centred on the tab** — measured, not merely counted.
/// `cargo-mutants` on #173 showed `tooltip_anchor` could return any point at all and
/// `a_tabs_panel_hangs_below_the_strip` would still see a panel drawn.
#[test]
fn a_tabs_panel_is_placed_under_its_tab() {
    // **Short names, since MDEDIT-173.** The panel holds a path now, and with long names its own
    // width exceeded the room to the tab's left — so `place` clamped it to the window and there was
    // no centring left to assert. The claim here is that the anchor is the tab rather than some
    // arbitrary point, which needs a panel that fits; the clamp has its own rule and its own test.
    // The fifth tab, not the first: a panel this wide cannot be centred on a tab near the left edge,
    // and `place` clamping it there is the behaviour of a different rule.
    let (_dir, mut editor) = editor_with(&distinct(SHORT, 6));
    let before = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let rect = editor.tab_rects()[4];

    let at = on_tab(&editor, 4);
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

// ---- a path too wide for the window ----

/// An absolute path the running platform's own `Path` parses as one: a drive on Windows, `/`
/// elsewhere.
///
/// **The elision is platform-agnostic and is tested on both**, which is why this exists rather than
/// a `#[cfg(windows)]`. A hardcoded `C:\…` is not absolute on Linux — `C:` is read as an ordinary
/// directory with no prefix component — so the head these tests talk about would be empty and
/// `C:/plan.md` would gain an ellipsis it should not have. That is how the first version of these
/// two failed on the Linux runner, and it also took the mutants job down with a red baseline.
fn rooted(parts: &[&str]) -> PathBuf {
    let mut path = PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" });
    path.extend(parts);
    path
}

/// The candidates, in order, with nothing measured: the whole path first, then one interior
/// component dropped at a time, keeping the head and the tail.
///
/// **Both ends are the informative parts** — the drive says which volume, the last components say
/// which folder and which file — so it is the middle that goes.
#[test]
fn the_elisions_drop_the_middle_and_keep_both_ends() {
    let path = rooted(&["Users", "craig", "work", "notes", "plan.md"]);
    let head = rooted(&[]).display().to_string();
    let said = tooltip::elisions(&path);

    assert_eq!(said[0], path.display().to_string(), "the whole path first");
    assert_eq!(
        said.last().map(String::as_str),
        Some(format!("{head}…{}plan.md", std::path::MAIN_SEPARATOR).as_str()),
        "and the shortest keeps the root and the file name",
    );
    for candidate in said.iter().skip(1) {
        assert!(
            candidate.ends_with("plan.md") && candidate.starts_with(&head),
            "every candidate keeps both ends: {candidate:?}",
        );
    }
    // Shrinking is asserted from the *second* candidate on, not the first. Every candidate past
    // that point carries the same single ellipsis and one fewer component, so it is strictly
    // shorter; the step from the whole path to the first elision is not, because `…` can be longer
    // than the component it replaces. `a_one_character_folder_makes_an_elision_longer` is that case.
    for (shorter, longer) in said.iter().skip(2).zip(said.iter().skip(1)) {
        assert!(
            shorter.len() < longer.len(),
            "each elision is shorter than the last: {longer:?} then {shorter:?}",
        );
    }
}

/// **The list is ordered by elisions, not by width**, and this is the case that tells them apart.
///
/// Dropping a one-character folder replaces it with an ellipsis, which is three bytes and — in a
/// proportional font — several ems. So the "shorter" candidate is longer. It costs nothing, because
/// `tooltip_line` measures every candidate and returns the first that *fits* rather than trusting
/// the order; this test exists so that a later reader who adds such an assumption finds it red
/// instead of finding a panel drawn off the window.
#[test]
fn a_one_character_folder_makes_an_elision_longer() {
    let path = rooted(&["a", "Documents and Settings", "plan.md"]);
    let said = tooltip::elisions(&path);

    assert_eq!(said[0], path.display().to_string());
    assert!(
        said[1].len() > said[0].len(),
        "the first elision is longer than the whole path: {:?} then {:?}",
        said[0],
        said[1],
    );
}

/// **The whole path is reported exactly as stored, not normalised** — the same string
/// `Editor::tooltip_text` answers with, so the panel and the accessor cannot disagree about the
/// hover they are both describing.
///
/// `Path::components` drops `.` and doubled separators and re-joins with the platform's own
/// separator, so rebuilding the longest candidate from it would quietly rewrite the path.
/// `session::path::validate` has nothing to say about separators — it checks the device prefix,
/// absoluteness, `..` and is-a-file — and the session file is untrusted input, so a tab really can
/// hold a path spelled this way.
#[test]
#[cfg(windows)]
fn the_whole_path_is_not_quietly_normalised() {
    let path = PathBuf::from("C:/Users/craig//./plan.md");
    assert_eq!(tooltip::elisions(&path)[0], path.display().to_string());
}

/// **A path with nothing in the middle gets no ellipsis.** One candidate, because there is nothing
/// to lose — and an ellipsis would claim there was something there.
#[test]
fn a_path_at_the_root_has_nothing_to_elide() {
    let path = rooted(&["plan.md"]);
    assert_eq!(tooltip::elisions(&path), vec![path.display().to_string()]);
}

/// The drawn panel fits inside the window, which is the whole reason the elisions exist:
/// `tooltip::place` clamps a panel to the edge but cannot shorten one.
#[test]
fn a_deep_path_is_shortened_until_the_panel_fits() {
    // Deep enough that the full path is far wider than the window at the stub's 8 px a character.
    let deep: PathBuf = (0..12).fold(TempDir::new().unwrap().keep(), |path, index| {
        path.join(format!("a-folder-named-at-some-length-{index}"))
    });
    std::fs::create_dir_all(&deep).unwrap();
    let path = deep.join(SHORT);
    std::fs::write(&path, "text\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(path.clone());
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let full = path.display().to_string();
    #[allow(
        clippy::cast_precision_loss,
        reason = "a path a few hundred bytes long"
    )]
    let full_width = full.chars().count() as f32 * ADVANCE;
    assert!(
        full_width > WIDTH,
        "the fixture's path has to be wider than the window: {full_width} against {WIDTH}",
    );

    let at = on_tab(&editor, 0);
    rest_at(&mut editor, at);
    assert_eq!(
        editor.tooltip_text(),
        Some(full),
        "the answer is still the whole path, however it is drawn",
    );

    // What was drawn: every quad of the panel's ground and its text, which must sit inside the
    // window. The ground is the widest of them, so its right edge is the panel's.
    let drawn = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let right = drawn
        .iter()
        .map(|quad| quad.rect[0] + quad.rect[2])
        .fold(f32::MIN, f32::max);
    assert!(
        right <= WIDTH,
        "something was drawn past the window's right edge at {right}",
    );
}

/// How wide the panel drawn for the current hover is, taken from its ground quad.
///
/// The ground is the widest solid quad the hover added, and its width is `tooltip::width_of` of
/// whatever string was chosen — which is how these tests read a decision made inside the renderer
/// without an accessor for it.
fn panel_width(editor: &mut Editor<Grid>, at: (f32, f32)) -> f32 {
    // Diffed against the frame before the hover, because the widest solid quad in any frame is a
    // band of chrome the width of the window — which is what the first version of this measured.
    let before = editor.build_frame(WIDTH, HEIGHT, 1.0);
    rest_at(editor, at);
    let after = editor.build_frame(WIDTH, HEIGHT, 1.0);
    // **The ground, by its colour, not the widest quad the hover added.** The panel also draws a
    // hairline edge a pixel proud on each side, so the widest is the border and is two pixels out —
    // which is exactly the sort of off-by-a-constant that makes a width assertion meaningless.
    let ground = view::Theme::default().panel;
    let wanted = [ground.r, ground.g, ground.b, ground.a];
    let width = after
        .iter()
        .filter(|quad| quad.colour == wanted && !before.contains(quad))
        .map(|quad| quad.rect[2])
        .fold(f32::MIN, f32::max);
    assert!(width.is_finite(), "the hover drew no panel at all");
    width
}

/// The width the panel would be for `text` at the stub's metrics.
#[allow(
    clippy::cast_precision_loss,
    reason = "a path a few hundred bytes long"
)]
fn width_for(text: &str) -> f32 {
    mdedit_app::tooltip::width_of(text.chars().count() as f32 * ADVANCE)
}

/// **As much of the path as fits, not as little as possible.** A path that fits is drawn whole.
///
/// Found by hand-mutation: making the renderer always take the *shortest* elision passed every other
/// test here, because they only ever asserted that the panel fitted and that `tooltip_text` was the
/// full path. Nothing said the panel showed what it had room for.
#[test]
fn a_path_that_fits_is_drawn_whole() {
    let (dir, mut editor) = editor_with(&[SHORT]);
    let full = dir.path().join(SHORT).display().to_string();
    assert!(
        width_for(&full) <= WIDTH,
        "the fixture's path has to fit, or this asserts the wrong thing",
    );

    let at = on_tab(&editor, 0);
    assert_eq!(
        panel_width(&mut editor, at),
        width_for(&full),
        "the panel is not as wide as the whole path: it was elided when it did not need to be",
    );
}

/// And a path that does not fit keeps as much as it can: the panel is wider than the most-elided
/// candidate, which is what distinguishes "shortened until it fits" from "shortened to nothing".
#[test]
fn a_deep_path_keeps_as_much_as_fits() {
    let deep: PathBuf = (0..12).fold(TempDir::new().unwrap().keep(), |path, index| {
        path.join(format!("a-folder-named-at-some-length-{index}"))
    });
    std::fs::create_dir_all(&deep).unwrap();
    let path = deep.join(SHORT);
    std::fs::write(&path, "text\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(path.clone());
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let candidates = tooltip::elisions(&path);
    let shortest = candidates.last().expect("a deep path has elisions");
    assert!(
        width_for(shortest) < WIDTH,
        "the shortest candidate must fit, or there is nothing to choose between",
    );

    let at = on_tab(&editor, 0);
    let drawn = panel_width(&mut editor, at);
    assert!(
        drawn > width_for(shortest),
        "the panel is {drawn} wide, the most-elided candidate {} — it gave up too early",
        width_for(shortest),
    );
    assert!(drawn <= WIDTH, "and it still has to fit: {drawn}");
}
