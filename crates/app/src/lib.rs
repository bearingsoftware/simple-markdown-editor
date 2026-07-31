//! The editor window.
//!
//! Split into a library and a thin binary so the parts that can be tested without a GPU — the
//! decoration in [`view`], the atlas packing in [`gpu`], the strip geometry in [`tabs`], the
//! offset arithmetic in [`scroll`], the chunking in [`blocks`], the geometry in [`content`], the
//! hit-testing in [`hit`], and the
//! frame they produce — actually are.
//!
//! Grown from the vertical slice in
//! [MDEDIT-11](https://chattersley.atlassian.net/browse/MDEDIT-11) through files, undo, tabs,
//! scrolling, per-block layout, a keystroke that re-parses a region rather than the document, an
//! input method that composes beside the buffer rather than into it, a caret you can point at, and
//! now a clipboard to do something with what you selected.

pub mod blocks;
pub mod clipboard;
pub mod config;
pub mod content;
pub mod detect;
pub mod editor;
pub mod gpu;
pub mod hit;
pub mod ime;
pub mod latency;
pub mod pane;
pub mod prompt;
pub mod scroll;
pub mod status;
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
