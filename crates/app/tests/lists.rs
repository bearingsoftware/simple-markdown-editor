//! A list looks like a list in preview (MDEDIT-141).
//!
//! Concealing a list marker drew nothing in its place, so a bulleted list was pixel-identical to
//! consecutive paragraphs — no bullet, no number, and no indent, which took nesting with it. The fix
//! substitutes text for the concealed marker rather than dropping its run.
//!
//! Every assertion here is about **which runs exist and where they sit**, which the grid stub answers
//! deterministically. Whether a real font's bullet collides with the text beside it is a different
//! question, and MDEDIT-133's is the harness for it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use mdedit_app::editor::Editor;
use mdedit_app::view::{Theme, ViewRun, build_runs};
use mdedit_document::{Document, Selection};
use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_text::{Layout, TextLayout};

use crate::stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT, MARGIN};

fn runs_of(source: &str) -> Vec<ViewRun<'_>> {
    build_runs(&Document::from(source), &Theme::default(), source)
}

/// Everything drawn, concatenated — the preview as a reader sees it.
fn drawn(source: &str) -> String {
    runs_of(source)
        .iter()
        .map(|run| run.text.as_ref())
        .collect()
}

fn laid_out(source: &str) -> Layout {
    let runs = runs_of(source);
    let styled = mdedit_app::view::as_styled(&runs);
    Grid::new()
        .layout(&styled, f32::MAX)
        .expect("the stub lays out anything")
}

/// Where the first glyph that came from the buffer sits on `row`.
///
/// A substituted glyph is told from a real one by its empty source: it stands for bytes rather than
/// being them, which is exactly the distinction "where does the *text* start" needs.
fn text_left(layout: &Layout, row: usize) -> f32 {
    layout.lines[row]
        .glyphs
        .iter()
        .find(|glyph| !glyph.source.is_empty())
        .expect("the row has text of its own")
        .pos
        .x
}

// ---- what is drawn ----

#[test]
fn a_bulleted_item_draws_a_bullet() {
    assert_eq!(drawn("- alpha\n"), "\u{2022} alpha\n");
    assert_eq!(drawn("* alpha\n"), "\u{2022} alpha\n");
    assert_eq!(drawn("+ alpha\n"), "\u{2022} alpha\n");
}

/// CommonMark keeps the author's start number rather than renumbering from one, so the bytes are
/// both the right answer and the easy one. The work is not discarding them.
#[test]
fn a_numbered_item_keeps_the_authors_number() {
    assert_eq!(drawn("7. seven\n8. eight\n"), "7. seven\n8. eight\n");
    assert_eq!(drawn("12) twelve\n"), "12) twelve\n");
}

/// The marker's leading whitespace is part of the marker, so carrying it into the substitute is the
/// whole of what makes nesting visible — and a deeper level gets its own bullet.
#[test]
fn a_nested_item_starts_further_right_than_its_parent() {
    let source = "- parent\n  - child\n";
    assert_eq!(drawn(source), "\u{2022} parent\n  \u{25e6} child\n");

    let layout = laid_out(source);
    assert!(
        text_left(&layout, 1) > text_left(&layout, 0),
        "the child's text should be indented past the parent's: {:?} vs {:?}",
        text_left(&layout, 1),
        text_left(&layout, 0),
    );
}

// ---- what must not change ----

/// The rule that outranks everything: the bullet is drawn, never inserted.
#[test]
fn the_marker_bytes_are_unchanged() {
    let source = "- alpha\n";
    let document = Document::from(source);
    let runs = build_runs(&document, &Theme::default(), source);

    assert_eq!(document.to_string(), source, "the buffer was rewritten");
    let bullet = runs
        .iter()
        .find(|run| run.text.contains('\u{2022}'))
        .expect("a bullet was drawn");
    assert_eq!(
        bullet.source.slice(source),
        "- ",
        "the substituted run must still name the bytes it stands for",
    );
}

