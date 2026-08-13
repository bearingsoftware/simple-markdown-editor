//! A table as rows and cells, editable as a grid and written back aligned (MDEDIT-128).
//!
//! # Why this re-serializes, when nothing else may
//!
//! `AGENTS.md`'s first rule is that the buffer is the source of truth and the document is never
//! re-serialized. This module rewrites a whole table block, and it is inside that rule rather than a
//! fourth exception to it: the rule permits re-serializing **the block under the cursor**, spliced
//! back at its span. Two things make it safe, and both are load-bearing:
//!
//! - **Every rewrite is inside one [`Document::transact`]**, so one Ctrl+Z restores every byte.
//! - **Nothing here is reachable from a save or a timer.** Only a user asking for a row or a column
//!   gets here. That is the whole difference between "the editor reformatted my file" and "I asked
//!   it to".
//!
//! The stated cost, accepted on the ticket: an edit touches rows the user did not edit, so the diff
//! of a one-cell change can show the whole table. A table whose source is ragged is unreadable in
//! every tool that is not this editor, which is the trade.
//!
//! # What is *not* rewritten
//!
//! **A cell's text is moved, never re-rendered.** It is taken as bytes and put back as bytes, so
//! inline emphasis keeps its own token, an escaped `\|` stays escaped, and a link keeps whatever
//! form it was written in. Only the padding around a cell and the dashes under it are this module's
//! to write.
//!
//! **Column alignment survives.** Changing `:---` is out of scope for the editing controls, which
//! makes preserving it a requirement rather than a nicety: a re-pad that quietly dropped a colon
//! would change how the table renders everywhere else without anyone asking it to.

use mdedit_language::BlockKind;
use mdedit_offsets::{ByteOffset, ByteSpan};

use crate::Document;

/// The narrowest a column may be written, because `---` is the shortest delimiter GFM accepts.
const MIN_WIDTH: usize = 3;

/// Whether `at` is inside `span`, counting the end.
///
/// A caret at the end of a cell is in that cell — it is where typing appends to it — so this is
/// inclusive at both ends rather than a half-open range.
fn holds(span: ByteSpan, at: ByteOffset) -> bool {
    span.start().get() <= at.get() && at.get() <= span.end().get()
}

/// What a column's delimiter says about its alignment.
///
/// Read and written back, never changed here — see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    /// `---`, the default.
    None,
    /// `:---`
    Left,
    /// `:---:`
    Centre,
    /// `---:`
    Right,
}

impl Alignment {
    /// Reads one cell of a delimiter row.
    fn of(cell: &str) -> Self {
        let cell = cell.trim();
        match (cell.starts_with(':'), cell.ends_with(':')) {
            (true, true) => Self::Centre,
            (true, false) => Self::Left,
            (false, true) => Self::Right,
            (false, false) => Self::None,
        }
    }

    /// Writes one, `width` characters wide.
    fn write(self, width: usize) -> String {
        let dashes = |count: usize| "-".repeat(count.max(1));
        match self {
            Self::None => dashes(width),
            Self::Left => format!(":{}", dashes(width - 1)),
            Self::Right => format!("{}:", dashes(width - 1)),
            Self::Centre => format!(":{}:", dashes(width - 2)),
        }
    }
}

/// A table, taken apart into the pieces an edit acts on.
///
/// The cells are owned text rather than spans because every operation moves them: inserting a column
/// shifts every cell after it, and a span would be stale the moment the first byte changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// The whole table in the buffer, its trailing newline excluded.
    pub span: ByteSpan,
    /// Every row, header first. The delimiter row is not one of them: it is syntax describing the
    /// columns, and carrying it as a row would put an empty row under every header.
    pub rows: Vec<Vec<String>>,
    /// One per column, from the delimiter row.
    pub alignments: Vec<Alignment>,
}

impl Table {
    /// How many columns the widest row has.
    ///
    /// Ragged tables are legal — GFM pads a short row and drops a long one's extra cells — so this
    /// is the count everything is written to.
    #[must_use]
    pub fn columns(&self) -> usize {
        self.rows.iter().map(Vec::len).max().unwrap_or(0)
    }

    /// Which bytes row `row` covers, from its first cell to its last.
    ///
    /// The cells, not the source line: the pipes are syntax the grid draws, and a selection that
    /// took them in would put the table's scaffolding on the clipboard.
    #[must_use]
    pub fn row_span(&self, row: usize) -> Option<ByteSpan> {
        // Rendered rows are one source line each, in order, with the delimiter row after the first.
        let mut at = self.span.start().get();
        let widths = self.widths();
        for (index, cells) in self.rows.iter().enumerate() {
            let line = Self::write_row(cells, &widths);
            if index == row {
                let text = line.trim_end_matches('\n');
                let first = text.find('|')? + 1;
                let last = text.rfind('|')?;
                return Some(ByteSpan::new(
                    ByteOffset::new(at + first),
                    ByteOffset::new(at + last),
                ));
            }
            at += line.len();
            if index == 0 {
                at += self.write_delimiter(&widths).len();
            }
        }
        None
    }

