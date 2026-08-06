//! The formatting bar: what it offers, what a click does, and what it costs per frame.
//!
//! Three claims are worth more than the rest, and they are the three every piece of chrome before
//! this one got wrong at least once:
//!
//! - a click in the bar never reaches the document, **including between two buttons**;
//! - a click below the bar still lands on the byte it would have without it — the drift test, which
//!   is the whole reason `ContentRect` exists;
//! - a still frame lays out no labels, which is the per-frame-layout trap the gutter, the status bar
//!   and the file pane each paid for in turn.
//!
//! The buttons themselves are checked by pressing them and reading the document, rather than by
//! asserting on quads: what a button *does* is the editor method it names, and that method is
//! already tested in `mdedit-document`. What is tested here is that the wiring reaches it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::content::Offset;
use mdedit_app::editor::Editor;
use mdedit_app::toolbar::{self, Action};
use mdedit_app::view::{Appearance, Theme};
use mdedit_document::Selection;
use mdedit_document::format::Inline;
use mdedit_offsets::ByteOffset;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 400.0;

fn editor(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// An editor whose one open file has this extension, so the language resolver sees it.
fn editor_named(dir: &TempDir, name: &str, contents: &str) -> Editor<Grid> {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// Puts the caret at a byte, which is what lighting reads.
fn caret_at(editor: &mut Editor<Grid>, offset: usize) {
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(offset)));
}

/// Selects `[start, end)`, deliberately **not** through `select_all` — which takes the trailing
/// newline with it, so `Ctrl+A` then bold gives `**prose\n**`. That is a real wart in the inline
/// toggles and it belongs to MDEDIT-50; a bar button reaches the same method either way, so these
/// select the words rather than let that wart decide what the toolbar looks like it did.
fn select(editor: &mut Editor<Grid>, start: usize, end: usize) {
    editor
        .document_mut()
        .set_selection(Selection::new(ByteOffset::new(start), ByteOffset::new(end)));
}

/// The middle of the button for `action`, in window coordinates.
fn centre_of(editor: &Editor<Grid>, action: Action) -> (f32, f32) {
    let button = editor
        .toolbar_buttons()
        .iter()
        .find(|button| button.action == action)
        .unwrap_or_else(|| panic!("{action:?} should be on the bar"));
    (button.x + button.width / 2.0, toolbar::HEIGHT / 2.0)
}

// ---- what it offers ----

/// What is on the bar for a Markdown document: the standard set, then Markdown's own.
#[test]
fn a_markdown_document_gets_the_standard_set_and_then_its_own() {
    let editor = editor("# Heading\n\nprose\n");
    let shown: Vec<Action> = editor
        .toolbar_buttons()
        .iter()
        .map(|button| button.action)
        .collect();
    let expected: Vec<Action> = [toolbar::STANDARD, toolbar::MARKDOWN].concat();
    assert_eq!(shown, expected, "at 900px all of them should fit");
}

/// MDEDIT-84's first half, and the one that makes the second half safe.
///
/// JSON used to get **no bar at all**, on the rule that an empty bar is worse than none. That rule
/// held while the bar was below the tab strip; once it is above one, a bar that comes and goes moves
/// the strip on every tab switch. The standard set is what a JSON document gets instead — and the
/// two top edges being equal is the whole claim, because it is the thing that stops moving.
#[test]
fn a_json_document_gets_the_standard_set_and_the_same_top_edge() {
    let dir = TempDir::new().unwrap();
    // Both from a workspace, so both have a tab strip above them and the only difference between
    // the two top edges could be the bar.
    let markdown = editor_named(&dir, "notes.md", "# Heading\n");
    let json = editor_named(&dir, "data.json", "{\"a\": 1}\n");

    let on_json: Vec<Action> = json
        .toolbar_buttons()
        .iter()
        .map(|button| button.action)
        .collect();
    assert_eq!(on_json, toolbar::STANDARD, "the standard set, and no more");
    assert!(
        !on_json
            .iter()
            .any(|action| toolbar::MARKDOWN.contains(action)),
        "and none of Markdown's, which would do nothing to a JSON document"
    );
    assert_eq!(
        markdown.content_rect().top_edge(),
        json.content_rect().top_edge(),
        "both have a bar now, so the document starts in the same place in both"
    );
}