/// MDEDIT-118, on the new run. A substituted glyph occupies width, and the caret must not be able to
/// land in the middle of the bytes it stands for.
#[test]
fn the_caret_steps_over_the_substituted_marker() {
    let layout = laid_out("- alpha\n");
    let substituted: Vec<_> = layout.lines[0]
        .glyphs
        .iter()
        .filter(|glyph| glyph.source.is_empty())
        .collect();
    assert_eq!(substituted.len(), 2, "the bullet and the space after it");
    assert!(
        substituted
            .iter()
            .all(|glyph| glyph.source.start() == ByteOffset::ZERO),
        "every substituted glyph addresses the marker's start, never its interior",
    );

    let mut editor = Editor::new("- alpha\n", Grid::new());
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::ZERO));
    assert!(editor.press_right(false));
    assert_eq!(
        editor.document().selection().head().get(),
        3,
        "past the whole marker and the `a`, as it was before a bullet was drawn there",
    );
}

/// Where a click at character `column` of the first row puts the caret.
fn click_column(source: &str, column: usize) -> usize {
    let mut editor = Editor::new(source, Grid::new());
    editor.build_frame(600.0, 400.0, 1.0);
    #[allow(clippy::cast_precision_loss, reason = "small test coordinates")]
    editor.click_at(
        ADVANCE.mul_add(column as f32, MARGIN) + ADVANCE / 2.0,
        MARGIN + mdedit_app::toolbar::HEIGHT + LINE_HEIGHT / 2.0,
    );
    editor.document().selection().head().get()
}

/// A click on the bullet is a click on syntax that is not there. It resolves to the line's start
/// rather than between the `-` and the space after it.
#[test]
fn clicking_the_bullet_does_not_land_inside_the_marker() {
    assert_eq!(click_column("- alpha\n", 0), 0);
}

/// **The case the bullet's own test cannot reach.** An ordered marker substitutes to its own bytes,
/// so it is the one substitute whose text and source are identical — and a first version of this
/// inferred "substituted" from their lengths differing, which made `7. ` the exception that carried
/// per-character offsets. A click on the `.` then landed at byte 1, inside a concealed marker, which
/// MDEDIT-118 forbids.
#[test]
fn clicking_a_number_does_not_land_inside_the_marker() {
    //  7  .  ␣  s  e  v  e  n
    //  0  1  2  3
    for column in 0..3 {
        let caret = click_column("7. seven\n", column);
        assert!(
            !(1..3).contains(&caret),
            "a click at column {column} put the caret at {caret}, inside `7. `",
        );
    }
}

/// **`push_substitute` pushes one run and does not split at newlines**, on the grounds that a list
/// marker is within a line by definition. That is an assumption about `mdedit-markers`, so it is
/// asked rather than asserted in a comment — a marker carrying a `\n` would take the inline styling
/// state across a line boundary with it, which is the bug `push_split` exists to prevent.
#[test]
fn no_substituted_marker_spans_a_line() {
    for source in [
        "-\n- two\n",
        "- \n",
        "*\n",
        "1.\n",
        "  -\n",
        "- a\n  - b\n",
        "> - quoted\n",
    ] {
        let document = Document::from(source);
        for run in build_runs(&document, &Theme::default(), source) {
            assert!(
                !run.substituted || !run.text.contains('\n'),
                "{source:?}: the substitute {:?} carries a newline",
                run.text,
            );
        }
    }
}

// ---- MDEDIT-142: a task draws its box ----

#[test]
fn a_task_draws_its_box_in_place_of_the_bullet() {
    assert_eq!(drawn("- [ ] buy milk\n"), "\u{E739} buy milk\n");
    assert_eq!(drawn("- [x] call bank\n"), "\u{E73A} call bank\n");
    assert_eq!(drawn("* [X] shout\n"), "\u{E73A} shout\n");
}

/// The number is information and stays; only a bullet is redundant beside a box.
#[test]
fn a_numbered_task_keeps_its_number() {
    assert_eq!(
        drawn("1. [ ] step\n2. [x] done\n"),
        "1. \u{E739} step\n2. \u{E73A} done\n",
    );
}

/// The bullet goes and its indent stays, which is what keeps a nested task nested.
#[test]
fn a_nested_task_keeps_its_indent() {
    assert_eq!(
        drawn("- parent\n  - [ ] child\n"),
        "\u{2022} parent\n  \u{E739} child\n",
    );
}

