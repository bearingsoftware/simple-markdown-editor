//! Resting on a toolbar button says what it does.
//!
//! Three claims carry the ticket, and two of them are about things that are not the panel:
//!
//! - the **dwell** — crossing a row of buttons shows nothing, and stopping on one shows something;
//! - the **two deadlines** — a hover dwell and the reconcile timer share one `ControlFlow`, and
//!   neither may swallow the other. That failure is silent and intermittent, which is why it is a
//!   test rather than a comment;
//! - the panel is **never hit-tested**, so a click where it hangs over the document reaches the
//!   document.
//!
//! Timing is driven rather than waited on. `Editor::hover` and `Editor::on_dwell` are public for the
//! same reason `Editor::on_idle` is: a test cannot deliver a `CursorMoved` or a
//! `StartCause::ResumeTimeReached`, and sleeping four hundred milliseconds a case is time nobody
//! gets back.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::toolbar::{self, Action};
use mdedit_app::tooltip;
use mdedit_app::view::{Appearance, Rgba, Theme};
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;
use mdedit_session::Workspace;
use std::time::{Duration, Instant};
use tempfile::TempDir;

mod stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 400.0;

fn editor(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// The middle of a button, in window coordinates. The bar is the topmost chrome since MDEDIT-84, so
/// its own top is the window's.
fn centre_of(editor: &Editor<Grid>, action: Action) -> (f32, f32) {
    let button = editor
        .toolbar_buttons()
        .iter()
        .find(|button| button.action == action)
        .unwrap_or_else(|| panic!("{action:?} should be on the bar"));
    (button.x + button.width / 2.0, toolbar::HEIGHT / 2.0)
}

/// Rests the pointer on a button and lets the dwell elapse.
fn rest_on(editor: &mut Editor<Grid>, action: Action) {
    let (x, y) = centre_of(editor, action);
    editor.hover(x, y);
    editor.on_dwell();
}

// ---- the dwell ----

#[test]
fn resting_on_a_button_shows_its_tooltip() {
    let mut editor = editor("prose\n");
    let (x, y) = centre_of(&editor, Action::Bold);

    editor.hover(x, y);
    assert_eq!(editor.tooltip(), None, "not before the dwell has elapsed");

    assert!(editor.on_dwell(), "and the panel is a redraw");
    assert_eq!(editor.tooltip(), Some(Action::Bold));
}

/// The panel says what the button does *and* the chord that does the same. Both halves, because a
/// tooltip that only named the action would teach nobody a shortcut.
#[test]
fn the_tooltip_reads_the_action_and_its_chord() {
    assert_eq!(Action::Bold.tooltip(), "Bold — Ctrl+B");
    assert_eq!(Action::Save.tooltip(), "Save — Ctrl+S");
    assert_eq!(Action::Heading(2).tooltip(), "Heading 2 — Ctrl+2");
}

/// A block action has no chord and never has had one — the bar is the only way to reach a fence, and
/// a tooltip inventing "Ctrl+…" for it would be the MDEDIT-65 failure in miniature.
#[test]
fn a_button_with_no_chord_says_only_what_it_does() {
    assert_eq!(Action::Fence.chord(), None);
    assert_eq!(Action::Fence.tooltip(), "Code block");
}

/// **Every chord a tooltip advertises has to exist.** MDEDIT-85's plan asked for the phrasing to be
/// looked up out of `shortcuts::ALL` so the two cannot drift; the phrasing is in `toolbar.rs`
/// instead, because the reference document reads like one where a button label has to read like a
/// label — so this is the guard that replaces the lookup, and it guards the half that matters. A
/// tooltip promising Ctrl+B when nothing answers to it is MDEDIT-65 exactly.
#[test]
fn every_chord_a_tooltip_advertises_is_in_the_shortcut_list() {
    let listed: Vec<&str> = mdedit_app::shortcuts::ALL
        .iter()
        .flat_map(|group| group.shortcuts.iter())
        .map(|shortcut| shortcut.keys)
        .collect();

    for action in [toolbar::STANDARD, toolbar::MARKDOWN].concat() {
        let Some(chord) = action.chord() else {
            continue;
        };
        // Headings are one row covering six levels, so the list spells them as a range. Every other
        // chord appears verbatim.
        let found = listed.iter().any(|keys| {
            *keys == chord || (matches!(action, Action::Heading(_)) && keys.starts_with("Ctrl+1"))
        });
        assert!(
            found,
            "{action:?} advertises {chord}, which is not documented"
        );
    }
}

/// Passing over buttons shows nothing, and the mechanism is that each new button restarts the clock
/// rather than that the panel is suppressed. Asserted on the deadline, because a pointer that never
/// stops never reaches one.
#[test]
fn crossing_buttons_pushes_the_dwell_out_each_time() {
    let mut editor = editor("prose\n");

    let (x, y) = centre_of(&editor, Action::Bold);
    editor.hover(x, y);
    let first = editor.next_wake().expect("hovering arms the dwell");

    let (x, y) = centre_of(&editor, Action::Italic);
    editor.hover(x, y);
    let second = editor.next_wake().expect("and moving on re-arms it");

    assert!(second >= first, "the deadline moved forward, not back");
    assert_eq!(editor.tooltip(), None, "and nothing was shown on the way");
}

/// Moving *within* the button you are already on must not restart the clock, or a dwell would never
/// elapse for anybody whose hand is not perfectly still.
#[test]
fn moving_inside_one_button_leaves_the_dwell_alone() {
    let mut editor = editor("prose\n");
    let button = editor
        .toolbar_buttons()
        .iter()
        .find(|button| button.action == Action::Bold)
        .copied()
        .unwrap();

    editor.hover(button.x + 1.0, toolbar::HEIGHT / 2.0);
    let armed = editor.next_wake().expect("hovering arms the dwell");

    editor.hover(button.right() - 1.0, toolbar::HEIGHT / 2.0);
    assert_eq!(editor.next_wake(), Some(armed), "still the same rest");
}

/// The gaps between groups are not buttons, and resting in one says nothing. A panel reading
/// "Bold — Ctrl+B" while the pointer is demonstrably beside the Bold button is worse than none.
#[test]
fn resting_in_a_gap_shows_nothing() {
    let mut editor = editor("prose\n");
    let buttons = editor.toolbar_buttons();
    let (left, right) = buttons
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .find(|(left, right)| right.x > left.right())
        .expect("the bar has group gaps in it");
    let gap = f32::midpoint(left.right(), right.x);

    editor.hover(gap, toolbar::HEIGHT / 2.0);
    assert!(!editor.on_dwell(), "nothing to describe");
    assert_eq!(editor.tooltip(), None);
    assert_eq!(editor.next_wake(), None, "and no wake-up wanted");
}

/// Below the bar is the document, which has no tooltips.
#[test]
fn resting_below_the_bar_shows_nothing() {
    let mut editor = editor("some prose here\n");
    editor.hover(50.0, toolbar::HEIGHT + 40.0);
    assert!(!editor.on_dwell());
    assert_eq!(editor.tooltip(), None);
}

// ---- dismissal ----

#[test]
fn moving_off_the_button_takes_it_away() {
    let mut editor = editor("prose\n");
    rest_on(&mut editor, Action::Bold);
    assert!(editor.tooltip().is_some());

    assert!(
        editor.hover(50.0, toolbar::HEIGHT + 40.0),
        "taking it away is a redraw"
    );
    assert_eq!(editor.tooltip(), None);
}

/// Clicking dismisses it **and the button still acts** — the panel going is not instead of the
/// press, it is alongside it.
#[test]
fn clicking_the_button_dismisses_it_and_still_acts() {
    let mut editor = editor("prose\n");
    editor
        .document_mut()
        .set_selection(Selection::new(ByteOffset::new(0), ByteOffset::new(5)));
    rest_on(&mut editor, Action::Bold);

    let (x, y) = centre_of(&editor, Action::Bold);
    assert!(editor.click(x, y));

    assert_eq!(editor.tooltip(), None, "the panel went");
    assert_eq!(editor.document().to_string(), "**prose**\n", "and it acted");
}

#[test]
fn typing_dismisses_it() {
    let mut editor = editor("prose\n");
    rest_on(&mut editor, Action::Bold);

    editor.insert_text("x");
    assert_eq!(editor.tooltip(), None);
}

/// A click that lands nowhere in particular still takes it away, and a click *in the document* is
/// the commonest such click.
#[test]
fn clicking_in_the_document_dismisses_it() {
    let mut editor = editor("some prose here\n");
    rest_on(&mut editor, Action::Bold);

    editor.click(60.0, editor.content_rect().top_edge() + 2.0);
    assert_eq!(editor.tooltip(), None);
}

/// Switching tab dismisses it, and does so through the click that switched — rather than through a
/// hook on every path that activates a tab, which is the list somebody forgets to add to.
#[test]
fn switching_tab_dismisses_it() {
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

    rest_on(&mut editor, Action::Bold);
    assert!(editor.tooltip().is_some());

    let tab = editor.tab_rects()[0];
    editor.click(tab.x + 5.0, toolbar::HEIGHT + 5.0);

    assert_eq!(editor.workspace().active(), Some(0), "the tab switched");
    assert_eq!(editor.tooltip(), None, "and the panel went with it");
}

/// A dimmed button still explains itself. Somebody who cannot press Paste is exactly the person
/// wondering what it would have done.
#[test]
fn a_dimmed_button_still_gets_a_tooltip() {
    let mut editor = editor("prose\n");
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(2)));
    assert!(!editor.is_enabled(Action::Cut), "the precondition");

    rest_on(&mut editor, Action::Cut);
    assert_eq!(editor.tooltip(), Some(Action::Cut));

    let (x, y) = centre_of(&editor, Action::Cut);
    editor.click(x, y);
    assert_eq!(editor.document().to_string(), "prose\n", "and stayed inert");
}

