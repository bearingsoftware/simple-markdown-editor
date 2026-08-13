//! Editing a table as a grid: rows and columns in and out, and the re-pad that follows (MDEDIT-128).
//!
//! Two things carry the weight here.
//!
//! **The bytes of each operation**, because a table is the one block this editor rewrites whole and
//! a rewrite that drops a pipe is a table that stops being a table.
//!
//! **One undo restores every byte**, including the re-pad. That is what keeps re-serializing the
//! block under the cursor inside `AGENTS.md`'s rule rather than an exception to it, so it is
//! property-tested rather than asserted once.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_document::table::Alignment;
use mdedit_document::{Document, Selection};
use mdedit_offsets::ByteOffset;
use proptest::prelude::*;

const TABLE: &str = "| Keys   | Does |\n| ------ | ---- |\n| Ctrl+B | Bold |\n";

/// A document with the caret at the first byte of `needle`.
fn at(text: &str, needle: &str) -> Document {
    let mut doc = Document::from(text);
    let at = text.find(needle).expect("the needle is in the text");
    doc.set_selection(Selection::caret(ByteOffset::new(at)));
    doc
}

// ---- the model ----

#[test]
fn a_table_is_taken_apart_into_rows_and_cells() {
    let doc = at(TABLE, "Ctrl+B");
    let table = doc.table_at(doc.selection().head()).expect("in a table");

    assert_eq!(
        table.rows,
        vec![
            vec!["Keys".to_owned(), "Does".to_owned()],
            vec!["Ctrl+B".to_owned(), "Bold".to_owned()],
        ],
        "the delimiter row is not one of them"
    );
    assert_eq!(table.alignments, vec![Alignment::None, Alignment::None]);
}

#[test]
fn the_caret_knows_which_cell_it_is_in() {
    assert_eq!(at(TABLE, "Keys").caret_cell(), Some((0, 0)));
    assert_eq!(at(TABLE, "Does").caret_cell(), Some((0, 1)));
    assert_eq!(at(TABLE, "Ctrl+B").caret_cell(), Some((1, 0)));
    assert_eq!(at(TABLE, "Bold").caret_cell(), Some((1, 1)));
    assert_eq!(at("just prose\n", "prose").caret_cell(), None);
}

// ---- the operations ----

#[test]
fn a_row_is_inserted_below() {
    let mut doc = at(TABLE, "Ctrl+B");
    assert!(doc.insert_table_row(true));

    assert_eq!(
        doc.to_string(),
        "| Keys   | Does |\n| ------ | ---- |\n| Ctrl+B | Bold |\n|        |      |\n"
    );
}

#[test]
fn a_row_is_inserted_above() {
    let mut doc = at(TABLE, "Ctrl+B");
    assert!(doc.insert_table_row(false));

    assert_eq!(
        doc.to_string(),
        "| Keys   | Does |\n| ------ | ---- |\n|        |      |\n| Ctrl+B | Bold |\n"
    );
}

/// **A row above the header would become the header**, silently demoting the one that was written.
/// GFM has no table without a header, so there is nowhere honest to put it but below.
#[test]
fn a_row_asked_for_above_the_header_goes_below_it() {
    let mut doc = at(TABLE, "Keys");
    assert!(doc.insert_table_row(false));

    assert_eq!(
        doc.to_string(),
        "| Keys   | Does |\n| ------ | ---- |\n|        |      |\n| Ctrl+B | Bold |\n"
    );
}

#[test]
fn a_column_is_inserted_right() {
    let mut doc = at(TABLE, "Keys");
    assert!(doc.insert_table_column(true));

    assert_eq!(
        doc.to_string(),
        "| Keys   |     | Does |\n| ------ | --- | ---- |\n| Ctrl+B |     | Bold |\n"
    );
}

#[test]
fn a_column_is_inserted_left() {
    let mut doc = at(TABLE, "Does");
    assert!(doc.insert_table_column(false));

    assert_eq!(
        doc.to_string(),
        "| Keys   |     | Does |\n| ------ | --- | ---- |\n| Ctrl+B |     | Bold |\n"
    );
}

#[test]
fn a_row_is_deleted() {
    let mut doc = at(TABLE, "Ctrl+B");
    assert!(doc.delete_table_row());

    assert_eq!(doc.to_string(), "| Keys | Does |\n| ---- | ---- |\n");
}

/// Deleting the header would leave something that is not a table. A control labelled "delete row"
/// taking the whole table with it is not what anybody pressed it for — Delete Table is beside it.
#[test]
fn the_header_row_will_not_be_deleted_on_its_own() {
    let mut doc = at(TABLE, "Keys");
    assert!(!doc.delete_table_row());
    assert_eq!(doc.to_string(), TABLE, "and nothing moved");
}

#[test]
fn a_column_is_deleted() {
    let mut doc = at(TABLE, "Does");
    assert!(doc.delete_table_column());

    assert_eq!(doc.to_string(), "| Keys   |\n| ------ |\n| Ctrl+B |\n");
}

#[test]
fn the_last_column_will_not_be_deleted() {
    let mut doc = at("| a |\n| --- |\n| b |\n", "a");
    assert!(!doc.delete_table_column());
}

