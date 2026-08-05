//! Byte preservation across a real filesystem round trip.
//!
//! Deliberately **not** driven by the CommonMark corpus. That corpus is entirely UTF-8 with LF
//! endings, so it would pass against an implementation that mangles every other encoding — which is
//! exactly the bug class this exists to catch. The fixtures here are built byte by byte.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use encoding_rs::{SHIFT_JIS, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};
use mdedit_file::{FileError, Newline, load, load_as, save};
use tempfile::TempDir;

/// Writes `bytes` to a scratch file and returns its path, with the directory kept alive.
fn fixture(dir: &TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn utf16le(text: &str, bom: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if bom {
        out.extend_from_slice(&[0xFF, 0xFE]);
    }
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

fn utf16be(text: &str, bom: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if bom {
        out.extend_from_slice(&[0xFE, 0xFF]);
    }
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

/// Every fixture, as `(name, bytes, encoding to read it as)`.
fn fixtures() -> Vec<(&'static str, Vec<u8>, &'static encoding_rs::Encoding)> {
    let mut all: Vec<(&'static str, Vec<u8>, &'static encoding_rs::Encoding)> = vec![
        ("utf8-lf.md", b"# Title\n\nbody\n".to_vec(), UTF_8),
        ("utf8-crlf.md", b"# Title\r\n\r\nbody\r\n".to_vec(), UTF_8),
        // Mixed endings: the case that newline normalisation would silently rewrite.
        ("utf8-mixed.md", b"one\r\ntwo\nthree\r\n".to_vec(), UTF_8),
        // No trailing newline: the case a naive writer likes to add one to.
        ("utf8-no-trailing.md", b"no newline at eof".to_vec(), UTF_8),
        ("utf8-empty.md", Vec::new(), UTF_8),
        (
            "utf8-bom.md",
            [b"\xEF\xBB\xBF".as_slice(), b"# With BOM\n"].concat(),
            UTF_8,
        ),
        (
            "utf8-multibyte.md",
            "汉字 🎉 é\n".as_bytes().to_vec(),
            UTF_8,
        ),
        (
            "utf16le-bom.md",
            utf16le("# Title\r\nbody\r\n", true),
            UTF_16LE,
        ),
        ("utf16be-bom.md", utf16be("# Title\nbody\n", true), UTF_16BE),
        // Without a BOM these need an explicit encoding; `load` would read them as UTF-8.
        ("utf16le-nobom.md", utf16le("plain\n", false), UTF_16LE),
        ("utf16be-nobom.md", utf16be("plain\n", false), UTF_16BE),
        // MDEDIT-103. `FileFormat::encode` does UTF-16 by hand, because `encoding_rs` maps the
        // UTF-16 *output* encodings to UTF-8 and returns UTF-8 bytes with no error — which behind a
        // UTF-16 BOM is a file no reader can parse, reported as a successful save. A surrogate pair
        // is the case that hand-written encoder exists for, and until now nothing exercised it in
        // either byte order.
        ("utf16le-astral.md", utf16le("汉字 🎉 é\n", true), UTF_16LE),
        ("utf16be-astral.md", utf16be("汉字 🎉 é\n", true), UTF_16BE),
    ];

    // windows-1252: every byte that has a mapping, so the whole table is exercised.
    let (cp1252, _, _) = WINDOWS_1252.encode("naïve café — “quoted” • 90%\r\n");
    all.push(("windows-1252.md", cp1252.into_owned(), WINDOWS_1252));

    let (sjis, _, _) = SHIFT_JIS.encode("日本語のテキスト\nと英語 mixed\n");
    all.push(("shift-jis.md", sjis.into_owned(), SHIFT_JIS));

    all
}

/// The acceptance criterion: open, save without editing, get the same bytes.
#[test]
fn open_then_save_unedited_is_byte_identical() {
    let dir = TempDir::new().unwrap();

    for (name, bytes, encoding) in fixtures() {
        let path = fixture(&dir, name, &bytes);
        let file = load_as(&path, encoding)
            .unwrap_or_else(|e| panic!("{name} should load as {}: {e}", encoding.name()));

        save(&path, file.text(), &file).unwrap_or_else(|e| panic!("{name} should save: {e}"));

        let after = std::fs::read(&path).unwrap();
        assert_eq!(
            after, bytes,
            "{name}: saving an unedited file changed it on disk"
        );
    }
}

/// The same, after an edit that only touches text the encoding can represent.
#[test]
fn an_edit_rewrites_only_what_changed() {
    let dir = TempDir::new().unwrap();

    for (name, bytes, encoding) in fixtures() {
        let path = fixture(&dir, name, &bytes);
        let file = load_as(&path, encoding).unwrap();
        if !file.round_trips() {
            continue;
        }

        // Append ASCII, which every encoding here can hold.
        let edited = format!("{}tail", file.text());
        save(&path, &edited, &file).unwrap_or_else(|e| panic!("{name} should save edited: {e}"));

        let reloaded = load_as(&path, encoding).unwrap();
        assert_eq!(
            reloaded.text(),
            edited,
            "{name}: the edit did not survive the round trip"
        );
    }
}

#[test]
fn a_bom_survives_and_is_not_duplicated() {
    let dir = TempDir::new().unwrap();
    let bytes = [b"\xEF\xBB\xBF".as_slice(), b"# With BOM\n"].concat();
    let path = fixture(&dir, "bom.md", &bytes);

    let file = load(&path).unwrap();
    assert!(file.format().has_bom());
    assert_eq!(
        file.text(),
        "# With BOM\n",
        "the BOM is format, not content, and must not appear in the text"
    );

    let edited = format!("{}more\n", file.text());
    save(&path, &edited, &file).unwrap();

    let after = std::fs::read(&path).unwrap();
    assert_eq!(
        &after[..3],
        b"\xEF\xBB\xBF",
        "the BOM should be written back exactly once"
    );
    assert_eq!(&after[3..6], b"# W", "and content follows it immediately");
}

#[test]
fn a_file_without_a_bom_does_not_gain_one() {
    let dir = TempDir::new().unwrap();
    let path = fixture(&dir, "nobom.md", b"# Plain\n");

    let file = load(&path).unwrap();
    assert!(!file.format().has_bom());

    save(&path, "# Plain\nedited\n", &file).unwrap();
    let after = std::fs::read(&path).unwrap();
    assert_ne!(&after[..3.min(after.len())], b"\xEF\xBB\xBF");
}

/// Line endings are never rewritten, which is what keeps a mixed-ending file intact.
#[test]
fn line_endings_are_left_exactly_as_they_were() {
    let dir = TempDir::new().unwrap();
    let bytes = b"one\r\ntwo\nthree\r\n";
    let path = fixture(&dir, "mixed.md", bytes);

    let file = load(&path).unwrap();
    assert_eq!(file.format().newline(), Newline::Mixed);
    assert_eq!(
        file.text(),
        "one\r\ntwo\nthree\r\n",
        "CRLF must reach the buffer intact, not normalised to LF"
    );

    save(&path, file.text(), &file).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn newline_detection_reports_the_dominant_style() {
    assert_eq!(Newline::detect("a\nb\n"), Newline::Lf);
    assert_eq!(Newline::detect("a\r\nb\r\n"), Newline::CrLf);
    assert_eq!(Newline::detect("a\r\nb\n"), Newline::Mixed);
    assert_eq!(Newline::detect("no newlines"), Newline::Lf);
    assert_eq!(Newline::detect(""), Newline::Lf);

    assert_eq!(Newline::Lf.as_str(), "\n");
    assert_eq!(Newline::CrLf.as_str(), "\r\n");
}

/// The refusal that stops silent corruption.
///
/// `encoding_rs::Encoding::encode` substitutes an HTML numeric character reference for anything the
/// target cannot represent, so "saved successfully" would mean a literal `&#128512;` in the user's
/// windows-1252 file.
#[test]
fn an_unrepresentable_character_is_refused_rather_than_mangled() {
    let dir = TempDir::new().unwrap();
    let (bytes, _, _) = WINDOWS_1252.encode("plain latin\n");
    let path = fixture(&dir, "latin.md", &bytes);

    let file = load_as(&path, WINDOWS_1252).unwrap();
    let result = save(&path, "plain latin 😀\n", &file);

    assert!(
        matches!(result, Err(FileError::Unrepresentable { .. })),
        "expected a refusal, got {result:?}"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes.as_ref(),
        "a refused save must leave the file untouched"
    );
}

#[test]
fn malformed_bytes_are_reported_rather_than_decoded_to_replacements() {
    let dir = TempDir::new().unwrap();
    // 0xFF is never valid UTF-8.
    let path = fixture(&dir, "broken.md", b"valid \xFF invalid\n");

    let result = load(&path);
    assert!(
        matches!(result, Err(FileError::Malformed { .. })),
        "expected a malformed error, got {:?}",
        result.map(|f| f.text().to_owned())
    );
}

#[test]
fn a_missing_file_reports_which_file_and_why() {
    let result = load(Path::new("definitely-not-here.md"));
    let Err(FileError::Io { path, action, .. }) = result else {
        panic!("expected an io error");
    };
    assert_eq!(action, "reading");
    assert!(path.to_string_lossy().contains("definitely-not-here"));
}

/// A save must not leave its temporary file behind.
#[test]
fn saving_leaves_no_temporary_file() {
    let dir = TempDir::new().unwrap();
    let path = fixture(&dir, "clean.md", b"content\n");

    let file = load(&path).unwrap();
    save(&path, "content\nedited\n", &file).unwrap();

    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "clean.md")
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}

/// A file that claims to round-trip must re-encode to exactly the bytes it came from.
///
/// **This test used to assert nothing** (MDEDIT-103), and it is the only one behind the whole
/// non-UTF-8 write path. It saved an edit and then saved the original text again — and that second
/// save takes the byte-identical fast path, writing a copy of the original bytes held in memory. The
/// final comparison was therefore against bytes the encoder had never touched, and would have passed
/// had `FileFormat::encode` emitted anything at all.
///
/// Both halves are asserted now, separately, because they are two different claims: the *edited*
/// save runs the encoder and has to decode back, and the *unedited* save has to be byte-identical.
#[test]
fn the_round_trip_flag_tells_the_truth() {
    let dir = TempDir::new().unwrap();
    let mut skipped = Vec::new();

    for (name, bytes, encoding) in fixtures() {
        let path = fixture(&dir, name, &bytes);
        let file = load_as(&path, encoding).unwrap();
        if !file.round_trips() {
            // Recorded rather than quietly passed over. `round_trips` is computed *by the encoder
            // this test is checking*, so a broken encoder drops its own fixtures out of the loop and
            // the test goes green with nothing left in it. Verified: mutating the UTF-16 byte order
            // left all fourteen passing.
            skipped.push(name);
            continue;
        }

        // The encoder, held up against the decoder. Nothing here reads the original bytes.
        let edited = format!("{}x", file.text());
        save(&path, &edited, &file).unwrap();
        let reread = load_as(&path, encoding).unwrap();
        assert_eq!(
            reread.text(),
            edited,
            "{name}: what the encoder wrote does not decode back"
        );

        // And an unedited save is byte-identical, which is the guarantee the original is kept for.
        std::fs::write(&path, &bytes).unwrap();
        let file = load_as(&path, encoding).unwrap();
        save(&path, file.text(), &file).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "{name}: claims to round-trip, but does not"
        );
    }

    // **None of them, as it turns out.** Every fixture here is canonical in its own encoding — the
    // legacy ones are built by encoding text rather than by hand — so the `continue` above is a
    // guard that never fires, and saying so is what keeps it that way. A fixture that starts being
    // skipped is either a lossy byte sequence somebody added deliberately, in which case this line
    // is where it gets named, or an encoder that has stopped round-tripping and would otherwise
    // have removed itself from its own test.
    assert!(
        skipped.is_empty(),
        "these fixtures no longer round-trip, so nothing above tested them: {skipped:?}"
    );
}

/// The case that proves the whole design, and that the naive one would have corrupted files.
///
/// Shift-JIS encodes some characters twice. `ED 40` and `FA 5C` are both 纊; the decoder accepts
/// either and the encoder only ever emits `FA 5C`. So decode-then-encode silently rewrites bytes in
/// a part of the document nobody edited — which is precisely what the byte-preservation invariant
/// forbids, and what recording "the encoding" as metadata does nothing to prevent.
#[test]
fn a_file_that_cannot_round_trip_is_detected_and_never_silently_rewritten() {
    let dir = TempDir::new().unwrap();
    // 纊 written the NEC-selected way, which the encoder does not round-trip.
    let bytes = b"\xED\x40 kanji\n".to_vec();
    let path = fixture(&dir, "sjis-duplicate.md", &bytes);

    let file = load_as(&path, SHIFT_JIS).unwrap();
    assert_eq!(file.text(), "纊 kanji\n", "it still decodes correctly");
    assert!(
        !file.round_trips(),
        "ED 40 re-encodes as FA 5C, so this file cannot reproduce its own bytes"
    );

    // Saving it untouched is still exactly safe, because the original bytes are written back.
    save(&path, file.text(), &file).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "an unedited save must be byte-identical even when the encoder is not symmetric"
    );

    // Saving it *edited* would have to re-encode, which would rewrite ED 40 to FA 5C. Refused.
    let result = save(&path, "纊 edited\n", &file);
    assert!(
        matches!(result, Err(FileError::NotRoundTrippable { .. })),
        "expected a refusal, got {result:?}"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "a refused save must leave the file untouched"
    );
}

/// Why a caller must re-read after saving.
///
/// [`LoadedFile`] holds the bytes as they were *when it was read*. After a save those bytes are
/// stale, and the next save would compare the buffer against the wrong text — deciding the document
/// is still "edited" when it matches the disk, and taking the re-encoding path instead of the
/// copy-the-original one. `reload` is what keeps the fast path honest.
#[test]
fn reloading_after_a_save_keeps_the_byte_identical_path_correct() {
    let dir = TempDir::new().unwrap();
    let path = fixture(&dir, "sequence.md", b"first\n");

    let file = load(&path).unwrap();
    save(&path, "second\n", &file).unwrap();

    // Without this the editor would still believe the file says "first".
    let file = mdedit_file::reload(&path, &file).unwrap();
    assert_eq!(file.text(), "second\n");

    // Now an unedited save takes the copy-the-original path against the *current* bytes.
    save(&path, file.text(), &file).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"second\n");
}

const FRAGMENTS: &[&str] = &[
    "", "a", "word ", "\n", "\r\n", "# ", "> ", "- ", "```", "汉", "🎉", "é", "\t", "  ",
];

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

    /// Whatever UTF-8 text goes in, an unedited save brings the same bytes back out.
    #[test]
    fn utf8_round_trips_for_arbitrary_content(
        parts in proptest::collection::vec(0..FRAGMENTS.len(), 0..24),
    ) {
        let text: String = parts.into_iter().map(|i| FRAGMENTS[i]).collect();
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("prop.md");
        std::fs::write(&path, text.as_bytes()).unwrap();

        let file = load(&path).unwrap();
        proptest::prop_assert_eq!(file.text(), text.as_str());

        save(&path, file.text(), &file).unwrap();
        proptest::prop_assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
    }
}