/// A bullet is only dropped for a box that is really there. `[ ]` with nothing after it is not a
/// task in GFM, and `[y]` never is.
#[test]
fn brackets_that_are_not_a_task_leave_the_bullet_alone() {
    assert_eq!(drawn("- [y] maybe\n"), "\u{2022} [y] maybe\n");
    assert_eq!(drawn("- a [ ] later\n"), "\u{2022} a [ ] later\n");
}

/// Plain markup is the file as it is, box and all.
#[test]
fn plain_markup_draws_the_brackets() {
    let source = "- [x] done\n";
    let document = Document::from(source);
    let whole = ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len()));
    let theme = Theme::default();
    let plain = mdedit_app::view::Decoration {
        theme: &theme,
        mode: mdedit_app::view::Mode::Plain,
    };
    let text: String = mdedit_app::view::build_runs_in(&document, plain, whole, source)
        .iter()
        .map(|run| run.text.as_ref())
        .collect();
    assert_eq!(text, source);
}

/// Drawing a box writes nothing, and the box names the bytes it stands for.
#[test]
fn the_task_bytes_are_unchanged() {
    let source = "- [x] done\n";
    let document = Document::from(source);
    let runs = build_runs(&document, &Theme::default(), source);

    assert_eq!(document.to_string(), source, "the buffer was rewritten");
    let task = runs
        .iter()
        .find(|run| run.text.starts_with('\u{E73A}'))
        .expect("a box was drawn");
    assert!(task.substituted);
    assert_eq!(task.source.slice(source), "[x]");
}

/// Every place Right stops on, from the start of `source` to its end.
fn caret_stops(source: &str) -> Vec<usize> {
    let mut editor = Editor::new(source, Grid::new());
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::ZERO));
    let mut stops = vec![0];
    while editor.press_right(false) {
        let at = editor.document().selection().head().get();
        if stops.last() == Some(&at) {
            break;
        }
        stops.push(at);
    }
    stops
}

/// **The box is not a caret position**, which is the ticket's own warning. The caret stepping over
/// the bullet walks straight on across the box, so neither `- ` nor `[ ]` is ever stood inside.
#[test]
fn the_caret_never_stands_inside_a_task_marker() {
    for (source, markers) in [("- [ ] alpha\n", 1..5), ("1. [x] alpha\n", 1..6)] {
        let stops = caret_stops(source);
        assert!(
            stops.iter().all(|stop| !markers.contains(stop)),
            "{source:?}: the caret stopped inside the markers: {stops:?}",
        );
    }
}

/// A point on the box addresses its `[`, between the concealed bullet and the concealed box, where
/// typing would break the item. It lands on the text after the box instead. A plain click on a box
/// ticks it (`tests/tasks.rs`), so this is a drag across one — which, like a shift+click or a
/// right-click, still has to put the selection's end somewhere.
#[test]
fn a_point_on_a_box_puts_the_caret_after_it() {
    //  ☐  ␣  a       the drawn row
    //  2  2  6       the bytes each glyph addresses
    let mut editor = Editor::new("- [ ] alpha\n", Grid::new());
    editor.build_frame(600.0, 400.0, 1.0);
    // The box's left half, which is the half a nearest-boundary hit resolves to the box itself.
    editor.drag_to(
        MARGIN + ADVANCE / 4.0,
        MARGIN + mdedit_app::toolbar::HEIGHT + LINE_HEIGHT / 2.0,
    );
    assert_eq!(editor.document().selection().head().get(), 6);
}

/// Plain markup draws `[`, so before it is a place the caret can be like any other.
#[test]
fn in_plain_markup_a_click_before_the_brackets_stays_there() {
    let mut editor = Editor::new("- [ ] alpha\n", Grid::new());
    editor.toggle_mode();
    editor.build_frame(600.0, 400.0, 1.0);
    editor.click_at(
        ADVANCE.mul_add(2.0, MARGIN) + ADVANCE / 4.0,
        MARGIN + mdedit_app::toolbar::HEIGHT + LINE_HEIGHT / 2.0,
    );
    assert_eq!(editor.document().selection().head().get(), 2);
}

// ---- MDEDIT-147: the units a hanging indent is assembled from ----

