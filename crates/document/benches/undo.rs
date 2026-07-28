//! What an undo group costs, against what it would cost re-parsing per edit.
//!
//! MDEDIT-7 deferred the "re-parse once per transaction" optimisation until a transaction shape
//! existed to justify it. An undo group is that shape, so this measures the two sides:
//!
//! - **typing** 20 characters is 20 edits and therefore 20 re-parses;
//! - **undoing** them is 20 splices and **one** re-parse.
//!
//! If undo is not far cheaper than the typing that produced it, the grouping is not doing its job.

// `criterion_group!` expands to an undocumented public function.
#![allow(missing_docs)]

use criterion::{Criterion, criterion_group, criterion_main};
use mdedit_document::Document;
use mdedit_offsets::ByteOffset;
use std::hint::black_box;

/// Prose with the delimiter density of real notes, repeated to the target size.
fn document(target_bytes: usize) -> String {
    let unit = "\
## Section heading

Some prose with **strong**, *emphasis*, `code` and a [link](http://example.com \"title\").

- a list item
- another with `code`

> a quoted line

```rust
fn example() -> u32 { 42 }
```

";
    let mut out = String::with_capacity(target_bytes + unit.len());
    while out.len() < target_bytes {
        out.push_str(unit);
    }
    out
}

/// Types `count` characters at the end, one edit at a time, as a keyboard would.
fn type_tail(doc: &mut Document, count: usize) {
    for _ in 0..count {
        let at = ByteOffset::new(doc.len_bytes());
        let _ = doc.insert(at, "x");
    }
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("undo");
    const KEYSTROKES: usize = 20;

    for kib in [6_usize, 60] {
        let source = document(kib * 1024);

        // The cost of making the edits: one re-parse each.
        group.bench_function(format!("type-{KEYSTROKES}/{kib}KiB"), |b| {
            b.iter_batched(
                || Document::from(source.as_str()),
                |mut doc| {
                    type_tail(&mut doc, KEYSTROKES);
                    black_box(doc.len_bytes())
                },
                criterion::BatchSize::SmallInput,
            );
        });

        // The cost of taking them back: one re-parse for the whole group.
        group.bench_function(format!("undo-{KEYSTROKES}/{kib}KiB"), |b| {
            b.iter_batched(
                || {
                    let mut doc = Document::from(source.as_str());
                    type_tail(&mut doc, KEYSTROKES);
                    doc
                },
                |mut doc| {
                    black_box(doc.undo());
                    black_box(doc.len_bytes())
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