/// Hiding it gives the height back, which is the point of hiding it.
#[test]
fn hiding_the_bar_gives_its_height_back() {
    let mut editor = editor("prose\n");
    let with = editor.content_rect().top_edge();

    editor.toggle_toolbar();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let without = editor.content_rect().top_edge();

    assert!(!editor.toolbar_shown());
    assert!(
        without < with,
        "hidden should reclaim the bar's height: {with} then {without}"
    );
    assert!(editor.toolbar_buttons().is_empty(), "and nothing to click");
}

#[test]
fn the_bar_comes_back() {
    let mut editor = editor("prose\n");
    let shown = editor.content_rect().top_edge();

    editor.toggle_toolbar();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor.toggle_toolbar();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(editor.toolbar_shown());
    assert_eq!(editor.content_rect().top_edge(), shown);
}

/// Narrow enough that the last buttons have nowhere to go. Stopping at the edge is the tab strip's
/// rule, and for the same reason: laying out a button nobody can see is work with no pixel.
///
/// **Which end is cut is the decision worth pinning.** The standard set is leftmost, so a narrow
/// window loses Rule and Link before it loses Save — the standard actions are the ones somebody who
/// does not know the chords is looking for, and every formatting button has a chord and a menu item
/// as well.
#[test]
fn a_narrow_window_cuts_the_language_buttons_first() {
    let mut editor = Editor::new("prose\n", Grid::new());
    let _ = editor.build_frame(200.0, HEIGHT, 1.0);

    let shown: Vec<Action> = editor
        .toolbar_buttons()
        .iter()
        .map(|button| button.action)
        .collect();
    assert!(!shown.is_empty(), "there is room for some of them");
    assert!(
        shown.len() < toolbar::STANDARD.len() + toolbar::MARKDOWN.len(),
        "200px is not enough for all twenty-two"
    );
    assert!(
        !shown
            .iter()
            .any(|action| toolbar::MARKDOWN.contains(action)),
        "the language's buttons go before any of the standard set does: {shown:?}"
    );
    assert!(
        editor.toolbar_buttons().last().unwrap().right() <= 200.0,
        "nothing may hang off the edge"
    );
}

// ---- what a click does ----

#[test]
fn every_markdown_button_does_something() {
    for action in toolbar::MARKDOWN {
        let mut editor = editor("some prose\n");
        select(&mut editor, 0, 10);
        let before = editor.document().to_string();

        let (x, y) = centre_of(&editor, *action);
        assert!(editor.click(x, y), "the bar should swallow the click");

        assert_ne!(
            editor.document().to_string(),
            before,
            "{action:?} should have changed the document"
        );
    }
}

#[test]
fn a_button_does_what_its_chord_does() {
    let cases: &[(Action, &str)] = &[
        (Action::Bold, "**prose**\n"),
        (Action::Italic, "*prose*\n"),
        (Action::Heading(2), "## prose\n"),
        (Action::Bullet, "- prose\n"),
        (Action::Quote, "> prose\n"),
    ];

    for (action, expected) in cases {
        let mut editor = editor("prose\n");
        select(&mut editor, 0, 5);
        let (x, y) = centre_of(&editor, *action);
        editor.click(x, y);
        assert_eq!(&editor.document().to_string(), expected, "{action:?}");
    }
}

/// The rule every piece of chrome carries: a click on it is not a click in the text.
#[test]
fn a_click_in_the_bar_never_reaches_the_document() {
    let mut editor = editor("hello world\n");
    let before = editor.document().selection();

    // Between two buttons, where `hit_test` finds nothing — the case a naive guard would let
    // through, putting the caret in the text under the bar.
    //
    // Found rather than indexed: which pairs have a gap between them is a property of
    // `starts_a_group`, and hard-coding a pair means this test quietly stops testing anything the
    // day a group boundary moves. It moved once already, when MDEDIT-84 put the standard set in
    // front of Bold.
    let buttons = editor.toolbar_buttons();
    let (left, right) = buttons
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .find(|(left, right)| right.x > left.right())
        .expect("the bar has group gaps in it");
    let gap = f32::midpoint(left.right(), right.x);

    assert!(
        editor.click(gap, toolbar::HEIGHT / 2.0),
        "the bar should swallow it"
    );
    assert_eq!(
        editor.document().selection(),
        before,
        "nothing should have moved"
    );
    assert_eq!(editor.document().to_string(), "hello world\n");
}

