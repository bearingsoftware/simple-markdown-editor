//! Choosing an encoding, and the one exception to byte-identical saves.
//!
//! The rule that matters most here is the refusal: **content the target encoding cannot hold must
//! fail the save and write nothing**, rather than silently substituting replacement characters. An
//! editor that turns 汉 into `&#27721;` and reports success has lost the user's document while
//! telling them it saved it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_file::{FileError, choices, load, load_as, save};
use tempfile::TempDir;

fn write(dir: &TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

// ---- the list ----

/// **The order is the contract**: a menu click reports a position, so reordering this list changes
/// what every stored index means. Written down so a reorder is a decision rather than an accident.
#[test]
fn the_choices_are_in_the_order_the_menu_shows_them() {
    let labels: Vec<&str> = choices().iter().map(|choice| choice.label).collect();
    assert_eq!(
        labels,
        [
            "UTF-8",
            "UTF-8 with BOM",
            "UTF-16 LE",
            "UTF-16 BE",
            "windows-1252",
            "GBK",
            "Shift_JIS",
        ]
    );
}

/// UTF-8 and UTF-8 with BOM are the same encoding and different answers, which is the whole reason a
/// choice is an encoding *and* a BOM rather than just an encoding.
#[test]
fn utf8_with_and_without_a_bom_are_different_choices() {
    let dir = TempDir::new().unwrap();
    let plain = load(&write(&dir, "plain.txt", b"hello\n")).unwrap();
    let marked = load(&write(&dir, "marked.txt", b"\xef\xbb\xbfhello\n")).unwrap();

    assert!(choices()[0].matches(plain.format()));
    assert!(!choices()[1].matches(plain.format()));
    assert!(choices()[1].matches(marked.format()));
    assert!(!choices()[0].matches(marked.format()));
}

// ---- reopening ----

/// The acceptance criterion. `é` is one byte in windows-1252 and not valid UTF-8 on its own, so the
/// file cannot even be read as UTF-8 — which is the "opens as an error" gap `AGENTS.md` recorded.
#[test]
fn a_windows_1252_file_reopens_correctly_and_round_trips() {
    let dir = TempDir::new().unwrap();
    let bytes = b"caf\xe9 na\xefve\n";
    let path = write(&dir, "legacy.txt", bytes);

    assert!(load(&path).is_err(), "not valid UTF-8, and not guessed at");

    let file = load_as(&path, encoding_rs::WINDOWS_1252).unwrap();
    assert_eq!(file.text(), "café naïve\n");
    assert_eq!(file.format().encoding(), "windows-1252");

    // Byte-identical on save, because nothing was edited.
    save(&path, file.text(), &file).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

/// A UTF-8 file with a BOM keeps it through everything, which is the acceptance criterion phrased as
/// a test.
#[test]
fn a_bom_survives() {
    let dir = TempDir::new().unwrap();
    let bytes = b"\xef\xbb\xbfhello\n";
    let path = write(&dir, "bom.txt", bytes);

    let file = load(&path).unwrap();
    assert!(file.format().has_bom());
    assert_eq!(file.text(), "hello\n", "the BOM is format, not content");

    save(&path, "hello there\n", &file).unwrap();
    let written = std::fs::read(&path).unwrap();
    assert!(written.starts_with(b"\xef\xbb\xbf"), "the BOM must be back");
}

// ---- saving with a different encoding ----

/// **The bug this would have had.** `save` writes the original bytes verbatim when the text is
/// unedited, so without disarming that shortcut an encoding change on an untouched buffer would
/// write the *old* bytes and report success.
#[test]
fn changing_the_encoding_rewrites_even_an_unedited_buffer() {
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "plain.txt", b"hello\n");
    let mut file = load(&path).unwrap();

    // UTF-8 with BOM.
    assert!(file.set_encoding(choices()[1]));
    save(&path, file.text(), &file).unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), b"\xef\xbb\xbfhello\n");
}

#[test]
fn saving_as_utf16_writes_utf16() {
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "plain.txt", b"hi\n");
    let mut file = load(&path).unwrap();

    assert!(file.set_encoding(choices()[2]), "UTF-16 LE");
    save(&path, file.text(), &file).unwrap();

    let written = std::fs::read(&path).unwrap();
    assert_eq!(written, b"\xff\xfeh\x00i\x00\n\x00");

    // And it reads back as what it now is.
    let back = load(&path).unwrap();
    assert_eq!(back.text(), "hi\n");
    assert_eq!(back.format().encoding(), "UTF-16LE");
}

#[test]
fn choosing_the_encoding_it_already_has_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "plain.txt", b"hello\n");
    let mut file = load(&path).unwrap();

    assert!(
        !file.set_encoding(choices()[0]),
        "already UTF-8 without a BOM"
    );
}