/// What `list::units` cuts a chunk into, as text, so a wrong boundary reads as a wrong string.
fn units_of(source: &str) -> Vec<&str> {
    let document = Document::from(source);
    let whole = ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(source.len()));
    mdedit_app::list::units(&document, whole, source)
        .into_iter()
        .map(|unit| unit.slice(source))
        .collect()
}

#[test]
fn a_flat_list_is_one_unit_per_item() {
    assert_eq!(units_of("- alpha\n- beta\n"), vec!["- alpha\n", "- beta\n"],);
}

/// The reason this needs no recursion: a nested item's marker is a marker like any other, so it
/// opens its own unit and brings its own deeper indent with it.
#[test]
fn a_nested_item_is_a_unit_of_its_own() {
    assert_eq!(
        units_of("- parent\n  - child\n- sibling\n"),
        vec!["- parent\n", "  - child\n", "- sibling\n"],
    );
}

/// A continuation paragraph belongs to the item above it, which is where its bytes are.
#[test]
fn a_continuation_paragraph_stays_with_its_item() {
    assert_eq!(
        units_of("- alpha\n  more of alpha\n- beta\n"),
        vec!["- alpha\n  more of alpha\n", "- beta\n"],
    );
}

/// Nothing to cut. The caller then lays the chunk out as ordinary lines rather than as an empty list.
#[test]
fn a_chunk_with_no_list_marker_has_no_units() {
    assert!(units_of("just a paragraph\n").is_empty());
}

// ---- MDEDIT-147: where a wrapped item's rows begin ----

/// Where the **content** starts on each drawn row, top to bottom.
///
/// Content rather than every glyph, and that is what makes this express the criterion: a bullet is
/// drawn in the `key` colour and prose in `text`, so filtering to `text` asks "where does the item's
/// text begin" without having to know which row carries a bullet. The stub gives every glyph on a row
/// the same y, so a row is a y.
fn content_lefts(source: &str, width: f32) -> Vec<i32> {
    content_lefts_of(&mut Editor::new(source, Grid::new()), width)
}

/// The same measurement against an editor a test has already set up.
fn content_lefts_of(editor: &mut Editor<Grid>, width: f32) -> Vec<i32> {
    let quads = editor.build_frame(width, 400.0, 1.0);
    let theme = Theme::default();
    let wanted = [theme.text.r, theme.text.g, theme.text.b, theme.text.a];

    // **Inside the document, not the chrome around it.** Line numbers sit left of the margin and the
    // status bar's own text sits above it, both drawn in this colour — without the bounds the
    // leftmost thing on a row is a gutter digit and the topmost "row" is a band of chrome.
    let left_edge = MARGIN;
    let top_edge = MARGIN + mdedit_app::toolbar::HEIGHT;
    let mut rows: Vec<(i32, i32)> = Vec::new();
    for quad in quads.iter().filter(|quad| {
        quad.colour == wanted && quad.rect[0] >= left_edge && quad.rect[1] >= top_edge
    }) {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "stub metrics are whole pixels"
        )]
        let (y, x) = (quad.rect[1] as i32, quad.rect[0] as i32);
        match rows.iter_mut().find(|(row, _)| *row == y) {
            Some((_, left)) => *left = (*left).min(x),
            None => rows.push((y, x)),
        }
    }
    rows.sort_unstable();
    rows.into_iter().map(|(_, left)| left).collect()
}

/// **The defect MDEDIT-147 exists for.** A long item's second row begins under its text, not back at
/// the left edge where its bullet is.
#[test]
fn a_wrapped_item_hangs_under_its_own_text() {
    let lefts = content_lefts("- alpha beta gamma delta\n", MARGIN + 12.0 * ADVANCE);
    assert!(lefts.len() >= 2, "the item should have wrapped: {lefts:?}");
    assert_eq!(
        lefts[1], lefts[0],
        "every row of one item starts at the same x: {lefts:?}",
    );
}

