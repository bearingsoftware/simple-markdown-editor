//! What comes back out of a PNG, and what a hostile one gets instead (MDEDIT-146).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use mdedit_image::{MAX_FILE_BYTES, Undecodable, decode, measure, read};

/// Encodes a PNG so the tests decode something a real encoder wrote rather than bytes we asserted
/// about. `samples` is in `colour`'s own channel order, at eight bits.
fn encoded(width: u32, height: u32, colour: png::ColorType, samples: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(colour);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(samples).unwrap();
    writer.finish().unwrap();
    bytes
}

/// A decompression bomb: a valid, tiny PNG whose header claims an enormous surface.
///
/// **Built by patching a real one's `IHDR` rather than by writing a header alone**, because
/// `png`'s `read_info` reads through to the first `IDAT` and refuses a file that has none — so a
/// header on its own never reaches the dimension check and would prove nothing about it.
///
/// This is what a bomb actually looks like. The file stays a few dozen bytes and the surface it
/// declares is gigabytes, which is the entire attack: the cost is in believing the header.
fn bomb(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = encoded(1, 1, png::ColorType::Rgba, &[0; 4]);
    // After the 8-byte signature: a 4-byte length, `IHDR`, then width and height as big-endian u32.
    bytes[16..20].copy_from_slice(&width.to_be_bytes());
    bytes[20..24].copy_from_slice(&height.to_be_bytes());
    // The chunk's CRC covers its type and its data — bytes 12 through 28 — and a wrong one is
    // rejected before anything looks at the dimensions.
    let crc = crc32(&bytes[12..29]);
    bytes[29..33].copy_from_slice(&crc.to_be_bytes());
    bytes
}

/// CRC-32 as PNG defines it, so [`bomb`] can re-stamp the header it edits.
///
/// Written out rather than depended on: `crc32fast` is here only as something `png` uses, and one
/// bit-at-a-time loop over seventeen bytes in a test is not worth making that a direct dependency.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn an_rgba_png_comes_back_unchanged() {
    let samples = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
    let picture = decode(&encoded(2, 1, png::ColorType::Rgba, &samples)).unwrap();

    assert_eq!((picture.width, picture.height), (2, 1));
    assert_eq!(picture.rgba, samples);
}

#[test]
fn an_rgb_png_gains_an_opaque_alpha() {
    let picture = decode(&encoded(2, 1, png::ColorType::Rgb, &[1, 2, 3, 4, 5, 6])).unwrap();

    assert_eq!(picture.rgba, [1, 2, 3, 0xff, 4, 5, 6, 0xff]);
}

#[test]
fn a_greyscale_png_becomes_grey_in_three_channels() {
    let picture = decode(&encoded(2, 1, png::ColorType::Grayscale, &[0x40, 0xd0])).unwrap();

    assert_eq!(
        picture.rgba,
        [0x40, 0x40, 0x40, 0xff, 0xd0, 0xd0, 0xd0, 0xff]
    );
}

#[test]
fn a_greyscale_png_with_alpha_keeps_its_alpha() {
    let samples = [0x40, 0x10, 0xd0, 0xf0];
    let picture = decode(&encoded(2, 1, png::ColorType::GrayscaleAlpha, &samples)).unwrap();

    assert_eq!(
        picture.rgba,
        [0x40, 0x40, 0x40, 0x10, 0xd0, 0xd0, 0xd0, 0xf0]
    );
}

#[test]
fn a_palette_png_arrives_expanded() {
    // Two one-bit indices into a two-entry palette, which is the case `normalize_to_color8` is
    // carrying: without the transformation this comes back `Indexed` and a byte a pixel.
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::One);
    encoder.set_palette(vec![0xff, 0x00, 0x00, 0x00, 0x00, 0xff]);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&[0b0100_0000]).unwrap();
    writer.finish().unwrap();

    let picture = decode(&bytes).unwrap();

    assert_eq!(picture.rgba, [0xff, 0, 0, 0xff, 0, 0, 0xff, 0xff]);
}

#[test]
fn every_pixel_gets_four_bytes() {
    let picture = decode(&encoded(3, 2, png::ColorType::Rgb, &[0; 18])).unwrap();

    assert_eq!(picture.rgba.len(), 3 * 2 * 4);
}

#[test]
fn a_file_that_is_not_a_png_is_a_value_rather_than_a_panic() {
    let error = decode(b"\xff\xd8\xff\xe0 this is the start of a JPEG").unwrap_err();

    assert!(matches!(error, Undecodable::Malformed(_)), "{error:?}");
}

