//! What a decode costs on the thread that draws (MDEDIT-151).
//!
//! Ignored by default, like `find_cost` and `frame_cost`: a measurement, not an assertion.
//!
//! MDEDIT-146 put PNG decoding on the layout path, which is the UI thread. `AGENTS.md` says I/O
//! belongs on a worker; MDEDIT-151 is about whether that sentence describes something worth
//! building or something to correct. This is the number that decides it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::cast_precision_loss,
    reason = "a measurement harness: it prints numbers and asserts nothing"
)]

use std::time::Instant;

/// A PNG of `width` x `height`, with content that does not compress to nothing.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let mut pixels = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for y in 0..height {
        for x in 0..width {
            // A gradient rather than a flat fill: a solid colour deflates to almost nothing and
            // would measure the decompressor doing no work.
            let across = u8::try_from(x % 256).unwrap_or(0);
            let down = u8::try_from(y % 256).unwrap_or(0);
            pixels.extend_from_slice(&[across, down, across ^ down, 0xff]);
        }
    }
    writer.write_image_data(&pixels).unwrap();
    writer.finish().unwrap();
    bytes
}

#[test]
#[ignore = "a measurement, not an assertion"]
fn what_a_decode_costs() {
    println!(
        "{:>14}  {:>10}  {:>12}  {:>12}  {:>10}",
        "pixels", "file KB", "best ms", "worst ms", "MB RGBA"
    );
    for (w, h, label) in [
        (640, 480, "a screenshot"),
        (1920, 1080, "a full-HD screenshot"),
        (3840, 2160, "a 4K screenshot"),
        (4000, 4000, "16 megapixels — MAX_PIXELS"),
    ] {
        let bytes = png(w, h);
        // **Both ends of three, not just the best.** A single figure from this harness invites
        // being quoted as settled, and it is not: two runs on the same machine gave 65 ms and
        // 84 ms for the 4K case. Printing the spread makes one run tell you how much to trust it.
        let mut best = f64::MAX;
        let mut worst = 0.0_f64;
        for _ in 0..3 {
            let started = Instant::now();
            let picture = mdedit_image::decode(&bytes).unwrap();
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            best = best.min(ms);
            worst = worst.max(ms);
            assert_eq!(picture.rgba.len(), (w as usize) * (h as usize) * 4);
        }
        println!(
            "{:>14}  {:>10.0}  {:>12.1}  {:>12.1}  {:>10.1}   {label}",
            format!("{w}x{h}"),
            bytes.len() as f64 / 1024.0,
            best,
            worst,
            f64::from(w) * f64::from(h) * 4.0 / 1_048_576.0
        );
    }
}