/// The drift test. A bar that pushes the document down without telling `ContentRect` puts every
/// click a bar-height out, which is the one failure mode chrome reliably introduces.
#[test]
fn a_click_below_the_bar_lands_on_the_right_byte() {
    let source = "hello world\n\nsecond line\n";

    let mut without = editor(source);
    without.toggle_toolbar();
    let _ = without.build_frame(WIDTH, HEIGHT, 1.0);
    let top = without
        .content_rect()
        .to_window(0.0, 0.0, Offset::down(0.0))
        .1;
    let left = without.content_rect().left_edge();
    without.click_at(
        left + 4.0 * ADVANCE,
        top + LINE_HEIGHT * 2.0 + LINE_HEIGHT / 2.0,
    );
    let expected = without.document().selection().head();

    let mut with = editor(source);
    let top = with.content_rect().to_window(0.0, 0.0, Offset::down(0.0)).1;
    let left = with.content_rect().left_edge();
    with.click_at(
        left + 4.0 * ADVANCE,
        top + LINE_HEIGHT * 2.0 + LINE_HEIGHT / 2.0,
    );

    assert_eq!(
        with.document().selection().head(),
        expected,
        "the same column of the same line is the same byte, bar or no bar"
    );
}

// ---- lighting ----

#[test]
fn a_button_lights_for_what_the_caret_is_in() {
    let mut editor = editor("**bold** and plain\n");

    caret_at(&mut editor, 4);
    assert!(editor.is_lit(Action::Bold), "the caret is inside the bold");

    caret_at(&mut editor, 12);
    assert!(!editor.is_lit(Action::Bold), "and not once it is outside");

    // And pressing the lit button is what clears it.
    select(&mut editor, 2, 6);
    let (x, y) = centre_of(&editor, Action::Bold);
    editor.click(x, y);
    assert_eq!(editor.document().to_string(), "bold and plain\n");
}

#[test]
fn the_heading_button_lights_at_its_own_level_only() {
    let mut editor = editor("## a heading\n");
    caret_at(&mut editor, 5);

    assert!(editor.is_lit(Action::Heading(2)));
    assert!(!editor.is_lit(Action::Heading(1)));
    assert!(!editor.is_lit(Action::Heading(3)));
}

/// A setext heading carries no marker on its first line, so lighting it means asking the block list
/// rather than reading the line — the one case that separates the two.
#[test]
fn a_setext_heading_lights_too() {
    let mut editor = editor("A heading\n=========\n\nprose\n");
    caret_at(&mut editor, 3);
    assert!(editor.is_lit(Action::Heading(1)));
}

#[test]
fn the_list_buttons_tell_each_other_apart() {
    let mut bullet = editor("- a bullet\n");
    caret_at(&mut bullet, 5);
    assert!(bullet.is_lit(Action::Bullet));
    assert!(!bullet.is_lit(Action::Ordered));
    assert!(!bullet.is_lit(Action::Task), "a bullet is not a task");

    let mut task = editor("- [ ] a task\n");
    caret_at(&mut task, 8);
    assert!(task.is_lit(Action::Task));
    assert!(!task.is_lit(Action::Bullet), "a task is not a bullet");
}

#[test]
fn quoting_lights_alongside_the_list_it_quotes() {
    let mut editor = editor("> - quoted bullet\n");
    caret_at(&mut editor, 8);
    assert!(editor.is_lit(Action::Quote));
    assert!(
        editor.is_lit(Action::Bullet),
        "quoting is orthogonal, and so is its lighting"
    );
}

#[test]
fn nothing_lights_in_plain_prose() {
    let mut editor = editor("just some prose\n");
    caret_at(&mut editor, 5);
    for action in toolbar::MARKDOWN {
        assert!(!editor.is_lit(*action), "{action:?} should be dark");
    }
}

/// Lighting says "the caret is already inside this". None of the standard set is a state the caret
/// can be in, so none of them ever lights — including on a document where every one of them is a
/// perfectly reasonable thing to do.
#[test]
fn the_standard_buttons_never_light() {
    let mut editor = editor("**bold** prose\n");
    select(&mut editor, 0, 8);
    editor.document_mut().toggle_inline(Inline::Italic);

    for action in toolbar::STANDARD {
        assert!(
            !editor.is_lit(*action),
            "{action:?} is a thing you do, not a state to reflect"
        );
    }
}

// ---- what can be done right now ----

