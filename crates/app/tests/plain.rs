//! A new file is plain text until it is saved as something (MDEDIT-93).
//!
//! **This reverses a decision that was written down.** `detect.rs` said an unknown extension, or
//! none, is Markdown — "this is a Markdown editor; that is the right default and not an admission of
//! defeat". The reversal is that guessing Markdown means *styling a file that is not Markdown*, and
//! concealment makes a wrong guess worse than a wrong colour: a `notes.txt` full of shell commands
//! draws its `#` comments as headings and loses the asterisks in `*.log`.
//!
//! So the cue that a file is Markdown is the same cue that makes it Markdown when you save it — its
//! name. Everything here is about the moment that name arrives.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_app::editor::Editor;
use mdedit_app::view::{Decoration, Mode, Theme, build_runs_in};
use mdedit_offsets::{ByteOffset, ByteSpan};
use tempfile::TempDir;

mod stub;
use stub::{Grid, MARGIN};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = MARGIN + 400.0;
const MARKDOWN: &str = "# Heading\n\n**bold** text\n";

/// An editor showing one untitled buffer with `text` typed into it.
///
/// **Through `new_file`, not `Editor::new`.** The first goes through `Workspace::add_untitled`,
/// which is where MDEDIT-93's "a new buffer is plain" lives; the second builds a bare `Document`
/// for a test that does not care about tabs, and is still Markdown.
fn typed(text: &str) -> Editor<Grid> {
    let mut editor = Editor::new("", Grid::new());
    editor.new_file();
    editor.insert_text(text);
    let _ = editor.build_frame(WIDTH, HEIGHT, 1.0);
    editor
}

/// Everything the decoration step would draw for the active document, as one string.
fn drawn(editor: &Editor<Grid>) -> String {
    let doc = editor.document();
    let text = doc.to_string();
    build_runs_in(
        doc,
        Decoration {
            theme: &Theme::default(),
            mode: Mode::Preview,
        },
        ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(text.len())),
        &text,
    )
    .iter()
    .map(|run| run.text)
    .collect()
}

// ---- a new buffer ----

#[test]
fn a_new_buffer_is_plain_text() {
    let editor = typed(MARKDOWN);
    assert_eq!(editor.document().language().name(), "Plain Text");
}

/// **Plain means every byte is drawn**, which is the visible half of it: nothing concealed, so the
/// `#` and the `**` are still there.
#[test]
fn a_new_buffer_conceals_nothing() {
    let editor = typed(MARKDOWN);
    assert!(!editor.document().language().conceals());
    assert_eq!(
        drawn(&editor),
        MARKDOWN,
        "a plain document draws itself exactly"
    );
}

/// And its segments still tile, because tiling is not optional for any language.
#[test]
fn a_plain_document_still_tiles() {
    let editor = typed(MARKDOWN);
    let doc = editor.document();
    let text = doc.to_string();
    let rebuilt: String = doc
        .segments()
        .iter()
        .map(|segment| segment.span().slice(&text))
        .collect();
    assert_eq!(rebuilt, text);
}

#[test]
fn an_empty_plain_document_has_no_segments() {
    let editor = Editor::new("", Grid::new());
    assert!(editor.document().segments().is_empty());
}

// ---- the moment it is saved ----

