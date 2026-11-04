//! Raster image previews and inexpensive header metadata.

use std::path::Path;

use super::Thumbnail;
use super::affinity::{from_affinity_thumbnail, is_affinity_extension};
use super::decode::{from_reader, oriented_dimensions};
use super::psd::from_psd_thumbnail;

/// Whether an extension has an in-process raster preview reader. This only
/// inspects compiled capabilities; it never opens the file or probes the OS.
pub fn is_image_preview_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case("psd")
        || is_affinity_extension(extension)
        || image::ImageFormat::from_extension(extension)
            .is_some_and(|format| format.reading_enabled())
}

/// Metadata of an image read from the HEADER only (no pixel decoding):
/// `(width, height, color bits per pixel, alpha present)`. When an
/// alpha channel exists, its depth is subtracted from the third element: an
/// RGBA8 image therefore returns `(width, height, 24, true)`. `None` if the
/// format isn't handled by `image` or the file is unreadable. Fast
/// (a few KB read) → suited to filling columns on the fly.
pub fn image_meta(path: &Path) -> Option<(u32, u32, u16, bool)> {
    use image::ImageDecoder;
    let mut decoder = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let (w, h) = oriented_dimensions(decoder.dimensions(), orientation);
    let color = decoder.color_type();
    let has_alpha = color.has_alpha();
    let total_bits = color.bits_per_pixel();
    // The ColorType variants exposed by `image` have channels of equal depth
    // (RGBA8, RGBA16, RGBA32F, LA8, LA16). Removing one channel therefore gives
    // the correct color depth, without counting alpha twice in the display.
    let color_bits = if has_alpha {
        total_bits - total_bits / u16::from(color.channel_count())
    } else {
        total_bits
    };
    Some((w, h, color_bits, has_alpha))
}

/// Thumbnail of an image file (pure-Rust decoding). `None` if the format
/// isn't supported / the file is unreadable.
pub fn from_image(path: &Path, max_px: u32) -> Option<Thumbnail> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    if extension.eq_ignore_ascii_case("psd") {
        return from_psd_thumbnail(path, max_px);
    }
    if is_affinity_extension(extension) {
        return from_affinity_thumbnail(path, max_px);
    }

    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    from_reader(reader, max_px)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::tempdir;
    use super::*;

    #[test]
    fn preview_extensions_follow_compiled_readers_and_embedded_previews() {
        for extension in [
            "png", "JPG", "jpeg", "gif", "webp", "bmp", "tiff", "tga", "hdr", "ico", "PSD", "af",
            "afphoto", "afdesign", "afpub",
        ] {
            assert!(is_image_preview_extension(extension), "{extension}");
        }
        for extension in ["xcf", "heic", "raw", "svg", "txt", ""] {
            assert!(!is_image_preview_extension(extension), "{extension}");
        }
        for format in image::ImageFormat::all() {
            for extension in format.extensions_str() {
                assert_eq!(
                    is_image_preview_extension(extension),
                    format.reading_enabled(),
                    "{extension}"
                );
            }
        }
    }

    #[test]
    fn exif_orientations_agree_for_pixels_metadata_and_encoded_previews() {
        use super::super::{from_encoded, test_support::oriented_image};
        let dir = tempdir();
        // Pixel indices for all eight EXIF transforms, independently specified.
        let orders = [
            [0, 1, 2, 3, 4, 5],
            [2, 1, 0, 5, 4, 3],
            [5, 4, 3, 2, 1, 0],
            [3, 4, 5, 0, 1, 2],
            [0, 3, 1, 4, 2, 5],
            [3, 0, 4, 1, 5, 2],
            [5, 2, 4, 1, 3, 0],
            [2, 5, 1, 4, 0, 3],
        ];
        for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg] {
            let baseline = from_encoded(&oriented_image(format, 1), 3).unwrap();
            for orientation in 1..=8 {
                let bytes = oriented_image(format, orientation);
                let path = dir.join("my_file.bin");
                std::fs::write(&path, &bytes).unwrap();
                let expected_dimensions = if orientation >= 5 { (2, 3) } else { (3, 2) };
                let meta = image_meta(&path).unwrap();
                assert_eq!(
                    (meta.0, meta.1),
                    expected_dimensions,
                    "{format:?} {orientation}"
                );
                let thumb = from_image(&path, 3).unwrap();
                assert_eq!((thumb.width, thumb.height), expected_dimensions);
                let expected: Vec<u8> = orders[usize::from(orientation - 1)]
                    .iter()
                    .flat_map(|index| baseline.rgba[index * 4..index * 4 + 4].iter().copied())
                    .collect();
                assert_eq!(thumb.rgba, expected, "{format:?} {orientation}");
                assert_eq!(from_encoded(&bytes, 3).unwrap(), thumb);
                let reduced = from_encoded(&bytes, 2).unwrap();
                assert_eq!(
                    (reduced.width, reduced.height),
                    if orientation >= 5 { (1, 2) } else { (2, 1) }
                );
                assert_eq!(std::fs::read(&path).unwrap(), bytes);
            }
            for orientation in [0, 9, 255] {
                assert_eq!(
                    from_encoded(&oriented_image(format, orientation), 3).unwrap(),
                    baseline
                );
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn image_thumbnail_preserves_aspect_and_bounds() {
        let dir = tempdir();
        let path = dir.join("red.png");
        // 100×50 opaque red, encoded as PNG via `image`.
        let buf = image::RgbaImage::from_pixel(100, 50, image::Rgba([255, 0, 0, 255]));
        buf.save(&path).unwrap();

        let thumb = from_image(&path, 32).expect("decode");
        // 2:1 aspect preserved, bounded to 32 → 32×16.
        assert!(thumb.width <= 32 && thumb.height <= 32);
        assert_eq!(thumb.width, 32);
        assert_eq!(thumb.height, 16);
        assert_eq!(thumb.rgba.len() as u32, thumb.width * thumb.height * 4);
        // Red pixel preserved.
        assert_eq!(&thumb.rgba[0..4], &[255, 0, 0, 255]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn image_meta_reads_dimensions_and_depth() {
        let dir = tempdir();
        let path = dir.join("rgba.png");
        // 64×32 RGBA8: 24 color bits + one separate alpha channel.
        image::RgbaImage::from_pixel(64, 32, image::Rgba([1, 2, 3, 4]))
            .save(&path)
            .unwrap();
        let (w, h, bits, alpha) = image_meta(&path).expect("metadata readable");
        assert_eq!((w, h), (64, 32));
        assert_eq!(bits, 24);
        assert!(alpha);
        // A non-image file → None.
        let txt = dir.join("x.txt");
        std::fs::write(&txt, b"nope").unwrap();
        assert!(image_meta(&txt).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unsupported_path_returns_none() {
        let dir = tempdir();
        let path = dir.join("not-an-image.txt");
        std::fs::write(&path, b"hello").unwrap();
        assert!(from_image(&path, 32).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