/// Save is the clearest case: nothing to save is the state a document spends most of its life in,
/// and a live Save button that writes nothing is the failure MDEDIT-52 named.
#[test]
fn save_is_dimmed_until_there_is_something_to_save() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_named(&dir, "notes.md", "# Heading\n");
    assert!(
        !editor.is_enabled(Action::Save),
        "a document straight off disk has nothing to write"
    );

    caret_at(&mut editor, 9);
    editor.insert_text("!");
    assert!(editor.is_enabled(Action::Save), "and now it does");
}

#[test]
fn undo_and_redo_follow_the_history() {
    let mut editor = editor("prose\n");
    assert!(!editor.is_enabled(Action::Undo), "nothing has happened yet");
    assert!(!editor.is_enabled(Action::Redo));

    caret_at(&mut editor, 5);
    editor.insert_text("!");
    assert!(editor.is_enabled(Action::Undo));
    assert!(!editor.is_enabled(Action::Redo), "nothing undone to redo");

    editor.document_mut().undo();
    assert!(editor.is_enabled(Action::Redo));
}

#[test]
fn cut_and_copy_need_a_selection() {
    let mut editor = editor("prose\n");
    caret_at(&mut editor, 2);
    assert!(
        !editor.is_enabled(Action::Cut),
        "a caret is not a selection"
    );
    assert!(!editor.is_enabled(Action::Copy));

    select(&mut editor, 0, 5);
    assert!(editor.is_enabled(Action::Cut));
    assert!(editor.is_enabled(Action::Copy));
}

/// **A deliberate departure from MDEDIT-84's plan, which asked for Paste to dim on an empty
/// clipboard.** Deciding needs the system clipboard, and both ways of having that answer are worse
/// than not having it: reading it as the frame is drawn means opening the platform clipboard sixty
/// times a second, and caching it means that when the cache is stale the button is dimmed while the
/// clipboard has text — a wrong dim costs the user a paste, where a live Paste that inserts nothing
/// costs them nothing at all. It is what Ctrl+V already does.
#[test]
fn paste_is_never_dimmed() {
    let editor = editor("prose\n");
    assert!(
        editor.is_enabled(Action::Paste),
        "even with the inert clipboard every test gets"
    );
}

/// New and Open have no precondition — there is always somewhere to put a new document, and the
/// picker can always be opened.
#[test]
fn new_and_open_are_always_live() {
    let editor = editor("prose\n");
    assert!(editor.is_enabled(Action::New));
    assert!(editor.is_enabled(Action::Open));
}

/// The language's own buttons never dim. Each acts on the caret's line or the selection and does
/// something sensible with either, so there is no state in which one of them cannot act.
#[test]
fn the_language_buttons_are_always_live() {
    let mut editor = editor("prose\n");
    caret_at(&mut editor, 2);
    for action in toolbar::MARKDOWN {
        assert!(editor.is_enabled(*action), "{action:?} should be live");
    }
}

/// Dimming has to reach the pixels, not only the click handler. A button that ignored clicks while
/// still drawing in `text` would look broken rather than unavailable — and every assertion above
/// this one would pass.
///
/// Counted rather than located: line numbers are off by default, so the only thing on a frame drawn
/// in `gutter` is a dimmed label. Five are dimmed on an untouched document (Save, Undo, Redo, Cut,
/// Copy) and one is after an edit and a selection (Redo alone), so the count has to fall.
#[test]
fn a_dimmed_button_is_drawn_in_the_dim_colour() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_named(&dir, "notes.md", "some prose here\n");
    // Nothing here calls `with_config`, so the palette is `Theme::default()` rather than the
    // machine's — see `caret_line.rs` for what happens when a colour assertion forgets that.
    assert_eq!(editor.appearance(), Appearance::Dark);
    caret_at(&mut editor, 2);
    let dim = gutter_coloured_quads(&mut editor);
    assert!(dim > 0, "five buttons are dimmed, so something must be dim");

    editor.insert_text("!");
    select(&mut editor, 0, 9);
    let lit = gutter_coloured_quads(&mut editor);

    assert!(
        lit < dim,
        "four of the five became live, so less should be drawn dim: {dim} then {lit}"
    );
}

/// How many quads on a freshly drawn frame carry the palette's dim role.
fn gutter_coloured_quads(editor: &mut Editor<Grid>) -> usize {
    let dim = Theme::default().gutter;
    editor
        .build_frame(WIDTH, HEIGHT, 1.0)
        .iter()
        .filter(|quad| quad.colour == [dim.r, dim.g, dim.b, dim.a])
        .count()
}

