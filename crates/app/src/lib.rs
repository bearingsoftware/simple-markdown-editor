//! The editor window.
//!
//! Split into a library and a thin binary so the parts that can be tested without a GPU — the
//! decoration in [`view`] and the atlas packing in [`gpu`] — actually are.
//!
//! This is the vertical slice from [MDEDIT-11](https://chattersley.atlassian.net/browse/MDEDIT-11):
//! one hardcoded document, minimal input, and a latency harness. No file I/O, no tabs, no undo, no
//! scrolling, no virtualization.

pub mod editor;
pub mod gpu;
pub mod latency;
pub mod view;

/// The document the slice opens with.
///
/// Chosen to exercise the things that are hard rather than the things that look good: nested
/// emphasis, a fenced block, a multi-byte line, and Han characters that only render if font
/// fallback works — the failure MDEDIT-5 found in `fontique` and the reason this uses DirectWrite.
pub const SAMPLE: &str = "\
# Simple Markdown Editor

Syntax is **concealed** until the caret is *near* it, and the buffer still holds
the literal `Markdown` bytes underneath.

## Fallback

Latin, 汉字编辑器, and محرر all in one paragraph.

```rust
fn main() {
    println!(\"fenced code stays monospaced\");
}
```

- a list item
- another one

> a quoted line
";
