//! MDEDIT-47 in the editor: when the check runs, when its answers are thrown away, and where the
//! menu item takes you.
//!
//! The checker itself is tested in `mdedit-json`. What is asserted here is the wiring, which is
//! where this project's defects actually live — the count has to appear on a timer and vanish on an
//! edit, and "go to the first problem" has to recompute before it moves the caret rather than acting
//! on offsets from before the last keystroke.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    // Quad colours come from the same palette constants the renderer read, so they are bit-identical
    // rather than merely close.
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_session::Workspace;
use tempfile::TempDir;

use crate::stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 400.0;

/// Opens `name` holding `contents`, through the workspace the app uses, and draws one frame.
///
/// No snapshot store, and none is wanted: since MDEDIT-68 an editor is built with nowhere to keep
/// unsaved work, so a test that calls `on_idle` writes nothing anywhere. It did not used to be, and
/// these tests were how that was found.
fn open(dir: &TempDir, name: &str, contents: &str) -> Editor<Grid> {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    let mut workspace = Workspace::new();
    workspace.add(path);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    // **The palette is pinned**, because the tests below name colours from `Theme::dark`. Since
    // MDEDIT-78 the default choice is to follow Windows, and the idle timer — which these tests call
    // directly — is one of the places that asks it. On a light-themed machine, which is what a CI
    // runner is, an unpinned editor would be drawing the other palette by the second assertion.
    editor.with_config(mdedit_app::config::Config {
        theme: mdedit_app::view::ThemeChoice::Dark,
        ..mdedit_app::config::Config::default()
    });
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

fn json(dir: &TempDir, contents: &str) -> Editor<Grid> {
    open(dir, "data.json", contents)
}

// ---- when it runs ----

/// The motivating case, end to end: a trailing comma is counted once the typing stops, and fixing it
/// clears the count.
#[test]
fn a_broken_document_is_counted_after_the_idle_timer() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");

    editor.on_idle();
    assert_eq!(editor.diagnostic_count(), 1);
    assert_eq!(editor.diagnostics()[0].message, "trailing comma");
}

/// **Found by eye, not by a test.** The idle timer is set by a key event, so a document that was
/// only *opened* never reached it — a file with three errors in it showed none until you typed in
/// it, which is the state it is least useful in. The word count has the same gap and the same
/// once-per-document catch-up on the drawing path; this is that, and this test is the one that was
/// missing. Note it never calls `on_idle`.
#[test]
fn a_document_that_was_only_opened_is_still_checked() {
    let dir = TempDir::new().unwrap();
    let editor = json(&dir, "{\"a\": 1,}\n");

    assert_eq!(
        editor.diagnostic_count(),
        1,
        "one frame is enough; nobody has typed and the idle timer has never fired"
    );
}

/// And the catch-up must not become a per-frame check. Once a document has been looked at, drawing
/// it again does not re-run the walk — the idle timer owns it from then on, which is the whole
/// reason the check is not on this path.
#[test]
fn drawing_again_after_an_edit_does_not_recheck() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");
    assert_eq!(editor.diagnostic_count(), 1);

    editor.insert_text("  ");
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    assert_eq!(
        editor.diagnostic_count(),
        0,
        "a frame after an edit must not pay for the check; the count waits for the timer"
    );
}

#[test]
fn a_valid_document_counts_nothing() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1}\n");

    editor.on_idle();
    assert_eq!(editor.diagnostic_count(), 0);
}

/// A Markdown document never has a count, because Markdown has no notion of invalid. It opts out of
/// the whole feature by not implementing it.
#[test]
fn markdown_never_reports_anything() {
    let dir = TempDir::new().unwrap();
    let mut editor = open(&dir, "notes.md", "# A heading\n\nSome {\"unclosed prose.\n");

    editor.on_idle();
    assert_eq!(editor.diagnostic_count(), 0);
}

/// An empty document is not shouted at, which matters here rather than only in the checker: a new
/// file is empty, and an error on it before anything is typed would be the first thing the user saw.
#[test]
fn an_empty_document_has_no_count() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "");

    editor.on_idle();
    assert_eq!(editor.diagnostic_count(), 0);
}

// ---- when its answers are thrown away ----

