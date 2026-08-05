//! The supreme invariant, end to end across the crates that implement it.
//!
//! `AGENTS.md` puts it above everything else: *opening a file and saving it must produce an
//! identical byte sequence.* Each crate proves its own half — `mdedit-file` that a load and a save
//! round-trip, `mdedit-document` that an undo restores the bytes — and until now **nothing put the
//! two together** (MDEDIT-110). The seam between them is where a violation would live: the buffer
//! goes through a rope, an undo history and a re-parse, and only then back to a `save` that decides
//! between writing the original bytes and re-encoding.
//!
//! This crate is where the two meet — an `OpenTab` holds the `Document` and the `LoadedFile`
//! together — which is why the test lives here rather than in either half.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use mdedit_document::Document;
use mdedit_file::{choices, load_with, save};
use mdedit_offsets::{ByteOffset, ByteSpan};
use proptest::prelude::*;
use tempfile::TempDir;

/// A file that is UTF-8 and therefore readable as every choice's own encoding is not — so the
/// fixtures below are built by *saving* through the encoding rather than by hand, which is the only
/// way to get canonical bytes for the legacy ones without a table.
fn fixtures() -> Vec<(&'static str, Vec<u8>, usize)> {
    let text = "# Title\r\n\r\nbody with café and 漢字\nand a second line\n";
    let plain = "# Title\n\nplain ascii body\n";

    let mut all: Vec<(&'static str, Vec<u8>, usize)> = vec![
        ("utf8-mixed", text.as_bytes().to_vec(), 0),
        ("utf8-plain", plain.as_bytes().to_vec(), 0),
        (
            "utf8-bom",
            [b"\xEF\xBB\xBF".as_slice(), plain.as_bytes()].concat(),
            1,
        ),
        ("utf8-no-trailing-newline", b"no newline at eof".to_vec(), 0),
        ("utf8-empty", Vec::new(), 0),
    ];

    // UTF-16 LE and BE, built by encoding the text through the same path the editor writes with.
    for (name, index) in [("utf16le", 2), ("utf16be", 3)] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("seed.md");
        std::fs::write(&path, text.as_bytes()).unwrap();
        let mut seed = mdedit_file::load(&path).unwrap();
        assert!(seed.set_encoding(choices()[index]));
        save(&path, seed.text(), &seed).unwrap();
        all.push((name, std::fs::read(&path).unwrap(), index));
    }
    all
}

/// Loads, edits, undoes the edit, and saves — the whole seam.
fn round_trip(name: &str, bytes: &[u8], choice: usize, edit: impl Fn(&mut Document)) {
    let dir = TempDir::new().unwrap();
    let path: PathBuf = dir.path().join("notes.md");
    std::fs::write(&path, bytes).unwrap();

    let file = load_with(&path, choices()[choice]).unwrap_or_else(|error| {
        panic!("{name} should load as {}: {error}", choices()[choice].label)
    });
    let mut doc = Document::from(file.text());

    edit(&mut doc);
    // Every edit this makes is one undo group, so one press takes it all back. If that stops being
    // true the assertion below fails on the text rather than on the bytes, which is the right place
    // for it to fail.
    while doc.undo() {}
    assert_eq!(
        doc.to_string(),
        file.text(),
        "{name}: undo did not restore the buffer"
    );

    save(&path, &doc.to_string(), &file)
        .unwrap_or_else(|error| panic!("{name} should save: {error}"));
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "{name}: a load, an edit, an undo and a save changed the file"
    );
}

#[test]
fn typing_and_undoing_leaves_every_byte_where_it_was() {
    for (name, bytes, choice) in fixtures() {
        round_trip(name, &bytes, choice, |doc| {
            doc.insert(ByteOffset::ZERO, "typed").unwrap();
        });
    }
}

/// A delete rather than an insert, because the two take different paths through the history.
#[test]
fn deleting_and_undoing_leaves_every_byte_where_it_was() {
    for (name, bytes, choice) in fixtures() {
        round_trip(name, &bytes, choice, |doc| {
            let end = doc.len_bytes().min(3);
            let _ = doc.delete(ByteSpan::new(ByteOffset::ZERO, ByteOffset::new(end)));
        });
    }
}

/// And an inline toggle, which is two edits in one transaction and the operation most likely to
/// leave a stray delimiter behind — the differentiator this project is built on.
#[test]
fn formatting_and_undoing_leaves_every_byte_where_it_was() {
    for (name, bytes, choice) in fixtures() {
        round_trip(name, &bytes, choice, |doc| {
            doc.set_selection(mdedit_document::Selection::caret(ByteOffset::new(
                doc.len_bytes().min(4),
            )));
            doc.toggle_inline(mdedit_document::format::Inline::Bold);
        });
    }
}

proptest! {
    /// The same thing over generated edits: whatever is typed, wherever it is typed, undoing it all
    /// gives back a file identical to the one that was opened.
    #[test]
    fn any_sequence_of_edits_undoes_back_to_the_original_bytes(
        edits in prop::collection::vec((0_usize..80, 0_usize..4, "(?s)[a-z\n*# ]{0,6}"), 1..8),
    ) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("notes.md");
        let bytes = "# Title\r\n\r\nbody with café\nand more\n".as_bytes().to_vec();
        std::fs::write(&path, &bytes).unwrap();

        let file = mdedit_file::load(&path).unwrap();
        let mut doc = Document::from(file.text());

        for (at, len, replacement) in edits {
            let current = doc.to_string();
            let start = boundary(&current, at);
            let end = boundary(&current, start + len);
            doc.replace(
                ByteSpan::new(ByteOffset::new(start), ByteOffset::new(end)),
                &replacement,
            )
            .unwrap();
        }
        while doc.undo() {}

        prop_assert_eq!(doc.to_string(), file.text());
        save(&path, &doc.to_string(), &file).unwrap();
        prop_assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

/// Snaps an offset onto a character boundary at or below it.
fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}