    /// Writes the table back out with its pipes aligned.
    ///
    /// Widths are counted in **characters, not bytes or display columns**. Bytes would misalign
    /// every non-ASCII table; true display width needs the font, which this crate deliberately
    /// cannot see. A CJK table is therefore aligned by count and will still look ragged in a
    /// proportional editor — the same compromise every Markdown formatter makes.
    #[must_use]
    pub fn render(&self) -> String {
        let widths = self.widths();
        let mut out = String::new();
        for (index, row) in self.rows.iter().enumerate() {
            out.push_str(&Self::write_row(row, &widths));
            if index == 0 {
                out.push_str(&self.write_delimiter(&widths));
            }
        }
        out
    }

    /// The width each column is written to: its widest cell, and never under [`MIN_WIDTH`].
    fn widths(&self) -> Vec<usize> {
        (0..self.columns())
            .map(|column| {
                self.rows
                    .iter()
                    .filter_map(|row| row.get(column))
                    .map(|cell| cell.chars().count())
                    .max()
                    .unwrap_or(0)
                    .max(MIN_WIDTH)
            })
            .collect()
    }

    fn write_row(row: &[String], widths: &[usize]) -> String {
        let mut out = String::from("|");
        for (column, width) in widths.iter().enumerate() {
            let cell = row.get(column).map_or("", String::as_str);
            let pad = width.saturating_sub(cell.chars().count());
            out.push(' ');
            out.push_str(cell);
            out.push_str(&" ".repeat(pad));
            out.push_str(" |");
        }
        out.push('\n');
        out
    }

    fn write_delimiter(&self, widths: &[usize]) -> String {
        let mut out = String::from("|");
        for (column, width) in widths.iter().enumerate() {
            let alignment = self
                .alignments
                .get(column)
                .copied()
                .unwrap_or(Alignment::None);
            out.push(' ');
            out.push_str(&alignment.write(*width));
            out.push_str(" |");
        }
        out.push('\n');
        out
    }
}

impl Document {
    /// The table the caret is in, taken apart (MDEDIT-128).
    ///
    /// `None` when the caret is not in one, which is what every operation below checks first and
    /// what the toolbar's contextual group asks.
    #[must_use]
    pub fn table_at(&self, at: ByteOffset) -> Option<Table> {
        let table = self
            .blocks()
            .iter()
            .find(|block| block.kind() == BlockKind::Table && holds(block.span(), at))?
            .span();

        let text = self.rope().byte_slice(table.range()).to_string();
        // The span may take in the newline that ends the last row; the model is the rows themselves.
        let span = ByteSpan::new(
            table.start(),
            ByteOffset::new(table.start().get() + text.trim_end_matches('\n').len()),
        );

        let mut lines = text.trim_end_matches('\n').lines();
        let header = split_row(lines.next()?);
        let alignments: Vec<Alignment> = split_row(lines.next()?)
            .iter()
            .map(|cell| Alignment::of(cell))
            .collect();

        let mut rows = vec![header];
        rows.extend(lines.map(split_row));

        Some(Table {
            span,
            rows,
            alignments,
        })
    }

    /// Which row and column of its table the caret is in, header row included as row zero.
    #[must_use]
    pub fn caret_cell(&self) -> Option<(usize, usize)> {
        let at = self.selection().head();
        let table = self
            .blocks()
            .iter()
            .find(|block| block.kind() == BlockKind::Table && holds(block.span(), at))?
            .span();

        let mut row = 0;
        let mut column = 0;
        let mut seen_row = false;
        for block in self.blocks() {
            if block.span().start() < table.start() || block.span().end() > table.end() {
                continue;
            }
            match block.kind() {
                BlockKind::TableRow => {
                    if seen_row {
                        row += 1;
                    }
                    seen_row = true;
                    column = 0;
                }
                BlockKind::TableCell => {
                    if holds(block.span(), at) {
                        return Some((row, column));
                    }
                    column += 1;
                }
                _ => {}
            }
        }
        None
    }
}

