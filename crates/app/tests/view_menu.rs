//! The View menu's commands.
//!
//! A native menu cannot be driven by a test — which is exactly why `Command` exists, and why every
//! command is reachable without one. These call `apply_command`, which is the same thing the menu
//! calls. The menu itself is verified by hand on Windows.
//!
//! The ticks are not asserted here: `MenuBar` only exists once there is a window, so `set_view` has
//! nowhere to be observed from a headless test. What *is* asserted is that every path which changes
//! a toggle goes through the same state the ticks are computed from.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::config::{Config, Settings};
use mdedit_app::editor::Editor;
use mdedit_app::view::{Chrome, Mode};
use mdedit_menu_win::Command;

mod stub;
use stub::Grid;

fn fresh() -> Editor<Grid> {
    Editor::new("# Heading\n\nprose\n", Grid::new())
}

#[test]
fn plain_markup_toggles_from_the_menu() {
    let mut editor = fresh();
    assert_eq!(editor.mode(), Mode::Preview);

    editor.apply_command(Command::PlainMarkup);
    assert_eq!(editor.mode(), Mode::Plain);

    editor.apply_command(Command::PlainMarkup);
    assert_eq!(editor.mode(), Mode::Preview);
}

#[test]
fn the_file_pane_toggles_from_the_menu() {
    let mut editor = fresh();
    assert_eq!(editor.chrome(), Chrome::Strip);

    editor.apply_command(Command::FilePane);
    assert_eq!(editor.chrome(), Chrome::Pane);

    editor.apply_command(Command::FilePane);
    assert_eq!(editor.chrome(), Chrome::Strip);
}

/// The setting existed from MDEDIT-22 and could only be changed by editing the file and restarting.
#[test]
fn line_numbers_toggle_from_the_menu() {
    let mut editor = fresh();
    assert!(!editor.config().line_numbers);

    editor.apply_command(Command::LineNumbers);
    assert!(editor.config().line_numbers);

    editor.apply_command(Command::LineNumbers);
    assert!(!editor.config().line_numbers);
}

#[test]
fn the_zoom_items_do_what_the_chords_do() {
    let mut editor = fresh();
    let normal = editor.body_size();

    editor.apply_command(Command::ZoomIn);
    assert!(editor.body_size() > normal);

    editor.apply_command(Command::ZoomOut);
    editor.apply_command(Command::ZoomOut);
    assert!(editor.body_size() < normal);

    editor.apply_command(Command::ResetZoom);
    assert_eq!(editor.body_size(), normal);
}

/// The chord and the menu item must be the same operation, not two implementations that agree
/// today. Asserted by driving both and comparing.
#[test]
fn the_menu_and_the_chord_reach_the_same_place() {
    let mut by_chord = fresh();
    by_chord.apply_chord("/", false);

    let mut by_menu = fresh();
    by_menu.apply_command(Command::PlainMarkup);

    assert_eq!(by_chord.mode(), by_menu.mode());

    let mut chrome_chord = fresh();
    chrome_chord.apply_chord("E", true);
    let mut chrome_menu = fresh();
    chrome_menu.apply_command(Command::FilePane);
    assert_eq!(chrome_chord.chrome(), chrome_menu.chrome());
}

/// Neither fires twice. muda's accelerators are labels and do not fire — if they ever did, a
/// Ctrl+/ would toggle and toggle back, and this is the test that would notice.
#[test]
fn one_press_is_one_toggle() {
    let mut editor = fresh();
    editor.apply_chord("/", false);
    assert_eq!(
        editor.mode(),
        Mode::Plain,
        "a single press must land on the other mode, not back on this one"
    );
}

/// Line numbers change the document's width, so every chunk wraps differently — a layout cached at
/// the old width would draw the old wrapping.
#[test]
fn toggling_line_numbers_lays_the_document_out_again() {
    let mut editor = Editor::new(
        "some fairly long prose to wrap\n".repeat(40).as_str(),
        Grid::new(),
    );
    editor.build_frame(600.0, 400.0, 1.0);
    editor.build_frame(600.0, 400.0, 1.0);

    let before = editor.text_backend().layouts();
    editor.apply_command(Command::LineNumbers);
    editor.build_frame(600.0, 400.0, 1.0);

    assert!(
        editor.text_backend().layouts() > before,
        "the gutter changes the wrapping width, so the chunks must be laid out again"
    );
}

/// All three toggles persist, which is what MDEDIT-60 was built for and what makes the menu worth
/// having rather than a per-run curiosity.
#[test]
fn the_view_choices_survive_a_restart() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("config.json");

    let mut editor = fresh();
    editor.with_settings(Settings::at(file.clone()));
    editor.apply_command(Command::FilePane);
    editor.apply_command(Command::LineNumbers);
    editor.apply_command(Command::ZoomIn);
    editor.on_idle();

    let mut next = fresh();
    next.with_settings(Settings::at(file));
    assert_eq!(next.chrome(), Chrome::Pane);
    assert!(next.config().line_numbers);
    assert!(next.zoom() > 1.0);
}

/// Plain markup is the one that does *not* persist, and that is deliberate: the session file records
/// handles rather than view state, and the mode is global across every tab. Pinned so that changing
/// it is a decision rather than an accident.
#[test]
fn plain_markup_does_not_persist() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("config.json");

    let mut editor = fresh();
    editor.with_settings(Settings::at(file.clone()));
    editor.apply_command(Command::PlainMarkup);
    editor.on_idle();

    let mut next = fresh();
    next.with_settings(Settings::at(file));
    assert_eq!(next.mode(), Mode::Preview);
}

/// A stored choice has to reach the editor, not just sit in the file.
#[test]
fn stored_view_choices_are_applied_on_open() {
    let mut editor = fresh();
    editor.with_config(Config {
        chrome: Chrome::Pane,
        line_numbers: true,
        zoom: 1.5,
        ..Config::default()
    });

    assert_eq!(editor.chrome(), Chrome::Pane);
    assert!(editor.config().line_numbers);
}
