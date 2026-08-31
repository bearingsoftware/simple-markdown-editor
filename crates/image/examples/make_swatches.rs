//! Writes the PNGs the by-hand rendering check uses (MDEDIT-146).
//!
//! An example rather than a test, because it produces files for a human to look at: CI cannot see
//! whether a picture is drawn the right way up, and this is what a person opens the editor on.
//!
//! ```text
//! cargo run -p mdedit-image --example make_swatches -- C:\tmp\imgcheck
//! ```

#![allow(clippy::expect_used, clippy::print_stdout)]

fn main() {
    let out = std::env::args().nth(1).expect("an output directory");
    write(&format!("{out}/swatch.png"), 160, 120);
    write(&format!("{out}/wide.png"), 4000, 1000);
    println!("wrote swatch.png and wide.png to {out}");
}

/// A gradient rather than a flat fill, so a picture drawn mirrored or upside down is obvious.
fn write(path: &str, width: u32, height: u32) {
    let file = std::fs::File::create(path).expect("create");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("header");

    let mut pixels = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for y in 0..height {
        for x in 0..width {
            let across = u8::try_from(x * 255 / width.max(1)).unwrap_or(255);
            let down = u8::try_from(y * 255 / height.max(1)).unwrap_or(255);
            // Red rises left to right, green top to bottom: the top-left corner is near black and
            // the bottom-right near yellow, so every flip and rotation looks different.
            pixels.extend_from_slice(&[across, down, 0x60, 0xff]);
        }
    }
    writer.write_image_data(&pixels).expect("data");
    writer.finish().expect("finish");
}
