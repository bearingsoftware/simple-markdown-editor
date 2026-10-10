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
use mdedit_app::tabs;
use mdedit_app::toolbar::{self, Action};
use mdedit_app::view::{Appearance, Rgba, Theme};
use mdedit_document::Selection;
use mdedit_document::format::Inline;
use mdedit_offsets::ByteOffset;
use mdedit_session::Workspace;
use tempfile::TempDir;

use crate::stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT, MARGIN};

/// Wider than the 900-pixel default window, and MDEDIT-92 is why.
///
/// This grid has no icon font — it reports `.notdef` for every private-use code point, which is
/// what a machine without Segoe Fluent Icons does — so every button here draws its **word**, and a
/// row of words is wider than a row of icons. At 900 the last of them fall off the end, which is
/// the overflow rule working rather than a defect; `a_narrow_window_cuts_the_language_buttons_first`
/// is where that is asserted, and `tests/icons.rs` is where the real font is measured against the
/// real default width.
const WIDTH: f32 = 1000.0;
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

        // MDEDIT-116's two exceptions, and they are exceptions to *this* rule rather than to the
        // bar's: a button that opens a list has done something, and what it did is
        // `a_menu_button_opens_its_list_rather_than_editing`.
        if action.opens().is_some() {
            assert_eq!(
                editor.document().to_string(),
                before,
                "{action:?} opens a list and must not also edit"
            );
            assert!(
                editor.menu_rows().is_some(),
                "{action:?} should have opened"
            );
            continue;
        }

        // MDEDIT-129's two, for the same reason one step along: a button that opens a *dialog* has
        // done something, and asking for a link's parts is what doing it means now.
        if matches!(action, Action::Link | Action::Image) {
            assert_eq!(
                editor.document().to_string(),
                before,
                "{action:?} asks before it writes, so nothing may change yet"
            );
            assert!(
                editor.dialog().is_some(),
                "{action:?} should have opened its dialog"
            );
            continue;
        }

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
        // `Heading(2)` is no longer *on* the bar — MDEDIT-116 replaced the three heading buttons
        // with one that opens a list, and the levels are its rows. Reaching one is two clicks, and
        // `popup.rs` is where that is tested. Bullet went the same way in MDEDIT-144, into Lists;
        // Task stayed a button.
        (Action::Task, "- [ ] prose\n"),
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

/// The kind and the modifier light independently (MDEDIT-144). Until then a task lit Task alone —
/// "a task is not a bullet" — which was right for three peer buttons and wrong for a kind and a box.
#[test]
fn the_list_buttons_tell_each_other_apart() {
    let mut bullet = editor("- a bullet\n");
    caret_at(&mut bullet, 5);
    assert!(bullet.is_lit(Action::Lists));
    assert!(bullet.is_lit(Action::Bullet));
    assert!(!bullet.is_lit(Action::Ordered));
    assert!(!bullet.is_lit(Action::Task), "a bullet is not a task");

    let mut task = editor("- [ ] a task\n");
    caret_at(&mut task, 8);
    assert!(task.is_lit(Action::Task));
    assert!(task.is_lit(Action::Lists), "a task is in a list");
    assert!(task.is_lit(Action::Bullet), "and its kind is bullet");
    assert!(!task.is_lit(Action::Ordered));

    let mut numbered = editor("1. [ ] a numbered task\n");
    caret_at(&mut numbered, 9);
    assert!(numbered.is_lit(Action::Task), "MDEDIT-142");
    assert!(numbered.is_lit(Action::Lists));
    assert!(numbered.is_lit(Action::Ordered));
    assert!(!numbered.is_lit(Action::Bullet));

    let mut prose = editor("just prose\n");
    caret_at(&mut prose, 4);
    assert!(!prose.is_lit(Action::Lists));
}

/// One Lists button where Bullet and Numbered were, with Task straight after it (MDEDIT-144).
#[test]
fn the_bar_has_one_lists_button_and_task_beside_it() {
    let markdown = toolbar::MARKDOWN;
    assert!(!markdown.contains(&Action::Bullet) && !markdown.contains(&Action::Ordered));
    let lists = markdown
        .iter()
        .position(|action| *action == Action::Lists)
        .expect("Lists should be on the bar");
    assert_eq!(markdown.get(lists + 1), Some(&Action::Task));
    assert_eq!(Action::Lists.opens(), Some(toolbar::Menu::Lists));
    assert_eq!(
        Action::Lists.chord(),
        None,
        "no chord, as Bullet and Ordered had none"
    );
}