/// The gap that says where the standard set ends and the language's begins. Grouping is the only
/// thing standing in for labels on this bar, and the boundary between "the application's buttons"
/// and "this document's" is the one worth the most.
#[test]
fn a_gap_separates_the_standard_set_from_the_language() {
    let editor = editor("prose\n");
    let buttons = editor.toolbar_buttons();
    let last_standard = buttons
        .iter()
        .position(|button| button.action == Action::Paste)
        .expect("Paste is the last of the standard set");
    let first_language = &buttons[last_standard + 1];

    assert_eq!(first_language.action, toolbar::MARKDOWN[0]);
    assert!(
        first_language.x > buttons[last_standard].right(),
        "the two sets run into each other with no gap between them"
    );
}

/// A dimmed button swallows its click and does nothing with it. **Both halves matter**: doing
/// nothing is the point of the dimming, and swallowing it is the rule every piece of chrome carries
/// — a click that fell through to the document would start a selection in text the user cannot even
/// see under the bar.
///
/// Every action, rather than one: the property is what protects a future dimmable action whose
/// method is not itself a no-op, and one hand-picked case would not.
#[test]
fn no_dimmed_button_changes_anything_and_all_of_them_swallow_the_click() {
    for action in [toolbar::STANDARD, toolbar::MARKDOWN].concat() {
        let mut editor = editor("some prose here\n");
        caret_at(&mut editor, 2);
        if editor.is_enabled(action) {
            continue;
        }

        let before = editor.document().to_string();
        let selection = editor.document().selection();
        let tabs = editor.workspace().tabs().len();
        let (x, y) = centre_of(&editor, action);
        assert!(editor.click(x, y), "{action:?}: the bar still swallows it");

        assert_eq!(editor.document().to_string(), before, "{action:?} edited");
        assert_eq!(editor.document().selection(), selection, "{action:?} moved");
        assert_eq!(editor.workspace().tabs().len(), tabs, "{action:?} opened");
    }
}

/// **The one dimmed button whose method would otherwise act**, and therefore the one that proves
/// the gate in `click_toolbar` is doing something. `Editor::save` does not itself ask whether there
/// is anything to save — it writes, because Ctrl+S on an unmodified document is allowed to be a
/// harmless rewrite. A *click* on a button drawn as unavailable is not.
#[test]
fn a_dimmed_save_does_not_write() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("notes.md");
    let mut editor = editor_named(&dir, "notes.md", "# Heading\n");
    assert!(!editor.is_enabled(Action::Save), "nothing to save yet");

    std::fs::remove_file(&path).unwrap();
    let (x, y) = centre_of_over_strip(&editor, Action::Save);
    assert!(editor.click(x, y), "the bar swallows it");

    assert!(!path.exists(), "a dimmed Save put the file back");
}

/// [`centre_of`], for an editor that has a tab strip above the bar.
///
/// MDEDIT-84's second item removes the need for this by moving the bar to the top of the window;
/// until it lands, the bar's own top is the strip's height and a test with an open file has to say
/// so. It is derived from the two content rects rather than hardcoded, so it cannot drift.
fn centre_of_over_strip(editor: &Editor<Grid>, action: Action) -> (f32, f32) {
    let (x, y) = centre_of(editor, action);
    let strip = editor.content_rect().top_edge() - toolbar::HEIGHT - MARGIN;
    (x, y + strip)
}

/// The same button, live, does act — otherwise the test above passes for the wrong reason.
#[test]
fn the_same_button_acts_once_it_is_live() {
    let mut editor = editor("prose\n");
    select(&mut editor, 0, 5);
    assert!(editor.is_enabled(Action::Cut));

    let (x, y) = centre_of(&editor, Action::Cut);
    editor.click(x, y);
    assert_eq!(editor.document().to_string(), "\n", "the selection is gone");
}

/// Geometry does not change with state. A row that reflowed as the user selected and deselected
/// would move every other button out from under the pointer — the same jitter this whole ticket is
/// avoiding at the tab strip, one level down.
#[test]
fn a_dimmed_button_keeps_its_rect() {
    let mut editor = editor("prose\n");
    caret_at(&mut editor, 2);
    let dimmed: Vec<toolbar::ButtonRect> = editor.toolbar_buttons().to_vec();

    select(&mut editor, 0, 5);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(
        editor.is_enabled(Action::Cut),
        "the state really did change"
    );
    assert_eq!(
        editor.toolbar_buttons(),
        dimmed.as_slice(),
        "but not one button moved"
    );
}

