//! Following the OS theme, and switching palettes while running (MDEDIT-78).
//!
//! The light palette became reachable here. Half the design pack was unused until it did.
//!
//! What cannot be tested: the registry read itself, and whether Windows tells us it changed. Both
//! are platform behaviour with no headless equivalent — `ThemeChoice::resolve` takes what Windows
//! said as an argument precisely so the *decision* can be tested without asking it, and the event
//! that triggers a re-read is verified by hand on Windows.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    // Quad colours and sizes come from the same constants the renderer read.
    clippy::float_cmp
)]

use mdedit_app::config::{Config, Settings};
use mdedit_app::editor::Editor;
use mdedit_app::view::{Appearance, Rgba, Theme, ThemeChoice};
use mdedit_menu_win::{AppearanceChoice, Command};
use tempfile::TempDir;

use crate::stub;
use stub::Grid;

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 400.0;

fn editor() -> Editor<Grid> {
    let mut editor = Editor::new("# Heading\n\nprose, and `code`\n", Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// Every colour a frame is drawn in.
fn palette_used(editor: &mut Editor<Grid>) -> Vec<[f32; 4]> {
    let mut colours: Vec<[f32; 4]> = editor
        .build_frame(WIDTH, HEIGHT, 1.0)
        .iter()
        .map(|quad| quad.colour)
        .collect();
    colours.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    colours.dedup();
    colours
}

fn is(colour: Rgba) -> [f32; 4] {
    [colour.r, colour.g, colour.b, colour.a]
}

// ---- what a choice means ----

/// The decision, without asking Windows anything: `resolve` takes what Windows said as an argument.
#[test]
fn a_choice_resolves_against_what_windows_said() {
    assert_eq!(ThemeChoice::System.resolve(Some(true)), Appearance::Light);
    assert_eq!(ThemeChoice::System.resolve(Some(false)), Appearance::Dark);
    // Windows did not say — an old build, or a user who never opened the personalisation page.
    assert_eq!(ThemeChoice::System.resolve(None), Appearance::Dark);

    // Pinned means pinned, whatever Windows says or does not.
    for said in [Some(true), Some(false), None] {
        assert_eq!(ThemeChoice::Dark.resolve(said), Appearance::Dark);
        assert_eq!(ThemeChoice::Light.resolve(said), Appearance::Light);
    }
}

#[test]
fn the_default_choice_is_to_follow_windows() {
    assert_eq!(Config::default().theme, ThemeChoice::System);
}

// ---- switching, in the editor ----

#[test]
fn pinning_light_draws_the_light_palette() {
    let mut editor = editor();
    assert!(editor.set_theme_choice(ThemeChoice::Light));
    assert_eq!(editor.appearance(), Appearance::Light);

    let colours = palette_used(&mut editor);
    assert!(
        colours.contains(&is(Theme::light().text)),
        "nothing was drawn in the light palette's text colour"
    );
}

/// **The failure to look for**, and the ticket says so: a cache that kept its colours would leave
/// half the window in the palette that is no longer chosen.
#[test]
fn switching_leaves_nothing_drawn_at_the_old_colours() {
    let mut editor = editor();
    let dark = Theme::dark();
    let before = palette_used(&mut editor);
    assert!(before.contains(&is(dark.text)));

    editor.set_theme_choice(ThemeChoice::Light);
    let after = palette_used(&mut editor);

    // **Every role, not seven of them** (MDEDIT-110). Seven was enough to catch a switch that did
    // nothing at all and nothing narrower — a single role left behind is exactly the shape a missed
    // draw site takes.
    let light = Theme::light();
    for (role, dark_colour, light_colour) in roles(&dark, &light) {
        // A role the two palettes spell the same way cannot be told apart by its colour, and there
        // is nothing to assert about it. `token_punctuation` and friends may legitimately match.
        if dark_colour == light_colour {
            continue;
        }
        assert!(
            !after.contains(&is(dark_colour)),
            "a quad is still drawn in the dark palette's {role} after switching to light"
        );
    }
}

/// Every colour role, paired between the two palettes.
///
/// Hand-kept, because nothing can derive it from a struct — the same position `chordlist` is in. A
/// role added and not listed here is simply unchecked, which is the failure mode to know about.
fn roles(dark: &Theme, light: &Theme) -> Vec<(&'static str, Rgba, Rgba)> {
    vec![
        ("background", dark.background, light.background),
        ("text", dark.text, light.text),
        ("marker", dark.marker, light.marker),
        ("caret", dark.caret, light.caret),
        ("selection", dark.selection, light.selection),
        ("find_match", dark.find_match, light.find_match),
        ("status_error", dark.status_error, light.status_error),
        ("span_error", dark.span_error, light.span_error),
        ("warn", dark.warn, light.warn),
        ("link", dark.link, light.link),
        ("muted", dark.muted, light.muted),
        ("gutter", dark.gutter, light.gutter),
        ("caret_line", dark.caret_line, light.caret_line),
        ("line", dark.line, light.line),
        ("panel", dark.panel, light.panel),
        ("strip", dark.strip, light.strip),
        ("tab_active", dark.tab_active, light.tab_active),
        ("tab_inactive", dark.tab_inactive, light.tab_inactive),
        ("tab_modified", dark.tab_modified, light.tab_modified),
        ("token_key", dark.token_key, light.token_key),
        ("token_string", dark.token_string, light.token_string),
        ("token_number", dark.token_number, light.token_number),
        ("token_literal", dark.token_literal, light.token_literal),
        (
            "token_punctuation",
            dark.token_punctuation,
            light.token_punctuation,
        ),
        ("token_comment", dark.token_comment, light.token_comment),
        (
            "scrollbar_thumb",
            dark.scrollbar_thumb,
            light.scrollbar_thumb,
        ),
    ]
}

/// And back again, because a switch that only works one way is half a feature.
#[test]
fn switching_back_restores_the_dark_palette() {
    let mut editor = editor();
    editor.set_theme_choice(ThemeChoice::Light);
    editor.set_theme_choice(ThemeChoice::Dark);

    assert_eq!(editor.appearance(), Appearance::Dark);
    let colours = palette_used(&mut editor);
    assert!(colours.contains(&is(Theme::dark().text)));
    assert!(!colours.contains(&is(Theme::light().text)));
}

/// Nothing is cleared on a switch, and this is why that is allowed: colour is not in `TextStyle` and
/// not in `ChunkKey`, so a laid-out chunk is the same in both palettes. If that ever stops being
/// true, this test says so by the layout count moving.
#[test]
fn a_switch_lays_nothing_out_again() {
    let mut editor = editor();

    // What an ordinary frame costs. Never zero: the chunk holding the caret is deliberately laid
    // out fresh every time, because what it draws depends on where in it the caret sits.
    let start = editor.text_backend().layouts();
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let ordinary = editor.text_backend().layouts() - start;

    let before = editor.text_backend().layouts();
    editor.set_theme_choice(ThemeChoice::Light);
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let after_switch = editor.text_backend().layouts() - before;

    assert_eq!(
        after_switch, ordinary,
        "the frame after a palette switch laid out more than an ordinary one, which means colour \
         has got into a layout key and the switch needs a cache clear"
    );
}

/// Choosing what is already chosen changes nothing, so a caller can skip the redraw.
#[test]
fn choosing_the_same_thing_twice_is_not_a_change() {
    let mut editor = editor();
    assert!(editor.set_theme_choice(ThemeChoice::Light));
    assert!(!editor.set_theme_choice(ThemeChoice::Light));
}

/// A pinned palette ignores Windows changing its mind. That is what pinning is.
#[test]
fn a_pinned_palette_ignores_the_system_changing() {
    let mut editor = editor();
    editor.set_theme_choice(ThemeChoice::Dark);
    assert!(!editor.system_appearance_changed());
    assert_eq!(editor.appearance(), Appearance::Dark);
}

// ---- the menu, and the file ----

#[test]
fn the_menu_commands_choose_a_palette() {
    let mut editor = editor();

    assert!(editor.apply_command(Command::SetAppearance(AppearanceChoice::Light)));
    assert_eq!(editor.appearance(), Appearance::Light);

    assert!(editor.apply_command(Command::SetAppearance(AppearanceChoice::Dark)));
    assert_eq!(editor.appearance(), Appearance::Dark);

    // Following Windows again. What that resolves to depends on the machine, so what is asserted is
    // that the *choice* was taken, not which palette it produced.
    editor.apply_command(Command::SetAppearance(AppearanceChoice::System));
    assert_eq!(editor.config().theme, ThemeChoice::System);
}

/// It persists like every other setting — through the settings' write path, not on exit.
#[test]
fn the_choice_is_written_to_the_settings() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("config.json");

    let mut editor = Editor::new("prose\n", Grid::new());
    editor.with_settings(Settings::at(file.clone()));
    editor.set_theme_choice(ThemeChoice::Light);
    editor.on_idle();

    let stored = Config::load_from(&file);
    assert_eq!(stored.theme, ThemeChoice::Light);
    // And it is the word, not a number or a path.
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("\"theme\": \"light\""),
        "the config should hold the choice as a word: {text}"
    );
}

