//! A local PNG draws as a picture rather than as its alt text (MDEDIT-146).
//!
//! Driven through the real `Editor`, with a real file on disk and the grid stub for text. That is
//! deliberate and it is most of the value here: `unit-tests-miss-bugs-in-the-join` is the lesson
//! MDEDIT-126 left behind, and every part of this feature — finding the image, resolving the path,
//! decoding it, sizing it, placing it, and getting the chunk's height right — is correct on its own
//! and could still be wrong together.
//!
//! **What this cannot see is whether the picture looks right**, because CI cannot open a window.
//! It can see that the rectangle is where the arithmetic says, which is why `Drawn` is data.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::path::{Path, PathBuf};

use mdedit_app::editor::Editor;
use mdedit_app::image::Drawn;
use mdedit_document::Selection;
use mdedit_offsets::{ByteOffset, ByteSpan};
use mdedit_session::Workspace;
use tempfile::TempDir;

mod stub;
use stub::{Grid, LINE_HEIGHT, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = 600.0;

/// A PNG of `width` x `height`, opaque mid-grey.
fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&vec![0x80; (width * height * 4) as usize])
        .unwrap();
    writer.finish().unwrap();
    bytes
}

/// The same, written to `path`.
fn write_png(path: &Path, width: u32, height: u32) {
    std::fs::write(path, png_bytes(width, height)).unwrap();
}

/// An editor on a saved document containing `source`, in a directory of its own.
fn editor_on(dir: &TempDir, source: &str) -> (Editor<Grid>, PathBuf) {
    let path = dir.path().join("notes.md");
    std::fs::write(&path, source).unwrap();

    let mut workspace = Workspace::new();
    workspace.add(path.clone());
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.activate_saved_tab();
    (editor, path)
}

/// The pictures one frame placed.
fn pictures_of(editor: &mut Editor<Grid>) -> Vec<Drawn> {
    editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor.drawn_pictures().to_vec()
}

/// How long a test waits for the decode worker before calling it broken.
///
/// Generous, and it costs nothing when things work: [`Editor::await_pictures`] blocks on the
/// worker's answer, not on this. A machine so loaded that a 100-pixel PNG takes ten seconds has
/// worse problems than a red test.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(10);

/// Waits for every picture the last frame asked for, then builds another (MDEDIT-156).
///
/// The decode is off the drawing thread now, so "the pixels are there" is a thing that becomes true
/// rather than a thing that is. Waiting on the channel rather than on a clock is what keeps that
/// from being flaky.
fn once_decoded(editor: &mut Editor<Grid>) -> Vec<Drawn> {
    // A frame first, because laying the picture out is what asks for it. Waiting before anything
    // has been requested waits for nothing, which is a ten-second test rather than a passing one.
    pictures_of(editor);
    assert!(
        editor.await_pictures(PATIENCE),
        "the decode worker answered nothing in {PATIENCE:?}"
    );
    pictures_of(editor)
}

/// How many glyphs were drawn inside `rect`.
///
/// **Told from a solid fill by its atlas region, not by its colour.** `Editor::solid` gives every
/// caret, rule and panel the single opaque texel at `[0, 0, 1, 1]`, and a glyph gets a real region.
/// Filtering on `theme.text` instead — which is what `lists.rs` does — counts nothing here: an
/// image's alt text is a link's text, so it draws in the link colour rather than the prose one.
///
/// A rectangle rather than the whole document area, because the chrome that draws glyphs is not all
/// above the document: the status bar is *below* it, and a top bound alone counts its text.
fn glyphs_inside(editor: &mut Editor<Grid>, rect: [f32; 4]) -> usize {
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    quads
        .iter()
        .filter(|quad| {
            quad.uv != [0.0, 0.0, 1.0, 1.0]
                && quad.rect[0] >= rect[0]
                && quad.rect[0] < rect[0] + rect[2]
                && quad.rect[1] >= rect[1]
                && quad.rect[1] < rect[1] + rect[3]
        })
        .count()
}

/// An editor on a document that is one image, with `cat.png` beside it at `size`.
fn one_picture(dir: &TempDir, size: (u32, u32)) -> Editor<Grid> {
    write_png(&dir.path().join("cat.png"), size.0, size.1);
    editor_on(dir, "![a cat](cat.png)\n").0
}

