//! Tabs that do not fit the strip are still reachable from it (MDEDIT-179).
//!
//! Before this, `tabs::layout` stopped at the edge and said nothing about what it left off: a tab
//! past the edge could not be reached from the strip at all, and because the strip always started at
//! tab zero, the *active* tab could be one of the missing ones. The answer is the formatting bar's
//! (MDEDIT-116): an overflow button listing the rest, in `popup`'s list — plus keeping the active
//! tab in view, which the bar has no equivalent of.
//!
//! The geometry is asserted without a window, and the button and its list through the editor.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::cast_precision_loss
)]

use std::cell::Cell;

use mdedit_app::editor::Editor;
use mdedit_app::file_icon::Icon;
use mdedit_app::gpu::QuadInstance;
use mdedit_app::popup;
use mdedit_app::tabs::{self, Hit, STRIP_HEIGHT};
use mdedit_app::toolbar;
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 400.0;

// ---------------------------------------------------------------------------
// Geometry, without a window.
// ---------------------------------------------------------------------------

/// Every name the same width, so the arithmetic in a failure message is easy to follow.
fn same(_: usize) -> f32 {
    80.0
}

/// Everything fits: no button, nothing hidden, and the tabs are exactly what `layout` gives.
#[test]
fn a_strip_that_fits_has_no_overflow() {
    let strip = tabs::overflowing(3, Some(2), 1000.0, same);
    assert!(strip.hidden.is_empty());
    assert_eq!(strip.more, None);
    assert_eq!(strip.tabs, tabs::layout(&[80.0; 3], 1000.0));
}

/// **Every tab is either drawn or listed, never both and never neither.** The two halves come from
/// one call for exactly this reason — the toolbar's `every_button_is_either_drawn_or_in_the_overflow`
/// is the same rule for the same partition.
#[test]
fn every_tab_is_either_drawn_or_listed() {
    for count in [1, 2, 7, 30] {
        for active in [None, Some(0), Some(count / 2), Some(count - 1)] {
            for width in [50.0_f32, 300.0, 700.0, 5000.0] {
                let strip = tabs::overflowing(count, active, width, same);
                let mut seen: Vec<usize> = strip.tabs.iter().map(|rect| rect.index).collect();
                seen.extend(&strip.hidden);
                seen.sort_unstable();
                assert_eq!(
                    seen,
                    (0..count).collect::<Vec<_>>(),
                    "count {count}, active {active:?}, width {width}",
                );
                assert_eq!(strip.more.is_some(), !strip.hidden.is_empty());
            }
        }
    }
}

/// **The active tab is always drawn**, however far along it is. This is the half of the ticket
/// that was a defect rather than a design choice: switching to a tab past the edge used to light
/// nothing on the strip.
#[test]
fn the_active_tab_is_always_drawn() {
    for active in 0..30 {
        let strip = tabs::overflowing(30, Some(active), 700.0, same);
        assert!(
            strip.tabs.iter().any(|rect| rect.index == active),
            "tab {active} is active and not on the strip: {:?}",
            strip.tabs.iter().map(|rect| rect.index).collect::<Vec<_>>(),
        );
    }
}

/// An active tab that already fits counting from the first does not move the strip. Scrolling it
/// whenever something past the start is active would reshuffle the tabs under the pointer on every
/// click.
#[test]
fn an_active_tab_that_fits_does_not_move_the_strip() {
    let from_the_start = tabs::overflowing(30, None, 700.0, same);
    let last_shown = from_the_start.tabs.last().unwrap().index;
    for active in 0..=last_shown {
        let strip = tabs::overflowing(30, Some(active), 700.0, same);
        assert_eq!(strip.tabs, from_the_start.tabs, "active {active}");
    }
}