// ---- the two deadlines ----

/// **The risk the ticket named.** One `ControlFlow`, two things that want it. The reconcile timer
/// owned it outright before this story, and adding a dwell to that arrangement breaks it in both
/// directions — so both are kept and the earliest wins.
#[test]
fn a_dwell_does_not_push_the_reconcile_out() {
    let mut editor = editor("prose\n");
    editor.insert_text("x");
    editor.schedule_reconcile();
    let reconcile = editor.next_wake().expect("typing arms the idle beat");

    // A dwell arriving afterwards is *later* than the reconcile only if the dwell is longer, so the
    // assertion is on the minimum rather than on which one it is.
    let (x, y) = centre_of(&editor, Action::Bold);
    editor.hover(x, y);
    let next = editor.next_wake().expect("both are pending");

    assert!(
        next <= reconcile,
        "the reconcile was pushed out to {next:?} by a dwell"
    );
}

#[test]
fn a_reconcile_does_not_swallow_a_pending_dwell() {
    let mut editor = editor("prose\n");
    let (x, y) = centre_of(&editor, Action::Bold);
    editor.hover(x, y);
    let dwell = editor.next_wake().expect("hovering arms the dwell");

    editor.schedule_reconcile();
    let next = editor.next_wake().expect("both are pending");

    assert!(next <= dwell, "the dwell was lost behind a reconcile");
}