// ---------------------------------------------------------------------------------------------
// It draws.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_local_png_draws_as_a_picture() {
    let dir = TempDir::new().unwrap();
    let mut editor = one_picture(&dir, (100, 50));

    let drawn = pictures_of(&mut editor);

    assert_eq!(drawn.len(), 1, "{drawn:?}");
    assert_eq!(drawn[0].rect[2], 100.0, "width");
    assert_eq!(drawn[0].rect[3], 50.0, "height");
}

#[test]
fn the_picture_starts_at_the_text_margin() {
    let dir = TempDir::new().unwrap();
    let mut editor = one_picture(&dir, (100, 50));

    let drawn = pictures_of(&mut editor);

    assert_eq!(drawn[0].rect[0], MARGIN, "left edge");
}

#[test]
fn the_paragraph_is_as_tall_as_the_picture() {
    // The join that a unit test cannot see: the picture's height has to reach `blocks::stack`, or
    // everything below it is drawn over the top of it.
    let dir = TempDir::new().unwrap();
    let tall = one_picture(&dir, (100, 400));

    let mut with_picture = tall;
    let picture_top = pictures_of(&mut with_picture)[0].rect[1];

    // The same document with a paragraph after the image: it must sit below 400 pixels of picture.
    write_png(&dir.path().join("cat.png"), 100, 400);
    let (mut editor, _) = editor_on(&dir, "![a cat](cat.png)\n\nAfter.\n");
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    let lowest = quads
        .iter()
        .map(|quad| quad.rect[1])
        .fold(f32::MIN, f32::max);

    assert!(
        lowest > picture_top + 300.0,
        "text after a 400px picture was drawn at {lowest}, picture top {picture_top}"
    );
}

#[test]
fn the_alt_text_is_not_drawn_where_a_picture_is() {
    // The defect this ticket names, stated as the difference it makes: with the file present the
    // paragraph's glyphs go away and a picture takes their place; with it absent they stay. If both
    // ever draw, the alt text sits on top of the picture.
    let dir = TempDir::new().unwrap();
    // Tall enough that the whole first paragraph is inside it whichever way it draws.
    let mut drawing = one_picture(&dir, (400, 200));
    let (mut missing, _) = editor_on(&dir, "![a cat](nowhere.png)\n");

    let where_it_went = pictures_of(&mut drawing)[0].rect;

    assert_eq!(
        glyphs_inside(&mut drawing, where_it_went),
        0,
        "the alt text drew on top of the picture"
    );
    assert!(
        glyphs_inside(&mut missing, where_it_went) > 0,
        "the fallback drew nothing there either, so the check above proves nothing"
    );
}

#[test]
fn two_paragraphs_naming_one_file_share_a_picture() {
    // Two placements, one id — so one texture, one upload, and one decode.
    let dir = TempDir::new().unwrap();
    write_png(&dir.path().join("cat.png"), 60, 60);
    let (mut editor, _) = editor_on(&dir, "![](cat.png)\n\n![](cat.png)\n");

    let drawn = pictures_of(&mut editor);

    assert_eq!(drawn.len(), 2, "{drawn:?}");
    assert_eq!(drawn[0].id, drawn[1].id);

    let drawn = once_decoded(&mut editor);

    assert_eq!(drawn[0].id, drawn[1].id, "the id changed under the decode");
    assert!(editor.picture(drawn[0].id).is_some());
}

#[test]
fn a_picture_in_a_subdirectory_draws() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir(dir.path().join("images")).unwrap();
    write_png(&dir.path().join("images/cat.png"), 40, 40);
    let (mut editor, _) = editor_on(&dir, "![](images/cat.png)\n");

    assert_eq!(pictures_of(&mut editor).len(), 1);
}

// ---------------------------------------------------------------------------------------------
// Sizing, end to end.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_picture_wider_than_the_column_is_scaled_down_keeping_its_shape() {
    let dir = TempDir::new().unwrap();
    let mut editor = one_picture(&dir, (4000, 2000));

    let drawn = pictures_of(&mut editor);
    let [_, _, width, height] = drawn[0].rect;

    assert!(width <= WIDTH, "{width} did not fit a {WIDTH} window");
    assert!(
        (width / height - 2.0).abs() < 0.01,
        "2:1 became {width}x{height}"
    );
}

