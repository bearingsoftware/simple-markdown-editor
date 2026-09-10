//! What the editor needs to draw an image: where the file is, and what is in it.
//!
//! Two boundaries, and this crate is both of them (MDEDIT-146):
//!
//! - [`resolve`] turns a destination written in a document into an absolute path, or refuses it.
//!   A destination is the least trusted of the three sources of paths this codebase handles, and
//!   the module says why at length.
//! - [`decode`] turns PNG bytes into an RGBA surface, with the caps that a decompression bomb makes
//!   necessary.
//!
//! Headless and platform-agnostic, like everything below `mdedit-app`. Nothing here knows about a
//! window, a GPU or a text engine, and nothing here writes a byte to the document — an image is
//! drawn beside the buffer, never into it.
//!
//! **PNG only, deliberately.** JPEG is a separate decision rather than a missing feature: the crate
//! that decodes it carries `unsafe` SIMD *inside* the trust boundary, which is worth its own ticket
//! and its own reading. Everything this crate refuses draws its alt text, which is what the editor
//! did before it existed.

mod decode;
mod resolve;

pub use decode::{
    MAX_DIMENSION, MAX_FILE_BYTES, MAX_PIXELS, Picture, Undecodable, decode, measure, read,
};
pub use resolve::{Refused, resolve};
