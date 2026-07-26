//! Partitions Markdown source into syntax-marker and content spans.
//!
//! The editor renders Markdown by *decorating the source* rather than by converting it to a document
//! model: the buffer holds literal Markdown, and syntax is concealed when the cursor is elsewhere.
//! That needs the byte range of each individual delimiter — the `**`, the `#`, the `>` — which no
//! Rust CommonMark parser reports.
//!
//! Implementation lands in MDEDIT-3; this crate is currently a placeholder so CI has something to
//! build.
