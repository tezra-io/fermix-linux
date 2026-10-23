//! The chat's pictures decoded for drawing, in pure Rust: they come from phones and the web, so no
//! C decoder reads them. The format is known by the file's bytes, never its name; the header is
//! read before any pixel, and a picture past `media::PICTURE_PIXELS` is refused without
//! allocating; a photo is turned the way its EXIF says; and the result is straight RGBA scaled to
//! what is drawn. No GTK: the app makes a texture of the bytes.

use crate::media::{decode_size, within_ceiling, Fit};
use image::{DynamicImage, ImageDecoder, ImageError, ImageReader, Limits};
use std::fmt;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// The most one decode may allocate, a second guard behind the area check.
const MAX_ALLOC: u64 = 512 * 1024 * 1024;

/// Why a picture was not decoded: it shows as its icon, and opens in the computer's own app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// No decoder here reads it (HEIC, AVIF, TIFF), or it is no picture at all.
    Unreadable,
    /// Its header claims more than `PICTURE_PIXELS`.
    TooBig { width: u32, height: u32 },
    /// It could not be read: missing, cut short, or malformed.
    Failed(String),
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refused::Unreadable => write!(f, "no decoder here reads it"),
            Refused::TooBig { width, height } => write!(f, "{width}x{height} is too big"),
            Refused::Failed(why) => write!(f, "{why}"),
        }
    }
}

/// A decoded picture: straight (not premultiplied) RGBA, four bytes a pixel, rows packed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Decodes `path` for a square of `side` pixels (`media::decode_size`). Blocking, and as costly
/// as the picture is big: a worker thread's.
pub fn decode(path: &Path, side: u32, fit: Fit) -> Result<Decoded, Refused> {
    let (width, height) = reader(path)?.into_dimensions().map_err(refused)?;
    if !within_ceiling(width, height) {
        return Err(Refused::TooBig { width, height });
    }
    let upright = upright(path)?;
    let (w, h) = decode_size(upright.width(), upright.height(), side, fit);
    let scaled = if (w, h) == (upright.width(), upright.height()) {
        upright
    } else {
        upright.thumbnail_exact(w, h)
    };
    let rgba = scaled.into_rgba8();
    Ok(Decoded {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

/// A reader of `path` whose format its first bytes name; none they name is `Unreadable`.
fn reader(path: &Path) -> Result<ImageReader<BufReader<File>>, Refused> {
    let file = File::open(path).map_err(|e| Refused::Failed(e.to_string()))?;
    let reader = ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .map_err(|e| Refused::Failed(e.to_string()))?;
    if reader.format().is_none() {
        return Err(Refused::Unreadable);
    }
    Ok(reader)
}

/// Every pixel, under the allocation guard, turned the way the picture's EXIF says.
fn upright(path: &Path) -> Result<DynamicImage, Refused> {
    let mut reader = reader(path)?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(refused)?;
    let orientation = decoder.orientation().map_err(refused)?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(refused)?;
    image.apply_orientation(orientation);
    Ok(image)
}

fn refused(e: ImageError) -> Refused {
    match e {
        ImageError::Unsupported(_) => Refused::Unreadable,
        e => Refused::Failed(e.to_string()),
    }
}