#[test]
fn a_small_picture_is_not_blown_up_to_the_column() {
    let dir = TempDir::new().unwrap();
    let mut editor = one_picture(&dir, (16, 16));

    assert_eq!(pictures_of(&mut editor)[0].rect[2], 16.0);
}

#[test]
fn the_device_scale_multiplies_the_rectangle() {
    // A picture's pixels are logical, as the web's are: at 200% it covers the same amount of the
    // page and gets twice as many physical pixels, rather than shrinking to half the size.
    let dir = TempDir::new().unwrap();
    let mut editor = one_picture(&dir, (100, 50));

    editor.build_frame(WIDTH * 2.0, HEIGHT * 2.0, 2.0);
    let drawn = editor.drawn_pictures()[0];

    assert_eq!(drawn.rect[2], 200.0, "width at 200%");
    assert_eq!(drawn.rect[3], 100.0, "height at 200%");
}

// ---------------------------------------------------------------------------------------------
// It does not draw.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_missing_file_draws_its_alt_text_instead() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor_on(&dir, "![a cat](nowhere.png)\n");

    assert!(pictures_of(&mut editor).is_empty());
}

#[test]
fn a_unc_destination_draws_nothing_and_reaches_no_network() {
    // The attack. A document you were sent must not make an SMB connection when you open it.
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor_on(&dir, "![](\\\\evil.example\\share\\x.png)\n");

    assert!(pictures_of(&mut editor).is_empty());
}

#[test]
fn a_remote_destination_draws_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor_on(&dir, "![](https://example.com/cat.png)\n");

    assert!(pictures_of(&mut editor).is_empty());
}

#[test]
fn a_file_that_is_not_a_png_draws_its_alt_text_and_does_not_crash() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("cat.png"),
        b"\xff\xd8\xff\xe0 actually a JPEG",
    )
    .unwrap();
    let (mut editor, _) = editor_on(&dir, "![a cat](cat.png)\n");

    assert!(pictures_of(&mut editor).is_empty());
}

#[test]
fn a_sentence_containing_an_image_draws_no_picture() {
    let dir = TempDir::new().unwrap();
    write_png(&dir.path().join("cat.png"), 40, 40);
    let (mut editor, _) = editor_on(&dir, "Look at ![a cat](cat.png) there.\n");

    assert!(pictures_of(&mut editor).is_empty());
}

#[test]
fn plain_mode_draws_the_syntax_and_no_picture() {
    // `Ctrl+/` exists to show the bytes, and the bytes of an image are its syntax. Same rule the
    // grid follows, and for the same reason.
    let dir = TempDir::new().unwrap();
    let mut editor = one_picture(&dir, (100, 50));
    assert_eq!(pictures_of(&mut editor).len(), 1, "preview draws it");

    editor.toggle_mode();

    assert!(
        pictures_of(&mut editor).is_empty(),
        "plain mode still drew a picture"
    );
}

#[test]
fn an_untitled_buffer_draws_no_picture_for_a_relative_destination() {
    // Nothing to resolve against. The working directory is whatever the editor was launched from,
    // which is not a thing to guess about — so it draws the alt text.
    let mut editor = Editor::new("![](cat.png)\n", Grid::new());

    assert!(pictures_of(&mut editor).is_empty());
}

// ---------------------------------------------------------------------------------------------
// The cache.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_second_frame_reuses_the_decoded_picture() {
    let dir = TempDir::new().unwrap();
    let mut editor = one_picture(&dir, (100, 50));
    let first = once_decoded(&mut editor)[0].id;

    // Deleting the file must not change the third frame: if anything re-read it, this fails. Waited
    // for above, because the worker reads the file itself — deleting it before it gets there would
    // be testing the race rather than the cache.
    std::fs::remove_file(dir.path().join("cat.png")).unwrap();
    let third = pictures_of(&mut editor);

    assert_eq!(third.len(), 1, "the picture was re-read and lost");
    assert_eq!(third[0].id, first);
    assert!(editor.picture(first).is_some(), "the pixels were dropped");
}

// ---------------------------------------------------------------------------------------------
// The decode is off the drawing thread (MDEDIT-156).
// ---------------------------------------------------------------------------------------------