/// The stale-offset rule. An edit invalidates every span the last check produced, so the count goes
/// to nothing until the next idle rather than showing spans that point at bytes which have moved.
#[test]
fn an_edit_clears_the_count_until_the_next_idle() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");

    editor.on_idle();
    assert_eq!(editor.diagnostic_count(), 1);

    editor.insert_text("  ");
    assert_eq!(
        editor.diagnostic_count(),
        0,
        "an edit makes the last check's spans stale, so they must not be shown"
    );

    editor.on_idle();
    assert_eq!(
        editor.diagnostic_count(),
        1,
        "and the next idle repopulates"
    );
}

/// The `counted_for` lesson applied again: two documents neither of which has been typed in are both
/// at revision zero, so the revision alone cannot say which one a check belongs to.
#[test]
fn switching_tab_does_not_show_the_other_documents_problems() {
    let dir = TempDir::new().unwrap();
    let broken = dir.path().join("broken.json");
    let fine = dir.path().join("fine.json");
    std::fs::write(&broken, "{\"a\": 1,}\n").unwrap();
    std::fs::write(&fine, "{\"a\": 1}\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(broken);
    workspace.add(fine);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    editor.show_tab(0);
    editor.on_idle();
    assert_eq!(editor.diagnostic_count(), 1);

    editor.show_tab(1);
    assert_eq!(
        editor.diagnostic_count(),
        0,
        "the first document's problems must not follow the tab switch"
    );
}

// ---- where the menu item goes ----

#[test]
fn validate_selects_the_first_problem() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");

    assert!(editor.apply_command(mdedit_menu_win::Command::Validate));

    let selection = editor.document().selection();
    assert_eq!(selection.span().start().get(), 7, "the comma");
    assert_eq!(selection.span().end().get(), 8);
}

/// It recomputes before it moves. The count on the bar is at most one idle interval old, but a jump
/// that used offsets from before the last keystroke would put the caret somewhere innocent.
#[test]
fn validate_recomputes_before_it_jumps() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");
    editor.on_idle();

    // Push the comma along without ever running the idle timer.
    let start = mdedit_offsets::ByteOffset::ZERO;
    editor
        .document_mut()
        .set_selection(mdedit_document::Selection::new(start, start));
    editor.insert_text("    ");

    assert!(editor.apply_command(mdedit_menu_win::Command::Validate));
    assert_eq!(
        editor.document().selection().span().start().get(),
        11,
        "the comma has moved four bytes and the jump must have noticed"
    );
}

#[test]
fn validate_on_a_clean_document_does_nothing() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1}\n");

    assert!(!editor.apply_command(mdedit_menu_win::Command::Validate));
    assert!(editor.document().selection().is_collapsed());
}

// ---- what it looks like (MDEDIT-76) ----

/// How many quads this frame draws in the diagnostic colour.
///
/// The squiggle is a row of them and the flagged gutter number is a few more, so the count is not a
/// number worth asserting on its own — what it answers is "is anything rose on screen", which is
/// exactly the question every test below asks.
fn rose(editor: &mut Editor<Grid>) -> usize {
    let rose = mdedit_app::view::Theme::default().span_error;
    let wanted = [rose.r, rose.g, rose.b, rose.a];
    editor
        .build_frame(WIDTH, HEIGHT, 1.0)
        .iter()
        .filter(|quad| quad.colour == wanted)
        .count()
}

#[test]
fn a_broken_document_is_drawn_on_not_just_counted() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");

    editor.on_idle();
    assert_eq!(editor.diagnostic_count(), 1);
    assert!(
        rose(&mut editor) > 0,
        "a counted diagnostic that draws nothing is the gap this story closed"
    );
}

/// The same rule the count follows, for the same reason: a squiggle under bytes that have moved is
/// worse than no squiggle, so an edit takes it away until the timer has looked again.
#[test]
fn an_edit_takes_the_squiggle_away_and_the_idle_timer_brings_it_back() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");
    editor.on_idle();
    assert!(rose(&mut editor) > 0);

    editor.insert_text("  ");
    assert_eq!(
        rose(&mut editor),
        0,
        "the spans are stale the instant the bytes move"
    );

    editor.on_idle();
    assert!(rose(&mut editor) > 0, "and come back when they are not");
}

#[test]
fn a_valid_document_is_drawn_on_not_at_all() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1}\n");
    editor.on_idle();
    assert_eq!(rose(&mut editor), 0, "silence, not a clean bill of health");
}

