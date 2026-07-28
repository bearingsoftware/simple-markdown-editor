//! DirectWrite implementation of [`mdedit_text::TextLayout`].
//!
//! Required on Windows, not merely preferred. [MDEDIT-5](https://chattersley.atlassian.net/browse/MDEDIT-5)
//! confirmed on real hardware that `fontique` resolves `script = Hani` to a single fallback family
//! and stops, so 汉 renders as tofu with ten capable fonts installed.
//! [parley#689](https://github.com/linebender/parley/pull/689) fixes it and is unreleased; until it
//! ships, this is the only implementation that renders Simplified Chinese correctly on Windows.
//!
//! Everything here is behind `cfg(windows)`. On other platforms the crate compiles to nothing, so
//! the workspace still builds on Linux CI and the portable implementation can slot in beside it.

#![cfg_attr(not(windows), allow(unused_crate_dependencies))]

#[cfg(windows)]
mod dwrite;

#[cfg(windows)]
pub use dwrite::DirectWriteText;