#[test]
fn the_row_is_the_right_height_before_the_pixels_exist() {
    // **The whole point of measuring the header.** The first frame has no picture to draw and still
    // has to give the paragraph the 400 pixels it will need, or everything below it jumps when the
    // decode lands.
    let dir = TempDir::new().unwrap();
    let mut editor = one_picture(&dir, (100, 400));

    let drawn = pictures_of(&mut editor);

    assert_eq!(drawn.len(), 1, "{drawn:?}");
    assert_eq!(drawn[0].rect[3], 400.0, "height on the first frame");
    assert!(
        editor.picture(drawn[0].id).is_none(),
        "the decode happened on the thread that draws, which is the defect"
    );
}

#[test]
fn nothing_moves_when_the_picture_arrives() {
    // The promise, stated as the thing a reader would notice: the rectangle is identical before and
    // after, so no text reflows underneath it.
    let dir = TempDir::new().unwrap();
    write_png(&dir.path().join("cat.png"), 300, 200);
    let (mut editor, _) = editor_on(&dir, "![a cat](cat.png)\n\nAfter it.\n");

    let before = pictures_of(&mut editor)[0];
    let rows_before = glyph_rows(&mut editor);
    let after = once_decoded(&mut editor)[0];
    let rows_after = glyph_rows(&mut editor);

    assert_eq!(before.rect, after.rect, "the picture moved or resized");
    assert_eq!(rows_before, rows_after, "the text below moved");
    assert!(
        !rows_after.is_empty(),
        "no text below, so nothing was shown"
    );
    assert!(editor.picture(after.id).is_some(), "still no pixels");
}

#[test]
fn a_picture_over_the_caps_draws_its_alt_text_and_is_never_queued() {
    // 9000 is past `MAX_DIMENSION`, which is the hardware's texture limit rather than a policy. The
    // refusal happens in `measure` — *before* a row is sized for it and before the worker is asked
    // for anything, which is what stops a document you were sent from sizing a row it should not.
    let dir = TempDir::new().unwrap();
    write_png(&dir.path().join("cat.png"), 9000, 1);
    let (mut editor, _) = editor_on(&dir, "![a cat](cat.png)\n");

    assert!(pictures_of(&mut editor).is_empty());
    assert!(
        !editor.await_pictures(std::time::Duration::from_millis(250)),
        "a refused header still reached the decode worker"
    );
}

#[test]
fn a_header_that_measures_and_pixels_that_do_not_decode_leave_a_blank_row() {
    // A file being written while it is read, which is an ordinary thing to catch mid-copy. The
    // header is intact, so the row is sized; the image data is not, so no pixels ever arrive. What
    // must not happen is a panic, or a row that collapses under the reader afterwards.
    let dir = TempDir::new().unwrap();
    let whole = png_bytes(120, 90);
    std::fs::write(dir.path().join("cat.png"), &whole[..40]).unwrap();
    let (mut editor, _) = editor_on(&dir, "![a cat](cat.png)\n");

    let drawn = pictures_of(&mut editor);

    assert_eq!(drawn.len(), 1, "the header measured, so the row is sized");
    assert_eq!(drawn[0].rect[3], 90.0, "height from the header");

    assert!(
        !editor.await_pictures(PATIENCE),
        "a truncated file produced pixels"
    );
    let after = pictures_of(&mut editor);

    assert_eq!(after[0].rect, drawn[0].rect, "the row collapsed");
    assert!(editor.picture(after[0].id).is_none());
}

#[test]
fn editing_a_paragraph_beside_a_picture_does_not_disturb_it() {
    let dir = TempDir::new().unwrap();
    write_png(&dir.path().join("cat.png"), 100, 50);
    // Prose first, so the caret starts in it: typing into the image paragraph would stop it being
    // an image, which is correct behaviour and not what this is about.
    let (mut editor, _) = editor_on(&dir, "Prose.\n\n![](cat.png)\n");
    let before = pictures_of(&mut editor)[0];

    editor.insert_text("More ");
    let after = pictures_of(&mut editor);

    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id, before.id);
    assert_eq!(after[0].rect[2], before.rect[2]);
}

// ---------------------------------------------------------------------------------------------
// Against the real text engine.
// ---------------------------------------------------------------------------------------------

