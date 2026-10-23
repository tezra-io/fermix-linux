//! The chat's pictures decoded for drawing (`picture::decode`): what a phone or a web page sends,
//! read header first and refused past the ceiling before any pixel is allocated, turned upright
//! the way its EXIF says, and scaled to what is drawn.

use fermix_client::media::Fit;
use fermix_client::picture::{decode, Refused};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use std::path::PathBuf;
use tempfile::TempDir;

/// 64 by 48, red in its top-left 8 by 8 and blue elsewhere, so a turn shows where the corner went.
fn picture() -> DynamicImage {
    let red = |x: u32, y: u32| x < 8 && y < 8;
    let pixels = RgbImage::from_fn(64, 48, |x, y| {
        if red(x, y) {
            Rgb([255, 0, 0])
        } else {
            Rgb([0, 0, 255])
        }
    });
    DynamicImage::ImageRgb8(pixels)
}

fn encoded(format: ImageFormat) -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    picture().write_to(&mut bytes, format).expect("encoded");
    bytes.into_inner()
}

fn written(dir: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).expect("written");
    path
}

/// A JPEG with the EXIF segment a camera writes after its start marker: little-endian TIFF, one
/// entry, Orientation (0x0112) as a SHORT. The same 36 bytes as Fable's `exif6.jpg` for 6.
fn with_orientation(jpeg: &[u8], orientation: u8) -> Vec<u8> {
    let app1: [u8; 36] = [
        0xff,
        0xe1,
        0x00,
        0x22,
        b'E',
        b'x',
        b'i',
        b'f',
        0,
        0,
        b'I',
        b'I',
        0x2a,
        0,
        8,
        0,
        0,
        0,
        1,
        0,
        0x12,
        0x01,
        3,
        0,
        1,
        0,
        0,
        0,
        orientation,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    [&jpeg[..2], &app1[..], &jpeg[2..]].concat()
}

/// The CRC of a PNG chunk's type and data (ISO 3309), bit by bit.
fn crc(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 * (crc & 1));
        }
    }
    !crc
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let body: Vec<u8> = kind.iter().chain(data).copied().collect();
    let length = u32::try_from(data.len()).expect("a short chunk");
    [&length.to_be_bytes()[..], &body, &crc(&body).to_be_bytes()].concat()
}

/// The RGB of the pixel at `(x, y)` of `rgba`, `width` pixels a row.
fn rgb(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let at = usize::try_from((y * width + x) * 4).expect("an index");
    [rgba[at], rgba[at + 1], rgba[at + 2]]
}

fn reddish([r, _, b]: [u8; 3]) -> bool {
    r > 200 && b < 80
}

#[test]
fn a_small_picture_decodes_to_its_straight_rgba() {
    let dir = TempDir::new().expect("a directory");
    let pixels = image::RgbaImage::from_raw(
        2,
        2,
        vec![
            255, 0, 0, 255, 0, 255, 0, 128, //
            0, 0, 255, 0, 255, 255, 255, 255,
        ],
    )
    .expect("2 by 2");
    let mut png = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(pixels.clone())
        .write_to(&mut png, ImageFormat::Png)
        .expect("a png");
    let decoded =
        decode(&written(&dir, "two.png", png.get_ref()), 360, Fit::Cover).expect("decoded");
    assert_eq!((decoded.width, decoded.height), (2, 2));
    assert_eq!(decoded.rgba, pixels.into_raw());
}

#[test]
fn every_format_a_phone_or_a_page_sends_decodes() {
    let dir = TempDir::new().expect("a directory");
    let formats = [
        ("p.png", ImageFormat::Png),
        ("p.jpg", ImageFormat::Jpeg),
        ("p.gif", ImageFormat::Gif),
        ("p.webp", ImageFormat::WebP),
    ];
    for (name, format) in formats {
        let path = written(&dir, name, &encoded(format));
        let decoded = decode(&path, 360, Fit::Cover).expect(name);
        assert_eq!((decoded.width, decoded.height), (64, 48), "{name}");
        assert_eq!(decoded.rgba.len(), 64 * 48 * 4, "{name}");
        assert!(reddish(rgb(&decoded.rgba, 64, 2, 2)), "{name}");
        assert!(!reddish(rgb(&decoded.rgba, 64, 40, 30)), "{name}");
    }
}

