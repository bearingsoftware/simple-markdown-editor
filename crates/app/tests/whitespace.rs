//! Showing whitespace and line endings (MDEDIT-46).
//!
//! The motivating case is the last test in this file: the editor deliberately preserves pasted CRLF
//! into an LF file, so mixed endings exist and the status bar says "Mixed" without saying *where*.
//!
//! Everything else here defends the one rule the feature rests on — **it draws, and changes nothing
//! else**. No glyph moves, no row re-wraps, no click lands anywhere new, and the bytes are untouched.
//! A whitespace display that shifted the text by a pixel would be worse than none, because it would
//! be lying about the document at exactly the moment somebody switched it on to find out the truth.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::editor::Editor;
use mdedit_app::view::{self, Ending};
use mdedit_offsets::ByteOffset;

mod stub;
use stub::{ADVANCE, Grid, LINE_HEIGHT};

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 400.0;

fn editor(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

fn quads(editor: &mut Editor<Grid>) -> usize {
    editor.build_frame(WIDTH, HEIGHT, 1.0).len()
}

/// The window position of character `column` on row `row`.
fn at(editor: &Editor<Grid>, column: usize, row: usize) -> (f32, f32) {
    #[allow(clippy::cast_precision_loss, reason = "small test coordinates")]
    (
        ADVANCE.mul_add(column as f32, editor.content_rect().left_edge()) + ADVANCE / 2.0,
        editor.content_rect().to_window(0.0, 0.0, 0.0).1
            + LINE_HEIGHT.mul_add(row as f32, LINE_HEIGHT / 2.0),
    )
}

// ---- the rules ----

#[test]
fn it_is_off_to_begin_with() {
    assert!(!editor("a b\n").whitespace());
}

#[test]
fn showing_it_draws_more_and_hiding_it_draws_exactly_what_it_did_before() {
    let mut editor = editor("alpha beta gamma\n");
    let before = quads(&mut editor);

    assert!(editor.set_whitespace(true));
    let showing = quads(&mut editor);
    assert!(
        showing > before,
        "marks should have been added: {before} then {showing}"
    );

    assert!(editor.set_whitespace(false));
    assert_eq!(
        quads(&mut editor),
        before,
        "turning it off must restore exactly the previous rendering"
    );
}

/// **The drift test.** Clicking must land on the same byte with the marks on as with them off. It
/// does because nothing about the layout is consulted, and this is what says so.
#[test]
fn clicking_lands_on_the_same_byte_either_way() {
    let text = "alpha beta\n\tgamma delta\n";
    let places = [(0, 0), (5, 0), (7, 0), (3, 1), (9, 1)];

    let mut off = editor(text);
    let mut on = editor(text);
    assert!(on.set_whitespace(true));
    on.build_frame(WIDTH, HEIGHT, 1.0);

    for (column, row) in places {
        let (x, y) = at(&off, column, row);
        off.click(x, y);
        on.click(x, y);
        assert_eq!(
            on.document().selection().head(),
            off.document().selection().head(),
            "click at column {column}, row {row} landed somewhere else"
        );
    }
}

/// And the caret moves identically, which is the other half of the same claim.
#[test]
fn the_caret_moves_identically_either_way() {
    let text = "alpha beta\ngamma\n";
    let mut off = editor(text);
    let mut on = editor(text);
    assert!(on.set_whitespace(true));

    for _ in 0..8 {
        off.apply_chord("\u{0}", false);
        on.apply_chord("\u{0}", false);
    }
    assert_eq!(
        on.document().selection().head(),
        off.document().selection().head()
    );
}

/// Byte preservation, which is not really in doubt — nothing here touches the buffer — but is the
/// acceptance criterion and costs one assertion.
#[test]
fn the_document_is_untouched() {
    let text = "alpha  beta\r\n\tgamma\n";
    let mut editor = editor(text);
    assert!(editor.set_whitespace(true));
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert_eq!(editor.document().to_string(), text);
    assert!(!editor.document().can_undo());
}

/// Toggling clears no layout cache, because nothing about the layout depends on it. MDEDIT-46's
/// plan expected a clear by analogy with zoom; the analogy does not hold — zoom changes what the
/// engine is *asked for* and this changes nothing it is asked for — and this pins the difference so
/// nobody adds a clear back "for safety".
///
/// The marks themselves cost one layout each, once, which is the only thing switching it on sends
/// through the engine at all. The document does not go back through it, and the proof is that a
/// second frame is free.
#[test]
fn toggling_relays_out_nothing_but_the_marks_themselves() {
    let mut editor = editor("alpha beta gamma\n\ndelta epsilon\n");
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let before = editor.text_backend().layouts();

    assert!(editor.set_whitespace(true));
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let after_first = editor.text_backend().layouts();

    // A handful of marks, not a document's worth of chunks. Three distinct ones appear in this
    // fixture: the middot, and the two halves of an ending.
    assert!(
        after_first - before <= 4,
        "only the marks should have been laid out, but {} calls happened",
        after_first - before
    );

    // And in the steady state a frame costs exactly what it cost before. Compared against the
    // measured per-frame cost rather than against zero, because the chunk holding the caret is laid
    // out fresh every frame regardless — that is MDEDIT-17's rule, and nothing to do with this.
    let settled = editor.text_backend().layouts();
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let per_frame_on = editor.text_backend().layouts() - settled;

    assert!(editor.set_whitespace(false));
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let settled = editor.text_backend().layouts();
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let per_frame_off = editor.text_backend().layouts() - settled;

    assert_eq!(
        per_frame_on, per_frame_off,
        "a frame with the marks showing must cost what a frame without them costs"
    );
}

#[test]
fn setting_it_to_what_it_already_is_changes_nothing() {
    let mut editor = editor("a\n");
    assert!(!editor.set_whitespace(false));
    assert!(editor.set_whitespace(true));
    assert!(!editor.set_whitespace(true));
}

// ---- the marks themselves ----

#[test]
fn spaces_and_tabs_get_the_traditional_marks() {
    assert_eq!(view::mark_for(' '), Some('\u{b7}'));
    assert_eq!(view::mark_for('\t'), Some('\u{2192}'));
}

/// Ordinary text is left alone, which is most of the document.
#[test]
fn visible_characters_get_no_mark() {
    for character in ['a', 'Z', '0', '_', '\u{6c49}', '\u{1f980}', '\u{a0}'] {
        assert_eq!(view::mark_for(character), None, "{character:?}");
    }
}

/// The C0 controls that are neither tab nor a line ending — invisible, and nearly always a mistake.
#[test]
fn control_characters_get_their_control_picture() {
    assert_eq!(view::mark_for('\u{0}'), Some('\u{2400}'));
    assert_eq!(view::mark_for('\u{7}'), Some('\u{2407}'));
    assert_eq!(view::mark_for('\u{1b}'), Some('\u{241b}'));
    assert_eq!(view::mark_for('\u{7f}'), Some('\u{2421}'));
}

/// Line endings are not marked per character: one ending is one mark for what may be two bytes, and
/// it is drawn past the row rather than in a cell.
#[test]
fn line_endings_are_not_in_cell_marks() {
    assert_eq!(view::mark_for('\n'), None);
    assert_eq!(view::mark_for('\r'), None);
}

#[test]
fn each_ending_is_told_apart() {
    assert_eq!(view::ending_at("a\nb", 1), Some(Ending::Lf));
    assert_eq!(view::ending_at("a\r\nb", 1), Some(Ending::CrLf));
    assert_eq!(view::ending_at("a\rb", 1), Some(Ending::Cr));
    assert_eq!(view::ending_at("ab", 1), None);
    assert_eq!(view::ending_at("ab", 99), None);
}

/// **Found by looking at the window, not by a test.** Whether the terminator gets a glyph of its own
/// is the text engine's business: DirectWrite gives `\n` a zero-width one and the stub these tests
/// run on does not. So the offset past a row's last glyph is sometimes the newline and sometimes
/// past it, and every paragraph row lost its mark on the real backend while every test here passed.
///
/// `ending_before` answers both, and this is the test that could have caught it.
#[test]
fn a_row_finds_its_ending_whether_or_not_the_terminator_had_a_glyph() {
    let text = "alpha\nbeta\r\ngamma\rdelta";

    // The stub's shape: `after` is the offset *of* the terminator.
    assert_eq!(view::ending_before(text, 5), Some(Ending::Lf));
    assert_eq!(view::ending_before(text, 10), Some(Ending::CrLf));
    assert_eq!(view::ending_before(text, 17), Some(Ending::Cr));

    // DirectWrite's shape: `after` has stepped past it.
    assert_eq!(view::ending_before(text, 6), Some(Ending::Lf));
    assert_eq!(view::ending_before(text, 12), Some(Ending::CrLf));
    assert_eq!(view::ending_before(text, 18), Some(Ending::Cr));

    // And a row that ends in the middle of a line still has no ending.
    assert_eq!(view::ending_before(text, 3), None);
    assert_eq!(view::ending_before(text, text.len()), None);
    assert_eq!(view::ending_before(text, 9999), None);
}

/// CRLF must be read as one ending rather than as a CR followed by an LF, or every Windows file
/// would be full of the mark that means "this is probably damage".
#[test]
fn crlf_is_one_ending_not_two() {
    assert_eq!(view::ending_at("a\r\n", 1).unwrap().marks().len(), 2);
    assert!(!view::ending_at("a\r\n", 1).unwrap().is_suspect());
}

/// **Only a lone CR is flagged loudly.** A document that is entirely LF or entirely CRLF should read
/// as calm; a bare CR is almost always damage.
#[test]
fn only_a_lone_cr_is_suspect() {
    assert!(!Ending::Lf.is_suspect());
    assert!(!Ending::CrLf.is_suspect());
    assert!(Ending::Cr.is_suspect());
}

// ---- the case the feature exists for ----

/// **The motivating case.** The editor preserves pasted CRLF into an LF file, so mixed endings exist
/// and the status bar says "Mixed" without saying where. With the marks on, the lines are drawn
/// differently from each other — which is the whole point, and is visible here as a quad count that
/// changes when an ending does.
#[test]
fn a_mixed_document_draws_its_lines_differently() {
    let mut all_lf = editor("alpha\nbeta\ngamma\n");
    assert!(all_lf.set_whitespace(true));
    let lf_quads = quads(&mut all_lf);

    let mut mixed = editor("alpha\r\nbeta\ngamma\n");
    assert!(mixed.set_whitespace(true));
    let mixed_quads = quads(&mut mixed);

    assert!(
        mixed_quads > lf_quads,
        "a CRLF line draws two marks where an LF line draws one: {lf_quads} then {mixed_quads}"
    );
}

/// **Trailing whitespace is the thing people switch this on to find**, and it was the last thing to
/// work. DirectWrite does not shape whitespace at the end of a line, so there is no glyph to stamp a
/// mark into — found by looking at the window, where `trailing spaces here` had three invisible
/// spaces and no marks. The stub *does* shape them, so this test passes either way and pins the
/// count rather than the mechanism.
#[test]
fn trailing_spaces_are_marked_once_each() {
    // The *same* document either way, so what is counted is the marks alone. Comparing against a
    // document without the spaces would also count their glyphs, which the stub draws and
    // DirectWrite does not — the difference that made this feature's last defect invisible.
    let mut editor = editor("here   \n");
    let off = quads(&mut editor);
    assert!(editor.set_whitespace(true));

    assert_eq!(
        quads(&mut editor) - off,
        4,
        "three trailing spaces and one line ending, marked once each"
    );
}

/// An empty line has no glyphs to hang a mark on, and is exactly where somebody looks to find out
/// what a blank line is made of. Its ending is found from where the row starts instead.
#[test]
fn a_blank_line_still_shows_its_ending() {
    let mut without = editor("alpha\nbeta\n");
    assert!(without.set_whitespace(true));
    let fewer = quads(&mut without);

    let mut with = editor("alpha\n\nbeta\n");
    assert!(with.set_whitespace(true));
    assert!(
        quads(&mut with) > fewer,
        "the blank line's own ending should have been drawn"
    );
}

/// A wrapped row has no line ending after it, so it gets no mark — otherwise every wrapped
/// paragraph would look like it was full of hard breaks.
#[test]
fn a_wrapped_row_is_not_given_an_ending() {
    // The stub wraps at the character, so this is two rows and one newline.
    let wide = "x".repeat(200);
    let mut editor = Editor::new(&format!("{wide}\n"), Grid::new());
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let before = editor.build_frame(WIDTH, HEIGHT, 1.0).len();

    assert!(editor.set_whitespace(true));
    let after = editor.build_frame(WIDTH, HEIGHT, 1.0).len();

    assert_eq!(
        after - before,
        1,
        "exactly one ending mark for one newline, however many rows it wrapped to"
    );
}

/// The caret cannot be put on a line-ending mark, because the mark is not in the layout. Pressing
/// End goes to the last real character, not past it.
#[test]
fn the_ending_mark_is_not_a_column_the_caret_can_reach() {
    let text = "alpha\nbeta\n";
    let mut off = editor(text);
    let mut on = editor(text);
    assert!(on.set_whitespace(true));

    off.apply_chord("\u{0}", false);
    let (x, y) = at(&off, 20, 0);
    off.click(x, y);
    on.click(x, y);

    assert_eq!(
        on.document().selection().head(),
        off.document().selection().head(),
        "clicking past the end of a marked line must land where it always did"
    );
    assert!(on.document().selection().head() <= ByteOffset::new(5));
}