#[test]
fn an_empty_document_draws_nothing() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "");
    editor.on_idle();
    assert_eq!(rose(&mut editor), 0);
}

/// Markdown opts out of the whole feature by not implementing it, so there is nothing to draw.
#[test]
fn markdown_is_never_marked_up() {
    let dir = TempDir::new().unwrap();
    let mut editor = open(&dir, "notes.md", "# A heading\n\nSome {\"unclosed prose.\n");
    editor.on_idle();
    assert_eq!(rose(&mut editor), 0);
}

/// The gutter mark is the cue that does not depend on colour vision, so it has to be *more* than a
/// recolouring of the number: a `!` and a bold set of digits, which is more quads than the plain
/// number drew.
#[test]
fn the_flagged_line_number_is_marked_as_well_as_recoloured() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\n  \"a\": 1,\n}\n");
    editor.on_idle();

    let without_numbers = rose(&mut editor);
    editor.toggle_line_numbers();
    let with_numbers = rose(&mut editor);

    assert!(
        with_numbers > without_numbers,
        "turning the gutter on drew no extra rose quads, so the line is not marked: \
         {without_numbers} then {with_numbers}"
    );

    // And a clean document's numbers are not rose, so the mark means something.
    let mut clean = json(&dir, "{\n  \"a\": 1\n}\n");
    clean.toggle_line_numbers();
    clean.on_idle();
    assert_eq!(rose(&mut clean), 0);
}

/// Which rows carry a rose quad inside the gutter, as row indices from the top of the document.
fn flagged_rows(editor: &mut Editor<Grid>) -> Vec<usize> {
    let rose = mdedit_app::view::Theme::default().span_error;
    let wanted = [rose.r, rose.g, rose.b, rose.a];
    let gutter = editor.content_rect().left_edge();
    let top = editor
        .content_rect()
        .to_window(0.0, 0.0, mdedit_app::content::Offset::down(0.0))
        .1;
    let mut rows: Vec<usize> = editor
        .build_frame(WIDTH, HEIGHT, 1.0)
        .iter()
        .filter(|quad| quad.colour == wanted && quad.rect[0] < gutter)
        .map(|quad| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            // Digits sit on the row's baseline, one pixel up from it in the stub's metrics.
            let row = ((quad.rect[1] + 1.0 - stub::ASCENT - top) / stub::LINE_HEIGHT)
                .round()
                .max(0.0) as usize;
            row
        })
        .collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// The mark has to be on the line the error is on. An off-by-one here points the reader at innocent
/// bytes, which is worse than not marking at all.
#[test]
fn the_mark_lands_on_the_line_the_error_is_on() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\n  \"a\": 1,\n}\n");
    editor.toggle_line_numbers();
    editor.on_idle();

    assert_eq!(
        flagged_rows(&mut editor),
        vec![1],
        "the trailing comma is on the second line, which is row 1"
    );
}

// ---- at a scale and a font that are not the stub's defaults ----

/// The `(x, y, width, height)` of every rose quad wider than a glyph's single pixel, which is to say
/// the squiggles: the stub's glyphs are one pixel square, so nothing else rose is that wide.
fn squiggles(editor: &mut Editor<Grid>, scale: f32) -> Vec<[f32; 4]> {
    let rose = mdedit_app::view::Theme::dark().span_error;
    let wanted = [rose.r, rose.g, rose.b, rose.a];
    editor
        .build_frame(WIDTH * scale, HEIGHT * scale, scale)
        .iter()
        .filter(|quad| quad.colour == wanted && quad.rect[2] > 1.0)
        .map(|quad| quad.rect)
        .collect()
}

/// The wave's thickness is a logical size, so it grows with the device scale. At one the grown and
/// the shrunk thickness are both a pixel, which is why this runs at two.
#[test]
fn a_squiggle_is_as_much_thicker_as_the_scale_is_bigger() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");
    editor.on_idle();

    let one = squiggles(&mut editor, 1.0);
    let two = squiggles(&mut editor, 2.0);

    assert!(!one.is_empty(), "the test needs a squiggle to measure");
    assert_eq!(one.len(), two.len(), "the same wave at both scales");
    for (small, large) in one.iter().zip(&two) {
        assert_eq!(
            large[3],
            small[3] * 2.0,
            "thickness: {small:?} then {large:?}"
        );
        assert_eq!(
            large[2],
            small[2] * 2.0,
            "dash length: {small:?} then {large:?}"
        );
    }
}

