//! The editor window.
//!
//! Split into a library and a thin binary so the parts that can be tested without a GPU — the
//! decoration in [`view`], the atlas packing in [`gpu`], the strip geometry in [`tabs`], the
//! offset arithmetic in [`scroll`], the chunking in [`blocks`], and the frame they produce —
//! actually are.
//!
//! Grown from the vertical slice in
//! [MDEDIT-11](https://chattersley.atlassian.net/browse/MDEDIT-11) through files, undo, tabs,
//! scrolling and now per-block layout: a frame lays out only the blocks the viewport touches, and
//! caches them by content. The parse is still whole-document on every keystroke, which is
//! MDEDIT-18.

pub mod blocks;
pub mod editor;
pub mod gpu;
pub mod latency;
pub mod scroll;
pub mod tabs;
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