/// New is the standard action with the most obvious observable effect, and this is the wiring test:
/// the button reaches the method its chord reaches.
///
/// From a workspace with **no** tabs, deliberately. [`centre_of`] measures the bar from the top of
/// the window, which is only where it is when there is no tab strip above it — and that is what
/// MDEDIT-84's second item changes, at which point the helper is right for every case.
#[test]
fn the_new_button_opens_a_tab() {
    let mut editor = editor("prose\n");
    let before = editor.workspace().tabs().len();

    let (x, y) = centre_of(&editor, Action::New);
    assert!(editor.click(x, y));

    assert_eq!(editor.workspace().tabs().len(), before + 1);
}

/// The floating bar is inline styling anchored to a selection. New and Save have no business
/// floating over a paragraph, and the two sets being separate constants is what keeps them out.
#[test]
fn the_floating_bar_does_not_get_the_standard_set() {
    let mut editor = editor("some prose here\n");
    select(&mut editor, 0, 9);
    editor.settle_selection();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let floating: Vec<Action> = editor
        .floating_buttons()
        .iter()
        .map(|button| button.action)
        .collect();
    assert!(
        !floating.is_empty(),
        "there should be a floating bar at all"
    );
    assert!(
        !floating
            .iter()
            .any(|action| toolbar::STANDARD.contains(action)),
        "{floating:?} should be inline styling and headings only"
    );
}

// ---- what it costs ----

/// The trap the gutter, the status bar and the file pane each hit in turn. A fourth would be
/// indefensible, so it is asserted rather than intended.
#[test]
fn a_still_frame_lays_out_no_toolbar_labels() {
    let mut editor = editor("# Heading\n\nprose\n");
    let after_first = editor.text_backend().layouts();

    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let since = editor.text_backend().layouts() - after_first;

    assert!(
        since <= 2,
        "a second frame of an unchanged document should lay out the caret's chunk at most, \
         not {since}"
    );
}

/// Switching tabs swaps the *language's* buttons live, and moves nothing else.
///
/// This test used to assert the opposite of its second half — that a JSON tab took the bar away
/// entirely. MDEDIT-84 is the ticket that says it must not, because the bar is about to sit above
/// the tab strip and a bar that comes and goes would move the strip out from under the pointer
/// between one click and the next.
#[test]
fn switching_to_a_json_tab_keeps_the_bar_and_moves_nothing() {
    let dir = TempDir::new().unwrap();
    let markdown = dir.path().join("notes.md");
    let json = dir.path().join("data.json");
    std::fs::write(&markdown, "# Heading\n").unwrap();
    std::fs::write(&json, "{\"a\": 1}\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(markdown);
    workspace.add(json);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();

    editor.show_tab(0);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let on_markdown = editor.toolbar_buttons().len();
    let top = editor.content_rect().top_edge();
    assert!(
        on_markdown > toolbar::STANDARD.len(),
        "Markdown adds its own"
    );

    editor.show_tab(1);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(
        editor.toolbar_buttons().len(),
        toolbar::STANDARD.len(),
        "JSON keeps the standard set and loses Markdown's"
    );
    assert_eq!(
        editor.content_rect().top_edge(),
        top,
        "and nothing below the bar moves, which is the point"
    );

    editor.show_tab(0);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(
        editor.toolbar_buttons().len(),
        on_markdown,
        "and back again"
    );
}

// ---- geometry, on its own ----

#[test]
fn hit_testing_is_exclusive_at_the_edges() {
    let buttons = toolbar::layout(toolbar::MARKDOWN, &[10.0; 14], 900.0);
    let first = buttons[0];

    assert_eq!(
        toolbar::hit_test(&buttons, first.x, 1.0),
        Some(first.action),
        "the left edge belongs to the button"
    );
    assert_ne!(
        toolbar::hit_test(&buttons, first.right(), 1.0),
        Some(first.action),
        "its right edge belongs to whatever is next, not to it"
    );
}

#[test]
fn a_click_outside_the_bars_height_hits_nothing() {
    let buttons = toolbar::layout(toolbar::MARKDOWN, &[10.0; 14], 900.0);
    let x = buttons[0].x + 1.0;

    assert!(toolbar::hit_test(&buttons, x, -1.0).is_none());
    assert!(toolbar::hit_test(&buttons, x, toolbar::HEIGHT).is_none());
    assert!(toolbar::hit_test(&buttons, x, toolbar::HEIGHT / 2.0).is_some());
}
