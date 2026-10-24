//! The modal panel that asks for a link's or an image's parts (MDEDIT-129).
//!
//! **This reverses two decisions that were written down.** `make_link` called the clipboard case
//! "the case that needs no dialog at all", and `insert_image` called parking the caret in the empty
//! parentheses "the whole of why there is no dialog". Both were deliberate, and both missed that a
//! link has two parts and an image has three: a skeleton to fill in only helps when you already know
//! the shape.
//!
//! The fast path survives as one keystroke — the clipboard's URL and the selection are pre-filled,
//! so copy-select-Ctrl+K becomes copy-select-Ctrl+K-Enter, and you see it before it is written.
//!
//! Driven through the public methods rather than the key handler, for the reason `apply_chord`
//! gives: a winit `KeyEvent` carries a private platform field and cannot be constructed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    // Geometry constants compared against geometry constants: these are the same arithmetic, not two
    // roundings that happen to be close.
    clippy::float_cmp
)]

use mdedit_app::config::Config;
use mdedit_app::dialog::{self, Kind};
use mdedit_app::editor::Editor;
use mdedit_app::field::Press;
use mdedit_app::gpu::QuadInstance;
use mdedit_app::view::{Rgba, Theme, ThemeChoice};
use mdedit_document::Selection;
use mdedit_offsets::ByteOffset;

use crate::stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 400.0;