/// The stub answers every layout question by construction, and everything above goes through it.
///
/// This one does not. It exists because the stub cannot say whether the *real* pipeline reaches
/// `lay_out_picture` at all — the grid is a different `TextLayout`, and the layer between them is
/// exactly where MDEDIT-126's two defects lived. What is still unproven after this is only the wgpu
/// pass, which needs a device, and whether the picture *looks* right, which needs eyes.
///
/// Windows-only, like `icons.rs`, and for the same reason: `mdedit-text-dwrite` compiles to nothing
/// elsewhere.
#[cfg(windows)]
#[test]
fn a_picture_is_placed_by_the_real_text_engine_too() {
    use mdedit_text_dwrite::DirectWriteText;

    let dir = TempDir::new().unwrap();
    write_png(&dir.path().join("cat.png"), 100, 50);
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "![a cat](cat.png)\n\nProse after it.\n").unwrap();

    let mut workspace = Workspace::new();
    workspace.add(path);
    let engine = DirectWriteText::new().expect("DirectWrite is available on any supported Windows");
    let mut editor = Editor::with_workspace(workspace, engine);
    editor.activate_saved_tab();

    editor.build_frame(WIDTH, HEIGHT, 1.0);
    let drawn = editor.drawn_pictures().to_vec();

    assert_eq!(drawn.len(), 1, "{drawn:?}");
    assert_eq!(drawn[0].rect[2], 100.0, "width");
    assert_eq!(drawn[0].rect[3], 50.0, "height");

    assert!(
        editor.await_pictures(PATIENCE),
        "the decode worker answered nothing in {PATIENCE:?}"
    );
    editor.build_frame(WIDTH, HEIGHT, 1.0);

    assert!(editor.picture(drawn[0].id).is_some(), "no pixels to upload");
}

/// The rows glyphs were drawn on, inside the document area, deduplicated and in order.
fn glyph_rows(editor: &mut Editor<Grid>) -> Vec<i32> {
    let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "stub metrics are whole pixels"
    )]
    let mut rows: Vec<i32> = quads
        .iter()
        .filter(|quad| {
            quad.uv != [0.0, 0.0, 1.0, 1.0]
                && quad.rect[0] >= MARGIN
                && quad.rect[1] >= MARGIN + mdedit_app::toolbar::HEIGHT
                // The status bar draws glyphs *below* the document, so a top bound alone counts it.
                && quad.rect[1] < HEIGHT - mdedit_app::toolbar::HEIGHT
        })
        .map(|quad| quad.rect[1] as i32)
        .collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

#[test]
fn a_picture_is_followed_by_the_same_blank_line_a_paragraph_is() {
    // **The defect this test was written for.** The chunk's trailing line was synthesised with
    // height zero, and `Cached::new` subtracts a dropped line's height from the chunk's — so a
    // picture's chunk was exactly picture-tall and the blank line every other block gets after it
    // vanished. An image sat flush against the next paragraph while two paragraphs were a line
    // apart. Nothing else in this file could see it: the picture's rectangle was right, its height
    // was right, and the paragraph below it was still *below* it.
    //
    // Compared like for like by making the two first blocks **the same height** — a picture exactly
    // one line tall against a one-line paragraph — so the second block lands on the same row in
    // both, and the assertion needs no arithmetic about where a glyph sits inside its row.
    let dir = TempDir::new().unwrap();
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "stub metrics are whole positive pixels"
    )]
    let one_line = LINE_HEIGHT as u32;
    write_png(&dir.path().join("cat.png"), 100, one_line);

    let (mut after_picture, _) = editor_on(
        &dir,
        "![a](cat.png)

Second.
",
    );
    let (mut after_prose, _) = editor_on(
        &dir,
        "First.

Second.
",
    );

    let picture = pictures_of(&mut after_picture)[0].rect;
    assert_eq!(
        picture[3], LINE_HEIGHT,
        "the two first blocks must be the same height"
    );

    assert_eq!(
        glyph_rows(&mut after_picture),
        vec![glyph_rows(&mut after_prose)[1]],
        "the paragraph after a picture does not sit where one after a paragraph does"
    );
}

// ---------------------------------------------------------------------------------------------
// The caret, around a picture (MDEDIT-152).
//
// These live here rather than beside their siblings in `concealed_caret.rs` because a picture needs
// a real file on disk and a saved tab to resolve against, and that file's harness is `Editor::new`
// on a bare string. The rule they pin is that file's rule, though: a position where nothing is drawn
// is not a place the caret may be.
// ---------------------------------------------------------------------------------------------