/// And it hangs by the marker's own width, so a deeper item hangs further in.
#[test]
fn a_nested_wrapped_item_hangs_further_in_than_a_flat_one() {
    let width = MARGIN + 12.0 * ADVANCE;
    let flat = content_lefts("- alpha beta gamma delta\n", width);
    let nested = content_lefts("  - alpha beta gamma delta\n", width);
    assert!(
        nested[1] > flat[1],
        "nested {nested:?} should hang further in than flat {flat:?}",
    );
}

/// **A wrapped task hangs under its box** (MDEDIT-142). GFM makes the box the start of the item's
/// content, and that is where GitHub aligns a continuation row too. The box is not in the text colour,
/// so the first row's left edge is the space after it and the second row's is its first word.
#[test]
fn a_wrapped_task_hangs_under_its_box() {
    let width = MARGIN + 12.0 * ADVANCE;
    for (source, indent) in [
        ("- [ ] alpha beta gamma delta\n", 0.0),
        ("  - [ ] alpha beta gamma delta\n", 2.0),
        ("1. [ ] alpha beta gamma delta\n", 3.0),
    ] {
        let lefts = content_lefts(source, width);
        assert!(
            lefts.len() >= 2,
            "{source:?} should have wrapped: {lefts:?}"
        );
        #[allow(
            clippy::cast_possible_truncation,
            reason = "stub metrics are whole pixels"
        )]
        let under_the_box = ADVANCE.mul_add(indent, MARGIN) as i32;
        assert_eq!(
            lefts[1], under_the_box,
            "{source:?}: the second row does not start under the box: {lefts:?}",
        );
    }
}

/// **R1 on the plan, and the invariant most likely to catch a mistake here.** Every unit ends in a
/// newline, so the engine gives it a trailing empty line — stacked naively that is a blank row
/// between every pair of items. Asserted against the layout this replaces rather than against a
/// number written by hand: plain markup takes the linear path, so it is the control.
#[test]
fn a_list_that_does_not_wrap_keeps_the_rows_it_had() {
    let source = "- alpha\n- beta\n  - child\n\nafter\n";
    let hung = content_lefts(source, 600.0).len();

    let mut editor = Editor::new(source, Grid::new());
    editor.toggle_mode();
    let linear = content_lefts_of(&mut editor, 600.0).len();

    assert_eq!(
        hung, linear,
        "the hanging layout has a different number of rows than the linear one it replaces",
    );
}

/// The top of every row the document drew glyphs on, in order, one entry per row.
fn document_row_tops(source: &str) -> Vec<f32> {
    const WIDTH: f32 = 600.0;
    const HEIGHT: f32 = 400.0;
    let top = mdedit_app::toolbar::HEIGHT + MARGIN;
    let mut ys: Vec<f32> = Editor::new(source, Grid::new())
        .build_frame(WIDTH, HEIGHT, 1.0)
        .iter()
        .filter(|quad| quad.uv != [0.0, 0.0, 1.0, 1.0] && quad.rect[0] >= MARGIN)
        .filter(|quad| quad.rect[1] >= top && quad.rect[1] < HEIGHT - mdedit_app::toolbar::HEIGHT)
        .map(|quad| quad.rect[1])
        .collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ys.dedup();
    ys
}

/// **The count above cannot see a blank row, and a blank row is the defect it names.** It counts
/// rows *with text*, so an empty row stacked between every pair of items leaves the count unchanged.
/// The mutants job found that on #166 (MDEDIT-145), when the arithmetic moved into `place.rs`: making
/// `place::rows_kept` keep every unit's trailing row survived every test here. Spacing sees it.
#[test]
fn items_sit_one_line_apart_with_no_blank_row_between_them() {
    let tops = document_row_tops("- alpha\n- beta\n- gamma\n");

    assert_eq!(tops.len(), 3, "{tops:?}");
    for pair in tops.windows(2) {
        assert_eq!(
            pair[1] - pair[0],
            LINE_HEIGHT,
            "a blank row between items: {tops:?}"
        );
    }
}