/// The format is read from the bytes, never the name the file goes by.
#[test]
fn a_picture_is_known_by_its_bytes_not_its_name() {
    let dir = TempDir::new().expect("a directory");
    let path = written(&dir, "really-a-png.jpg", &encoded(ImageFormat::Png));
    assert!(decode(&path, 360, Fit::Cover).is_ok());
    let text = written(&dir, "notes.png", b"not a picture at all");
    assert_eq!(decode(&text, 360, Fit::Cover), Err(Refused::Unreadable));
}

#[test]
fn a_picture_is_decoded_no_larger_than_it_is_drawn() {
    let dir = TempDir::new().expect("a directory");
    let path = written(&dir, "p.png", &encoded(ImageFormat::Png));
    let held = decode(&path, 24, Fit::Contain).expect("contained");
    assert_eq!((held.width, held.height), (24, 18));
    let covered = decode(&path, 24, Fit::Cover).expect("covered");
    assert_eq!((covered.width, covered.height), (32, 24));
    assert_eq!(covered.rgba.len(), 32 * 24 * 4);
}

/// A phone's photo stored on its side, with EXIF Orientation 6, comes out upright: turned a
/// quarter clockwise, its top-left corner at the top right.
#[test]
fn a_photo_is_turned_the_way_its_exif_says() {
    let dir = TempDir::new().expect("a directory");
    let jpeg = with_orientation(&encoded(ImageFormat::Jpeg), 6);
    let decoded = decode(&written(&dir, "exif6.jpg", &jpeg), 360, Fit::Contain).expect("decoded");
    assert_eq!((decoded.width, decoded.height), (48, 64));
    assert!(
        reddish(rgb(&decoded.rgba, 48, 44, 3)),
        "the corner went to the top right"
    );
    assert!(!reddish(rgb(&decoded.rgba, 48, 3, 3)));
}

/// The header is read alone: a 57-byte PNG that claims 100,000 by 100,000 pixels (its image
/// data empty, read only as far as where it starts) is refused before a pixel is allocated.
#[test]
fn a_picture_claiming_too_many_pixels_is_refused_by_its_header() {
    let mut header = 100_000_u32.to_be_bytes().to_vec();
    header.extend(100_000_u32.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    let png = [
        &b"\x89PNG\r\n\x1a\n"[..],
        &chunk(b"IHDR", &header),
        &chunk(b"IDAT", &[]),
        &chunk(b"IEND", &[]),
    ]
    .concat();
    assert_eq!(png.len(), 57);
    let dir = TempDir::new().expect("a directory");
    let refused = decode(&written(&dir, "huge.png", &png), 360, Fit::Cover);
    assert_eq!(
        refused,
        Err(Refused::TooBig {
            width: 100_000,
            height: 100_000
        })
    );
}

#[test]
fn an_empty_file_is_no_picture() {
    let dir = TempDir::new().expect("a directory");
    let path = written(&dir, "empty.jpg", b"");
    assert_eq!(decode(&path, 360, Fit::Cover), Err(Refused::Unreadable));
}

/// A file cut short, as a copy interrupted would leave it, is refused, and nothing panics.
#[test]
fn a_picture_cut_short_is_refused() {
    let dir = TempDir::new().expect("a directory");
    for (name, format) in [
        ("cut.png", ImageFormat::Png),
        ("cut.jpg", ImageFormat::Jpeg),
    ] {
        let whole = encoded(format);
        let path = written(&dir, name, &whole[..whole.len() / 2]);
        let refused = decode(&path, 360, Fit::Cover);
        assert!(
            matches!(refused, Err(Refused::Failed(_))),
            "{name}: {refused:?}"
        );
    }
    let missing = dir.path().join("gone.png");
    assert!(matches!(
        decode(&missing, 360, Fit::Cover),
        Err(Refused::Failed(_))
    ));
}