/// Every caret position Right visits from the start of the document, until it stops moving.
fn walk_right(editor: &mut Editor<Grid>) -> Vec<usize> {
    while editor.press_left(false) && editor.document().selection().head().get() > 0 {}
    let mut seen = vec![editor.document().selection().head().get()];
    for _ in 0..40 {
        editor.press_right(false);
        let at = editor.document().selection().head().get();
        if seen.last() == Some(&at) {
            break;
        }
        seen.push(at);
    }
    seen
}

#[test]
fn right_steps_over_a_picture_in_one_press() {
    // `Be.\n\n![a](cat.png)\n\nAf.\n` — the construct is bytes 5..18, its alt text is byte 7.
    // Byte 7 draws nowhere once the picture is there, so stopping at it is a keypress that moves the
    // caret and changes nothing on screen.
    let dir = TempDir::new().unwrap();
    write_png(&dir.path().join("cat.png"), 60, 60);
    let (mut editor, _) = editor_on(&dir, "Be.\n\n![a](cat.png)\n\nAf.\n");
    assert_eq!(
        pictures_of(&mut editor).len(),
        1,
        "no picture, nothing to test"
    );

    let walk = walk_right(&mut editor);

    assert!(
        !walk.contains(&7),
        "the caret stopped inside the picture's alt text: {walk:?}"
    );
    assert!(walk.contains(&4) && walk.contains(&18), "{walk:?}");
}

#[test]
fn left_steps_over_a_picture_in_one_press() {
    let dir = TempDir::new().unwrap();
    write_png(&dir.path().join("cat.png"), 60, 60);
    let (mut editor, _) = editor_on(&dir, "Be.\n\n![a](cat.png)\n\nAf.\n");
    pictures_of(&mut editor);
    editor.select_all();
    editor.press_right(false);

    // Left from just after the construct must clear it in one press, not land in the alt text.
    // Bounded: `press_left` always reports true, so a regression that stops the caret moving would
    // hang here rather than fail, and a test that hangs tells nobody anything.
    for _ in 0..40 {
        if editor.document().selection().head().get() <= 18 {
            break;
        }
        editor.press_left(false);
    }
    assert_eq!(editor.document().selection().head().get(), 18, "setup");

    editor.press_left(false);

    assert!(
        editor.document().selection().head().get() <= 5,
        "Left landed at {} — inside the picture",
        editor.document().selection().head().get()
    );
}

#[test]
fn the_alt_text_of_a_broken_image_is_still_a_caret_stop() {
    // **The control.** The rule is "nothing is drawn there", not "it is an image". A missing file
    // draws its alt text, so the alt text is ordinary editable content and the caret belongs in it.
    // Without this, the fix for the two tests above would be to skip every image construct.
    let dir = TempDir::new().unwrap();
    let (mut editor, _) = editor_on(&dir, "Be.\n\n![a](nowhere.png)\n\nAf.\n");
    assert!(pictures_of(&mut editor).is_empty(), "setup: no picture");

    let walk = walk_right(&mut editor);

    assert!(
        walk.contains(&7),
        "the caret skipped alt text that is actually drawn: {walk:?}"
    );
}

#[test]
fn backspace_after_a_picture_takes_the_whole_image() {
    // **A consequence of the same predicate, not a separate decision.** Backspace and Delete ask
    // what is concealed just behind and just ahead of the caret, so teaching that predicate about
    // pictures changes these too — and it changes them the right way. `![](x.png)` with no alt text
    // is a single marker segment and has always been removed whole; without this, the same image
    // with alt text would come off one invisible byte at a time, which is the key that does nothing
    // until it suddenly does something that MDEDIT-118 forbids.
    let dir = TempDir::new().unwrap();
    write_png(&dir.path().join("cat.png"), 60, 60);
    let (mut editor, _) = editor_on(&dir, "Be.\n\n![a](cat.png)\n\nAf.\n");
    pictures_of(&mut editor);

    // Put the caret immediately after the construct, then take it off.
    for _ in 0..40 {
        if editor.document().selection().head().get() >= 18 {
            break;
        }
        editor.press_right(false);
    }
    assert_eq!(editor.document().selection().head().get(), 18, "setup");

    editor.press_backspace();

    assert_eq!(editor.document().to_string(), "Be.\n\n\n\nAf.\n");
}