/// And with nothing pending the loop is allowed to sleep. A wake-up that never cleared would turn
/// `ControlFlow::Wait` into a busy timer — which is exactly what an editor must not be when nobody
/// is typing.
#[test]
fn nothing_pending_means_no_wake_up() {
    let mut editor = editor("prose\n");
    assert_eq!(editor.next_wake(), None);

    let (x, y) = centre_of(&editor, Action::Bold);
    editor.hover(x, y);
    assert!(editor.next_wake().is_some());

    editor.hover(50.0, toolbar::HEIGHT + 40.0);
    assert_eq!(editor.next_wake(), None, "moving off disarms it");
}

// ---- the panel itself ----

/// A tooltip is drawn over the document, and must not take a click with it.
///
/// **Compared against the same click with no tooltip**, rather than asserted against a byte. The
/// claim is that the panel makes no difference to where a click goes, and stating it that way is
/// what makes the test impossible to satisfy by accident: it would still fail if a `click_tooltip`
/// were added to the chain, and it cannot go vacuous if the panel's geometry moves off the text.
#[test]
fn the_panel_never_swallows_a_click() {
    let source = "some prose here\nand a second line\n";

    // Where the panel is: directly under the Bold button, below the bar. Its lower edge is inside
    // the document on a default window, which is the case that matters.
    let reference = editor(source);
    let button = reference
        .toolbar_buttons()
        .iter()
        .find(|button| button.action == Action::Bold)
        .copied()
        .unwrap();
    let at = (
        button.x + button.width / 2.0,
        toolbar::HEIGHT + tooltip::GAP + tooltip::HEIGHT - 1.0,
    );

    let mut without = editor(source);
    without.click(at.0, at.1);

    let mut with = editor(source);
    rest_on(&mut with, Action::Bold);
    assert!(
        with.tooltip().is_some(),
        "there is a panel to click through"
    );
    with.click(at.0, at.1);

    assert_eq!(
        with.document().selection(),
        without.document().selection(),
        "the panel changed where a click landed"
    );
}

