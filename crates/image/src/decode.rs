//! PNG or JPEG bytes to a surface the renderer can upload.
//!
//! # This is the file-parsing trust boundary, and the caps are the point
//!
//! `AGENTS.md`: *"File parsing is a trust boundary. The 2023 Notepad++ CVEs were all heap overflows
//! in hand-rolled encoding detection — the class Rust removes, provided we don't reintroduce it in
//! `unsafe`."* Nothing here is hand-rolled and nothing here is `unsafe`: `png` 0.18 is
//! `#![forbid(unsafe_code)]`, which is why it was chosen over `image` — see the note on the
//! dependency in the workspace manifest. `zune-jpeg` keeps that sentence true only with its default
//! features off: they are the `unsafe` SIMD, `deny.toml` bans them, and without them the crate is
//! `#![forbid(unsafe_code)]` too (MDEDIT-148).
//!
//! What Rust does **not** remove is a decompression bomb. A PNG header is twenty-odd bytes and can
//! honestly declare a surface of billions of pixels; the allocation that follows is the attack, and
//! it is a memory-exhaustion crash rather than a memory-safety one. So the header is read first and
//! the dimensions checked **before** a buffer is sized — [`MAX_PIXELS`] and [`MAX_DIMENSION`] — and
//! the file is capped before it is read at all ([`MAX_FILE_BYTES`]).
//!
//! Every failure is a value. A truncated file, a bomb, a GIF somebody named `.png`: each one draws
//! the alt text, which is what the editor did before this crate existed and is never worse than what
//! the document says. The format is read from the first bytes, never from the file name.
//!
//! Known limit: EXIF orientation is ignored, so a phone photo may draw sideways.

use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::Path;

/// The largest file this will read, in bytes.
///
/// Checked from the directory entry, so an oversized file is refused without being opened. Sixteen
/// mebibytes is comfortably above any PNG a document sensibly embeds and far below the point where
/// reading one is a problem in itself.
pub const MAX_FILE_BYTES: u64 = 16 << 20;

/// The largest edge, in pixels.
///
/// **This one is not a policy, it is the hardware.** `wgpu::Limits::default()` puts
/// `max_texture_dimension_2d` at 8192, so a picture wider or taller than this cannot be uploaded
/// whatever anyone decides — refusing it here means saying so before spending the decode rather than
/// after.
pub const MAX_DIMENSION: u32 = 8192;

/// The largest surface, in pixels.
///
/// [`MAX_DIMENSION`] alone would still admit 8192 × 8192, which is 268 MB of RGBA against a baseline
/// RSS `AGENTS.md` already calls overrun. Sixteen megapixels is 64 MB, which is a large photograph
/// and a cost worth paying once.
pub const MAX_PIXELS: u64 = 16_000_000;

/// A decoded picture: straight RGBA, eight bits a channel, row-major, no padding.
///
/// One format rather than several because the renderer wants one, and converting here is where the
/// knowledge of what a PNG can be belongs. A greyscale PNG and a palette PNG both arrive as RGBA.
#[derive(Clone, PartialEq, Eq)]
pub struct Picture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Picture {
    /// Without this, one `assert_eq!` on a mismatched picture prints sixty megabytes.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Picture")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("rgba", &format_args!("{} bytes", self.rgba.len()))
            .finish()
    }
}

/// Why a file did not become a picture.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Undecodable {
    /// Could not be read at all.
    #[error("could not read the file")]
    Unreadable,

    /// Larger than [`MAX_FILE_BYTES`].
    #[error("the file is {0} bytes, over the {MAX_FILE_BYTES} byte limit")]
    TooLarge(u64),

    /// Declares a surface over [`MAX_DIMENSION`] or [`MAX_PIXELS`].
    #[error("{0}x{1} is larger than this editor will decode")]
    TooManyPixels(u32, u32),

    /// Not a PNG or JPEG, or a damaged one. Carries the decoder's own account, which is more use than ours.
    #[error("not a usable picture: {0}")]
    Malformed(String),
}

/// Reads and decodes the PNG or JPEG at `path`.
///
/// # Errors
///
/// [`Undecodable`], naming what stopped it.
pub fn read(path: &Path) -> Result<Picture, Undecodable> {
    within_the_file_cap(path)?;
    let bytes = std::fs::read(path).map_err(|_| Undecodable::Unreadable)?;
    decode(&bytes)
}