/// The drawn tabs are a contiguous run in order, abutting from the left edge, with the button after
/// the last one and nothing past the strip's right edge.
#[test]
fn an_overflowing_strip_is_a_run_of_whole_tabs_then_the_button() {
    for active in [None, Some(0), Some(15), Some(29)] {
        let strip = tabs::overflowing(30, active, 700.0, same);
        assert_eq!(strip.tabs[0].x, 0.0);
        for pair in strip.tabs.windows(2) {
            assert_eq!(pair[0].index + 1, pair[1].index, "in order, none skipped");
            assert_eq!(pair[0].right(), pair[1].x, "abutting");
        }
        let more = strip.more.expect("thirty tabs overflow 700 px");
        assert_eq!(more, strip.tabs.last().unwrap().right());
        assert!(
            more + tabs::MORE_WIDTH <= 700.0,
            "the button ends at {} in a 700 px strip",
            more + tabs::MORE_WIDTH,
        );
    }
}

/// A strip too narrow for even one tab still draws one, and it is the active one — better a
/// clipped tab than an empty strip that looks broken.
#[test]
fn a_strip_narrower_than_one_tab_still_draws_the_active_one() {
    let strip = tabs::overflowing(5, Some(3), 20.0, same);
    assert_eq!(strip.tabs.len(), 1);
    assert_eq!(strip.tabs[0].index, 3);
    assert_eq!(strip.hidden, vec![0, 1, 2, 4]);
}

/// **Only what might be drawn is measured.** Two hundred tabs in a strip with room for four must
/// not shape two hundred names every frame — the rule `layout` documents and the reason the width
/// is a closure rather than a slice. The bound is the run from the start, which finds whether it
/// overflows at all, and the run around the active tab: each what fits plus the one past the edge.
#[test]
fn only_tabs_that_might_be_drawn_are_measured() {
    for active in [None, Some(0), Some(100), Some(199)] {
        let calls = Cell::new(0);
        let strip = tabs::overflowing(200, active, 700.0, |_| {
            calls.set(calls.get() + 1);
            80.0
        });
        assert!(
            calls.get() <= (strip.tabs.len() + 1) * 2 + 1,
            "{} names measured to draw {} tabs, active {active:?}",
            calls.get(),
            strip.tabs.len(),
        );
    }
}

/// The button is its own target, and the tabs and their close regions are unchanged beside it.
#[test]
fn the_button_is_hit_and_the_tabs_still_are() {
    let strip = tabs::overflowing(30, Some(0), 700.0, same);
    let more = strip.more.unwrap();
    let middle = STRIP_HEIGHT / 2.0;

    assert_eq!(
        strip.hit_test(more + tabs::MORE_WIDTH / 2.0, middle),
        Some(Hit::More)
    );
    assert_eq!(strip.hit_test(more + tabs::MORE_WIDTH + 1.0, middle), None);
    assert_eq!(
        strip.hit_test(more + 1.0, STRIP_HEIGHT),
        None,
        "below the strip"
    );

    let first = strip.tabs[0];
    assert_eq!(strip.hit_test(first.x + 5.0, middle), Some(Hit::Select(0)));
    let (close_x, close_width) = first.close_region();
    assert_eq!(
        strip.hit_test(close_x + close_width / 2.0, middle),
        Some(Hit::Close(0))
    );
}

// ---------------------------------------------------------------------------
// The button and its list, through the editor.
// ---------------------------------------------------------------------------

