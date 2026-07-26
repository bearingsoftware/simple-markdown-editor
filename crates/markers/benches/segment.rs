//! Guards the parse budget in `AGENTS.md`: keystroke-to-pixel p50 <= 3 ms means a full re-parse of a
//! realistic document has to stay well inside a frame. `docs/research/04-parsing-benchmarks.md`
//! measured pulldown-cmark alone at 2.99 ms for 60 KB; this measures what segmentation adds.

// `criterion_group!` expands to an undocumented public function.
#![allow(missing_docs)]

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

/// Prose with the delimiter density of real notes, repeated to the target size.
fn document(target_bytes: usize) -> String {
    let unit = "\
## Section heading

Some prose with **strong**, *emphasis*, `code` and a [link](http://example.com \"title\").

- a list item
- another with `code`

> a quoted line
> and its continuation

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

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("segment");
    for kib in [6_usize, 60, 310] {
        let source = document(kib * 1024);
        group.throughput(criterion::Throughput::Bytes(
            source.len().try_into().unwrap_or(u64::MAX),
        ));

        // The parse alone, so the cost segmentation adds on top is visible rather than inferred.
        group.bench_function(format!("parse-only/{kib}KiB"), |b| {
            b.iter(|| {
                pulldown_cmark::Parser::new(black_box(&source))
                    .into_offset_iter()
                    .count()
            });
        });
        group.bench_function(format!("segment/{kib}KiB"), |b| {
            b.iter(|| mdedit_markers::segment(black_box(&source)));
        });
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