/// **The face says which kind the caret is in**, the way Headings says which level. This grid has
/// no icon font, so what is drawn is each face's fallback word: `•` for a bullet, `1.` for a number.
#[test]
fn the_lists_face_follows_the_caret() {
    for (source, caret, face) in [
        ("just prose\n", 4, "•"),
        ("- a bullet\n", 5, "•"),
        ("1. a number\n", 5, "1."),
        ("3. [x] a numbered task\n", 9, "1."),
        ("- [ ] a task\n", 8, "•"),
    ] {
        let mut editor = editor(source);
        caret_at(&mut editor, caret);
        let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
        assert_eq!(
            editor.button_face_drawn(Action::Lists),
            Some(face),
            "{source:?}"
        );
    }
}

/// Moving between kinds changes the face on the next frame without moving anything else: the face
/// is a cache key, and a stale one would be served for the wrong kind.
#[test]
fn the_lists_face_changes_as_the_caret_moves() {
    let source = "- a bullet\n\n1. a number\n";
    let mut editor = editor(source);
    caret_at(&mut editor, 3);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(editor.button_face_drawn(Action::Lists), Some("•"));

    caret_at(&mut editor, source.len() - 3);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(editor.button_face_drawn(Action::Lists), Some("1."));
}

/// The button, not only the document call behind it: Task on a numbered item keeps the number.
#[test]
fn the_task_button_keeps_a_numbered_item_numbered() {
    let mut editor = editor("1. first\n");
    caret_at(&mut editor, 4);
    assert!(editor.perform(Action::Task));
    assert_eq!(editor.document().to_string(), "1. [ ] first\n");
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
    let (x, y) = centre_of(&editor, Action::Save);
    assert!(editor.click(x, y), "the bar swallows it");

    assert!(!path.exists(), "a dimmed Save put the file back");
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

// ---- where the two bands are ----

/// MDEDIT-84's second item. The bar is the topmost chrome and the strip is directly below it, which
/// is a change to four y offsets and two hit tests and nothing else — `tabs.rs` and `toolbar.rs`
/// each lay out within a row whose origin the caller supplies.
#[test]
fn the_bar_is_the_topmost_chrome_and_the_strip_is_below_it() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_named(&dir, "notes.md", "# Heading\n");

    // A click at the very top of the window is the bar's, on a document that has a tab strip too.
    let (x, _) = centre_of(&editor, Action::Bold);
    assert!(editor.click(x, 1.0), "the top row belongs to the bar");
    assert_eq!(
        editor.document().to_string(),
        "****# Heading\n",
        "and it was Bold that ran, at the caret, which starts at byte zero"
    );
}

