//! The picture decoder, which is the editor's most exposed parser: its input is a file a stranger
//! wrote, and it runs before anything has checked the bytes are what they claim to be.
//!
//! An image destination is a trust boundary. The decoders underneath (`png`, and `zune-jpeg` with its
//! SIMD off) parse untrusted bytes, and `decode` is what sizes a buffer from what they declare. So the
//! assertions are the caps `mdedit-image` promises and the length `Picture` documents, not "does not
//! panic": a picture that comes back with the wrong number of bytes is a bad slice in the renderer.
//!
//! There is no UTF-8 gate here. A picture is bytes, and `decode` takes bytes.
//!
//! **A panic inside `png` or `zune-jpeg` shows as a crash in this target, even though the editor
//! contains it.** `decode` wraps both decoders in `catch_unwind`, but `libfuzzer-sys` installs a panic
//! hook that aborts before anything unwinds. So this target dies on a panic the shipped editor would
//! turn into alt text. That is the same situation as `markers_segment` and MDEDIT-114: a real finding
//! to report upstream, not a reason to weaken the target or to swallow its exit status.

#![no_main]

use libfuzzer_sys::fuzz_target;

use mdedit_image::{MAX_DIMENSION, MAX_PIXELS};

fuzz_target!(|data: &[u8]| {
    let Ok(picture) = mdedit_image::decode(data) else {
        return;
    };

    assert!(
        picture.width <= MAX_DIMENSION,
        "wider than the texture limit"
    );
    assert!(
        picture.height <= MAX_DIMENSION,
        "taller than the texture limit"
    );
    let pixels = u64::from(picture.width) * u64::from(picture.height);
    assert!(pixels <= MAX_PIXELS, "more pixels than the decode cap");
    assert_eq!(
        picture.rgba.len() as u64,
        pixels * 4,
        "the buffer is not width * height * 4 bytes",
    );
});