#[test]
fn the_table_is_deleted() {
    let mut doc = at(&format!("before\n\n{TABLE}\nafter\n"), "Ctrl+B");
    assert!(doc.delete_table());

    assert_eq!(doc.to_string(), "before\n\n\nafter\n");
}

#[test]
fn nothing_happens_outside_a_table() {
    let mut doc = at("just prose\n", "prose");
    assert!(!doc.insert_table_row(true));
    assert!(!doc.insert_table_column(true));
    assert!(!doc.delete_table_row());
    assert!(!doc.delete_table_column());
    assert!(!doc.delete_table());
    assert_eq!(doc.to_string(), "just prose\n");
}

// ---- the re-pad ----

/// The whole point of re-padding: a cell that outgrew its column widens the column in every row,
/// so the pipes line up in the file and not only on screen.
#[test]
fn widening_one_cell_re_pads_every_row() {
    // Written ragged, as a hand-edited table is: one cell far wider than its column.
    let source = "| a | b |\n| --- | --- |\n| a-much-longer-cell | b |\n";
    let mut doc = at(source, "a-much-longer-cell");
    assert!(doc.insert_table_row(true));

    let text = doc.to_string();
    let widths: Vec<usize> = text.lines().map(|line| line.chars().count()).collect();
    assert!(
        widths.windows(2).all(|pair| pair[0] == pair[1]),
        "every row is padded to the widest cell in its column:\n{text}"
    );
    assert!(
        // `b` goes to three, the narrowest a column may be written: `---` is the shortest delimiter
        // GFM accepts, so no column can be written narrower than its own dashes.
        text.starts_with("| a                  | b   |\n"),
        "and the column is as wide as that cell:\n{text}"
    );
}

/// **Alignment survives a re-pad.** Changing `:---` is out of scope for the controls, which makes
/// keeping it a requirement: a re-pad that dropped a colon would change how the table renders
/// everywhere else, with nobody having asked for it.
#[test]
fn column_alignment_survives_a_re_pad() {
    let source = "| a | b | c |\n| :-- | :-: | --: |\n| 1 | 2 | 3 |\n";
    let mut doc = at(source, "1");
    assert!(doc.insert_table_row(true));

    let delimiter = doc.to_string().lines().nth(1).unwrap().to_owned();
    assert_eq!(delimiter, "| :-- | :-: | --: |");
}

/// A `\|` is an escaped pipe inside a cell, not a cell boundary. Splitting on it would cut the cell
/// in half and the table would come back with a column nobody added.
#[test]
fn an_escaped_pipe_is_not_a_cell_boundary() {
    let source = "| a | b |\n| --- | --- |\n| x \\| y | z |\n";
    let doc = at(source, "x");
    let table = doc.table_at(doc.selection().head()).expect("in a table");

    assert_eq!(table.rows[1], vec!["x \\| y".to_owned(), "z".to_owned()]);
}

// ---- undo, and the invariant ----

#[test]
fn one_undo_takes_back_a_row_and_its_re_pad() {
    let mut doc = at(TABLE, "Ctrl+B");
    assert!(doc.insert_table_row(true));
    assert!(doc.undo());

    assert_eq!(doc.to_string(), TABLE);
}

proptest! {
    /// **One undo restores every byte, whatever the operation was.** The re-pad rewrites rows the
    /// user never touched, so undo restoring only the edit would leave the table changed — which is
    /// exactly the failure that would put this outside the byte-preservation rule.
    #[test]
    fn one_undo_takes_back_any_table_operation(which in 0_usize..5) {
        let mut doc = at(TABLE, "Ctrl+B");

        let did = match which {
            0 => doc.insert_table_row(true),
            1 => doc.insert_table_row(false),
            2 => doc.insert_table_column(true),
            3 => doc.delete_table_row(),
            _ => doc.delete_table_column(),
        };

        if did {
            prop_assert!(doc.undo());
        }
        prop_assert_eq!(doc.to_string(), TABLE);
    }

    /// **A table is still a table after any sequence of operations.** The one that catches a re-pad
    /// which loses a pipe: the parse has to still find a table, with every row holding the same
    /// number of cells as the header.
    #[test]
    fn a_table_survives_any_sequence_of_operations(
        operations in prop::collection::vec(0_usize..5, 1..8),
    ) {
        let mut doc = at(TABLE, "Ctrl+B");

        for operation in operations {
            match operation {
                0 => doc.insert_table_row(true),
                1 => doc.insert_table_row(false),
                2 => doc.insert_table_column(true),
                3 => doc.delete_table_row(),
                _ => doc.delete_table_column(),
            };
            // Back into the table, wherever the last operation left the caret.
            let text = doc.to_string();
            let head = ByteOffset::new(text.find("Keys").unwrap_or(0));
            doc.set_selection(Selection::caret(head));

            let table = doc.table_at(head).expect("it is still a table");
            let columns = table.columns();
            for row in &table.rows {
                prop_assert_eq!(
                    row.len(),
                    columns,
                    "every row keeps the header's cell count: {:?}",
                    table.rows
                );
            }
        }
    }
}