/// The size the PNG or JPEG at `path` declares, without decoding it (MDEDIT-156).
///
/// Reads the header and stops, which is the first few dozen bytes rather than the whole file. That
/// is what lets the editor give an image its row on the frame it appears in and decode the pixels
/// somewhere else: the geometry comes from here and only the picture comes from [`read`].
///
/// **The caps apply here too, and that is not belt and braces.** This is now the first thing to
/// believe a header, so it is where a bomb has to be refused — a row sized from an unchecked one
/// would be thirty thousand pixels tall before any decode was reached to say no.
///
/// # Errors
///
/// [`Undecodable`], naming what stopped it. A file whose header is intact measures even when its
/// image data is truncated or corrupt; that only shows up in [`read`].
pub fn measure(path: &Path) -> Result<(u32, u32), Undecodable> {
    contain(|| measure_file(path))
}

fn measure_file(path: &Path) -> Result<(u32, u32), Undecodable> {
    within_the_file_cap(path)?;
    let file = std::fs::File::open(path).map_err(|_| Undecodable::Unreadable)?;
    // Buffered, and the buffer is the point: unlike `read` this must not pull a 16 MB file into
    // memory to look at its first forty bytes.
    let mut reader = std::io::BufReader::new(file);
    let format = sniff(reader.fill_buf().map_err(|_| Undecodable::Unreadable)?)?;
    let (width, height) = match format {
        Format::Png => {
            let mut decoder = png::Decoder::new(reader);
            let info = decoder
                .read_header_info()
                .map_err(|error| Undecodable::Malformed(error.to_string()))?;
            (info.width, info.height)
        }
        Format::Jpeg => {
            within_the_segment_cap(&mut reader)?;
            jpeg_size(&mut jpeg_decoder(reader))?
        }
    };
    within_the_pixel_caps(width, height)?;
    Ok((width, height))
}

/// Refuses a file too big to be worth opening, from its directory entry.
fn within_the_file_cap(path: &Path) -> Result<(), Undecodable> {
    let length = std::fs::metadata(path)
        .map_err(|_| Undecodable::Unreadable)?
        .len();
    if length > MAX_FILE_BYTES {
        return Err(Undecodable::TooLarge(length));
    }
    Ok(())
}

/// Refuses a surface this editor will not allocate or upload.
fn within_the_pixel_caps(width: u32, height: u32) -> Result<(), Undecodable> {
    if width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(Undecodable::TooManyPixels(width, height));
    }
    Ok(())
}

enum Format {
    Png,
    Jpeg,
}

/// The format `bytes` start like. Never the file name: a picture is whatever its bytes are.
fn sniff(bytes: &[u8]) -> Result<Format, Undecodable> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Ok(Format::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Ok(Format::Jpeg)
    } else {
        Err(Undecodable::Malformed("not a PNG or JPEG".to_owned()))
    }
}

/// Decodes PNG or JPEG bytes to RGBA.
///
/// # Errors
///
/// [`Undecodable`], naming what stopped it.
pub fn decode(bytes: &[u8]) -> Result<Picture, Undecodable> {
    contain(|| decode_bytes(bytes))
}

fn decode_bytes(bytes: &[u8]) -> Result<Picture, Undecodable> {
    match sniff(bytes)? {
        Format::Png => decode_png(bytes),
        Format::Jpeg => decode_jpeg(bytes),
    }
}

fn jpeg_decoder<R: zune_core::bytestream::ZByteReaderTrait>(
    reader: R,
) -> zune_jpeg::JpegDecoder<R> {
    // Limit of `u16::MAX`: a JPEG cannot declare more than that an edge, so these limits refuse nothing themselves.
    // A smaller one would turn a bomb into an opaque `Malformed` before `within_the_pixel_caps` could
    // name it; the caps in this file are the ones that apply.
    let options = zune_core::options::DecoderOptions::default()
        .jpeg_set_out_colorspace(zune_core::colorspace::ColorSpace::RGBA)
        // Strict: lenient mode paints a truncated scan grey and says nothing; a bad file should be alt text.
        .set_strict_mode(true)
        .set_max_width(usize::from(u16::MAX))
        .set_max_height(usize::from(u16::MAX));
    zune_jpeg::JpegDecoder::new_with_options(reader, options)
}

/// Reads the headers and returns the declared size, not yet checked against the caps.
fn jpeg_size<R: zune_core::bytestream::ZByteReaderTrait>(
    decoder: &mut zune_jpeg::JpegDecoder<R>,
) -> Result<(u32, u32), Undecodable> {
    decoder
        .decode_headers()
        .map_err(|error| Undecodable::Malformed(error.to_string()))?;
    let info = decoder
        .info()
        .ok_or_else(|| Undecodable::Malformed("the decoder read no header".to_owned()))?;
    Ok((u32::from(info.width), u32::from(info.height)))
}