/// The gutter's `!` is placed by its own bitmap's offset from the pen, added to the pen's position.
/// The stub reports an offset of nothing, which hides the sign, so this one reports three: every
/// mark in the gutter must then sit exactly three pixels right of where it sits with none.
#[test]
fn the_flagged_mark_is_offset_by_its_own_bearing_rightwards() {
    fn gutter_rose(bearing: i32) -> Vec<[f32; 4]> {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("data.json");
        std::fs::write(&path, "{\n  \"a\": 1,\n}\n").unwrap();
        let mut workspace = Workspace::new();
        workspace.add(path);
        let mut editor = Editor::with_workspace(workspace, Grid::with_bearing(bearing));
        editor.activate_saved_tab();
        editor.with_config(mdedit_app::config::Config {
            theme: mdedit_app::view::ThemeChoice::Dark,
            line_numbers: true,
            ..mdedit_app::config::Config::default()
        });
        editor.build_frame(WIDTH, HEIGHT, 1.0);
        editor.on_idle();

        let rose = mdedit_app::view::Theme::dark().span_error;
        let wanted = [rose.r, rose.g, rose.b, rose.a];
        let text_left = editor.content_rect().left_edge();
        editor
            .build_frame(WIDTH, HEIGHT, 1.0)
            .iter()
            .filter(|quad| {
                quad.colour == wanted
                    && quad.rect[0] + quad.rect[2] <= text_left + 3.0
                    && quad.rect[1] < HEIGHT - mdedit_app::status::BAR_HEIGHT
            })
            .map(|quad| quad.rect)
            .collect()
    }

    let flat = gutter_rose(0);
    let bearing = gutter_rose(3);

    assert!(flat.len() >= 2, "a flagged number and its mark: {flat:?}");
    assert_eq!(flat.len(), bearing.len());
    for (flat, bearing) in flat.iter().zip(&bearing) {
        assert_eq!(bearing[0] - flat[0], 3.0, "{flat:?} then {bearing:?}");
        assert_eq!(bearing[1..], flat[1..]);
    }
}

/// Where the wave is, and not only how thick: its origin scales like the rest. Written against the
/// same document at two scales, since at one `x * scale` and `x / scale` are the same number.
#[test]
fn a_squiggle_starts_as_much_further_along_as_the_scale_is_bigger() {
    let dir = TempDir::new().unwrap();
    let mut editor = json(&dir, "{\"a\": 1,}\n");
    editor.on_idle();

    let one = squiggles(&mut editor, 1.0);
    let two = squiggles(&mut editor, 2.0);

    assert!(!one.is_empty(), "the test needs a squiggle to measure");
    assert_eq!(one.len(), two.len(), "the same wave at both scales");
    for (small, large) in one.iter().zip(&two) {
        assert!(small[0] > 0.0 && small[1] > 0.0, "{small:?}");
        assert_eq!(large[0], small[0] * 2.0, "x: {small:?} then {large:?}");
        assert_eq!(large[1], small[1] * 2.0, "y: {small:?} then {large:?}");
    }
}

/// A document of twelve JSON lines whose eleventh carries a trailing comma, so the gutter has a
/// flagged two-digit number among one- and two-digit plain ones.
fn flagged_eleventh() -> String {
    use std::fmt::Write as _;

    let mut text = String::from("{\n");
    for key in 0..9 {
        let _ = writeln!(text, "  \"k{key}\": {key},");
    }
    text.push_str("  \"z\": 1,\n}\n");
    text
}

