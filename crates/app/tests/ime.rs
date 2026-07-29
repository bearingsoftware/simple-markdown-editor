//! Input-method composition, as far as it can be tested without a human.
//!
//! The seven-step script in `spikes/font-fallback/README.md` needs a Japanese keyboard, a Windows
//! machine and somebody looking at the screen. What it does *not* need is a human to check that
//! composing seven keystrokes produces one edit, that cancelling produces none, and that a stale
//! cursor range from the platform cannot panic a render — so those are asserted here, and the steps
//! that genuinely need eyes are listed in the ticket.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::ime::{Action, Composition};

fn composing(text: &str) -> Composition {
    let mut composition = Composition::new();
    composition.preedit(text.to_owned(), None);
    composition
}

#[test]
fn nothing_is_being_composed_to_begin_with() {
    let composition = Composition::new();
    assert!(!composition.is_active());
    assert_eq!(composition.text(), "");
    assert_eq!(composition.cursor(), None);
}

#[test]
fn a_preedit_becomes_the_text_being_composed() {
    let mut composition = Composition::new();
    assert_eq!(
        composition.preedit("にほん".to_owned(), Some((3, 3))),
        Action::Redraw
    );
    assert!(composition.is_active());
    assert_eq!(composition.text(), "にほん");
    assert_eq!(composition.cursor(), Some((3, 3)));
}

/// Every keystroke of a composition arrives as a fresh pre-edit, so a repeat is not a frame.
#[test]
fn an_unchanged_preedit_does_not_cost_a_frame() {
    let mut composition = composing("に");
    assert_eq!(composition.preedit("に".to_owned(), None), Action::Nothing);
    assert_eq!(composition.preedit("にほ".to_owned(), None), Action::Redraw);
}

#[test]
fn an_empty_preedit_clears_the_composition() {
    let mut composition = composing("にほんご");
    assert_eq!(composition.preedit(String::new(), None), Action::Redraw);
    assert!(!composition.is_active());
}

/// Step 3 of the script: committed text replaces the pre-edit in **one** edit, not two.
#[test]
fn committing_produces_exactly_one_insertion() {
    let mut composition = composing("にほんご");
    assert_eq!(
        composition.commit("日本語".to_owned()),
        Action::Insert("日本語".to_owned())
    );
    assert!(
        !composition.is_active(),
        "the composition ends when it commits"
    );
}

/// winit sends an empty pre-edit immediately before every commit. That must not turn one edit into
/// two, nor lose the commit.
#[test]
fn the_empty_preedit_before_a_commit_changes_nothing_about_it() {
    let mut composition = composing("にほんご");
    assert_eq!(composition.preedit(String::new(), None), Action::Redraw);
    assert_eq!(
        composition.commit("日本語".to_owned()),
        Action::Insert("日本語".to_owned()),
        "the commit still arrives, and still as one edit"
    );
}

#[test]
fn committing_nothing_inserts_nothing() {
    let mut composition = composing("に");
    assert_eq!(composition.commit(String::new()), Action::Redraw);
    assert!(!composition.is_active());

    let mut idle = Composition::new();
    assert_eq!(idle.commit(String::new()), Action::Nothing);
}

/// Steps 4 and 5: focus moving away mid-composition, and Escape. Neither may touch the buffer —
/// which is the whole reason the pre-edit is not in it. slint#10861 is what happens when it is.
#[test]
fn cancelling_never_produces_an_edit() {
    let mut composition = composing("にほんご");
    assert_eq!(
        composition.cancel(),
        Action::Redraw,
        "the pre-edit was on screen and has to be taken off it"
    );
    assert!(!composition.is_active());

    let mut idle = Composition::new();
    assert_eq!(
        idle.cancel(),
        Action::Nothing,
        "nothing was being composed, so nothing happened"
    );
}

/// Step 6: undo must not be able to split a composition into fragments. It cannot, because the
/// composition produces one edit and only when it commits — there is nothing else for undo to see.
#[test]
fn a_composition_reaches_the_buffer_only_once() {
    let mut composition = Composition::new();
    let mut edits = Vec::new();

    // Seven keystrokes, as `nihongo` actually is.
    for typed in ["に", "にほ", "にほん", "にほんg", "にほんご"] {
        if let Action::Insert(text) = composition.preedit(typed.to_owned(), None) {
            edits.push(text);
        }
    }
    assert!(edits.is_empty(), "composing must not edit the buffer");

    if let Action::Insert(text) = composition.commit("日本語".to_owned()) {
        edits.push(text);
    }
    assert_eq!(edits, vec!["日本語".to_owned()], "one edit, at the end");
}

// ---------------------------------------------------------------------------
// The cursor range, which arrives from the platform and is therefore not trusted.
// ---------------------------------------------------------------------------

#[test]
fn a_cursor_range_past_the_end_is_clamped_rather_than_believed() {
    let mut composition = Composition::new();
    composition.preedit("にほ".to_owned(), Some((99, 200)));
    assert_eq!(composition.cursor(), Some((6, 6)), "clamped to the text");
}

#[test]
fn a_cursor_range_inside_a_character_is_snapped_to_a_boundary() {
    let mut composition = Composition::new();
    // Every one of these is three bytes, so 1, 2, 4 and 5 all split a character.
    composition.preedit("にほ".to_owned(), Some((1, 5)));
    let (start, end) = composition.cursor().expect("a cursor");
    assert!(
        "にほ".is_char_boundary(start) && "にほ".is_char_boundary(end),
        "got {start}..{end}, which splits a character"
    );
}

#[test]
fn a_backwards_cursor_range_comes_back_in_order() {
    let mut composition = Composition::new();
    composition.preedit("にほんご".to_owned(), Some((9, 3)));
    let (start, end) = composition.cursor().expect("a cursor");
    assert!(start <= end, "got {start}..{end}");
}

#[test]
fn no_cursor_is_a_valid_answer() {
    let mut composition = Composition::new();
    composition.preedit("に".to_owned(), None);
    assert_eq!(
        composition.cursor(),
        None,
        "the input method wants none shown"
    );
}