fn decode_jpeg(bytes: &[u8]) -> Result<Picture, Undecodable> {
    let mut cursor = std::io::Cursor::new(bytes);
    within_the_segment_cap(&mut cursor)?;
    let mut decoder = jpeg_decoder(cursor);
    // Before the pixel decode allocates, as for PNG.
    let (width, height) = jpeg_size(&mut decoder)?;
    within_the_pixel_caps(width, height)?;
    let rgba = decoder
        .decode()
        .map_err(|error| Undecodable::Malformed(error.to_string()))?;
    if rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
        return Err(Undecodable::Malformed(
            "the decoder returned the wrong number of bytes".to_owned(),
        ));
    }
    Ok(Picture {
        width,
        height,
        rgba,
    })
}

fn decode_png(bytes: &[u8]) -> Result<Picture, Undecodable> {
    // `Cursor` because `png` 0.18 wants `BufRead + Seek`, which a slice alone is not.
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // `EXPAND` takes a palette to RGB and a sub-byte greyscale up to eight bits, and turns a `tRNS`
    // chunk into a real alpha channel; `STRIP_16` brings sixteen-bit samples down to eight. Between
    // them every PNG colour type arrives as eight-bit Grey, GreyA, RGB or RGBA, which is four cases
    // below instead of the format's full matrix.
    decoder.set_transformations(png::Transformations::normalize_to_color8());

    let mut reader = decoder
        .read_info()
        .map_err(|error| Undecodable::Malformed(error.to_string()))?;

    // **Before the buffer is sized.** The header is cheap and honest about how big the surface will
    // be; this is the only point at which a bomb can be refused without first allocating it.
    let info = reader.info();
    within_the_pixel_caps(info.width, info.height)?;

    let mut buffer = vec![
        0;
        reader.output_buffer_size().ok_or_else(|| {
            Undecodable::Malformed("the decoder could not size its own output".to_owned())
        })?
    ];
    let frame = reader
        .next_frame(&mut buffer)
        .map_err(|error| Undecodable::Malformed(error.to_string()))?;

    let channels = match frame.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        // `normalize_to_color8` has already expanded a palette, so reaching this means the decoder
        // did something its own transformations say it does not.
        png::ColorType::Indexed => {
            return Err(Undecodable::Malformed(
                "the palette was not expanded".to_owned(),
            ));
        }
    };

    Ok(Picture {
        width: frame.width,
        height: frame.height,
        rgba: to_rgba(&buffer[..frame.buffer_size()], channels),
    })
}

/// Widens one, two or three channels to four.
///
/// Greyscale takes its one sample into all three colour channels; a missing alpha is opaque.
fn to_rgba(samples: &[u8], channels: usize) -> Vec<u8> {
    if channels == 4 {
        return samples.to_vec();
    }
    let mut rgba = Vec::with_capacity(samples.len() / channels * 4);
    for pixel in samples.chunks_exact(channels) {
        let (rgb, alpha) = match channels {
            1 => ([pixel[0], pixel[0], pixel[0]], 0xff),
            2 => ([pixel[0], pixel[0], pixel[0]], pixel[1]),
            _ => ([pixel[0], pixel[1], pixel[2]], 0xff),
        };
        rgba.extend_from_slice(&rgb);
        rgba.push(alpha);
    }
    rgba
}

/// Runs `work`, turning a panic inside a decoder into a value.
///
/// `zune-jpeg` has `unwrap` and `expect` on its decode path (`mcu.rs`, `color_convert/scalar.rs`,
/// `idct/scalar.rs`), and the bytes it reads are a stranger's. `measure` runs on the thread that
/// draws and `read` on the one worker that decodes, so an uncaught panic either closes the editor or
/// ends picture decoding for the session. **Depends on unwinding**: `panic = "abort"` would turn this
/// back into a crash, which is why the workspace manifest says not to set it. `AssertUnwindSafe` is
/// sound because `work` borrows its input immutably and nothing here outlives a panicking call.
fn contain<T>(work: impl FnOnce() -> Result<T, Undecodable>) -> Result<T, Undecodable> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
        .unwrap_or_else(|_| Err(Undecodable::Malformed("the decoder panicked".to_owned())))
}

/// The most application segments (`APP0`..`APP15`) a JPEG may carry before its scan.
const MAX_APP_SEGMENTS: u32 = 256;

/// Refuses a JPEG with more than [`MAX_APP_SEGMENTS`] application segments, then rewinds.
///
/// `zune-jpeg` 0.5.15 re-sorts and re-scans every Extended XMP segment collected so far after each
/// marker, and never drops an incomplete set, so a file of N such segments costs N squared: 16 000
/// of them took four seconds and 64 000 took fifty-five (MDEDIT-148). A real file has a handful.
/// This walks the markers from the start to the scan and stops there, so it never reads the image
/// data, and anything it cannot make sense of is left for the decoder to refuse.
fn within_the_segment_cap<R: Read + Seek>(reader: &mut R) -> Result<(), Undecodable> {
    let counted = count_app_segments(reader);
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|_| Undecodable::Unreadable)?;
    if counted > MAX_APP_SEGMENTS {
        return Err(Undecodable::Malformed(
            "the JPEG has too many application segments".to_owned(),
        ));
    }
    Ok(())
}

