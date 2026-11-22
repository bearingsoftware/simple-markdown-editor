//! Cost of one keystroke in a large document (MDEDIT-207): a one-byte edit mid-document, through the
//! public edit API, which is the path that re-parses only a scope. Prints the median and p99 of
//! each size, because the latency target is a p99 and criterion reports a mean.

use std::time::{Duration, Instant};

use mdedit_document::Document;
use mdedit_offsets::{ByteOffset, ByteSpan};

const SAMPLES: usize = 3000;

/// The `segment.rs` bench's unit, repeated to the target size.
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

fn percentile(sorted: &[Duration], per_mille: usize) -> Duration {
    sorted[(sorted.len() * per_mille / 1000).min(sorted.len() - 1)]
}

fn main() {
    for kib in [100_usize, 1024] {
        let text = document(kib * 1024);
        let mut doc = Document::from(text.as_str());
        let at = ByteOffset::new(text.len() / 2);
        let mut times = Vec::with_capacity(SAMPLES);
        for n in 0..SAMPLES + 200 {
            let start = Instant::now();
            if n % 2 == 0 {
                doc.insert(at, "x").expect("in range");
            } else {
                doc.delete(ByteSpan::new(at, ByteOffset::new(at.get() + 1)))
                    .expect("in range");
            }
            let took = start.elapsed();
            if n >= 200 {
                times.push(took);
            }
        }
        assert!(doc.is_scoped(), "the edit must take the scoped path");
        times.sort();
        println!(
            "scoped edit {kib:>5} KiB  median {:>9.1?}  p99 {:>9.1?}  max {:>9.1?}",
            percentile(&times, 500),
            percentile(&times, 990),
            times[times.len() - 1],
        );
    }
}