/// These files open on the strip, the first active, and one frame drawn at [`WIDTH`].
fn editor_with(dir: &TempDir, names: &[String]) -> Editor<Grid> {
    let mut workspace = Workspace::new();
    for name in names {
        let path = dir.path().join(name);
        std::fs::write(&path, "text\n").unwrap();
        workspace.add(path);
    }
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// Which of [`twelve`] is plain text, so its list row can be told from its neighbours by its badge.
const PLAIN: usize = 6;

/// Twelve names of one width, which overflow a 900 px strip at six.
fn twelve() -> Vec<String> {
    (0..12)
        .map(|n| {
            let extension = if n == PLAIN { "txt" } else { "md" };
            format!("file-{n:02}.{extension}")
        })
        .collect()
}

/// Where the strip's overflow button is, in window coordinates.
fn on_more(editor: &Editor<Grid>) -> (f32, f32) {
    let more = editor.strip().more.expect("the strip overflows");
    (
        more + tabs::MORE_WIDTH / 2.0,
        toolbar::HEIGHT + STRIP_HEIGHT / 2.0,
    )
}

/// **The button opens a list of exactly the tabs that did not fit**, under the button.
#[test]
fn the_button_lists_the_hidden_tabs() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with(&dir, &twelve());
    let hidden = editor.strip().hidden.clone();
    assert!(!hidden.is_empty(), "twelve tabs overflow {WIDTH} px");

    let (x, y) = on_more(&editor);
    assert!(editor.click(x, y));
    assert_eq!(editor.menu_kind(), Some(toolbar::Menu::Tabs));
    assert_eq!(editor.menu_tabs(), Some(hidden.as_slice()));
    let (_, top) = editor.menu_at().unwrap();
    assert!(
        top >= toolbar::HEIGHT + STRIP_HEIGHT,
        "the list hangs below the strip"
    );
}

/// **Choosing a row shows that tab, and the strip moves to include it.** That is what makes the
/// list a way to the tab rather than a second place to switch from.
#[test]
fn choosing_a_hidden_tab_shows_it_on_the_strip() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with(&dir, &twelve());
    let (x, y) = on_more(&editor);
    editor.click(x, y);
    let chosen = editor.menu_tabs().unwrap()[2];

    let (panel_x, panel_y) = editor.menu_at().unwrap();
    assert!(editor.click(
        panel_x + popup::ICON_COLUMN,
        panel_y + popup::row_top(2) + popup::ROW_HEIGHT / 2.0,
    ));

    assert_eq!(editor.menu_kind(), None, "choosing closes the list");
    assert_eq!(editor.workspace().active(), Some(chosen));
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert!(
        editor.tab_rects().iter().any(|rect| rect.index == chosen),
        "tab {chosen} is active and should be on the strip",
    );
    assert!(!editor.strip().hidden.contains(&chosen));
}

/// A click anywhere while the list is open belongs to it, the button included: it closes, as every
/// menu does, rather than opening a second one.
#[test]
fn clicking_the_button_again_closes_the_list() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with(&dir, &twelve());
    let (x, y) = on_more(&editor);
    editor.click(x, y);
    assert!(editor.menu_tabs().is_some());
    editor.click(x, y);
    assert_eq!(editor.menu_kind(), None);
}

/// **A list that no longer matches what is hidden closes.** Widened until everything fits, it would
/// otherwise offer tabs that are on the strip beside it.
#[test]
fn widening_the_window_closes_a_stale_list() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with(&dir, &twelve());
    let (x, y) = on_more(&editor);
    editor.click(x, y);
    assert!(editor.menu_tabs().is_some());

    let _ = editor.build_frame(WIDTH * 3.0, HEIGHT, 1.0);
    assert!(editor.strip().hidden.is_empty(), "everything fits now");
    assert_eq!(editor.menu_kind(), None);
}

/// **Each row draws its file's icon**, the same one its tab would. Without the icon font that is
/// its badge, counted by letters so the plain-text row's TXT is told from the Markdown rows' MD.
#[test]
fn each_row_draws_its_files_icon() {
    let dir = TempDir::new().unwrap();
    let names = twelve();
    let mut editor = editor_with(&dir, &names);
    let (x, y) = on_more(&editor);
    editor.click(x, y);
    let listed = editor.menu_tabs().unwrap().to_vec();
    let (panel_x, panel_y) = editor.menu_at().unwrap();
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);

    for (row, index) in listed.iter().enumerate() {
        let top = panel_y + popup::row_top(row);
        let in_icon_column = quads
            .iter()
            .filter(|quad| {
                quad.uv != [0.0, 0.0, 1.0, 1.0]
                    && quad.rect[0] >= panel_x
                    && quad.rect[0] < panel_x + popup::ICON_COLUMN
                    && quad.rect[1] >= top
                    && quad.rect[1] < top + popup::ROW_HEIGHT
            })
            .count();
        let want = if *index == PLAIN {
            Icon::Text
        } else {
            Icon::Note
        };
        assert_eq!(
            in_icon_column,
            want.badge().chars().count(),
            "row {row} ({}) should hold {:?}",
            names[*index],
            want.badge(),
        );
    }
}