/// The rightmost button's panel is wider than what is left of the window beside it, so it has to
/// slide left rather than hang off the edge.
#[test]
fn a_panel_at_the_right_edge_stays_inside_the_window() {
    let width = 400.0;
    let panel = tooltip::width_of(120.0);
    let (x, _) = tooltip::place(panel, width - 5.0, toolbar::HEIGHT, width);

    assert!(x >= 0.0, "and not off the left either");
    assert!(
        x + panel <= width,
        "a panel from {x} for {panel} runs past {width}"
    );
}

/// In a window narrower than the panel the left edge wins, because text clipped at the right is
/// readable from its start and text clipped at the left is not. Same rule as the floating bar's.
#[test]
fn a_window_narrower_than_the_panel_keeps_its_left_edge() {
    let (x, _) = tooltip::place(300.0, 40.0, toolbar::HEIGHT, 80.0);
    assert_eq!(x, 0.0);
}

/// The panel sits below the bar, never over it. A tooltip covering the row of buttons the pointer is
/// travelling would be the worst possible place for it.
#[test]
fn the_panel_is_below_the_bar() {
    let (_, y) = tooltip::place(100.0, 50.0, toolbar::HEIGHT, WIDTH);
    assert!(y >= toolbar::HEIGHT, "a panel at {y} overlaps the bar");
}

/// The panel reaches the frame, drawn in the palette's `panel` role over a `line` edge — both of
/// which had no consumer at all before this.
#[test]
fn the_panel_is_drawn() {
    let mut editor = editor("prose\n");
    // Nothing here calls `with_config`, so the palette is `Theme::default()` — see `caret_line.rs`
    // for what happens when a colour assertion forgets that.
    assert_eq!(editor.appearance(), Appearance::Dark);
    let theme = Theme::default();

    let without = count_of(&mut editor, theme.panel);
    rest_on(&mut editor, Action::Bold);
    let with = count_of(&mut editor, theme.panel);

    assert!(
        with > without,
        "the panel's ground should be on the frame: {without} then {with}"
    );
    assert!(count_of(&mut editor, theme.line) > 0, "and its edge");
}

fn count_of(editor: &mut Editor<Grid>, colour: Rgba) -> usize {
    editor
        .build_frame(WIDTH, HEIGHT, 1.0)
        .iter()
        .filter(|quad| quad.colour == [colour.r, colour.g, colour.b, colour.a])
        .count()
}

/// Laid out once and kept, like every other piece of chrome text here. The per-frame-layout trap has
/// been paid for often enough that a new cache is asserted rather than intended.
#[test]
fn a_still_frame_with_a_tooltip_lays_out_nothing_new() {
    let mut editor = editor("prose\n");
    rest_on(&mut editor, Action::Bold);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let after_first = editor.text_backend().layouts();

    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let since = editor.text_backend().layouts() - after_first;

    assert!(
        since <= 2,
        "a second frame should lay out the caret's chunk at most, not {since}"
    );
}

/// The dwell is a constant to change in one place, and a number to try in the window rather than
/// argue about here. Pinned so that changing it is a decision.
#[test]
fn the_dwell_is_shorter_than_the_windows_default() {
    assert!(tooltip::DWELL <= Duration::from_millis(500));
    assert!(
        tooltip::DWELL >= Duration::from_millis(200),
        "not a flicker"
    );
}

/// A sanity check on the harness itself: `next_wake` is a real instant in the future, not a stub.
#[test]
fn the_dwell_deadline_is_about_a_dwell_away() {
    let mut editor = editor("prose\n");
    let before = Instant::now();
    let (x, y) = centre_of(&editor, Action::Bold);
    editor.hover(x, y);

    let at = editor.next_wake().unwrap();
    assert!(at >= before + tooltip::DWELL);
    assert!(at <= Instant::now() + tooltip::DWELL);
}