/// Every number in the gutter, one group of quads per row, left to right, at `scale`.
fn gutter_rows(editor: &mut Editor<Grid>, scale: f32) -> Vec<Vec<mdedit_app::gpu::QuadInstance>> {
    let quads = editor.build_frame(WIDTH * scale, HEIGHT * scale, scale);
    let left = editor.content_rect().left_edge() * scale;
    let top = editor.content_rect().top_edge() * scale;
    let bar_top = (HEIGHT - mdedit_app::status::BAR_HEIGHT) * scale;

    let mut digits: Vec<_> = quads
        .iter()
        .filter(|quad| {
            quad.rect[0] + quad.rect[2] <= left
                && quad.rect[1] >= top
                && quad.rect[1] < bar_top
                && quad.rect[2] < 20.0 * scale
        })
        .copied()
        .collect();
    digits.sort_by(|a, b| {
        a.rect[1]
            .total_cmp(&b.rect[1])
            .then(a.rect[0].total_cmp(&b.rect[0]))
    });
    let mut rows: Vec<Vec<mdedit_app::gpu::QuadInstance>> = Vec::new();
    for quad in digits {
        match rows.last_mut() {
            Some(row) if row[0].rect[1] == quad.rect[1] => row.push(quad),
            _ => rows.push(vec![quad]),
        }
    }
    rows
}

/// **Where each digit and the `!` sit**, at two scales: the digits of a number one advance apart, the
/// `!` one advance before the first, every number ending the same distance short of the gutter's
/// edge, and a two-digit one centred in it. Relations rather than coordinates, so no arithmetic is
/// borrowed from the code that does it.
#[test]
fn the_numbers_and_their_mark_are_where_the_gutter_puts_them() {
    // A one-digit flagged number as well as a two-digit one: the mark of a two-digit number sits at
    // the gutter's very left edge, where multiplying and dividing by the scale agree.
    let documents = [
        (flagged_eleventh(), 10),
        ("{\n  \"a\": 1,\n}\n".to_owned(), 1),
    ];
    for ((source, flagged_row), scale) in documents
        .iter()
        .flat_map(|doc| [1.0, 2.0].map(|scale| (doc, scale)))
    {
        let dir = TempDir::new().unwrap();
        let mut editor = json(&dir, source);
        editor.toggle_line_numbers();
        editor.on_idle();

        let rows = gutter_rows(&mut editor, scale);
        assert!(
            rows.len() > *flagged_row,
            "{} rows at scale {scale}",
            rows.len()
        );

        let rose = mdedit_app::view::Theme::dark().span_error;
        let rose = [rose.r, rose.g, rose.b, rose.a];
        let advance = stub::ADVANCE * scale;
        let gutter_right = (editor.content_rect().left_edge() - MARGIN) * scale;
        let first_row_top = editor
            .content_rect()
            .to_window(0.0, 0.0, mdedit_app::content::Offset::down(0.0))
            .1;

        let mut padding = None;
        for (index, row) in rows.iter().enumerate() {
            let flagged = row[0].colour == rose;
            assert_eq!(
                flagged,
                index == *flagged_row,
                "row {index} at scale {scale}"
            );
            let digits = if flagged { &row[1..] } else { &row[..] };
            if flagged {
                assert_eq!(
                    digits.len(),
                    (index + 1).to_string().len(),
                    "a mark and every digit of the number, at scale {scale}"
                );
                assert_eq!(
                    row[0].rect[0] + advance,
                    digits[0].rect[0],
                    "the mark sits one advance before the number at scale {scale}"
                );
            }
            for pair in digits.windows(2) {
                assert_eq!(
                    pair[1].rect[0] - pair[0].rect[0],
                    advance,
                    "row {index}'s digits at scale {scale}"
                );
            }
            let last = digits.last().unwrap().rect[0] + advance;
            let gap = gutter_right - last;
            assert_eq!(
                *padding.get_or_insert(gap),
                gap,
                "row {index} ends where the others do, at scale {scale}"
            );
            if digits.len() == 2 {
                assert_eq!(
                    digits[0].rect[0], gap,
                    "a full-width number is centred, at scale {scale}"
                );
            }
            #[allow(clippy::cast_precision_loss, reason = "twelve rows")]
            let rows_down = index as f32;
            assert_eq!(
                digits[0].rect[1] - rows[0][0].rect[1],
                rows_down * stub::LINE_HEIGHT * scale,
                "row {index} is its row's height below row 0 at scale {scale}"
            );
            let band = (first_row_top + rows_down * stub::LINE_HEIGHT) * scale;
            assert!(
                digits[0].rect[1] >= band && digits[0].rect[1] < band + stub::LINE_HEIGHT * scale,
                "row {index}'s number is outside its row at scale {scale}"
            );
        }
    }
}