// ---------------------------------------------------------------------------
// Where things are drawn, at a scale factor of two.
//
// `cargo-mutants` on #173 found the button's and the list's placement could be rearranged — `*
// scale` as `+ scale`, a centring `-` as `+` — without a test noticing, because everything above
// counts glyphs rather than asking where they are. At scale one, multiplying and dividing by the
// scale are the same thing, so these draw at two.
// ---------------------------------------------------------------------------

const SCALE: f32 = 2.0;

/// The grid's glyphs are one pixel wide and advance eight, and a glyph quad starts one physical
/// pixel above its baseline — `stub::Grid::rasterize`'s `top: -1`.
const ADVANCE: f32 = 8.0;

/// A frame at [`SCALE`], the window the same logical size as every other frame here.
fn at_two(editor: &mut Editor<Grid>) -> Vec<QuadInstance> {
    editor.build_frame(WIDTH * SCALE, HEIGHT * SCALE, SCALE)
}

/// Where a glyph drawn at logical `(x, baseline)` lands, in physical pixels.
fn glyph_at(x: f32, baseline: f32) -> [f32; 2] {
    [x * SCALE, baseline * SCALE - 1.0]
}

fn is_glyph(quad: &QuadInstance) -> bool {
    quad.uv != [0.0, 0.0, 1.0, 1.0]
}

/// **A tab is exactly its measured name plus its fixed parts**, through the editor's own
/// measurement. `tabs.rs` pins the arithmetic on a width it is handed; this pins the width the
/// editor hands it, which is where a glyph's advance is added to its position.
#[test]
fn a_tab_is_as_wide_as_its_measured_name() {
    let dir = TempDir::new().unwrap();
    let editor = editor_with(&dir, &twelve());
    let name = "file-00.md".len() as f32 * ADVANCE;
    assert_eq!(
        editor.tab_rects()[0].width,
        name + 12.0 * 2.0 + 20.0 + tabs::ICON_WIDTH
    );
}

/// **The button's face is centred in it, on the tabs' baseline**, and its ground is lit exactly
/// over it while its list is open — and not before.
#[test]
fn the_button_is_drawn_where_it_is_hit() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with(&dir, &twelve());
    let closed = at_two(&mut editor);
    let more = editor.strip().more.unwrap();
    let top = toolbar::HEIGHT;

    // Without the icon font the face is the bar's own fallback, one character wide.
    let face: Vec<[f32; 2]> = closed
        .iter()
        .filter(|quad| {
            is_glyph(quad)
                && quad.rect[0] >= more * SCALE
                && quad.rect[0] < (more + tabs::MORE_WIDTH) * SCALE
                && quad.rect[1] >= top * SCALE
                && quad.rect[1] < (top + STRIP_HEIGHT) * SCALE
        })
        .map(|quad| [quad.rect[0], quad.rect[1]])
        .collect();
    assert_eq!(
        face,
        vec![glyph_at(
            more + (tabs::MORE_WIDTH - ADVANCE) / 2.0,
            top + STRIP_HEIGHT * 0.68,
        )],
    );

    let ground = [
        more * SCALE,
        top * SCALE,
        tabs::MORE_WIDTH * SCALE,
        STRIP_HEIGHT * SCALE,
    ];
    let lit = |quads: &[QuadInstance]| {
        quads
            .iter()
            .any(|quad| !is_glyph(quad) && quad.rect == ground)
    };
    assert!(!lit(&closed), "not lit before its list is open");
    let (x, y) = on_more(&editor);
    editor.click(x, y);
    assert!(lit(&at_two(&mut editor)), "lit while its list is open");
}

