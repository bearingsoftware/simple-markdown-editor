//! Every Ctrl chord this editor answers to, written down once.
//!
//! Two test binaries read it, and that is the point. `shortcuts.rs` asserts this list and the help
//! page describe the same set — **both ways**, which is what MDEDIT-101 was about: the page claimed
//! Ctrl+Tab and nothing implemented it, while `apply_chord` implemented Ctrl+F, Ctrl+G and Ctrl+H
//! and nothing wrote them down. A one-way check catches one of those. `chords.rs` presses every
//! entry, so a chord listed here and never wired up is a chord some test drives.
//!
//! It is kept by hand, because nothing can derive it from a `match`. What the two binaries buy is
//! that "kept by hand" now means *one* hand-kept list instead of two that drift apart.

// Each binary compiles this module separately and reads a different half — `shortcuts.rs` the
// labels, `chords.rs` the presses — so from inside either one the other field looks dead.
#![allow(dead_code)]

/// One chord, as the reference writes it and as the keyboard delivers it.
pub struct Chord {
    /// Exactly the string `shortcuts.rs` puts in the `Keys` column.
    pub label: &'static str,
    /// Every `(character, shift)` pair that reaches this chord through `Editor::apply_chord`.
    ///
    /// Several for a label that covers a range, and **empty** for one that never reaches
    /// `apply_chord` at all: Ctrl+Home and Ctrl+End arrive as `Key::Named` and are answered in
    /// `handle_chord` itself.
    pub presses: &'static [(&'static str, bool)],
}

const fn chord(label: &'static str, presses: &'static [(&'static str, bool)]) -> Chord {
    Chord { label, presses }
}

/// Ctrl+Tab is a named key too, but unlike Ctrl+Home it is passed on to `apply_chord` as `"\t"` —
/// so it is pressable from a test, and MDEDIT-101 made sure it is pressed.
pub const CHORDS: &[Chord] = &[
    chord("Ctrl+N", &[("n", false), ("N", true)]),
    chord("Ctrl+O", &[("o", false), ("O", true)]),
    chord("Ctrl+S", &[("s", false)]),
    chord("Ctrl+Shift+S", &[("S", true)]),
    chord("Ctrl+W", &[("w", false), ("W", true)]),
    chord("Ctrl+Tab", &[("\t", false)]),
    chord("Ctrl+Shift+Tab", &[("\t", true)]),
    chord("Ctrl+Z", &[("z", false)]),
    chord("Ctrl+Shift+Z", &[("Z", true)]),
    chord("Ctrl+Y", &[("y", false), ("Y", true)]),
    chord("Ctrl+X", &[("x", false)]),
    chord("Ctrl+C", &[("c", false), ("C", true)]),
    chord("Ctrl+V", &[("v", false), ("V", true)]),
    chord("Ctrl+A", &[("a", false), ("A", true)]),
    chord("Ctrl+/", &[("/", false)]),
    chord("Ctrl+B", &[("b", false), ("B", true)]),
    chord("Ctrl+I", &[("i", false), ("I", true)]),
    chord("Ctrl+Shift+X", &[("X", true)]),
    chord("Ctrl+E", &[("e", false)]),
    chord("Ctrl+Shift+E", &[("E", true)]),
    chord("Ctrl+K", &[("k", false), ("K", true)]),
    chord("Ctrl+F", &[("f", false), ("F", true)]),
    chord("Ctrl+H", &[("h", false), ("H", true)]),
    chord("Ctrl+G", &[("g", false), ("G", true)]),
    chord("Ctrl+1 … Ctrl+6", &[("1", false), ("6", false)]),
    chord(
        "Ctrl+= / Ctrl+-",
        &[("=", false), ("+", true), ("-", false)],
    ),
    chord("Ctrl+0", &[("0", false)]),
    chord("Ctrl+Home / Ctrl+End", &[]),
];