/// The operations the toolbar's table group performs.
///
/// Each is one [`Document::transact`], so the edit and the re-pad it forces are one undo step. Each
/// returns whether it did anything: `false` means the caret was not in a table, which is what the
/// caller shows a disabled button for.
impl Document {
    /// Inserts an empty row above or below the one the caret is in.
    ///
    /// **Above the header row inserts below it instead.** A row above the header would become the
    /// header, silently demoting the one the user wrote — GFM has no table without a header, so
    /// there is no honest way to put a body row first.
    pub fn insert_table_row(&mut self, below: bool) -> bool {
        let Some((row, _)) = self.caret_cell() else {
            return false;
        };
        let Some(mut table) = self.table_at(self.selection().head()) else {
            return false;
        };

        let at = if below || row == 0 { row + 1 } else { row };
        table.rows.insert(at, vec![String::new(); table.columns()]);
        self.rewrite(&table)
    }

    /// Inserts an empty column left or right of the one the caret is in.
    pub fn insert_table_column(&mut self, right: bool) -> bool {
        let Some((_, column)) = self.caret_cell() else {
            return false;
        };
        let Some(mut table) = self.table_at(self.selection().head()) else {
            return false;
        };

        let at = if right { column + 1 } else { column };
        for row in &mut table.rows {
            row.insert(at.min(row.len()), String::new());
        }
        table
            .alignments
            .insert(at.min(table.alignments.len()), Alignment::None);
        self.rewrite(&table)
    }

    /// Deletes the row the caret is in.
    ///
    /// **The header row cannot be deleted on its own.** Without it what is left is not a table, so
    /// the honest answers were to delete the whole table or to refuse; it refuses, because a control
    /// labelled "delete row" removing five is not what anybody pressed it for. Delete Table is
    /// beside it for that.
    pub fn delete_table_row(&mut self) -> bool {
        let Some((row, _)) = self.caret_cell() else {
            return false;
        };
        if row == 0 {
            return false;
        }
        let Some(mut table) = self.table_at(self.selection().head()) else {
            return false;
        };

        table.rows.remove(row);
        self.rewrite(&table)
    }

    /// Deletes the column the caret is in.
    ///
    /// Refuses on the last column, for the reason [`delete_table_row`](Self::delete_table_row)
    /// refuses on the header: a table with no columns is not a table.
    pub fn delete_table_column(&mut self) -> bool {
        let Some((_, column)) = self.caret_cell() else {
            return false;
        };
        let Some(mut table) = self.table_at(self.selection().head()) else {
            return false;
        };
        if table.columns() <= 1 {
            return false;
        }

        for row in &mut table.rows {
            if column < row.len() {
                row.remove(column);
            }
        }
        if column < table.alignments.len() {
            table.alignments.remove(column);
        }
        self.rewrite(&table)
    }

    /// Deletes the whole table, and the newline that ended it.
    pub fn delete_table(&mut self) -> bool {
        let Some(table) = self.table_at(self.selection().head()) else {
            return false;
        };
        let end = ByteOffset::new((table.span.end().get() + 1).min(self.rope().len_bytes()));
        let span = ByteSpan::new(table.span.start(), end);

        self.transact(|doc| {
            let ok = doc.replace(span, "").is_ok();
            if ok {
                doc.set_selection(crate::Selection::caret(span.start()));
            }
            ok
        })
    }

    /// Writes a changed table back over its own span, aligned.
    ///
    /// One transaction, so the caller's edit and this are one undo step — which is the whole reason
    /// re-padding on every edit is inside the byte-preservation rule rather than an exception to it.
    fn rewrite(&mut self, table: &Table) -> bool {
        let rendered = table.render();
        // `render` ends every row with a newline; the span does not include the last one.
        let rendered = rendered.trim_end_matches('\n').to_owned();

        self.transact(|doc| {
            let at = doc.selection().head();
            let ok = doc.replace(table.span, &rendered).is_ok();
            if ok {
                // The caret's byte moved with the padding around it. Clamping to the table keeps it
                // in the table rather than wherever the old offset now points.
                let end = ByteOffset::new(table.span.start().get() + rendered.len());
                doc.set_selection(crate::Selection::caret(at.min(end)));
            }
            ok
        })
    }
}

/// Splits one source row into its cells, without the pipes.
///
/// The leading and trailing pipes are optional in GFM, so an empty first or last piece from the
/// split is the pipe's own side rather than a cell. **A `\|` is not a cell boundary** — it is an
/// escaped pipe inside one, and splitting on it would cut a cell in half and lose the table.
fn split_row(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    for character in line.trim().chars() {
        match character {
            '\\' if !escaped => {
                escaped = true;
                cell.push(character);
            }
            '|' if !escaped => {
                cells.push(cell.trim().to_owned());
                cell = String::new();
            }
            _ => {
                escaped = false;
                cell.push(character);
            }
        }
    }
    cells.push(cell.trim().to_owned());

    // The pieces either side of the outer pipes, which are not cells.
    if cells.first().is_some_and(String::is_empty) {
        cells.remove(0);
    }
    if cells.last().is_some_and(String::is_empty) {
        cells.pop();
    }
    cells
}
