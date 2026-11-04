//! On-demand thumbnails, with format-specific generators in private modules.
//!
//! Images, embedded document previews, audio cover art and supported meshes
//! produce RGBA pixels without depending on the GUI. Optional local tools
//! provide video/PDF previews and decompression of selected 3D documents.
//! The GUI selects native Windows providers for video and PDF, supplies
//! platform fallbacks, renders SVG with Slint, and owns scheduling and caching.

use std::path::Path;

use crate::fs::FileKind;

mod affinity;
mod audio;
mod decode;
mod gltf;
mod images;
mod mesh;
mod model;
mod obj;
mod pdf;
mod psd;
mod stl;
mod tools;
mod video;

#[cfg(test)]
mod test_support;

pub use audio::from_audio;
pub use decode::from_encoded;
pub use images::{from_image, image_meta, is_image_preview_extension};
pub use pdf::from_pdf;
pub use tools::SEVEN_ZIP_PROGRAMS;
pub use video::from_video;

/// Extensions whose generic-file rows can request a model thumbnail.
/// Shared with the GUI so preview scheduling and row geometry stay aligned.
pub fn is_model_preview_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case("3mf")
        || extension.eq_ignore_ascii_case("stl")
        || extension.eq_ignore_ascii_case("obj")
        || extension.eq_ignore_ascii_case("glb")
        || extension.eq_ignore_ascii_case("gltf")
        || model::is_blend_extension(extension)
}

/// Decoded thumbnail ready for display: interleaved RGBA8 (`rgba.len() ==
/// width * height * 4`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Generates a thumbnail for `path` based on its type. `max_px` bounds the
/// largest side (aspect is preserved). Returns `None` if the type has no
/// preview or if generation fails (→ icon fallback on the caller's side).
pub fn generate(path: &Path, kind: FileKind, max_px: u32) -> Option<Thumbnail> {
    match kind {
        FileKind::Image => from_image(path, max_px),
        FileKind::Audio => from_audio(path, max_px),
        FileKind::Video => from_video(path, max_px),
        // PDFs are classified as `Document`; `from_pdf` filters on the extension
        // (other documents , doc/txt/… , have no preview).
        FileKind::Document => from_pdf(path, max_px),
        FileKind::File => {
            let extension = path.extension()?.to_str()?;
            if extension.eq_ignore_ascii_case("stl") {
                stl::from_stl(path, max_px)
            } else if extension.eq_ignore_ascii_case("obj") {
                obj::from_obj(path, max_px)
            } else if extension.eq_ignore_ascii_case("glb")
                || extension.eq_ignore_ascii_case("gltf")
            {
                gltf::from_gltf(path, max_px)
            } else {
                model::from_model_preview(path, max_px)
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::tempdir;
    use super::*;

    #[test]
    fn generate_dispatches_by_kind() {
        let dir = tempdir();
        let path = dir.join("p.png");
        image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 255, 0, 255]))
            .save(&path)
            .unwrap();
        // Image → thumbnail; other types → None (no preview).
        assert!(generate(&path, FileKind::Image, 16).is_some());
        assert!(generate(&path, FileKind::Document, 16).is_none());
        assert!(generate(&path, FileKind::Folder, 16).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