/// **The defect this ticket found while being planned.** `bind_to_path` set the origin and the file
/// and left the document's language alone, so an untitled buffer saved as `data.json` kept being
/// parsed as Markdown until the tab was closed and reopened.
///
/// "Plain until you save it as something" cannot work without this, and it was worth fixing either
/// way.
#[test]
fn saving_as_json_makes_it_json_without_a_reopen() {
    let dir = TempDir::new().unwrap();
    let mut editor = typed(r#"{"a": 1}"#);
    assert_eq!(editor.document().language().name(), "Plain Text");

    assert!(editor.save_as(&dir.path().join("data.json")));
    assert_eq!(editor.document().language().name(), "JSON");
}

#[test]
fn saving_as_md_makes_it_markdown() {
    let dir = TempDir::new().unwrap();
    let mut editor = typed(MARKDOWN);

    assert!(editor.save_as(&dir.path().join("notes.md")));
    assert_eq!(editor.document().language().name(), "Markdown");
    assert_eq!(
        drawn(&editor),
        "Heading\n\nbold text\n",
        "and it conceals its syntax now, which is the whole point of the name"
    );
}

/// Saving under a name the editor knows nothing about leaves it plain.
#[test]
fn saving_as_txt_leaves_it_plain() {
    let dir = TempDir::new().unwrap();
    let mut editor = typed(MARKDOWN);

    assert!(editor.save_as(&dir.path().join("notes.txt")));
    assert_eq!(editor.document().language().name(), "Plain Text");
}

/// **A language change is a cache change**, and the layout cache cannot see one: its key is a hash
/// of the bytes plus the block kind and the wrapping width, none of which moved. A re-languaged tab
/// drawing its old spans looks like a rendering bug and is a staleness bug.
///
/// Asserted against a **reference** editor that opened the same bytes as Markdown from the start,
/// rather than against a direction of travel. That is what makes it a staleness test: a stale cache
/// would leave the re-languaged frame equal to the *plain* one, and a correct one leaves it equal to
/// the reference. Either way round, "it changed" is not enough — it has to have changed into the
/// right thing.
#[test]
fn a_re_languaged_frame_is_the_frame_that_language_draws() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("notes.md");

    let mut editor = typed(MARKDOWN);
    let plain = editor.build_frame(WIDTH, HEIGHT, 1.0).len();
    assert!(editor.save_as(&path));
    let after = editor.build_frame(WIDTH, HEIGHT, 1.0).len();

    let reference = {
        let mut workspace = mdedit_session::Workspace::new();
        workspace.add(path.clone());
        let mut reference = Editor::with_workspace(workspace, Grid::new());
        reference.activate_saved_tab();
        // The same caret, or the comparison is one quad out for a reason that has nothing to do
        // with the language: typing left it at the end, and a freshly opened file starts at zero.
        reference
            .document_mut()
            .set_selection(mdedit_document::Selection::caret(
                editor.document().selection().head(),
            ));
        reference.build_frame(WIDTH, HEIGHT, 1.0).len()
    };

    assert_ne!(after, plain, "the frame is still the plain one");
    assert_eq!(
        after, reference,
        "and it is not the frame this file drawn as Markdown produces"
    );
}

/// The bytes do not move, and that is the rule this must not break while everything else changes.
#[test]
fn re_languaging_touches_no_byte() {
    let dir = TempDir::new().unwrap();
    let mut editor = typed(MARKDOWN);

    assert!(editor.save_as(&dir.path().join("notes.md")));
    assert_eq!(editor.document().to_string(), MARKDOWN);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("notes.md")).unwrap(),
        MARKDOWN
    );
}

/// The generated help page is Markdown the editor wrote, and has no path to resolve from. Its
/// comment already said so; this is what keeps it true.
#[test]
fn a_generated_page_is_still_markdown() {
    let mut workspace = mdedit_session::Workspace::new();
    let index = workspace.add_generated("Shortcuts", MARKDOWN);
    let mut editor = Editor::with_workspace(workspace, Grid::new());
    editor.show_tab(index);

    assert_eq!(editor.document().language().name(), "Markdown");
}

/// **The frame settles**, which is the half of the cache question a test can actually answer.
///
/// The language is in `ChunkKey` as well — see the comment there — but no pair of today's languages
/// can collide on the rest of the key, so removing it breaks nothing and a test claiming otherwise
/// would be vacuous. What is real is this: after a re-language the first frame and the second agree,
/// so nothing is alternating between a cached answer and a fresh one.
#[test]
fn a_re_languaged_frame_settles() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("notes.md");

    let mut editor = typed(MARKDOWN);
    let plain = editor.build_frame(WIDTH, HEIGHT, 1.0).len();
    assert!(editor.save_as(&path));

    // Twice: the first frame after the save is the one that could be served stale, and the second
    // is the one that proves the answer settled rather than alternating.
    let first = editor.build_frame(WIDTH, HEIGHT, 1.0).len();
    let second = editor.build_frame(WIDTH, HEIGHT, 1.0).len();

    assert_ne!(first, plain, "the plain layout was served for Markdown");
    assert_eq!(first, second, "the second frame disagreed with the first");
}
