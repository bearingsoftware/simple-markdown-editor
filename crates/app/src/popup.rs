//! The list a toolbar button opens: heading levels, or whatever did not fit on the bar.
//!
//! Geometry and vocabulary only — no GPU types, no text engine, no window — as `toolbar.rs`,
//! `tooltip.rs`, `tabs.rs` and `pane.rs` are, and for the reason they give: hit-testing is easy to
//! get subtly wrong and impossible to check by looking at a screenshot.
//!
//! # Drawn here rather than by Windows
//!
//! The right-click context menu is native, through muda. This is not, for three reasons. muda items
//! cannot show the icons these rows need. A native popup cannot be driven by a test, and where a
//! click lands is exactly the half of this worth asserting. And every other affordance the bar owns
//! — MDEDIT-85's tooltip, and the selection bar MDEDIT-119 removed — is drawn by the editor, so a
//! second *native* mechanism inside a component whose design is testable geometry would be the odd
//! one out.
//!
//! # It is a third z-order, and that is the risk
//!
//! The tooltip is drawn last and hit-tested **never**. A list is drawn after it and asked before
//! everything, and `Editor::click`'s order is the whole of that in either direction. Getting it
//! wrong is a control nobody can press — which it was, once.

use crate::toolbar::Action;

/// Height of one row.
pub const ROW_HEIGHT: f32 = 28.0;

/// Space above the first row and below the last.
pub const MARGIN: f32 = 4.0;

/// Space either side of a row's contents.
pub const PADDING: f32 = 10.0;

/// Width set aside for a row's icon, before its words start.
///
/// A fixed column rather than the measured icon width, so the words line up down the list. An icon
/// is 22 pixels and they are all the same width, but a row whose icon is missing still has to indent
/// its text to match the ones that have one.
///
/// **It has to clear [`PADDING`] plus the icon**, or the words start on top of the picture. Thirty
/// did not: the icon is drawn at `PADDING` and runs to 32, so the text began two pixels inside it.
/// Forty leaves eight pixels of air between the two, which is what makes the column read as a
/// column.
pub const ICON_COLUMN: f32 = 40.0;

/// Space between the button and the panel it opened.
pub const GAP: f32 = 2.0;

/// How tall a panel holding `rows` rows is.
#[must_use]
pub fn height_of(rows: usize) -> f32 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "a menu with more rows than an f32 can count is not reachable"
    )]
    let count = rows as f32;
    count.mul_add(ROW_HEIGHT, MARGIN * 2.0)
}

/// How wide a panel is, given the widest row's text.
#[must_use]
pub fn width_of(widest_text: f32) -> f32 {
    ICON_COLUMN + widest_text + PADDING * 2.0
}

/// Where a panel goes, given the button it hangs from and the room available.
///
/// **Left-aligned to the button, not centred on it.** A menu's left edge lines up with the control
/// that opened it — which is what every menu does, and what makes the eye travel straight down
/// rather than diagonally. The tooltip centres because it is a label about a thing; this is a
/// continuation of it.
///
/// Clamped so it never leaves the window, and `max` is applied last on purpose: in a window
/// narrower than the panel the left edge wins, because text clipped at the right is readable from
/// its start and text clipped at the left is not.
#[must_use]
pub fn place(width: f32, button_x: f32, below: f32, right_edge: f32) -> (f32, f32) {
    let x = button_x.min(right_edge - width).max(0.0);
    (x, below + GAP)
}

/// Which row a click at `(x, y)` landed on, if any.
///
/// `(x, y)` is in window coordinates and `(panel_x, panel_y)` is where [`place`] put the panel, so
/// this does the conversion rather than making three callers agree about it — the lesson `pane_hit`
/// and `strip_hit` both carry.
///
/// A click in the panel's own margin returns `None` and is **still swallowed by the caller**: a
/// click on a menu is never a click on what is behind the menu, and "it landed between two rows" is
/// not a reason to put the caret in the document underneath.
#[must_use]
pub fn hit_test(rows: usize, x: f32, y: f32, panel: (f32, f32), width: f32) -> Option<usize> {
    let (_, panel_y) = panel;
    if !contains(rows, x, y, panel, width) {
        return None;
    }
    let into = y - panel_y - MARGIN;
    // **The top margin is above row zero, not part of it.** `contains` says yes to it — it is on the
    // panel — so without this the subtraction goes negative and the cast to `usize` saturates to 0,
    // handing the margin to the first row. A click a pixel inside the panel's border would have run
    // whatever was at the top of the list.
    if into < 0.0 {
        return None;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked non-negative above, and bounded against `rows` below"
    )]
    let row = (into / ROW_HEIGHT) as usize;
    (row < rows).then_some(row)
}

/// Whether `(x, y)` is anywhere on the panel, margins included.
#[must_use]
pub fn contains(rows: usize, x: f32, y: f32, panel: (f32, f32), width: f32) -> bool {
    let (panel_x, panel_y) = panel;
    (panel_x..panel_x + width).contains(&x) && (panel_y..panel_y + height_of(rows)).contains(&y)
}

/// Where a row's contents sit, measured from the panel's own corner.
#[must_use]
pub fn row_top(row: usize) -> f32 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "a menu with more rows than an f32 can count is not reachable"
    )]
    let index = row as f32;
    index.mul_add(ROW_HEIGHT, MARGIN)
}

/// The words a row carries beside its face, if it needs any.
///
/// [`Action::describes`] rather than [`Action::label`] — a list has room for "Bullet list" where a
/// button has room for a bullet, and a menu of one-letter labels would be a worse version of the bar
/// it came from.
///
/// **`None` for the heading levels**, and they are the only rows that need nothing. `H1` says
/// heading one; "H1  Heading 1" says it twice, and the second time in smaller type. Every other row
/// is a picture, and a picture on its own is the guessing game MDEDIT-85 exists to answer — those
/// keep their words.
#[must_use]
pub fn text_for(action: Action) -> Option<String> {
    (!action.letters()).then(|| action.describes())
}

/// Whether a list needs a text column at all.
///
/// A heading list is six letterforms and nothing else, so it is a narrow strip rather than a panel
/// two thirds empty.
#[must_use]
pub fn any_text(rows: &[Action]) -> bool {
    rows.iter().any(|action| text_for(*action).is_some())
}