fn editor(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

fn select(editor: &mut Editor<Grid>, from: usize, to: usize) {
    editor
        .document_mut()
        .set_selection(Selection::new(ByteOffset::new(from), ByteOffset::new(to)));
}

/// Types into whichever field has the caret.
fn type_into(editor: &mut Editor<Grid>, text: &str) {
    editor.press_dialog(&Press::Text(text.to_owned()), false);
}

// ---- what it writes ----

/// Note what it replaces: the caret is in `prose`, and `dialog_target` takes the word at the caret
/// exactly as the toggles do — so the link is written *over* it rather than beside it.
#[test]
fn a_link_is_written_from_its_two_fields() {
    let mut editor = editor("prose\n");
    editor.link();
    type_into(&mut editor, "https://example.com");
    editor.tab_dialog(true);
    type_into(&mut editor, "the docs");

    assert!(editor.accept_dialog());
    assert_eq!(
        editor.document().to_string(),
        "[the docs](https://example.com)\n"
    );
}

#[test]
fn an_image_is_written_from_its_three() {
    let mut editor = editor("");
    editor.image();
    type_into(&mut editor, "pics/chart.png");
    editor.tab_dialog(true);
    type_into(&mut editor, "A bar chart");
    editor.tab_dialog(true);
    type_into(&mut editor, "Quarterly figures");

    assert!(editor.accept_dialog());
    assert_eq!(
        editor.document().to_string(),
        "![A bar chart](pics/chart.png \"Quarterly figures\")"
    );
}

/// A title is optional and an alt text is optional; only the destination is not.
#[test]
fn the_optional_fields_are_optional() {
    let mut editor = editor("");
    editor.image();
    type_into(&mut editor, "pic.png");

    assert!(editor.accept_dialog());
    assert_eq!(editor.document().to_string(), "![](pic.png)");
}

/// **The whole of the validation.** A link with no URL says nothing, so Enter refuses and the panel
/// stays up rather than closing on a press that did nothing.
#[test]
fn a_dialog_with_no_destination_refuses_and_stays_open() {
    let mut editor = editor("prose\n");
    editor.link();
    editor.tab_dialog(true);
    type_into(&mut editor, "some text but no url");

    assert!(!editor.accept_dialog());
    assert!(editor.dialog().is_some(), "it must not have closed");
    assert_eq!(editor.document().to_string(), "prose\n");
}

/// Escape writes nothing and leaves nothing to undo — the half a skeleton insertion never had.
#[test]
fn cancelling_writes_nothing() {
    let mut editor = editor("prose\n");
    editor.link();
    type_into(&mut editor, "https://example.com");

    assert!(editor.cancel_dialog());
    assert_eq!(editor.document().to_string(), "prose\n");
    assert!(!editor.document().can_undo());
}

// ---- what it replaces ----

/// The selection becomes the text, and the result replaces exactly the span the dialog showed.
#[test]
fn a_selection_becomes_the_text_and_is_replaced() {
    let mut editor = editor("see the docs here\n");
    select(&mut editor, 4, 12);
    editor.link();

    assert_eq!(editor.dialog().unwrap().fields()[1].text(), "the docs");
    type_into(&mut editor, "https://x.example");
    assert!(editor.accept_dialog());
    assert_eq!(
        editor.document().to_string(),
        "see [the docs](https://x.example) here\n"
    );
}

/// With no selection it takes the word at the caret, exactly as the toggles do.
#[test]
fn the_word_at_the_caret_becomes_the_text() {
    let mut editor = editor("see the docs here\n");
    editor
        .document_mut()
        .set_selection(Selection::caret(ByteOffset::new(9)));
    editor.image();

    assert_eq!(editor.dialog().unwrap().fields()[1].text(), "docs");
}

// ---- the fields ----

#[test]
fn tab_moves_forwards_and_wraps() {
    let mut editor = editor("");
    editor.link();
    assert_eq!(editor.dialog().unwrap().focused(), 0);

    editor.tab_dialog(true);
    assert_eq!(editor.dialog().unwrap().focused(), 1);
    editor.tab_dialog(true);
    assert_eq!(editor.dialog().unwrap().focused(), 0, "a form is a ring");
}

#[test]
fn shift_tab_moves_backwards() {
    let mut editor = editor("");
    editor.image();
    editor.tab_dialog(false);
    assert_eq!(editor.dialog().unwrap().focused(), 2, "wrapped to the last");
}

/// The caret starts in the first field with nothing in it, so it lands where there is work to do.
#[test]
fn the_caret_starts_in_the_first_empty_field() {
    let mut editor = editor("see the docs here\n");
    select(&mut editor, 4, 12);
    editor.link();

    assert_eq!(
        editor.dialog().unwrap().focused(),
        0,
        "the text was pre-filled from the selection, so the URL is what is left"
    );
}

// ---- escaping, which is the part that is easy to get wrong ----

/// **A space ends a destination as far as the parser is concerned**, so `](my file.png)` is a broken
/// link rather than a link to `my file.png`. CommonMark's answer is angle brackets, and a path with a
/// space in it is the commonest thing anybody will paste into this box.
#[test]
fn a_destination_with_a_space_is_bracketed() {
    let mut editor = editor("");
    editor.image();
    type_into(&mut editor, "pics/my chart.png");

    assert!(editor.accept_dialog());
    assert_eq!(editor.document().to_string(), "![](<pics/my chart.png>)");
}

/// And the round trip is what says the bracketing worked: the parser has to agree it is an image.
#[test]
fn a_bracketed_destination_still_parses_as_an_image() {
    let mut editor = editor("");
    editor.image();
    type_into(&mut editor, "pics/my chart.png");
    editor.accept_dialog();

    let doc = editor.document();
    assert!(
        doc.segments().iter().any(|segment| matches!(
            segment.kind(),
            mdedit_language::SegmentKind::Marker(mdedit_language::Marker::Image)
        )),
        "the parser did not read it as an image: {:?}",
        doc.to_string()
    );
}

/// A title is written inside quotes, so a quote in it has to be escaped or it ends the title early.
#[test]
fn a_quote_in_a_title_is_escaped() {
    let mut editor = editor("");
    editor.image();
    type_into(&mut editor, "pic.png");
    editor.tab_dialog(true);
    editor.tab_dialog(true);
    type_into(&mut editor, r#"the "good" one"#);

    assert!(editor.accept_dialog());
    assert_eq!(
        editor.document().to_string(),
        r#"![](pic.png "the \"good\" one")"#
    );
}

// ---- modality ----

/// **A click inside the panel never reaches the document**, even where the panel has no control.
/// That is what modal means, and letting one through would move the caret out from under the text
/// the dialog is about to replace.
#[test]
fn a_click_on_the_panels_background_is_swallowed() {
    let mut editor = editor("see the docs here\n");
    select(&mut editor, 4, 12);
    editor.link();
    let before = editor.document().selection();

    // The panel's own top-left corner, which is inside it and on nothing. `click` answers "does
    // the frame need redrawing", and nothing changed — what matters is that it went no further.
    let (x, y) = dialog::place(WIDTH, HEIGHT, dialog::height_of(2));
    editor.click(x + 2.0, y + 2.0);
    assert_eq!(editor.document().selection(), before, "the caret moved");
    assert!(editor.dialog().is_some(), "and it stayed open");
}

/// A click outside closes it, which is what every other panel here does.
#[test]
fn a_click_outside_closes_it() {
    let mut editor = editor("prose\n");
    editor.link();
    editor.click(WIDTH - 4.0, HEIGHT - 4.0);
    assert!(editor.dialog().is_none());
    assert_eq!(editor.document().to_string(), "prose\n");
}

/// Clicking a field's box puts the caret in it.
#[test]
fn clicking_a_field_focuses_it() {
    let mut editor = editor("");
    editor.link();
    let at = dialog::place(WIDTH, HEIGHT, dialog::height_of(2));
    let (bx, by, bw, bh) = dialog::field_rect(1, at);

    assert!(editor.click(bx + bw / 2.0, by + bh / 2.0));
    assert_eq!(editor.dialog().unwrap().focused(), 1);
}

/// And the two buttons do what they say.
#[test]
fn the_buttons_accept_and_cancel() {
    let at = dialog::place(WIDTH, HEIGHT, dialog::height_of(2));
    let [cancel, accept] = dialog::button_rects(2, at);

    let mut accepting = editor("");
    accepting.link();
    type_into(&mut accepting, "https://example.com");
    assert!(accepting.click(accept.0 + accept.2 / 2.0, accept.1 + accept.3 / 2.0));
    assert_eq!(accepting.document().to_string(), "[](https://example.com)");

    let mut cancelling = editor("prose\n");
    cancelling.link();
    type_into(&mut cancelling, "https://example.com");
    assert!(cancelling.click(cancel.0 + cancel.2 / 2.0, cancel.1 + cancel.3 / 2.0));
    assert!(cancelling.dialog().is_none());
    assert_eq!(cancelling.document().to_string(), "prose\n");
}

// ---- the panel's own geometry ----

#[test]
fn a_three_field_panel_is_taller_than_a_two_field_one() {
    assert!(dialog::height_of(3) > dialog::height_of(2));
}

/// Centred horizontally, and clamped rather than centred when the window is smaller than the panel:
/// text clipped at the right is readable from its start, and text clipped at the left is not.
#[test]
fn the_panel_is_centred_and_clamped() {
    let (x, _) = dialog::place(900.0, 600.0, 200.0);
    assert!((x - (900.0 - dialog::WIDTH) / 2.0).abs() < 0.01);

    let (x, y) = dialog::place(100.0, 100.0, 400.0);
    assert_eq!((x, y), (0.0, 0.0), "a tiny window shows the top-left");
}

/// Every field's box is inside the panel, at every size it comes in.
#[test]
fn no_field_hangs_off_the_panel() {
    for fields in 1..=3 {
        let at = (40.0, 60.0);
        for index in 0..fields {
            let (bx, by, bw, bh) = dialog::field_rect(index, at);
            assert!(
                bx >= at.0 && bx + bw <= at.0 + dialog::WIDTH,
                "field {index}"
            );
            assert!(
                by >= at.1 && by + bh <= at.1 + dialog::height_of(fields),
                "field {index} of {fields}"
            );
        }
        for rect in dialog::button_rects(fields, at) {
            assert!(rect.0 + rect.2 <= at.0 + dialog::WIDTH);
            assert!(rect.1 + rect.3 <= at.1 + dialog::height_of(fields));
        }
    }
}

/// The two dialogs' boxes line up, so the panel does not appear to jump when one replaces the other.
#[test]
fn both_dialogs_use_the_same_label_column() {
    let at = (0.0, 0.0);
    assert_eq!(dialog::field_rect(0, at).0, dialog::field_rect(1, at).0);
    assert_eq!(Kind::Link.labels().len(), 2);
    assert_eq!(Kind::Image.labels().len(), 3);
}

// ---- what the boxes draw (MDEDIT-129) ----

fn quads_of(editor: &mut Editor<Grid>) -> Vec<QuadInstance> {
    editor.build_frame(WIDTH, HEIGHT, 1.0)
}

fn matching(quads: &[QuadInstance], colour: Rgba) -> Vec<QuadInstance> {
    let wanted = [colour.r, colour.g, colour.b, colour.a];
    quads
        .iter()
        .filter(|quad| quad.colour == wanted)
        .copied()
        .collect()
}

fn dark(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new(text, Grid::new());
    // Pinned for the reason `caret_line.rs` gives: `system` resolves against the runner's machine,
    // and these assertions name colours out of `Theme::dark`.
    editor.with_config(Config {
        theme: ThemeChoice::Dark,
        ..Config::default()
    });
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// **Selecting text in the box drew nothing.** The field tracked a selection — Ctrl+A set one, and
/// typing replaced it — but the box painted only glyphs and a caret, so the state existed and was
/// invisible. Select-all then type looked like a keystroke that ate the field at random.
#[test]
fn a_selection_in_a_field_is_painted() {
    let mut editor = dark("");
    editor.link();
    editor.press_dialog(&Press::Text("https://example.com".to_owned()), false);

    let before = matching(&quads_of(&mut editor), Theme::dark().selection).len();
    editor.press_dialog(&Press::SelectAll, false);
    let after = matching(&quads_of(&mut editor), Theme::dark().selection);

    assert_eq!(before, 0, "nothing selected, nothing painted");
    assert_eq!(after.len(), 1, "the selected run wants exactly one quad");
    assert!(after[0].rect[2] > 0.0, "and it must have width");
}

/// The highlight belongs to the box it is in. A quad wider than the box painted over the label
/// column to its left and the panel's edge to its right — which is what "the text overflows past
/// the box" looked like once a long URL was selected.
#[test]
fn the_highlight_stays_inside_its_box() {
    let mut editor = dark("");
    editor.link();
    editor.press_dialog(
        &Press::Text("https://example.com/a/very/long/path/that/will/not/fit".to_owned()),
        false,
    );
    editor.press_dialog(&Press::SelectAll, false);

    let open = editor.dialog().expect("the dialog is open").clone();
    let at = dialog::place(WIDTH, HEIGHT, dialog::height_of(open.fields().len()));
    let (box_x, _, box_width, _) = dialog::field_rect(0, at);

    let quads = quads_of(&mut editor);
    let quad = matching(&quads, Theme::dark().selection);
    let quad = quad.first().expect("a selection is painted");
    assert!(
        quad.rect[0] >= box_x && quad.rect[0] + quad.rect[2] <= box_x + box_width,
        "the highlight left the box it belongs to"
    );
}

/// **A field you cannot see what you are typing into is not a field.** Text longer than the box
/// scrolls so the caret stays in view; without it, everything past the right edge was typed blind.
#[test]
fn a_long_value_scrolls_to_keep_the_caret_in_view() {
    let mut editor = dark("");
    editor.link();
    editor.press_dialog(
        &Press::Text("https://example.com/a/very/long/path/that/will/not/fit".to_owned()),
        false,
    );

    let open = editor.dialog().expect("the dialog is open").clone();
    let at = dialog::place(WIDTH, HEIGHT, dialog::height_of(open.fields().len()));
    let (box_x, box_y, box_width, box_height) = dialog::field_rect(0, at);

    let quads = quads_of(&mut editor);
    let caret = matching(&quads, Theme::dark().caret);
    // Narrow, because the focus ring is this colour too — and on the field's row, because **so is
    // the document's own caret**, which is drawn first and would otherwise be the one found.
    let caret = caret
        .iter()
        .find(|quad| {
            quad.rect[2] <= 4.0 && quad.rect[1] >= box_y && quad.rect[1] < box_y + box_height
        })
        .expect("the focused field draws a caret");
    assert!(
        caret.rect[0] >= box_x && caret.rect[0] + caret.rect[2] <= box_x + box_width,
        "the caret is off the end of the box, so the text is being typed blind"
    );
}

/// **The dim is translucent, and its alpha is its own.** `draw_dialog` takes the page colour and
/// overrides `a`; without that override the dim inherits the background's opaque alpha and the
/// document behind the panel vanishes instead of reading as behind glass. `Theme` has no dim colour
/// to name, so the test pins the alpha against the background's own, which is the value the defect
/// would produce.
#[test]
fn the_dim_is_translucent_and_keeps_the_page_colour() {
    let mut editor = dark("prose\n");
    let theme = Theme::dark();
    editor.link();

    let quads = quads_of(&mut editor);
    let dim: Vec<QuadInstance> = quads
        .iter()
        .filter(|quad| {
            quad.rect == [0.0, 0.0, WIDTH, HEIGHT]
                && quad.colour[..3] == [theme.background.r, theme.background.g, theme.background.b]
        })
        .copied()
        .collect();
    assert_eq!(dim.len(), 1, "one dim over the whole window");
    let alpha = dim[0].colour[3];
    assert!(
        alpha > 0.0 && alpha < theme.background.a,
        "the dim is {alpha} alpha, which is not translucent"
    );
}