/// The click that used to select a tab now presses a button, and the one that used to press a
/// button now selects a tab. Asserted in both directions, because a swap that moved the drawing and
/// not the hit test would look right and behave wrongly — which is the failure mode this whole
/// ticket's test list is built around.
#[test]
fn a_tab_is_clicked_at_its_new_position_and_not_at_its_old_one() {
    let dir = TempDir::new().unwrap();
    let first = dir.path().join("first.md");
    let second = dir.path().join("second.md");
    std::fs::write(&first, "one\n").unwrap();
    std::fs::write(&second, "two\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(first);
    workspace.add(second);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.show_tab(1);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(
        editor.workspace().active(),
        Some(1),
        "starting on the second"
    );

    // Where the first tab is now: below the bar.
    let tab = editor.tab_rects()[0];
    let middle = tab.x + 5.0;
    editor.click(middle, toolbar::HEIGHT + tabs::STRIP_HEIGHT / 2.0);
    assert_eq!(
        editor.workspace().active(),
        Some(0),
        "a click below the bar should select the tab under it"
    );

    // And where it used to be: inside the bar, which swallows the click without touching the tabs.
    //
    // At a **group gap**, so the bar itself does nothing either — clicking a button at the old
    // position would prove the same thing while also opening a tab or editing the document, which
    // makes the assertion about tabs harder to read than it needs to be.
    editor.show_tab(1);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let buttons = editor.toolbar_buttons();
    let (left, right) = buttons
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .find(|(left, right)| right.x > left.right())
        .expect("the bar has group gaps in it");
    let gap = f32::midpoint(left.right(), right.x);

    assert!(
        editor.click(gap, tabs::STRIP_HEIGHT / 2.0),
        "the old strip position is inside the bar now, so the bar swallows it"
    );
    assert_eq!(
        editor.workspace().active(),
        Some(1),
        "and it must not reach a tab"
    );
}

/// **The assertion the ticket asked for by name.** The two bands abut exactly: a gap would let a
/// click fall through to the document under chrome the user can see, and an overlap would let one
/// bar steal the other's clicks. Neither is visible on a screenshot.
#[test]
fn the_two_bands_abut_exactly() {
    let dir = TempDir::new().unwrap();
    let editor = editor_named(&dir, "notes.md", "# Heading\n");

    // Derived from the same constants the renderer used, rather than restated: the bar runs from
    // zero, the strip starts where it ends, and the document starts where the strip ends.
    let bar_ends = toolbar::HEIGHT;
    let strip_ends = bar_ends + tabs::STRIP_HEIGHT;
    assert_eq!(
        editor.content_rect().top_edge(),
        strip_ends + MARGIN,
        "the document starts immediately below the strip, with only its own margin between"
    );

    let tab = editor.tab_rects()[0];
    assert!(
        tabs::hit_test(editor.tab_rects(), tab.x + 5.0, -1.0).is_none(),
        "a pixel above the strip is not the strip's"
    );
    assert!(
        tabs::hit_test(editor.tab_rects(), tab.x + 5.0, 0.0).is_some(),
        "and its first pixel is"
    );
}

/// The regression the swap most plausibly breaks: `Insets::top` is a sum, and a sum does not care
/// about order — so moving the bar above the strip must not move the document one pixel.
#[test]
fn the_documents_top_edge_is_unchanged_by_the_swap() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_named(&dir, "notes.md", "# Heading\n");
    let with_both = editor.content_rect().top_edge();

    editor.toggle_toolbar();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let strip_only = editor.content_rect().top_edge();

    assert_eq!(
        with_both - strip_only,
        toolbar::HEIGHT,
        "the bar's height is all the bar costs, wherever it is drawn"
    );
}