/// **A wrapped item's rows are one line apart too — and on `main` they were not.** Found while
/// reviewing #166: a `LayoutLine`'s baseline is measured from the top of *its layout*, and the
/// assembly added it to the row's own top, so every continuation row was pushed down by the rows
/// above it in the item. Row two drew a row low, row three two rows low — below the next item,
/// on top of its text. Every hanging test here checks x, and none checked y, since MDEDIT-147.
#[test]
fn a_wrapped_items_rows_are_one_line_apart() {
    let long = "word ".repeat(40);
    let tops = document_row_tops(&format!("- {long}\n- next\n"));

    assert!(
        tops.len() >= 4,
        "the item did not wrap onto three rows: {tops:?}"
    );
    for pair in tops.windows(2) {
        assert_eq!(
            pair[1] - pair[0],
            LINE_HEIGHT,
            "a wrapped row is not where its row is: {tops:?}"
        );
    }
}

/// **A bullet sits on its item's first baseline, not above it or below.** The marker is laid out on
/// its own, so its glyph's y has to be re-measured onto the row it joins — and nothing checked that
/// it was. The same mutants run found a bullet placed at the chunk's top, or two ascents down,
/// passed every test in this file.
#[test]
fn a_bullet_sits_on_its_items_first_baseline() {
    // Straight at `list::assemble` rather than through a frame, so nothing about the chrome or the
    // window can hide a bullet that went somewhere odd: every glyph on the row, bullet included, is
    // on the row's baseline or the assembly is wrong.
    let items = [mdedit_app::list::Item {
        marker: laid_out("x"),
        text: laid_out("alpha\n"),
        bands: Vec::new(),
    }];
    let (assembled, _) = mdedit_app::list::assemble(&items);
    let row = &assembled.lines[0];

    assert!(row.glyphs.len() > 1, "no bullet and text to compare");
    for glyph in &row.glyphs {
        assert_eq!(
            glyph.pos.y, row.baseline,
            "a glyph is off its row's baseline"
        );
    }
}

/// **R2 on the plan, checked rather than assumed.** A table hands `hit` its row spans because a grid
/// *drops* a source line; this drops none, so counting newlines still finds a row's bytes. What that
/// argument does not cover is whether a glyph on a *moved* row still addresses the byte under it — the
/// placement rewrites positions, and MDEDIT-126's two defects both lived in exactly that seam.
#[test]
fn a_click_on_a_wrapped_row_lands_on_the_byte_under_it() {
    let source = "- alpha beta gamma delta\n";
    let width = MARGIN + 12.0 * ADVANCE;
    let mut editor = Editor::new(source, Grid::new());
    editor.build_frame(width, 400.0, 1.0);

    // The first character of the second row, which hangs one marker-width in.
    let x = MARGIN + 2.0 * ADVANCE + ADVANCE / 2.0;
    let y = MARGIN + mdedit_app::toolbar::HEIGHT + LINE_HEIGHT + LINE_HEIGHT / 2.0;
    editor.click_at(x, y);

    let caret = editor.document().selection().head().get();
    let under = source
        .get(caret..)
        .and_then(|rest| rest.chars().next())
        .expect("the caret landed inside the document");
    assert!(
        caret > 2 && under.is_alphabetic(),
        "clicking the start of the wrapped row put the caret at {caret} ({under:?})",
    );
}

/// **A wrapped item reaches no further right than unlisted text.** Laid out at the width its marker
/// leaves, its rows end where a paragraph's do; laid out at the full width and then hung, they would
/// end a marker's width past it. Every hanging test here looked at where rows start.
#[test]
fn a_wrapped_item_reaches_no_further_right_than_a_paragraph() {
    let long = "word ".repeat(60);
    let rightmost = |source: &str| {
        let top = mdedit_app::toolbar::HEIGHT + MARGIN;
        Editor::new(source, Grid::new())
            .build_frame(900.0, 600.0, 1.0)
            .iter()
            .filter(|quad| quad.uv != [0.0, 0.0, 1.0, 1.0])
            .filter(|quad| {
                quad.rect[1] >= top && quad.rect[1] < 600.0 - mdedit_app::toolbar::HEIGHT
            })
            .map(|quad| quad.rect[0])
            .fold(0.0_f32, f32::max)
    };

    let listed = rightmost(&format!("- {long}\n"));
    let plain = rightmost(&format!("{long}\n"));
    assert!(
        listed <= plain,
        "the item reached {listed}, past a paragraph's {plain}"
    );
}