#[test]
fn a_truncated_png_is_malformed() {
    let whole = encoded(4, 4, png::ColorType::Rgba, &[0; 64]);
    let error = decode(&whole[..whole.len() / 2]).unwrap_err();

    assert!(matches!(error, Undecodable::Malformed(_)), "{error:?}");
}

#[test]
fn nothing_at_all_is_malformed() {
    assert!(matches!(decode(&[]), Err(Undecodable::Malformed(_))));
}

#[test]
fn a_surface_with_too_many_pixels_is_refused_from_the_header_alone() {
    // 20000 x 20000 is 400 megapixels — 1.6 GB of RGBA. The header saying so is all this reads.
    let error = decode(&bomb(20_000, 20_000)).unwrap_err();

    assert_eq!(error, Undecodable::TooManyPixels(20_000, 20_000));
}

#[test]
fn a_surface_wider_than_a_texture_can_be_is_refused() {
    // Under the pixel cap, over the edge cap: 9000 x 1 is nine thousand pixels, and no GPU in
    // `wgpu`'s default limits will take a texture that wide.
    let error = decode(&bomb(9000, 1)).unwrap_err();

    assert_eq!(error, Undecodable::TooManyPixels(9000, 1));
}

#[test]
fn a_bomb_is_refused_before_the_buffer_is_sized() {
    // The point of the previous two tests, stated as the property rather than as a dimension: the
    // header alone is 45 bytes, so nothing here could have allocated the surface it declares and
    // still returned. If this ever starts taking a second, the check has moved after the allocation.
    let header = bomb(30_000, 30_000);

    assert!(header.len() < 100, "{} bytes", header.len());
    assert!(decode(&header).is_err());
}

#[test]
fn a_file_over_the_size_cap_is_refused_without_being_read() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("huge.png");
    // Sparse where the filesystem allows it, so this costs a directory entry rather than 16 MB.
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(MAX_FILE_BYTES + 1).unwrap();
    drop(file);

    let error = read(&path).unwrap_err();

    assert_eq!(error, Undecodable::TooLarge(MAX_FILE_BYTES + 1));
}

#[test]
fn a_file_at_the_cap_is_not_refused_for_its_size() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("exact.png");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(MAX_FILE_BYTES).unwrap();
    drop(file);

    // Zero bytes are not a PNG, so this fails — but on what it *is*, not on how big it is. The cap
    // is a limit rather than an off-by-one.
    assert!(matches!(read(&path), Err(Undecodable::Malformed(_))));
}

#[test]
fn a_file_that_is_not_there_is_unreadable() {
    let dir = tempfile::TempDir::new().unwrap();

    assert_eq!(
        read(&dir.path().join("absent.png")).unwrap_err(),
        Undecodable::Unreadable
    );
}

#[test]
fn a_real_file_round_trips() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("cat.png");
    std::fs::write(&path, encoded(2, 2, png::ColorType::Rgba, &[0x7f; 16])).unwrap();

    let picture = read(&path).unwrap();

    assert_eq!((picture.width, picture.height), (2, 2));
    assert_eq!(picture.rgba, [0x7f; 16]);
}

// ---------------------------------------------------------------------------------------------
// `measure`: the size without the pixels (MDEDIT-156).
// ---------------------------------------------------------------------------------------------

/// Writes `bytes` to a file in `dir` and hands back the path.
fn written(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn measuring_gives_the_size_the_header_declares() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = written(
        &dir,
        "cat.png",
        &encoded(7, 3, png::ColorType::Rgba, &[0x40; 7 * 3 * 4]),
    );

    assert_eq!(measure(&path).unwrap(), (7, 3));
}

#[test]
fn measuring_does_not_decode_the_pixels() {
    // **The property, not the timing.** A file cut off after its header has dimensions and no image
    // data: `read` cannot produce a picture from it and `measure` can still say how big it claims to
    // be. If this ever fails, `measure` has started decoding and the whole point of it is gone.
    let dir = tempfile::TempDir::new().unwrap();
    let whole = encoded(64, 64, png::ColorType::Rgba, &[0x20; 64 * 64 * 4]);
    let path = written(&dir, "cut.png", &whole[..40]);

    assert_eq!(measure(&path).unwrap(), (64, 64));
    assert!(read(&path).is_err(), "a truncated PNG decoded");
}

