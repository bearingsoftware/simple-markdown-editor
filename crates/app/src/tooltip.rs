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
/// window. `max` is applied last for the same reason [`crate::toolbar::place`] applies it last: in a
/// window narrower than the panel the left edge wins, because text clipped at the right is readable
/// from its start and text clipped at the left is not.
#[must_use]
pub fn place(width: f32, centre: f32, below: f32, right_edge: f32) -> (f32, f32) {
    let x = (centre - width / 2.0).min(right_edge - width).max(0.0);
    (x, below + GAP)
}