/// With the bar hidden, the strip is back at the top of the window — the acceptance criterion that
/// says the strip's origin follows the bar's *height*, not a constant.
#[test]
fn hiding_the_bar_puts_the_tabs_back_at_the_top() {
    let dir = TempDir::new().unwrap();
    let first = dir.path().join("first.md");
    let second = dir.path().join("second.md");
    std::fs::write(&first, "one\n").unwrap();
    std::fs::write(&second, "two\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(first);
    workspace.add(second);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.show_tab(1);
    editor.toggle_toolbar();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    let tab = editor.tab_rects()[0];
    editor.click(tab.x + 5.0, tabs::STRIP_HEIGHT / 2.0);
    assert_eq!(
        editor.workspace().active(),
        Some(0),
        "with no bar above it the strip starts at the window's top again"
    );
}

/// **Where the two bands are actually drawn**, which the hit-testing assertions above cannot see.
///
/// A build that moved the clicks and not the pixels — or the pixels and not the clicks — passes
/// every other test in this file, because the two halves are checked separately everywhere else.
/// The two backgrounds cannot tell each other apart (both are `strip`, both thirty pixels), so this
/// reads the things that are unique to each: an inactive tab's quad belongs only to the strip, and a
/// dimmed label's `gutter` belongs only to the bar, since line numbers are off by default.
#[test]
fn the_bar_is_drawn_above_the_strip() {
    let dir = TempDir::new().unwrap();
    let first = dir.path().join("first.md");
    let second = dir.path().join("second.md");
    std::fs::write(&first, "one\n").unwrap();
    std::fs::write(&second, "two\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(first);
    workspace.add(second);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.show_tab(0);
    // Nothing here calls `with_config`, so the palette is `Theme::default()` — see `caret_line.rs`.
    assert_eq!(editor.appearance(), Appearance::Dark);

    let theme = Theme::default();
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let tops = |colour: Rgba| -> Vec<f32> {
        let mut tops: Vec<f32> = quads
            .iter()
            .filter(|quad| quad.colour == [colour.r, colour.g, colour.b, colour.a])
            .map(|quad| quad.rect[1])
            .collect();
        tops.sort_by(f32::total_cmp);
        tops.dedup();
        tops
    };

    let tabs = tops(theme.tab_inactive);
    assert_eq!(
        tabs,
        vec![toolbar::HEIGHT],
        "an inactive tab is drawn at the strip's origin, which is the bar's height"
    );

    let dimmed = tops(theme.gutter);
    assert!(
        !dimmed.is_empty(),
        "some buttons are dimmed on a fresh file"
    );
    for top in dimmed {
        assert!(
            (0.0..toolbar::HEIGHT).contains(&top),
            "a dimmed label at {top} is outside the bar, which runs 0..{}",
            toolbar::HEIGHT
        );
    }

    // A tab's *name* and its close cross, which move with the strip's origin or float free of it.
    // `marker` is the inactive tab's ink and the breadcrumb's; the breadcrumb is on the status bar
    // at the bottom of a 424-pixel window, so nothing legitimately puts one in the bar's band.
    let ink = tops(theme.marker);
    assert!(!ink.is_empty(), "the inactive tab has a name drawn on it");
    for top in ink {
        assert!(
            !(0.0..toolbar::HEIGHT).contains(&top),
            "a tab's own ink at {top} is inside the formatting bar — the background moved with the \
             strip and the glyphs did not"
        );
    }
}

// ---- icons, and what happens without them ----

/// Every action has something to draw. A button with an empty face is a hole in the row.
#[test]
fn every_action_has_a_label() {
    for action in [toolbar::STANDARD, toolbar::MARKDOWN].concat() {
        assert!(!action.label().is_empty(), "{action:?} has no label");
    }
}

/// No two buttons draw the same icon. Two identical shapes side by side is the failure a table of
/// hand-copied code points makes, and it is invisible in a screenshot until somebody clicks one.
#[test]
fn no_two_actions_share_an_icon() {
    let mut seen: Vec<(Action, char)> = Vec::new();
    for action in [toolbar::STANDARD, toolbar::MARKDOWN].concat() {
        let Some(icon) = action.icon() else {
            continue;
        };
        if let Some((other, _)) = seen.iter().find(|(_, taken)| *taken == icon) {
            panic!("{action:?} and {other:?} both draw U+{:04X}", icon as u32);
        }
        seen.push((action, icon));
    }
    assert!(seen.len() >= 19, "most of the bar should be iconic");
}

/// **The fallback, exercised.** This grid reports `.notdef` for every private-use code point, which
/// is what a machine without Segoe Fluent Icons does — so these buttons draw their words, and the
/// bar lays out and clicks exactly as it did before MDEDIT-92. A row of empty boxes would be a bug
/// report; the old bar is merely the old bar.
#[test]
fn without_the_icon_font_the_bar_draws_its_words() {
    let mut editor = editor("prose\n");
    select(&mut editor, 0, 5);

    let shown: Vec<Action> = editor
        .toolbar_buttons()
        .iter()
        .map(|button| button.action)
        .collect();
    assert_eq!(
        shown,
        [toolbar::STANDARD, toolbar::MARKDOWN].concat(),
        "every button is still there"
    );

    // **And each of them drew its word rather than a box.** Asserting the actions alone would pass
    // just as well with the `.notdef` check taken out, which is a bar of tofu — the exact outcome
    // the fallback exists to prevent.
    for action in shown {
        assert_eq!(
            editor.button_face_drawn(action),
            Some(action.label().as_str()),
            "{action:?} drew something other than its word with no icon font present"
        );
    }

    // And it still works, which is the half that matters.
    let (x, y) = centre_of(&editor, Action::Bold);
    editor.click(x, y);
    assert_eq!(editor.document().to_string(), "**prose**\n");
}

/// The lettered heading buttons draw the same thing either way, so they are the one part of the bar
/// a missing font cannot change.
#[test]
fn the_heading_buttons_letter_whatever_the_font_has() {
    assert_eq!(Action::Heading(1).icon(), None);
    assert_eq!(Action::Heading(2).label(), "H2");
}

/// **A caret at the very end of the document is still inside the last block.**
///
/// `Document::block_at` is half-open — a block's span ends where the next begins, which is the right
/// rule for the renderer asking who owns a byte. A caret is not a byte: it sits between them, and
/// the position after the last character of the last block is a legitimate place to be.
///
/// It only bites where nothing follows, so a file ending `### Title` with no trailing newline had a
/// caret in no block at all: the heading button read its default `H1` while the caret was in an H3,
/// and did not light. Adding a newline fixed it, which is what made it look like a parser bug rather
/// than an off-by-one at a boundary.
#[test]
fn the_heading_button_reads_a_heading_that_ends_the_document() {
    let source = "### Title";
    let mut editor = editor(source);
    caret_at(&mut editor, source.len());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(
        editor.is_lit(Action::Headings),
        "the caret is in a heading, so the button should be lit"
    );
    assert_eq!(
        editor.button_face_drawn(Action::Headings),
        Some("H3"),
        "and it should say which level, not fall back to its default"
    );
}

/// The same document with a newline after it, which always worked. Here so the test above is known
/// to be about the boundary rather than about headings.
#[test]
fn the_heading_button_reads_one_that_does_not_end_the_document() {
    let source = "### Title\n";
    let mut editor = editor(source);
    caret_at(&mut editor, 5);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(editor.button_face_drawn(Action::Headings), Some("H3"));
}

/// With the caret in ordinary prose it falls back to its default, which is what makes `H1` on the
/// button ambiguous enough to be worth the test above.
#[test]
fn the_heading_button_defaults_outside_a_heading() {
    let mut editor = editor("just prose\n");
    caret_at(&mut editor, 4);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(!editor.is_lit(Action::Headings));
    assert_eq!(editor.button_face_drawn(Action::Headings), Some("H1"));
}

/// **The selection survives a formatting press**, which is what made bold-then-italic two clicks
/// rather than two selections — and it was the floating bar's headline property before MDEDIT-119
/// deleted it. The behaviour belonged to `Editor::perform` all along, not to the bar that called it,
/// which is exactly why removing one of the two callers was safe. Asserted here so that stays true.
#[test]
fn the_selection_survives_a_press_on_the_bar() {
    let mut editor = editor("some prose here\n");
    select(&mut editor, 0, 10);

    let (x, y) = centre_of(&editor, Action::Bold);
    editor.click(x, y);
    assert_eq!(editor.document().to_string(), "**some prose** here\n");

    let after = editor.document().selection();
    assert!(!after.is_collapsed(), "the selection should still be one");
    assert_ne!(
        after.span().len(),
        0,
        "and cover something, so a second press is one click rather than a re-selection"
    );
}

// ---- the insert group (MDEDIT-87) ----

/// **Image is the gap this closes.** It was the only element of the Markdown Guide's basic syntax
/// with no way in but typing — parsed, coloured, its destination underlined, and no button, no chord
/// and no menu item.
#[test]
fn the_bar_offers_an_image() {
    assert!(toolbar::MARKDOWN.contains(&Action::Image));
}

/// Link, Image and Table are the last three, in that order: the insert group, which MDEDIT-127
/// closed once the Table button had a size picker to open.
#[test]
fn the_insert_group_is_link_image_table_in_that_order() {
    let markdown = toolbar::MARKDOWN;
    assert_eq!(
        &markdown[markdown.len() - 3..],
        &[Action::Link, Action::Image, Action::Table]
    );
}

/// **Neither has a chord**, which is a decision rather than an omission: the bar is the only door,
/// and a chord is worth spending on something reached often enough to remember. Table could not have
/// one at all — a chord that opened a picker would be a chord that does nothing on its own.
#[test]
fn image_takes_no_chord() {
    assert_eq!(Action::Image.chord(), None);
    assert_eq!(Action::Table.chord(), None);
}

/// Neither lights. An image is something you insert, and there is no "inside an image" to reflect;
/// "inside a table" *is* a state, but nothing can answer it until MDEDIT-89 parses one — and the
/// buttons that should light for it are MDEDIT-128's controls over the table you are in, not this
/// picker for inserting a new one.
#[test]
fn image_is_not_a_state() {
    let editor = editor(
        "prose
",
    );
    assert!(!editor.is_lit(Action::Image));
    assert!(!editor.is_lit(Action::Table));
}

/// Both have a tooltip, because every button does and MDEDIT-85's test would have caught it anyway.
#[test]
fn image_says_what_it_does() {
    assert_eq!(Action::Image.tooltip(), "Image");
    assert_eq!(Action::Table.tooltip(), "Table");
}
