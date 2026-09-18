//! Tab strip geometry and hit-testing.
//!
//! A click landing on the wrong tab, or closing one when it meant to select it, is invisible in a
//! screenshot and obvious to a user. So the arithmetic is tested rather than eyeballed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::file_icon::Icon;
use mdedit_app::gpu::QuadInstance;
use mdedit_app::tabs::{self, Hit, STRIP_HEIGHT};
use mdedit_app::toolbar;
use mdedit_app::view::Chrome;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 400.0;

#[test]
fn tabs_are_laid_out_left_to_right_without_gaps_or_overlap() {
    let rects = tabs::layout(&[80.0, 40.0, 120.0], 1000.0);
    assert_eq!(rects.len(), 3);

    assert_eq!(rects[0].x, 0.0, "the first tab starts at the left edge");
    for pair in rects.windows(2) {
        assert_eq!(
            pair[0].right(),
            pair[1].x,
            "tabs must abut exactly: no gap to click through, no overlap to fight over"
        );
    }
}

#[test]
fn a_tab_is_never_narrower_than_its_click_target_or_wider_than_the_cap() {
    // A one-character name still needs something to hit.
    let narrow = tabs::layout(&[1.0], 1000.0);
    assert!(narrow[0].width >= 70.0, "got {}", narrow[0].width);

    // And one very long name must not consume the whole strip.
    let wide = tabs::layout(&[10_000.0], 1000.0);
    // The cap is on the name's room; the icon's column goes on top of it (MDEDIT-178).
    assert!(
        wide[0].width <= 200.0 + tabs::ICON_WIDTH,
        "got {}",
        wide[0].width
    );
}

/// The same discipline the document renderer is missing: do not lay out what cannot be seen.
#[test]
fn only_tabs_that_fit_are_laid_out() {
    let many = vec![100.0_f32; 200];
    let rects = tabs::layout(&many, 400.0);

    assert!(
        rects.len() < 10,
        "200 tabs in a 400px strip should produce a handful of rects, got {}",
        rects.len()
    );
    assert!(!rects.is_empty(), "at least one tab is visible");
    assert!(
        rects.last().unwrap().x < 400.0,
        "no tab should start past the right edge"
    );
}

#[test]
fn clicking_a_tab_body_selects_it() {
    let rects = tabs::layout(&[80.0, 80.0, 80.0], 1000.0);

    // A few pixels in from each tab's left edge is unambiguously its body.
    for rect in &rects {
        let hit = tabs::hit_test(&rects, rect.x + 5.0, STRIP_HEIGHT / 2.0);
        assert_eq!(hit, Some(Hit::Select(rect.index)), "at x {}", rect.x + 5.0);
    }
}

/// The close region sits inside the tab, so the body must not swallow it.
#[test]
fn clicking_the_close_region_closes_rather_than_selects() {
    let rects = tabs::layout(&[80.0, 80.0], 1000.0);

    for rect in &rects {
        let (close_x, close_width) = rect.close_region();
        let middle = close_x + close_width / 2.0;
        assert_eq!(
            tabs::hit_test(&rects, middle, STRIP_HEIGHT / 2.0),
            Some(Hit::Close(rect.index)),
            "the close region of tab {} must close it",
            rect.index
        );

        // And one pixel to its left is still the body.
        assert_eq!(
            tabs::hit_test(&rects, close_x - 1.0, STRIP_HEIGHT / 2.0),
            Some(Hit::Select(rect.index))
        );
    }
}

#[test]
fn clicking_below_the_strip_is_not_a_tab_click() {
    let rects = tabs::layout(&[80.0], 1000.0);
    assert_eq!(tabs::hit_test(&rects, 10.0, STRIP_HEIGHT + 1.0), None);
    assert_eq!(tabs::hit_test(&rects, 10.0, STRIP_HEIGHT), None);
    assert_eq!(
        tabs::hit_test(&rects, 10.0, STRIP_HEIGHT - 1.0),
        Some(Hit::Select(0)),
        "the last row of the strip still belongs to it"
    );
}

#[test]
fn clicking_empty_strip_space_hits_nothing() {
    let rects = tabs::layout(&[80.0], 1000.0);
    let past_the_end = rects[0].right() + 50.0;
    assert_eq!(tabs::hit_test(&rects, past_the_end, 5.0), None);
}

#[test]
fn no_tabs_means_no_hits() {
    assert_eq!(tabs::hit_test(&[], 10.0, 10.0), None);
    assert!(tabs::layout(&[], 1000.0).is_empty());
}

#[test]
fn a_strip_narrower_than_one_tab_still_shows_one() {
    // Better a clipped tab than an empty strip that looks broken.
    let rects = tabs::layout(&[80.0, 80.0], 10.0);
    assert_eq!(rects.len(), 1);
    assert_eq!(rects[0].index, 0);
}

#[test]
fn the_label_never_runs_under_the_close_region() {
    for width in [1.0_f32, 40.0, 500.0] {
        let rects = tabs::layout(&[width], 1000.0);
        let rect = rects[0];
        let (close_x, _) = rect.close_region();
        assert!(
            rect.label_x() + rect.label_width() <= close_x,
            "label for width {width} overlaps the close region"
        );
    }
}

