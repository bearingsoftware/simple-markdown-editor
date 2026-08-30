//! What a destination written in a document is allowed to name (MDEDIT-146).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::path::Path;

use mdedit_image::{Refused, resolve};

/// A directory with `cat.png` in it, standing in for a document's own directory.
fn beside_a_document() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("cat.png"), b"not really a png").unwrap();
    std::fs::create_dir(dir.path().join("images")).unwrap();
    std::fs::write(dir.path().join("images/nested.png"), b"nor is this").unwrap();
    dir
}

#[test]
fn a_relative_destination_resolves_against_the_documents_directory() {
    let dir = beside_a_document();

    let resolved = resolve("cat.png", Some(dir.path())).unwrap();

    assert_eq!(resolved, dir.path().join("cat.png"));
}

#[test]
fn a_destination_in_a_subdirectory_resolves() {
    let dir = beside_a_document();

    let resolved = resolve("images/nested.png", Some(dir.path())).unwrap();

    assert_eq!(resolved, dir.path().join("images").join("nested.png"));
}

#[test]
fn a_parent_component_is_normalised_rather_than_refused() {
    // `![](../assets/cat.png)` is ordinary in a repository, and there is nothing to escape from:
    // reading a file the user can already read, to draw it on their own screen, is not an
    // escalation. What matters is that no `..` survives into the resolved path.
    let dir = beside_a_document();

    let resolved = resolve("images/../cat.png", Some(dir.path())).unwrap();

    assert_eq!(resolved, dir.path().join("cat.png"));
    assert!(
        !resolved.components().any(|part| part.as_os_str() == ".."),
        "{resolved:?} still has a parent component"
    );
}

#[test]
fn an_absolute_destination_resolves_to_itself() {
    let dir = beside_a_document();
    let absolute = dir.path().join("cat.png");

    let resolved = resolve(&absolute.to_string_lossy(), None).unwrap();

    assert_eq!(resolved, absolute);
}

#[test]
fn a_destination_in_angle_brackets_loses_them() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("a cat.png"), b"x").unwrap();

    let resolved = resolve("<a cat.png>", Some(dir.path())).unwrap();

    assert_eq!(resolved, dir.path().join("a cat.png"));
}

#[test]
fn a_percent_escape_is_decoded() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("a cat.png"), b"x").unwrap();

    let resolved = resolve("a%20cat.png", Some(dir.path())).unwrap();

    assert_eq!(resolved, dir.path().join("a cat.png"));
}

#[test]
fn something_that_is_not_an_escape_is_left_alone() {
    // `100%` is a plausible filename and `%zz` is not an escape. Dropping either would name a
    // different file, which is the one thing a decoder like this must not do.
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("100%.png"), b"x").unwrap();

    let resolved = resolve("100%.png", Some(dir.path())).unwrap();

    assert_eq!(resolved, dir.path().join("100%.png"));
}

#[test]
fn a_missing_file_is_refused() {
    let dir = beside_a_document();

    assert!(matches!(
        resolve("absent.png", Some(dir.path())),
        Err(Refused::NotAFile(_))
    ));
}

#[test]
fn a_directory_is_not_a_file() {
    let dir = beside_a_document();

    assert!(matches!(
        resolve("images", Some(dir.path())),
        Err(Refused::NotAFile(_))
    ));
}

#[test]
fn a_relative_destination_in_a_document_with_no_directory_is_unusable() {
    // An untitled buffer has no directory to resolve against, and the working directory is whatever
    // the editor was launched from — not a thing to guess about.
    assert!(matches!(
        resolve("cat.png", None),
        Err(Refused::Unusable(_))
    ));
}

#[test]
fn an_empty_destination_is_unusable() {
    let dir = beside_a_document();

    assert!(matches!(
        resolve("   ", Some(dir.path())),
        Err(Refused::Unusable(_))
    ));
}

// ---------------------------------------------------------------------------------------------
// The rules that are about an attacker rather than a typo.
// ---------------------------------------------------------------------------------------------

#[test]
fn every_remote_scheme_is_refused() {
    let dir = beside_a_document();
    for destination in [
        "https://example.com/cat.png",
        "http://example.com/cat.png",
        "file:///etc/passwd",
        "data:image/png;base64,iVBORw0KGgo=",
        "ftp://example.com/cat.png",
        "javascript:alert(1)",
    ] {
        assert!(
            matches!(
                resolve(destination, Some(dir.path())),
                Err(Refused::Remote(_))
            ),
            "{destination} was not refused as remote"
        );
    }
}

#[test]
fn a_unc_destination_is_refused_without_touching_it() {
    // **The attack this rule exists for.** A UNC path causes an outbound SMB connection the moment
    // it is touched and Windows attempts NTLM authentication automatically, leaking a hash with no
    // dialog and nothing on screen. A Markdown document is something you are sent, so this needs no
    // write access to anything of the victim's.
    let dir = beside_a_document();
    for destination in [
        r"\\evil.example\share\x.png",
        "//evil.example/share/x.png",
        r"\\.\pipe\x",
        r"\\?\C:\x.png",
    ] {
        assert!(
            matches!(
                resolve(destination, Some(dir.path())),
                Err(Refused::NetworkOrDevice(_))
            ),
            "{destination} was not refused as network or device"
        );
    }
}

#[test]
fn a_percent_encoded_unc_destination_is_refused_too() {
    // **The ordering test.** `%5C` is a backslash, so this decodes to `\\evil.example\…`. A prefix
    // check run before the decode waves it straight through, which is why the decode happens first.
    let dir = beside_a_document();

    let refused = resolve("%5C%5Cevil.example%5Cshare%5Cx.png", Some(dir.path()));

    assert!(
        matches!(refused, Err(Refused::NetworkOrDevice(_))),
        "{refused:?}"
    );
}

#[test]
fn a_percent_encoded_scheme_is_refused_too() {
    let dir = beside_a_document();

    // `%68` is `h`, so this decodes to `https://…`.
    let refused = resolve("%68ttps://example.com/cat.png", Some(dir.path()));

    assert!(matches!(refused, Err(Refused::Remote(_))), "{refused:?}");
}

#[test]
fn a_windows_drive_letter_is_not_a_url_scheme() {
    // `C:` has to survive the scheme check or every absolute Windows destination ever written is
    // refused. Two characters is the rule WHATWG settled on for the same collision.
    let dir = beside_a_document();
    let refused = resolve(r"C:\definitely\not\here\cat.png", Some(dir.path()));

    assert!(
        matches!(refused, Err(Refused::NotAFile(_))),
        "a drive letter was read as a scheme: {refused:?}"
    );
}

#[test]
fn a_refusal_that_is_not_notafile_costs_no_filesystem_access() {
    // Stated as a property because it is the one that matters: a document full of hostile
    // destinations must not become a document full of syscalls. `beside` names a directory that does
    // not exist, so anything reaching the filesystem would report `NotAFile` against it.
    let nowhere = Path::new("/this/directory/does/not/exist");
    for destination in [
        "https://example.com/cat.png",
        r"\\evil.example\share\x.png",
        "",
    ] {
        let refused = resolve(destination, Some(nowhere)).unwrap_err();
        assert!(
            !matches!(refused, Refused::NotAFile(_)),
            "{destination} reached the filesystem: {refused:?}"
        );
    }
}