/// **A row's icon sits in the icon column, its name after it, both on one baseline in the row**,
/// and the row under the pointer is tinted edge to edge less the panel's border.
#[test]
fn a_list_row_is_drawn_where_it_is_hit() {
    let dir = TempDir::new().unwrap();
    let mut editor = editor_with(&dir, &twelve());
    let (x, y) = on_more(&editor);
    editor.click(x, y);
    let (panel_x, panel_y) = editor.menu_at().unwrap();
    let width = editor.menu_width().unwrap();
    editor.hover(
        panel_x + popup::ICON_COLUMN,
        panel_y + popup::row_top(1) + popup::ROW_HEIGHT / 2.0,
    );
    let quads = at_two(&mut editor);

    let hovered = [
        (panel_x + 2.0) * SCALE,
        (panel_y + popup::row_top(1)) * SCALE,
        (width - 4.0) * SCALE,
        popup::ROW_HEIGHT * SCALE,
    ];
    assert!(
        quads
            .iter()
            .any(|quad| !is_glyph(quad) && quad.rect == hovered),
        "the hovered row's ground at {hovered:?}",
    );

    // Row zero's badge in the icon column, then its name. The body size is the theme's default
    // sixteen.
    let first = editor.menu_tabs().unwrap()[0];
    let badge = if first == PLAIN {
        Icon::Text
    } else {
        Icon::Note
    }
    .badge();
    let name = &twelve()[first];
    let top = panel_y + popup::row_top(0);
    let baseline = top + (popup::ROW_HEIGHT / 2.0 + 16.0 * 0.22);
    let in_row: Vec<[f32; 2]> = quads
        .iter()
        .filter(|quad| {
            is_glyph(quad)
                && quad.rect[0] >= panel_x * SCALE
                && quad.rect[0] < (panel_x + width) * SCALE
                && quad.rect[1] >= top * SCALE
                && quad.rect[1] < (top + popup::ROW_HEIGHT) * SCALE
        })
        .map(|quad| [quad.rect[0], quad.rect[1]])
        .collect();
    let mut want: Vec<[f32; 2]> = (0..badge.len())
        .map(|n| glyph_at(panel_x + popup::PADDING + n as f32 * ADVANCE, baseline))
        .collect();
    want.extend(
        (0..name.len())
            .map(|n| glyph_at(panel_x + popup::ICON_COLUMN + n as f32 * ADVANCE, baseline)),
    );
    assert_eq!(in_row, want);
}

/// **A name longer than the window is clipped at the panel's right padding**, and not before it.
/// The panel is never wider than the window, so this is the one case where a row's name meets its
/// edge.
#[test]
fn a_long_name_in_the_list_stops_at_the_panels_padding() {
    const NARROW: f32 = 400.0;
    let dir = TempDir::new().unwrap();
    let long = "a-name-long-enough-to-be-wider-than-the-whole-window-by-far";
    let names: Vec<String> = (0..4).map(|n| format!("{long}-{n}.md")).collect();
    let mut editor = editor_with(&dir, &names);
    let _ = editor.build_frame(NARROW, HEIGHT, 1.0);
    let more = editor.strip().more.expect("four long tabs overflow 400 px");
    editor.click(
        more + tabs::MORE_WIDTH / 2.0,
        toolbar::HEIGHT + STRIP_HEIGHT / 2.0,
    );
    let (panel_x, panel_y) = editor.menu_at().unwrap();
    let width = editor.menu_width().unwrap();
    assert_eq!(width, NARROW, "no wider than the window");
    let quads = editor.build_frame(NARROW, HEIGHT, 1.0);

    let top = panel_y + popup::row_top(0);
    let last = quads
        .iter()
        .filter(|quad| {
            is_glyph(quad)
                && quad.rect[0] >= panel_x + popup::ICON_COLUMN
                && quad.rect[1] >= top
                && quad.rect[1] < top + popup::ROW_HEIGHT
        })
        .map(|quad| quad.rect[0])
        .fold(f32::MIN, f32::max);
    let edge = panel_x + width - popup::PADDING;
    assert!(last <= edge, "a glyph starts at {last}, past {edge}");
    assert!(
        last > edge - ADVANCE,
        "the name stops at {last}, short of {edge}"
    );
}
