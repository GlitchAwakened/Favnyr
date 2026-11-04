//! Shared synthetic image fixtures for thumbnail unit tests.

pub(super) fn tempdir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "favnyr-thumb-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub(super) fn encoded_cover(format: image::ImageFormat, rgba: [u8; 4]) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(20, 10, image::Rgba(rgba));
    let mut cursor = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut cursor, format)
        .unwrap();
    cursor.into_inner()
}

/// A six-pixel asymmetric image with a single TIFF orientation entry.
pub(super) fn oriented_image(format: image::ImageFormat, orientation: u8) -> Vec<u8> {
    use image::ImageEncoder;
    fn write(encoder: impl ImageEncoder, exif: Vec<u8>) {
        let mut encoder = encoder;
        encoder.set_exif_metadata(exif).unwrap();
        encoder
            .write_image(
                &[
                    20, 20, 20, 50, 50, 50, 80, 80, 80, 110, 110, 110, 140, 140, 140, 170, 170, 170,
                ],
                3,
                2,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
    }
    let exif = vec![
        b'I',
        b'I',
        42,
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
    let mut bytes = Vec::new();
    match format {
        image::ImageFormat::Png => write(image::codecs::png::PngEncoder::new(&mut bytes), exif),
        image::ImageFormat::Jpeg => write(
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 100),
            exif,
        ),
        _ => panic!("unsupported fixture format"),
    }
    bytes
}
