//! Shared bounded image decoding and RGBA thumbnail resizing.

use super::Thumbnail;
use image::metadata::Orientation;

/// Most pixels an image may declare before its thumbnail is refused.
///
/// A crafted file can announce enormous dimensions in a few hundred bytes; the
/// decoder would then try to materialise them. This is checked against the
/// HEADER, so such a file costs nothing beyond the header read. The crate's own
/// allocation ceiling is documented as non-strict , some decoders ignore it ,
/// which is why the real bound is enforced here rather than delegated.
///
/// Eighty megapixels sits well above the largest consumer sensor of the day
/// (61 Mpx) and above most scans, and caps one decode near 320 MB. Above it the
/// entry falls back to its type icon, exactly as a format the crate cannot
/// decode already does. The thumbnail workers run in parallel, so the figure is
/// paid several times over , that is what makes the ceiling worth having.
const MAX_DECODE_PIXELS: u64 = 80_000_000;

/// Allocation ceiling handed to the decoder, aligned with the pixel budget
/// above (80 Mpx in RGBA8 needs ~320 MB, plus room for the decoder's own
/// scratch). Best-effort by design, hence the strict check beside it.
const MAX_DECODE_ALLOC: u64 = 384 * 1024 * 1024;
/// Limits handed to every decoder reading untrusted bytes.
fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    limits
}

/// Decodes what `reader` holds, refusing anything past [`MAX_DECODE_PIXELS`].
///
/// The dimensions come from the header, so an oversized image is turned away
/// before a single pixel is allocated for it.
pub(super) fn from_reader<R: std::io::BufRead + std::io::Seek>(
    reader: image::ImageReader<R>,
    max_px: u32,
) -> Option<Thumbnail> {
    from_reader_bounded(reader, max_px, MAX_DECODE_PIXELS)
}

fn from_reader_bounded<R: std::io::BufRead + std::io::Seek>(
    mut reader: image::ImageReader<R>,
    max_px: u32,
    max_pixels: u64,
) -> Option<Thumbnail> {
    use image::ImageDecoder;
    reader.limits(decode_limits());
    let mut decoder = reader.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > max_pixels {
        return None;
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    Some(downscale_oriented(
        image::DynamicImage::from_decoder(decoder).ok()?,
        max_px,
        orientation,
    ))
}

/// Display dimensions include mirrored quarter-turn orientations as well.
pub(super) fn oriented_dimensions(
    (width, height): (u32, u32),
    orientation: Orientation,
) -> (u32, u32) {
    match orientation {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270FlipH => (height, width),
        _ => (width, height),
    }
}

/// Decodes ENCODED image bytes (PNG/BMP/JPEG…) into a thumbnail bounded to
/// `max_px`. Used for renders produced outside `image` , e.g. a PDF page rendered
/// by the WinRT API on Windows (cf. `favnyr-gui/src/winthumb.rs`).
pub fn from_encoded(bytes: &[u8], max_px: u32) -> Option<Thumbnail> {
    from_encoded_bounded(bytes, max_px, MAX_DECODE_PIXELS)
}

pub(super) fn from_encoded_bounded(
    bytes: &[u8],
    max_px: u32,
    max_pixels: u64,
) -> Option<Thumbnail> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    from_reader_bounded(reader, max_px, max_pixels)
}

/// Shrinks `img` to fit within `max_px × max_px` (aspect preserved) and produces
/// RGBA8. `DynamicImage::thumbnail` is fast (suited to generating many
/// thumbnails).
pub(super) fn downscale(img: image::DynamicImage, max_px: u32) -> Thumbnail {
    downscale_oriented(img, max_px, Orientation::NoTransforms)
}

fn downscale_oriented(
    img: image::DynamicImage,
    max_px: u32,
    orientation: Orientation,
) -> Thumbnail {
    // A square bound is invariant under rotation. Transform the reduced image
    // to avoid allocating a second full-resolution camera image for quarter turns.
    let mut small = img.thumbnail(max_px, max_px);
    small.apply_orientation(orientation);
    let small = small.to_rgba8();
    Thumbnail {
        width: small.width(),
        height: small.height(),
        rgba: small.into_raw(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60,
        0x60, 0x60, 0x00, 0x00, 0x00, 0x04, 0x00, 0x01, 0xf6, 0x17, 0x38, 0x55, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    const OVERSIZED_PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0xea, 0x60, 0x00, 0x00, 0xea, 0x60, 0x08, 0x02, 0x00, 0x00, 0x00, 0x0f,
        0xb0, 0xe2, 0x15, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60,
        0x60, 0x60, 0x00, 0x00, 0x00, 0x04, 0x00, 0x01, 0xf6, 0x17, 0x38, 0x55, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn an_ordinary_image_still_decodes() {
        assert!(from_encoded(TINY_PNG, 256).is_some());
    }

    #[test]
    fn an_image_declaring_absurd_dimensions_is_refused() {
        // Sixty-nine bytes announcing 3.6 gigapixels. The refusal comes from
        // the header, so nothing is ever allocated for it , which is the whole
        // point: a decompression bomb costs the reader, not the memory.
        assert_eq!(OVERSIZED_PNG.len(), TINY_PNG.len());
        assert!(from_encoded(OVERSIZED_PNG, 256).is_none());
    }
}