#[test]
fn one_undo_restores_a_picture_deleted_whole() {
    // The bytes came off in one edit, so they must come back in one — the rule `delete_marker`
    // already follows for a link's two halves.
    let dir = TempDir::new().unwrap();
    write_png(&dir.path().join("cat.png"), 60, 60);
    let source = "Be.\n\n![a](cat.png)\n\nAf.\n";
    let (mut editor, _) = editor_on(&dir, source);
    pictures_of(&mut editor);
    for _ in 0..40 {
        if editor.document().selection().head().get() >= 18 {
            break;
        }
        editor.press_right(false);
    }
    editor.press_backspace();

    editor.apply_chord("z", false);

    assert_eq!(editor.document().to_string(), source);
}

#[test]
fn no_two_consecutive_caret_positions_draw_in_the_same_place() {
    // **The property, rather than a byte number.** `concealed_caret.rs` states the rule as "Left and
    // Right through them were a key that appeared to do nothing" — so the check that matters is not
    // "byte 7 is skipped" but "every press moves the caret somewhere the reader can see". Walking
    // Right across the whole document and comparing drawn rectangles says exactly that, and it would
    // still fail if the fix skipped the wrong span or a later change added a second dead position.
    let dir = TempDir::new().unwrap();
    write_png(&dir.path().join("cat.png"), 100, 100);
    let (mut editor, _) = editor_on(&dir, "Be.\n\n![a](cat.png)\n\nAf.\n");
    assert_eq!(pictures_of(&mut editor).len(), 1, "setup");

    let theme = mdedit_app::view::Theme::default();
    let wanted = [theme.caret.r, theme.caret.g, theme.caret.b, theme.caret.a];
    let mut seen: Vec<(usize, [f32; 4])> = Vec::new();
    for _ in 0..40 {
        let at = editor.document().selection().head().get();
        let quads = editor.build_frame(WIDTH, HEIGHT, 1.0);
        let caret = quads
            .iter()
            .find(|quad| quad.colour == wanted && quad.rect[2] < 4.0)
            .map_or_else(|| panic!("no caret drawn at byte {at}"), |quad| quad.rect);
        seen.push((at, caret));
        editor.press_right(false);
        if editor.document().selection().head().get() == at {
            break;
        }
    }

    for pair in seen.windows(2) {
        let [(before, first), (after, second)] = [pair[0], pair[1]];
        assert_ne!(
            first, second,
            "Right from byte {before} to {after} drew the caret in the same place"
        );
    }
}

#[test]
fn a_picture_span_the_buffer_has_outgrown_is_ignored() {
    // **The sharp edge of using the last frame's answer.** `picture_spans` describes the document as
    // it was drawn; an edit between two key presses can shorten it under one of those spans, and the
    // caret would then be handed an offset past the end of the buffer.
    //
    // Found by probing rather than by reasoning, and it was worse than the out-of-range offset it
    // was looked for: Backspace *silently did nothing* — `press_right` left the caret at 17 in a
    // ten-byte document, and the delete that followed found nothing to remove.
    let dir = TempDir::new().unwrap();
    write_png(&dir.path().join("cat.png"), 60, 60);
    // The image construct is bytes 4..17 of an eighteen-byte document.
    let (mut editor, _) = editor_on(&dir, "Xs\n\n![a](cat.png)\n");
    assert_eq!(pictures_of(&mut editor).len(), 1, "setup");

    // Truncate through the middle of the construct **without drawing a frame**, so the span from the
    // last frame outlives the bytes it described.
    editor
        .document_mut()
        .delete(ByteSpan::new(ByteOffset::new(10), ByteOffset::new(18)))
        .unwrap();
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(10)));

    editor.press_right(false);
    let after = editor.document().selection().head().get();
    assert!(
        after <= editor.document().len_bytes(),
        "the caret went to {after} in a {}-byte document",
        editor.document().len_bytes()
    );

    editor.press_backspace();
    assert_eq!(
        editor.document().to_string(),
        "Xs\n\n![a](",
        "Backspace did nothing"
    );
}