/// MDEDIT-103. Picking a different encoding and then picking the original one back has to leave the
/// file exactly as it was found — including the two flags that decide how it is written.
///
/// `set_encoding` used to latch `re_encode = true` and `round_trips = true` and never restore
/// either, and `choice.matches(self.format)` is false on the way back as well as on the way out. So
/// after two clicks an **unedited** buffer skipped both the byte-identical fast path and the
/// `NotRoundTrippable` refusal, and was re-encoded. For Shift-JIS, whose duplicate mappings this
/// crate's own fixtures prove are not reversible, that silently rewrites bytes nobody touched.
#[test]
fn choosing_an_encoding_and_changing_back_leaves_the_bytes_alone() {
    let dir = TempDir::new().unwrap();
    // `ED 40` is one of the Shift-JIS duplicate mappings: it decodes to a character that encodes
    // back as `FA 5C`, so this file provably does not round-trip.
    let original = b"\x82\xa0\xed\x40\n".to_vec();
    let path = write(&dir, "sjis.txt", &original);

    let mut file = load_as(&path, encoding_rs::SHIFT_JIS).unwrap();
    assert!(!file.round_trips(), "the fixture has to be a lossy one");

    assert!(file.set_encoding(choices()[0]), "UTF-8");
    assert!(file.set_encoding(choices()[6]), "and back to Shift-JIS");

    save(&path, file.text(), &file).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        original,
        "a toggle and a toggle back must not rewrite a byte"
    );
}

/// The same, for a file that *does* round-trip: back where it started means the fast path again, not
/// a re-encode that happens to agree.
#[test]
fn changing_back_disarms_the_re_encode() {
    let dir = TempDir::new().unwrap();
    let original = b"hello\n".to_vec();
    let path = write(&dir, "plain.txt", &original);

    let mut file = load(&path).unwrap();
    assert!(file.set_encoding(choices()[2]), "UTF-16 LE");
    assert!(file.set_encoding(choices()[0]), "and back to UTF-8");

    save(&path, file.text(), &file).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

/// MDEDIT-103. Reopen with Encoding exists for the file the editor cannot decide about, so the one
/// thing it must never do is decide anyway.
///
/// `Encoding::decode` sniffs for a byte-order mark and switches encoding on one. A windows-1252 file
/// beginning `FF FE` — two perfectly ordinary characters in that encoding — was therefore reopened
/// as UTF-16LE, overriding the explicit choice this whole feature exists to honour.
#[test]
fn reopening_with_an_encoding_ignores_a_byte_order_mark() {
    let dir = TempDir::new().unwrap();
    // `FF FE` is a UTF-16LE BOM, and in windows-1252 it is `ÿþ`.
    let path = write(&dir, "cp1252.txt", b"\xff\xfehello\n");

    let file = load_as(&path, encoding_rs::WINDOWS_1252).unwrap();
    assert_eq!(file.format().encoding(), "windows-1252");
    assert_eq!(file.text(), "ÿþhello\n");
    assert!(
        !file.format().has_bom(),
        "those bytes are content in this encoding, not a mark"
    );
}

/// And the BOM is still stripped when it *is* the mark for the encoding being read, or every
/// UTF-8-with-BOM file would grow a zero-width no-break space at the front.
#[test]
fn a_matching_byte_order_mark_is_still_format_rather_than_content() {
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "bom.txt", b"\xef\xbb\xbfhello\n");

    let file = load_as(&path, encoding_rs::UTF_8).unwrap();
    assert_eq!(file.text(), "hello\n");
    assert!(file.format().has_bom());
}

/// **The refusal, and the reason this ticket names it.** `encoding_rs` substitutes HTML numeric
/// character references for anything it cannot map, so a save that ignored the error would write
/// `&#27721;` where the user had 汉 and report success.
#[test]
fn content_the_target_cannot_hold_fails_the_save_and_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "cjk.txt", "汉字\n".as_bytes());
    let before = std::fs::read(&path).unwrap();

    let mut file = load(&path).unwrap();
    assert!(file.set_encoding(choices()[4]), "windows-1252");

    let error = save(&path, file.text(), &file).unwrap_err();
    assert!(
        matches!(error, FileError::Unrepresentable { .. }),
        "got {error:?}"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "a refused save must leave the file exactly as it was"
    );
}

/// And the same content in an encoding that *can* hold it goes through, so the refusal is about the
/// content rather than about legacy encodings being distrusted.
#[test]
fn the_same_content_in_an_encoding_that_can_hold_it_saves() {
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "cjk.txt", "汉字\n".as_bytes());
    let mut file = load(&path).unwrap();

    assert!(file.set_encoding(choices()[5]), "GBK");
    save(&path, file.text(), &file).unwrap();

    let back = load_as(&path, encoding_rs::GBK).unwrap();
    assert_eq!(back.text(), "汉字\n");
}