/// Application segments before the scan, stopping early once there are more than the cap.
fn count_app_segments<R: Read>(reader: &mut R) -> u32 {
    let mut count = 0;
    let mut byte = [0_u8; 1];
    if reader.read_exact(&mut [0_u8; 2]).is_err() {
        return count;
    }
    loop {
        if reader.read_exact(&mut byte).is_err() {
            return count;
        }
        // zune-jpeg tolerates stray bytes between segments, so resync on the next 0xff rather than stop.
        if byte[0] != 0xff {
            continue;
        }
        // Any number of 0xff fill bytes may precede a marker code.
        let marker = loop {
            if reader.read_exact(&mut byte).is_err() {
                return count;
            }
            if byte[0] != 0xff {
                break byte[0];
            }
        };
        match marker {
            // EOI and SOS: nothing more to count.
            0xd9 | 0xda => return count,
            // An escaped 0xff, which zune skips, and TEM, SOI and RSTn, which carry no length.
            0x00 | 0x01 | 0xd0..=0xd8 => continue,
            _ => {}
        }
        let mut length = [0_u8; 2];
        if reader.read_exact(&mut length).is_err() {
            return count;
        }
        let Some(payload) = u64::from(u16::from_be_bytes(length)).checked_sub(2) else {
            continue;
        };
        if (0xe0..=0xef).contains(&marker) {
            count += 1;
            if count > MAX_APP_SEGMENTS {
                return count;
            }
        }
        match std::io::copy(&mut reader.by_ref().take(payload), &mut std::io::sink()) {
            Ok(skipped) if skipped == payload => {}
            _ => return count,
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    fn segment(marker: u8, body: usize) -> Vec<u8> {
        let mut bytes = vec![0xff, marker];
        bytes.extend_from_slice(&u16::try_from(body + 2).unwrap().to_be_bytes());
        bytes.resize(bytes.len() + body, 0);
        bytes
    }

    fn stream(parts: &[&[u8]]) -> std::io::Cursor<Vec<u8>> {
        let mut bytes = vec![0xff, 0xd8];
        for part in parts {
            bytes.extend_from_slice(part);
        }
        std::io::Cursor::new(bytes)
    }

    fn apps(count: usize) -> Vec<u8> {
        (0..count).flat_map(|_| segment(0xe1, 3)).collect()
    }

    #[test]
    fn exactly_the_cap_of_application_segments_passes_and_one_more_is_refused() {
        assert_eq!(within_the_segment_cap(&mut stream(&[&apps(256)])), Ok(()));
        assert!(matches!(
            within_the_segment_cap(&mut stream(&[&apps(257)])),
            Err(Undecodable::Malformed(_))
        ));
    }

    #[test]
    fn the_count_stops_at_the_scan() {
        let fake_in_the_scan = apps(300);
        let mut reader = stream(&[&apps(10), &[0xff, 0xda, 0x00, 0x02], &fake_in_the_scan]);
        assert_eq!(count_app_segments(&mut reader), 10);
    }

    #[test]
    fn markers_without_a_length_are_not_read_as_having_one() {
        // Read as a length, `FF E1` is 65505 bytes, which would swallow everything after it.
        let mut parts: Vec<Vec<u8>> = Vec::new();
        for standalone in [
            [0xff, 0x01],
            [0xff, 0xd0],
            [0xff, 0xd7],
            [0xff, 0xd8],
            [0xff, 0x00],
        ] {
            parts.push(standalone.to_vec());
            parts.push(segment(0xe1, 3));
        }
        let parts: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
        assert_eq!(count_app_segments(&mut stream(&parts)), 5);
    }

    #[test]
    fn a_stream_cut_inside_a_segment_ends_the_count_with_what_was_seen() {
        // The cut segment is counted, as it was announced; nothing after it exists to be counted.
        let cut = &segment(0xe1, 50)[..10];
        assert_eq!(count_app_segments(&mut stream(&[&apps(3), cut])), 4);
    }

    #[test]
    fn a_panic_while_decoding_comes_back_as_malformed() {
        let result: Result<Picture, Undecodable> = contain(|| panic!("injected"));
        assert_eq!(
            result,
            Err(Undecodable::Malformed("the decoder panicked".to_owned()))
        );
    }

    #[test]
    fn a_panic_while_measuring_comes_back_as_malformed() {
        let result: Result<(u32, u32), Undecodable> = contain(|| panic!("injected"));
        assert_eq!(
            result,
            Err(Undecodable::Malformed("the decoder panicked".to_owned()))
        );
    }
}
