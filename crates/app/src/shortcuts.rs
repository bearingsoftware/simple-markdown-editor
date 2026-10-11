//! The keyboard-shortcuts reference, and the one list it is generated from.
//!
//! Shown as a Markdown document in a tab rather than in a dialog. That is not a shortcut taken for
//! want of dialog code — a Markdown editor showing its own help as Markdown is the cheapest possible
//! demonstration that it renders headings, tables and inline code, and it costs no UI at all.
//!
//! # Why a list rather than a string
//!
//! A help page and the code it describes drift the moment they are two separate things. [`ALL`] is
//! the single list; [`document`] renders it. When a chord changes, one line changes, and a test
//! asserts that every chord `handle_chord` answers to appears here — see `tests/shortcuts.rs`.
//!
//! It cannot go further than that on its own. Nothing here *implements* a binding, so this is a
//! description that is checked against the implementation rather than derived from it.

/// One binding: what to press, and what it does.
#[derive(Debug)]
pub struct Shortcut {
    /// The chord, as a person would say it.
    pub keys: &'static str,
    /// What it does.
    pub does: &'static str,
}

/// A group of bindings under a heading.
#[derive(Debug)]
pub struct Group {
    /// The heading.
    pub name: &'static str,
    /// What is in it.
    pub shortcuts: &'static [Shortcut],
}

const fn key(keys: &'static str, does: &'static str) -> Shortcut {
    Shortcut { keys, does }
}

/// Every binding the editor has, grouped as the reference shows them.
pub const ALL: &[Group] = &[
    Group {
        name: "Files",
        shortcuts: &[
            key("Ctrl+N", "New file"),
            key("Ctrl+O", "Open a file"),
            key("Ctrl+S", "Save"),
            key("Ctrl+Shift+S", "Save as a new file"),
            key(
                "(File menu)",
                "Reopen with Encoding — for a file that opened as mojibake, or did not open at all",
            ),
            key(
                "(File menu)",
                "Save with Encoding — takes effect at the next save, and refuses rather than \
                 writing characters the encoding cannot hold",
            ),
            key(
                "(File menu)",
                "Line Endings — convert every terminator to LF or CRLF, in one undo step. \
                 Saving never converts anything on its own",
            ),
            key("Ctrl+W", "Close the current tab"),
            key("Ctrl+Tab", "Next tab"),
            key("Ctrl+Shift+Tab", "Previous tab"),
        ],
    },
    Group {
        name: "Editing",
        shortcuts: &[
            key("Ctrl+Z", "Undo"),
            key("Ctrl+Y", "Redo"),
            key("Ctrl+Shift+Z", "Redo"),
            key("Ctrl+X", "Cut"),
            key("Ctrl+C", "Copy"),
            key("Ctrl+V", "Paste"),
            key("Ctrl+A", "Select all"),
        ],
    },
    Group {
        name: "Formatting",
        shortcuts: &[
            key("Ctrl+B", "Bold"),
            key("Ctrl+I", "Italic"),
            key("Ctrl+Shift+X", "Strikethrough"),
            key("Ctrl+E", "Inline code"),
            key(
                "Ctrl+K",
                "Link — uses a URL on the clipboard if there is one",
            ),
            key(
                "Ctrl+1 … Ctrl+6",
                "Heading level — press the same level again to clear it",
            ),
            key(
                "Ctrl+Enter",
                "Tick or untick the task on the caret's line. Across a selection, ticks them all \
                 unless they all are",
            ),
        ],
    },
    Group {
        name: "Finding",
        shortcuts: &[
            key("Ctrl+F", "Find — anything selected goes into the box"),
            key("Enter / F3", "Next match, wrapping at the end"),
            key("Shift+Enter / Shift+F3", "Previous match"),
            key("Esc", "Close the find bar"),
            key(
                "(the bar's Aa button)",
                "Match case — off to begin with, as a search box should be",
            ),
            key("Ctrl+H", "Replace — the same bar, with a second box"),
            key("Tab", "Move between the two boxes"),
            key(
                "(the bar's Replace / All buttons)",
                "Replace this match, or every match as one undo step",
            ),
            key("Ctrl+G", "Go to line — digits only, and Enter goes"),
        ],
    },
    Group {
        name: "Tools",
        shortcuts: &[
            key(
                "(Tools menu)",
                "Validate Document — JSON only; goes to the first problem. The count in the status \
                 bar keeps itself up to date without this",
            ),
            key(
                "(Tools menu)",
                "Format Document — JSON only, and greyed elsewhere. One undo step puts every byte \
                 back. Never happens on save",
            ),
        ],
    },
    Group {
        name: "Moving about",
        shortcuts: &[
            key(
                "Home / End",
                "Start or end of the row — a wrapped line has one per row. In preview, Home stops \
                 after a list bullet or heading mark rather than before it",
            ),
            key("Ctrl+Home / Ctrl+End", "Start or end of the document"),
            key("Page Up / Page Down", "Scroll a screen, leaving the caret"),
            key("Shift + any motion", "Extend the selection instead"),
        ],
    },
    Group {
        name: "View",
        shortcuts: &[
            key(
                "Ctrl+/",
                "Show the Markdown as plain text, or conceal it again",
            ),
            key("Ctrl+Shift+E", "Swap the tab strip for the file pane"),
            key(
                "(View menu)",
                "Line numbers — no chord, and it does not need one",
            ),
            key(
                "(View menu)",
                "Formatting bar — every button here is one of the chords above",
            ),
            key(
                "(View menu)",
                "Whitespace and line endings — spaces, tabs, control characters, and which \
                 lines are LF and which are CRLF",
            ),
            key(
                "(View menu)",
                "Word Wrap — turn it off and long lines run right instead of folding",
            ),
            key(
                "Shift + wheel",
                "Pan sideways, when word wrap is off. There is no horizontal scrollbar",
            ),
            key("Ctrl+= / Ctrl+-", "Make the text bigger or smaller"),
            key("Ctrl+0", "Back to the normal size"),
        ],
    },
];

/// The name the tab carries.
pub const TITLE: &str = "Keyboard Shortcuts";

/// Renders [`ALL`] as a Markdown document.
///
/// A table per group, because that is what the content is, and it exercises the table rendering on
/// the way past.
#[must_use]
pub fn document() -> String {
    use std::fmt::Write as _;

    let mut out = String::from("# Keyboard Shortcuts\n");
    for group in ALL {
        let _ = write!(
            out,
            "\n## {}\n\n| Keys | Does |\n| --- | --- |\n",
            group.name
        );
        for shortcut in group.shortcuts {
            let _ = writeln!(out, "| `{}` | {} |", shortcut.keys, shortcut.does);
        }
    }
    out.push_str(
        "\nMenu items show these too, but the menu does not fire them — the accelerator text is a \
         label and the key handler owns the chord. Pressing the keys is what works.\n",
    );
    out
}
