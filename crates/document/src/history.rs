//! Undo and redo.
//!
//! # What one undo step is
//!
//! Not one call to [`Document::replace`](crate::Document::replace). Typing `hello` is five calls
//! and one undo, or the feature is useless. So edits are grouped, and the grouping rule is the
//! whole design.
//!
//! **The rule is structural, not timed.** A new edit joins the group in front of it when it
//! continues the same motion: an insertion that begins exactly where the last one ended, or a
//! backspace that ends exactly where the last one began. Anything else starts a new group —
//! inserting a newline, deleting forwards after inserting, typing somewhere else.
//!
//! **Moving the caret ends the group.** That is what makes a clock unnecessary. Editors usually
//! coalesce on a timer, but the timer is a proxy for "the user stopped and went somewhere else",
//! and the caret says so exactly. Keeping time out of it also keeps this crate deterministic and
//! testable, which a timer would not: a test would have to sleep to prove anything.
//!
//! # What is recorded
//!
//! Each edit keeps the bytes it removed and the bytes it inserted, so undo is just the opposite
//! replacement. Nothing is re-derived from the document, which is what keeps undo honest under the
//! byte-preservation invariant — it restores the exact bytes that were there, not a re-serialized
//! approximation of them.

use mdedit_offsets::{ByteOffset, ByteSpan};

use crate::Selection;

/// How many undo groups are kept.
///
/// A bound rather than unlimited history: a long editing session would otherwise grow without
/// limit, and `AGENTS.md` commits to a 50 MB baseline. Dropping the oldest group is the usual
/// trade — the edits people actually reach for are recent.
const MAX_GROUPS: usize = 512;

/// One replacement, with everything needed to reverse it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edit {
    /// Where the replacement happened.
    pub(crate) at: ByteOffset,
    /// The bytes that were there before. Kept verbatim, never re-derived.
    pub(crate) removed: String,
    /// The bytes that replaced them.
    pub(crate) inserted: String,
}

impl Edit {
    /// The span this edit's inserted bytes now occupy.
    fn inserted_span(&self) -> ByteSpan {
        ByteSpan::new(
            self.at,
            ByteOffset::new(self.at.get() + self.inserted.len()),
        )
    }

    /// The span the removed bytes used to occupy.
    fn removed_span(&self) -> ByteSpan {
        ByteSpan::new(self.at, ByteOffset::new(self.at.get() + self.removed.len()))
    }

    /// Whether this edit only adds bytes.
    fn is_insertion(&self) -> bool {
        self.removed.is_empty() && !self.inserted.is_empty()
    }

    /// Whether this edit only removes bytes.
    fn is_deletion(&self) -> bool {
        self.inserted.is_empty() && !self.removed.is_empty()
    }
}

/// A run of edits undone and redone as one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Group {
    pub(crate) edits: Vec<Edit>,
    /// Where the caret was before the first edit — where undo puts it back.
    pub(crate) selection_before: Selection,
    /// Where the caret ended up after the last edit.
    pub(crate) selection_after: Selection,
}

/// The undo and redo stacks.
#[derive(Debug, Clone, Default)]
pub(crate) struct History {
    undo: Vec<Group>,
    redo: Vec<Group>,
    /// Set when something has ended the current run, so the next edit starts a new group.
    barrier: bool,
}

impl History {
    /// Records an edit, joining the group in front of it when it continues the same motion.
    pub(crate) fn record(&mut self, edit: Edit, before: Selection, after: Selection) {
        // Any new edit invalidates the redo stack: the future being redone no longer follows from
        // the present.
        self.redo.clear();

        if !self.barrier
            && let Some(group) = self.undo.last_mut()
            && let Some(previous) = group.edits.last()
            && continues(previous, &edit)
        {
            group.edits.push(edit);
            group.selection_after = after;
            return;
        }

        self.barrier = false;
        self.undo.push(Group {
            edits: vec![edit],
            selection_before: before,
            selection_after: after,
        });

        if self.undo.len() > MAX_GROUPS {
            // Oldest first. `remove(0)` is O(n) but happens once every 512 groups.
            self.undo.remove(0);
        }
    }

    /// Ends the current run, so the next edit starts a new group.
    pub(crate) fn break_group(&mut self) {
        self.barrier = true;
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Takes the most recent group for undoing, moving it to the redo stack.
    pub(crate) fn pop_undo(&mut self) -> Option<Group> {
        let group = self.undo.pop()?;
        self.redo.push(group.clone());
        // An undo always ends a run: typing after it must not merge into the group before it.
        self.barrier = true;
        Some(group)
    }

    /// Takes the most recently undone group for redoing, moving it back.
    pub(crate) fn pop_redo(&mut self) -> Option<Group> {
        let group = self.redo.pop()?;
        self.undo.push(group.clone());
        self.barrier = true;
        Some(group)
    }
}

/// Whether `next` continues the motion `previous` started.
///
/// Two cases, and only two. Typing forwards: each insertion begins where the last ended.
/// Backspacing: each deletion ends where the last began. A newline ends a run either way, which is
/// what makes undo land on paragraph boundaries rather than swallowing a whole document.
fn continues(previous: &Edit, next: &Edit) -> bool {
    // A newline is a boundary on *both* sides. Ending the run before it is not enough: without
    // this the newline would start a group and then absorb everything typed after it, so pressing
    // Enter and typing a line would undo both at once.
    if has_newline(next) || has_newline(previous) {
        return false;
    }

    if previous.is_insertion() && next.is_insertion() {
        return next.at == previous.inserted_span().end();
    }

    if previous.is_deletion() && next.is_deletion() {
        return next.removed_span().end() == previous.at;
    }

    false
}

impl Group {
    /// The replacements that undo this group, in the order they must be applied.
    ///
    /// Reversed: the last edit made is the first undone, because each edit's offsets assume every
    /// earlier edit has already happened.
    pub(crate) fn undo_steps(&self) -> impl Iterator<Item = (ByteSpan, &str)> {
        self.edits
            .iter()
            .rev()
            .map(|edit| (edit.inserted_span(), edit.removed.as_str()))
    }

    /// The replacements that redo this group, in order.
    pub(crate) fn redo_steps(&self) -> impl Iterator<Item = (ByteSpan, &str)> {
        self.edits
            .iter()
            .map(|edit| (edit.removed_span(), edit.inserted.as_str()))
    }
}

/// Whether an edit touches a line break, in either direction.
fn has_newline(edit: &Edit) -> bool {
    edit.inserted.contains('\n') || edit.removed.contains('\n')
}