// ---------------------------------------------------------------------------
// The type icon (MDEDIT-178).
// ---------------------------------------------------------------------------

/// **The icon is added to the tab, not taken from its name.** Every name that had room before
/// MDEDIT-178 still has it: a name narrow enough not to be capped fits its tab whole. Taking the
/// icon out of the label's width instead — as the fixed-width pane has to — would fail this for
/// every name within eighteen pixels of the cap.
#[test]
fn a_name_keeps_the_room_it_had() {
    for name in [1.0_f32, 40.0, 100.0, 156.0] {
        let rect = tabs::layout(&[name], 1000.0)[0];
        assert!(
            rect.label_width() >= name,
            "a {name} px name has {} px of room",
            rect.label_width(),
        );
    }
}

/// The name starts clear of the icon, and the icon is at the tab's left.
#[test]
fn the_name_starts_after_the_icon() {
    let rects = tabs::layout(&[80.0, 80.0], 1000.0);
    for rect in &rects {
        assert!(rect.icon_x() > rect.x, "the icon is inside its own tab");
        assert_eq!(rect.label_x(), rect.icon_x() + tabs::ICON_WIDTH);
    }
}

/// **A click on the icon selects the tab**, as a click on its name does, and the close region still
/// wins over the body.
#[test]
fn a_click_on_the_icon_selects_the_tab() {
    let rects = tabs::layout(&[80.0, 80.0, 80.0], 1000.0);
    for rect in &rects {
        let at = rect.icon_x() + tabs::ICON_WIDTH / 2.0;
        assert_eq!(
            tabs::hit_test(&rects, at, STRIP_HEIGHT / 2.0),
            Some(Hit::Select(rect.index)),
        );
        let (close_x, close_width) = rect.close_region();
        assert_eq!(
            tabs::hit_test(&rects, close_x + close_width / 2.0, STRIP_HEIGHT / 2.0),
            Some(Hit::Close(rect.index)),
        );
    }
}

/// An editor with one file per name open on the strip, and one frame drawn by `grid`.
fn strip_of(dir: &TempDir, names: &[&str], grid: Grid) -> (Editor<Grid>, Vec<QuadInstance>) {
    let mut workspace = Workspace::new();
    for name in names {
        let path = dir.path().join(name);
        std::fs::write(&path, "text\n").unwrap();
        workspace.add(path);
    }
    let mut editor = Editor::with_workspace(workspace, grid);
    editor.activate_saved_tab();
    assert_eq!(editor.chrome(), Chrome::Strip);
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    (editor, quads)
}

/// How many glyphs were drawn in a tab's icon column.
fn glyphs_in_icon_column(rect: tabs::TabRect, quads: &[QuadInstance]) -> usize {
    let top = toolbar::HEIGHT;
    quads
        .iter()
        .filter(|quad| {
            // A glyph, not one of the strip or tab fills: those carry the whole-texture uv.
            quad.uv != [0.0, 0.0, 1.0, 1.0]
                && quad.rect[0] >= rect.icon_x()
                && quad.rect[0] < rect.label_x()
                && quad.rect[1] >= top
                && quad.rect[1] < top + STRIP_HEIGHT
        })
        .count()
}

/// **Without the icon font, each tab draws its file's badge, not `.notdef`.** The grid stub answers
/// `.notdef` for every private-use code point by default (MDEDIT-92), so this is the fallback — and
/// counting letters is what tells a badge from one box of tofu, and TXT from MD or {}. So it also
/// says each tab asked for its *own* file's icon.
#[test]
fn without_the_icon_font_each_tab_draws_its_badge() {
    let dir = TempDir::new().unwrap();
    let (editor, quads) = strip_of(&dir, &["a.md", "b.json", "c.txt"], Grid::new());

    let rects = editor.tab_rects();
    assert_eq!(rects.len(), 3, "all three fit");
    for (rect, want) in rects.iter().zip([Icon::Note, Icon::Structured, Icon::Text]) {
        assert_eq!(
            glyphs_in_icon_column(*rect, &quads),
            want.badge().chars().count(),
            "tab {} should hold {:?}",
            rect.index,
            want.badge(),
        );
    }
}

/// **With the icon font, the column holds the icon: one glyph, not the badge.** Only a run that
/// asked for `FontFamily::Icon` resolves a private-use code point on `Grid::with_icon_font`, as only
/// the icon font does on Windows — so this is what says the tab asked for the right family.
#[test]
fn with_the_icon_font_each_tab_draws_its_icon() {
    let dir = TempDir::new().unwrap();
    let (editor, quads) = strip_of(&dir, &["a.md", "b.json", "c.txt"], Grid::with_icon_font());

    for rect in editor.tab_rects() {
        assert_eq!(
            glyphs_in_icon_column(*rect, &quads),
            1,
            "tab {} should hold one icon glyph",
            rect.index,
        );
    }
}
