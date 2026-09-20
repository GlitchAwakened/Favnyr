//! Small shared Windows utilities (avoids duplication between `openwith`,
//! `winthumb`, `clipboard`) — keeps the implementations cohesive.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::Win32::Storage::FileSystem::GetLongPathNameW;
use windows::core::PCWSTR;

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, GetDC, GetDIBits, GetObjectW,
    HBITMAP, HGDIOBJ, ReleaseDC,
};

/// NUL-terminated wide (UTF-16) string for the Win32 `*W` APIs.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// RGBA pixels `(buf, w, h)` of a 32 bpp `HBITMAP` via GDI (`GetObjectW` +
/// `GetDIBits`), converted BGRA→RGBA, alpha forced opaque if the bitmap has no
/// alpha channel (old icons). `None` if dimensions are zero / extraction
/// failed. **Does NOT free** the `HBITMAP` (caller's responsibility).
///
/// # Safety
/// `hbitmap` must be a valid GDI bitmap handle.
pub unsafe fn hbitmap_to_rgba(hbitmap: HBITMAP) -> Option<(Vec<u8>, u32, u32)> {
    unsafe {
        let mut bm = BITMAP::default();
        GetObjectW(
            HGDIOBJ(hbitmap.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );
        let (w, h) = (bm.bmWidth.max(0) as u32, bm.bmHeight.max(0) as u32);
        if w == 0 || h == 0 {
            return None;
        }
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w as i32;
        bi.bmiHeader.biHeight = -(h as i32); // top-down
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB.0;
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let hdc = GetDC(None);
        let lines = GetDIBits(
            hdc,
            hbitmap,
            0,
            h,
            Some(buf.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, hdc);
        if lines == 0 {
            return None;
        }
        // BGRA → RGBA.
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        // Bitmap with no alpha channel → force opaque (otherwise fully transparent).
        if !buf.chunks_exact(4).any(|px| px[3] != 0) {
            for px in buf.chunks_exact_mut(4) {
                px[3] = 255;
            }
        }
        Some((buf, w, h))
    }
}

/// The path with its 8.3 short components replaced by the real names.
///
/// `%TEMP%` commonly expands to a shortened form when the account name is long
/// — `C:\Users\SOMEO~1.NAM\…` — where Explorer shows the full one. Matching
/// the system's spelling is not only about looks: a folder reached under two
/// spellings is two folders to anything that keys on its path, and a short name
/// is not something a case rule can reconcile.
///
/// This asks the filesystem, so the caller must only reach for it when a
/// component actually looks mangled — see [`has_short_component`] — and never
/// on a network path, whose round trip would be paid on the UI thread.
///
/// `None` when nothing can be resolved, which includes a path that does not
/// exist: the caller then keeps what it had, and that stays perfectly usable.
pub fn long_path(path: &Path) -> Option<PathBuf> {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // First call sizes the buffer, second fills it — the usual Win32 shape.
    let needed = unsafe { GetLongPathNameW(PCWSTR(wide.as_ptr()), None) };
    if needed == 0 {
        return None;
    }
    let mut buf = vec![0u16; needed as usize];
    let written = unsafe { GetLongPathNameW(PCWSTR(wide.as_ptr()), Some(&mut buf)) };
    if written == 0 || written as usize > buf.len() {
        return None;
    }
    Some(PathBuf::from(OsString::from_wide(&buf[..written as usize])))
}

/// Does any component look like an 8.3 short name?
///
/// The mangling Windows applies is a tilde followed by a digit, as in
/// `SOMEO~1.NAM`. Testing for it costs nothing and is what keeps the
/// filesystem query off every ordinary navigation — a tilde alone is a
/// perfectly common character in a backup name, hence the digit.
pub fn has_short_component(path: &Path) -> bool {
    path.components().any(|component| {
        let text = component.as_os_str().to_string_lossy();
        text.as_bytes()
            .windows(2)
            .any(|pair| pair[0] == b'~' && pair[1].is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An already-full path must come back untouched: the resolution is meant
    /// to normalise, never to rewrite what is already right.
    #[test]
    fn a_path_already_spelled_in_full_survives_the_round_trip() {
        let dir = std::env::current_dir().expect("current dir");
        assert!(
            !has_short_component(&dir),
            "the test only means something on a full path"
        );
        assert_eq!(long_path(&dir).as_deref(), Some(dir.as_path()));
    }

    /// The whole point, expressed on the one variable known to answer short.
    /// Skipped where it does not — a short account name, or 8.3 turned off on
    /// the volume — rather than asserting something the machine cannot show.
    #[test]
    fn the_temp_variable_is_brought_back_to_its_full_spelling() {
        let Ok(temp) = std::env::var("TEMP") else {
            return;
        };
        let temp = PathBuf::from(temp);
        if !has_short_component(&temp) {
            return;
        }
        let long = long_path(&temp).expect("an existing path resolves");
        assert!(
            !has_short_component(&long),
            "no mangled component is left: {}",
            long.display()
        );
    }

    #[test]
    fn only_a_tilde_followed_by_a_digit_reads_as_a_short_name() {
        assert!(has_short_component(Path::new(
            r"C:\Users\SOMEO~1.NAM\AppData"
        )));
        assert!(has_short_component(Path::new(r"C:\PROGRA~2")));
        // A tilde is an ordinary character in a name, and on its own it says
        // nothing: querying the filesystem for those would be pure waste.
        assert!(!has_short_component(Path::new(
            r"C:\Users\someone\backup~\notes.txt"
        )));
        assert!(!has_short_component(Path::new(
            r"C:\Users\someone\Documents"
        )));
    }
}
