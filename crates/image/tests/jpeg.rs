//! JPEG decoding (MDEDIT-148).
//!
//! `fixtures/rgb.jpg` was made on Windows with
//! `Add-Type -AssemblyName System.Drawing; $b = New-Object System.Drawing.Bitmap 4,3;`
//! `fill every pixel with FromArgb(200,40,40); $b.Save('rgb.jpg', [System.Drawing.Imaging.ImageFormat]::Jpeg)`.
//! GDI+ writes a three-component file even from an 8-bit indexed bitmap, so there is no greyscale
//! fixture; and it only writes baseline, so there is no progressive one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mdedit_image::{Undecodable, decode, measure, read};

const RGB: &[u8] = include_bytes!("fixtures/rgb.jpg");

fn position(bytes: &[u8], marker: [u8; 2]) -> usize {
    bytes
        .windows(2)
        .position(|pair| pair == marker)
        .expect("fixture has the marker")
}

fn file_named(name: &str, bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    (directory, path)
}

#[test]
fn a_jpeg_decodes_to_rgba_of_its_declared_size() {
    let picture = decode(RGB).unwrap();
    assert_eq!((picture.width, picture.height), (4, 3));
    assert_eq!(picture.rgba.len(), 4 * 3 * 4);
}

#[test]
fn the_colour_survives_to_within_the_loss_and_is_opaque() {
    let picture = decode(RGB).unwrap();
    let expected = [200_i32, 40, 40];
    for pixel in picture.rgba.as_chunks::<4>().0 {
        for (got, want) in pixel[..3].iter().zip(expected) {
            assert!((i32::from(*got) - want).abs() <= 12, "{pixel:?}");
        }
        assert_eq!(pixel[3], 255);
    }
}

#[test]
fn a_jpeg_is_recognised_by_its_bytes_not_its_name() {
    let (_directory, path) = file_named("x.png", RGB);
    let picture = read(&path).unwrap();
    assert_eq!((picture.width, picture.height), (4, 3));
    assert_eq!(measure(&path), Ok((4, 3)));
}

#[test]
fn measure_reads_a_jpegs_size_without_decoding_it() {
    let scan = position(RGB, [0xff, 0xda]);
    let (_directory, path) = file_named("cut.jpg", &RGB[..scan + 14]);
    assert_eq!(measure(&path), Ok((4, 3)));
    assert!(matches!(read(&path), Err(Undecodable::Malformed(_))));
}

#[test]
fn a_jpeg_declaring_too_many_pixels_is_refused_before_decoding() {
    let mut bytes = RGB.to_vec();
    let sof = position(&bytes, [0xff, 0xc0]);
    // marker (2), length (2), precision (1), then height and width, big-endian.
    bytes[sof + 5..sof + 7].copy_from_slice(&20000_u16.to_be_bytes());
    bytes[sof + 7..sof + 9].copy_from_slice(&20000_u16.to_be_bytes());
    // On a thread with a deadline: the caps must run before the pixel decode, and a decode after
    // them takes about 96 s.
    let (sender, receiver) = std::sync::mpsc::channel();
    let copy = bytes.clone();
    std::thread::spawn(move || {
        let _ = sender.send(decode(&copy));
    });
    let decoded = receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("decode refused the bomb within 10 s");
    assert_eq!(decoded, Err(Undecodable::TooManyPixels(20000, 20000)));
    let (_directory, path) = file_named("bomb.jpg", &bytes);
    assert_eq!(
        measure(&path),
        Err(Undecodable::TooManyPixels(20000, 20000))
    );
}

#[test]
fn garbage_and_an_empty_file_are_malformed_not_a_panic() {
    let noise: Vec<u8> = (0..64_u32)
        .map(|i| u8::try_from((i * 37 + 11) % 256).unwrap())
        .collect();
    for bytes in [&b""[..], b"\xFF\xD8\xFF", &noise] {
        assert!(matches!(decode(bytes), Err(Undecodable::Malformed(_))));
        let (_directory, path) = file_named("x.jpg", bytes);
        assert!(matches!(measure(&path), Err(Undecodable::Malformed(_))));
        assert!(matches!(read(&path), Err(Undecodable::Malformed(_))));
    }
}

#[test]
fn a_png_still_decodes() {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[1, 2, 3, 4]).unwrap();
    }
    let picture = decode(&bytes).unwrap();
    assert_eq!(
        (picture.width, picture.height, picture.rgba),
        (1, 1, vec![1, 2, 3, 4])
    );
}

/// SOI, `segments` Extended XMP segments with a fixed GUID and a gap between their offsets, then
/// the rest of `rgb.jpg`. `zune-jpeg` 0.5.15 spends N squared on these: 16 000 took 4.1 s and
/// 64 000 took 55 s.
fn xmp_flood(segments: usize) -> Vec<u8> {
    xmp_flood_after_first(segments, &[])
}

/// As [`xmp_flood`], with `between` after the first segment.
fn xmp_flood_after_first(segments: usize, between: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8];
    for index in 0..segments {
        let mut body = b"http://ns.adobe.com/xmp/extension/\0".to_vec();
        body.extend_from_slice(&[7; 32]);
        body.extend_from_slice(&(1_u32 << 30).to_be_bytes());
        body.extend_from_slice(&u32::try_from(index * 100 + 100).unwrap().to_be_bytes());
        body.push(b'x');
        bytes.extend_from_slice(&[0xff, 0xe1]);
        bytes.extend_from_slice(&u16::try_from(body.len() + 2).unwrap().to_be_bytes());
        bytes.extend_from_slice(&body);
        if index == 0 {
            bytes.extend_from_slice(between);
        }
    }
    bytes.extend_from_slice(&RGB[2..]);
    bytes
}

fn within_ten_seconds<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("finished within 10 s")
}

#[test]
fn a_flood_of_application_segments_is_refused_without_quadratic_work() {
    let bytes = xmp_flood(16_000);
    let copy = bytes.clone();
    let decoded = within_ten_seconds(move || decode(&copy));
    assert!(
        matches!(decoded, Err(Undecodable::Malformed(_))),
        "{decoded:?}"
    );
    let (_directory, path) = file_named("flood.jpg", &bytes);
    let measured = within_ten_seconds(move || measure(&path));
    assert!(
        matches!(measured, Err(Undecodable::Malformed(_))),
        "{measured:?}"
    );
}

#[test]
fn a_handful_of_application_segments_still_decodes() {
    let bytes = xmp_flood(200);
    let picture = decode(&bytes).unwrap();
    assert_eq!((picture.width, picture.height), (4, 3));
    let (_directory, path) = file_named("some.jpg", &bytes);
    assert_eq!(measure(&path), Ok((4, 3)));
}

#[test]
fn stray_bytes_between_segments_do_not_hide_a_flood() {
    // zune-jpeg tolerates stray bytes before a marker and skips `FF 00`, then goes on collecting.
    for between in [&[0x55_u8][..], &[0xff, 0x00]] {
        let bytes = xmp_flood_after_first(16_000, between);
        let copy = bytes.clone();
        let decoded = within_ten_seconds(move || decode(&copy));
        assert!(
            matches!(decoded, Err(Undecodable::Malformed(_))),
            "{between:?} {decoded:?}"
        );
        let (_directory, path) = file_named("flood.jpg", &bytes);
        let measured = within_ten_seconds(move || measure(&path));
        assert!(
            matches!(measured, Err(Undecodable::Malformed(_))),
            "{between:?} {measured:?}"
        );
    }
}