/// A stored choice has to reach the palette on open, not merely sit in the file.
#[test]
fn a_stored_choice_is_applied_on_open() {
    let mut editor = Editor::new("prose\n", Grid::new());
    editor.with_config(Config {
        theme: ThemeChoice::Light,
        ..Config::default()
    });

    assert_eq!(editor.appearance(), Appearance::Light);
}

/// And it must not undo the zoom on its way in. Two settings applied at startup, in an order that
/// used to matter and now does not.
#[test]
fn a_stored_choice_and_a_stored_zoom_both_survive() {
    let mut editor = Editor::new("prose\n", Grid::new());
    editor.with_config(Config {
        theme: ThemeChoice::Light,
        zoom: 2.0,
        ..Config::default()
    });

    assert_eq!(editor.appearance(), Appearance::Light);
    assert_eq!(editor.body_size(), Theme::light().body_size * 2.0);
}

/// Switching palettes keeps the zoom, for the same reason: `body_size` is a scaled value and the
/// palette's own is not.
#[test]
fn a_switch_keeps_the_zoom() {
    let mut editor = Editor::new("prose\n", Grid::new());
    editor.with_config(Config {
        zoom: 1.5,
        ..Config::default()
    });
    let zoomed = editor.body_size();

    editor.set_theme_choice(ThemeChoice::Light);
    assert_eq!(editor.body_size(), zoomed);
}