#[test]
fn measuring_refuses_a_bomb_on_its_header_alone() {
    // The reason `measure` applies the caps rather than leaving them to the decode it precedes: a
    // row sized from this header would be thirty thousand pixels tall, and the decode would never
    // be reached to refuse it.
    let dir = tempfile::TempDir::new().unwrap();
    let path = written(&dir, "bomb.png", &bomb(30_000, 30_000));

    assert_eq!(
        measure(&path).unwrap_err(),
        Undecodable::TooManyPixels(30_000, 30_000)
    );
}

#[test]
fn measuring_refuses_an_edge_over_the_hardware_limit() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = written(&dir, "wide.png", &bomb(9000, 1));

    assert_eq!(
        measure(&path).unwrap_err(),
        Undecodable::TooManyPixels(9000, 1)
    );
}

#[test]
fn measuring_refuses_a_file_over_the_size_cap() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("huge.png");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(MAX_FILE_BYTES + 1).unwrap();
    drop(file);

    assert_eq!(
        measure(&path).unwrap_err(),
        Undecodable::TooLarge(MAX_FILE_BYTES + 1)
    );
}

#[test]
fn measuring_something_that_is_not_a_png_says_so() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = written(&dir, "cat.png", b"\xff\xd8\xff\xe0 actually a JPEG");

    assert!(matches!(measure(&path), Err(Undecodable::Malformed(_))));
}

#[test]
fn measuring_a_file_that_is_not_there_is_unreadable() {
    let dir = tempfile::TempDir::new().unwrap();

    assert_eq!(
        measure(&dir.path().join("absent.png")).unwrap_err(),
        Undecodable::Unreadable
    );
}

// ---------------------------------------------------------------------------------------------
// The caps at their edges.
//
// **Every test above this point exceeds the *width*.** `MAX_DIMENSION` is checked first, so a wide
// enough picture short-circuits the whole condition and the other two clauses were never once
// evaluated by a test — a 1x9000 PNG would have been accepted, and then aborted the process at
// texture creation, which is the failure MDEDIT-146 already shipped to a human. cargo-mutants found
// this on MDEDIT-156 by turning `||` into `&&` and watching nothing go red.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_picture_too_tall_is_refused_even_though_it_is_narrow() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = written(&dir, "tall.png", &bomb(1, 9000));

    assert_eq!(
        measure(&path).unwrap_err(),
        Undecodable::TooManyPixels(1, 9000)
    );
}

#[test]
fn a_picture_at_the_largest_edge_exactly_is_allowed() {
    // 8192 is the limit, not the first refusal: `>` rather than `>=`, in both directions.
    let dir = tempfile::TempDir::new().unwrap();
    let wide = written(&dir, "wide.png", &bomb(8192, 1));
    let tall = written(&dir, "tall.png", &bomb(1, 8192));

    assert_eq!(measure(&wide).unwrap(), (8192, 1));
    assert_eq!(measure(&tall).unwrap(), (1, 8192));
}

#[test]
fn a_surface_over_the_pixel_cap_is_refused_with_both_edges_under_the_limit() {
    // **The only test that reaches the `MAX_PIXELS` clause at all.** 8000 x 8000 is inside
    // `MAX_DIMENSION` on both edges and 64 megapixels of RGBA, which is 256 MB — four times the cap
    // and the allocation the cap exists to refuse.
    let dir = tempfile::TempDir::new().unwrap();
    let path = written(&dir, "square.png", &bomb(8000, 8000));

    assert_eq!(
        measure(&path).unwrap_err(),
        Undecodable::TooManyPixels(8000, 8000)
    );
}

#[test]
fn a_surface_at_the_pixel_cap_exactly_is_allowed() {
    // 4000 x 4000 is `MAX_PIXELS` to the pixel, and the doc comment calls sixteen megapixels a cost
    // worth paying once. If this starts failing, the cap became `>=` and the largest picture the
    // editor claims to support stopped working.
    let dir = tempfile::TempDir::new().unwrap();
    let path = written(&dir, "cap.png", &bomb(4000, 4000));

    assert_eq!(measure(&path).unwrap(), (4000, 4000));
}

#[test]
fn the_pixel_cap_multiplies_rather_than_adds() {
    // 5000 + 5000 is 10,000 and 5000 x 5000 is 25 million. Only one of those is over the cap, and
    // the picture is 100 MB of RGBA either way.
    let dir = tempfile::TempDir::new().unwrap();
    let path = written(&dir, "product.png", &bomb(5000, 5000));

    assert_eq!(
        measure(&path).unwrap_err(),
        Undecodable::TooManyPixels(5000, 5000)
    );
}
