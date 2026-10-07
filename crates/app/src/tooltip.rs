//! The panel that appears when the pointer rests on a toolbar button.
//!
//! Geometry and timing only — no GPU types, no text engine, no window — as `toolbar.rs`, `tabs.rs`
//! and `pane.rs` are, and for the reason they give.
//!
//! # What this module is really for
//!
//! `toolbar.rs` said until MDEDIT-85 that there were no tooltips "because there is no hover
//! affordance in this editor to hang one on", and that sentence was the whole ticket. The editor has
//! always tracked the pointer — `self.cursor`, updated on `CursorMoved` — and used it for dragging,
//! but nothing anywhere was *hover* state: nothing knew the pointer had been still, over something,
//! for a while.
//!
//! So the panel is the least of it. What had to be built is a **dwell** (hovering as a state with a
//! delay, because a panel that appears the instant the pointer crosses a button flickers as the
//! pointer travels a row of twenty-two of them) and a **second deadline** on an event loop that
//! sleeps on `ControlFlow::Wait` — "the pointer stopped moving 400 ms ago" is not an event. See
//! `Editor::next_wake` for the second, which is the part with a trap in it.
//!
//! # It is not an accessible name
//!
//! A tooltip is mouse-only and a screen reader will not read it. The accesskit pass still owes the
//! bar its names and its keyboard reach, and this module must not be taken as having paid that
//! debt. What it does help is anybody who cannot remember what `❝` means.

use std::path::{Component, Path};
use std::time::Duration;

/// How long the pointer must rest before a panel appears.
///
/// Windows' own default is 500 ms, which feels slow on a bar this dense. A number to try in the
/// window rather than argue about here — it is a constant precisely so it is one line to change.
pub const DWELL: Duration = Duration::from_millis(400);

/// Height of the panel.
pub const HEIGHT: f32 = 22.0;

/// Space either side of the text inside the panel.
pub const PADDING: f32 = 6.0;

/// Space between the bar's bottom edge and the panel's top.
pub const GAP: f32 = 4.0;

/// How wide a panel holding text `text_width` wide is.
#[must_use]
pub fn width_of(text_width: f32) -> f32 {
    text_width + PADDING * 2.0
}

/// Where a panel `width` wide goes, given the button it belongs to and the room available.
///
/// **Below the button, not under the pointer.** A panel under the pointer covers the thing you are
/// about to click, and the pointer is on its way somewhere.
///
/// Centred on the button and then clamped, so the rightmost button's panel does not hang off the
/// window. `max` is applied last on purpose: in a window narrower than the panel the left edge wins,
/// because text clipped at the right is readable from its start and text clipped at the left is not.
#[must_use]
pub fn place(width: f32, centre: f32, below: f32, right_edge: f32) -> (f32, f32) {
    let x = (centre - width / 2.0).min(right_edge - width).max(0.0);
    (x, below + GAP)
}

/// Ways to write `path`, fewest elisions first, for a panel that has to fit in a window
/// (MDEDIT-173).
///
/// # Why the middle goes and not the end
///
/// A tab's tooltip is its file's full path, and deep paths are ordinary on Windows — `place` clamps a
/// panel to the window's edge but cannot shorten one, so a path wider than the window would be drawn
/// off it. **Both ends of a path are the informative parts**: the drive says which volume, the last
/// components say which folder and which file, and it is the long run of middle directories that
/// nobody is reading. So the first interior component goes, then the next, and so on:
///
/// ```text
/// C:\Users\craig\work\notes\plan.md
/// C:\…\craig\work\notes\plan.md
/// C:\…\work\notes\plan.md
/// C:\…\notes\plan.md
/// C:\…\plan.md
/// ```
///
/// The caller measures each in turn and draws the first that fits, which is why this returns the
/// candidates rather than a width: measuring needs a text engine, and this module deliberately has
/// none.
///
/// # "Fewest elisions", not "widest"
///
/// Each candidate holds one fewer component than the last, but that is not the same as being
/// narrower, because `…` can be wider than what it replaces: `C:\a\Documents and Settings\plan.md`
/// *grows* when `a` goes. The inversion is confined to the first step — every candidate after it
/// carries the same single ellipsis, so from there the order is strictly decreasing — and it costs
/// nothing but a wasted measurement, because the caller measures every candidate and returns on the
/// first that fits rather than reasoning about the order. **Do not add a width-ordering assumption
/// to that loop on the strength of this list.**
///
/// The last candidate still has a head, an ellipsis and a file name, so it can itself be too wide in
/// a very narrow window. That is left to `place`, whose rule is that a panel wider than the window
/// keeps its left edge — text clipped at the right is readable from its start.
///
/// # The verbatim prefix is not handled here, because it cannot arrive
///
/// A canonicalised Windows path can carry `\\?\`, and showing that to a person would be wrong. No
/// such path reaches a tab: `mdedit_session::path::validate` **rejects** a path beginning with two
/// separators, and `from_command_line` uses `std::path::absolute` rather than
/// `std::fs::canonicalize` on Windows *precisely* so the prefix is never created — its own comment
/// says that canonicalising "would make every legitimate path look like a device path". Stripping a
/// prefix here would be unreachable code implying that invariant does not exist.
#[must_use]
pub fn elisions(path: &Path) -> Vec<String> {
    // The head is the drive and its root — `C:\` — and is kept by every candidate. A relative path
    // has no head, which is fine: it simply starts at its first interior component.
    let head: String = path
        .components()
        .take_while(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    let rest: Vec<String> = path
        .components()
        .skip_while(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();

    let joined = |parts: &[String]| parts.join(std::path::MAIN_SEPARATOR_STR);
    // The whole path is taken as the caller already reports it — `Path::display` — and *not*
    // rebuilt from the components above, which would quietly normalise it. `Components` drops `.`
    // and doubled separators and re-joins with the platform's own separator, so a path written
    // `C:/Users/craig//./plan.md` would come back `C:\Users\craig\plan.md`: a different string from
    // the one `Editor::tooltip_text` answers with, for the same hover. `session::path::validate`
    // accepts that spelling — it checks the device prefix, absoluteness, `..` and is-a-file, and
    // nothing about separators — and the session file is untrusted input, so it is reachable rather
    // than theoretical. The elided candidates are normalised and cannot help being: an ellipsis is
    // spliced between components, so there is nothing else to splice it into.
    let mut candidates = vec![path.display().to_string()];

    // One candidate per interior component dropped, the last keeping only the file name. A path with
    // nothing interior — `C:\plan.md` — produces no elisions at all, which is the point: there is
    // nothing in the middle to lose, and an ellipsis would claim there was.
    for dropped in 1..rest.len() {
        candidates.push(format!(
            "{head}…{}{}",
            std::path::MAIN_SEPARATOR,
            joined(&rest[dropped..])
        ));
    }
    candidates
}
