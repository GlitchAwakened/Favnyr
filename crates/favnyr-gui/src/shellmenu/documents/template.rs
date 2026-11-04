//! Self-contained ShellNew recipes, resolved only for the chosen document type.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, anyhow};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_PATH_NOT_FOUND};
use windows::Win32::Storage::FileSystem::MoveFileW;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::Registry::{
    HKEY_CLASSES_ROOT, REG_BINARY, REG_EXPAND_SZ, REG_ROUTINE_FLAGS, REG_SZ, REG_VALUE_TYPE,
    RRF_NOEXPAND, RRF_RT_ANY, RegGetValueW,
};
use windows::Win32::UI::Shell::{
    FOLDERID_CommonTemplates, FOLDERID_Templates, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath,
};
use windows::core::PCWSTR;

use crate::winutil::wide;

const MAX_VALUE_BYTES: u32 = 4 * 1024 * 1024;

pub struct DocumentTemplate {
    pub extension: String,
    content: Content,
}

enum Content {
    Empty,
    Data(Vec<u8>),
    File(String),
}

impl DocumentTemplate {
    /// Reads the current registration without constructing a folder's native menu.
    pub fn for_extension(extension: &str) -> Result<Option<Self>> {
        for_extension(extension)
    }

    /// The target is published only after its content is complete. A competing
    /// file, directory or broken link is never replaced, even after validation.
    pub fn create(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        favnyr_core::fs::ops::check_file_name(name)
            .map_err(|reason| anyhow!("invalid document name: {reason:?}"))?;
        let target = parent.join(name);
        let (mut staging, mut output) = Staging::create(parent)?;
        match &self.content {
            Content::Empty => {}
            Content::Data(data) => output.write_all(data)?,
            Content::File(template) => {
                let path = resolve_template(template)?;
                let mut input = File::open(path)?;
                if !input.metadata()?.is_file() {
                    return Err(anyhow!("the document template is not a file"));
                }
                io::copy(&mut input, &mut output)?;
            }
        }
        output.flush()?;
        drop(output);
        let source = wide(&staging.0.as_ref().unwrap().to_string_lossy());
        let destination = wide(&target.to_string_lossy());
        // MoveFile does not have MoveFileEx's replacement behavior. The staging
        // file is a sibling, so publishing stays on the same filesystem and
        // inherits the destination folder's file permissions directly.
        unsafe { MoveFileW(PCWSTR(source.as_ptr()), PCWSTR(destination.as_ptr())) }?;
        staging.0 = None;
        Ok(target)
    }
}

struct Staging(Option<PathBuf>);

impl Staging {
    fn create(parent: &Path) -> io::Result<(Self, File)> {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        for _ in 0..100 {
            let path = parent.join(format!(
                ".favnyr-new-{}-{}.tmp",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((Self(Some(path)), file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other("cannot allocate document staging file"))
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if let Some(path) = &self.0
            && let Err(error) = std::fs::remove_file(path)
        {
            tracing::warn!(%error, "document staging cleanup failed");
        }
    }
}

struct Value {
    kind: REG_VALUE_TYPE,
    bytes: Vec<u8>,
}

impl Value {
    fn string(&self) -> Result<String> {
        if !matches!(self.kind, REG_SZ | REG_EXPAND_SZ) || !self.bytes.len().is_multiple_of(2) {
            return Err(anyhow!("the document registration has an invalid string"));
        }
        let utf16: Vec<_> = self
            .bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let end = utf16
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(utf16.len());
        let text = String::from_utf16(&utf16[..end])?;
        if self.kind != REG_EXPAND_SZ {
            return Ok(text);
        }
        let value = wide(&text);
        let size = unsafe { ExpandEnvironmentStringsW(PCWSTR(value.as_ptr()), None) };
        if size == 0 || size > 32_768 {
            return Err(anyhow!("the document registration cannot be expanded"));
        }
        let mut expanded = vec![0u16; size as usize];
        let written =
            unsafe { ExpandEnvironmentStringsW(PCWSTR(value.as_ptr()), Some(&mut expanded)) };
        if written == 0 || written > size {
            return Err(anyhow!("the expanded document registration changed size"));
        }
        Ok(String::from_utf16(
            &expanded[..written.saturating_sub(1) as usize],
        )?)
    }
}

fn value(key: &str, name: &str) -> Result<Option<Value>> {
    let key = wide(key);
    let name = wide(name);
    let flags: REG_ROUTINE_FLAGS = RRF_RT_ANY | RRF_NOEXPAND;
    let mut size = 0;
    let status = unsafe {
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            flags,
            None,
            None,
            Some(&mut size),
        )
    };
    if matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
        return Ok(None);
    }
    status.ok()?;
    if size > MAX_VALUE_BYTES {
        return Err(anyhow!("the document registration exceeds the data limit"));
    }
    let mut bytes = vec![0; size as usize + 2];
    let mut kind = REG_VALUE_TYPE::default();
    let status = unsafe {
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            flags,
            Some(&mut kind),
            Some(bytes.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status == ERROR_MORE_DATA {
        return Err(anyhow!(
            "the document registration changed while being read"
        ));
    }
    status.ok()?;
    bytes.truncate(size as usize);
    Ok(Some(Value { kind, bytes }))
}

/// Resolve only known recipes. Unknown handlers, commands and folder recipes
/// stay under Windows' control rather than becoming an arbitrary empty file.
pub(super) fn for_extension(extension: &str) -> Result<Option<DocumentTemplate>> {
    resolve_extension(extension, value)
}

fn resolve_extension(
    extension: &str,
    mut read: impl FnMut(&str, &str) -> Result<Option<Value>>,
) -> Result<Option<DocumentTemplate>> {
    if !extension.starts_with('.')
        || extension.len() < 2
        || favnyr_core::fs::ops::check_file_name(&format!("File01{extension}")).is_err()
    {
        return Ok(None);
    }
    let mut keys = vec![format!("{extension}\\ShellNew")];
    if let Some(progid) = read(extension, "")? {
        let progid = progid.string()?;
        if !progid.is_empty() && !progid.contains(['\\', '/', '\0']) {
            keys.push(format!("{extension}\\{progid}\\ShellNew"));
            keys.push(format!("{progid}\\ShellNew"));
        }
    }
    for key in keys {
        if read(&key, "Command")?.is_some()
            || read(&key, "Handler")?.is_some()
            || read(&key, "Directory")?.is_some()
            || read(&format!("{key}\\Config"), "NoExtension")?.is_some()
        {
            return Ok(None);
        }
        match recipe(|name| read(&key, name))? {
            Recipe::Content(content) => {
                return Ok(Some(DocumentTemplate {
                    extension: extension.to_owned(),
                    content,
                }));
            }
            Recipe::Native => return Ok(None),
            Recipe::Missing => {}
        }
    }
    // Plain text can use the built-in empty-file creation when Windows does
    // not publish a recipe. An explicit unsupported recipe always wins above.
    Ok(extension
        .eq_ignore_ascii_case(".txt")
        .then(|| DocumentTemplate {
            extension: extension.to_owned(),
            content: Content::Empty,
        }))
}

enum Recipe {
    Missing,
    Native,
    Content(Content),
}

fn recipe(mut read: impl FnMut(&str) -> Result<Option<Value>>) -> Result<Recipe> {
    if read("NullFile")?.is_some() {
        return Ok(Recipe::Content(Content::Empty));
    }
    if let Some(value) = read("FileName")? {
        let filename = value.string()?;
        return if filename.is_empty() {
            Ok(Recipe::Native)
        } else {
            Ok(Recipe::Content(Content::File(filename)))
        };
    }
    if let Some(value) = read("Data")? {
        if value.kind != REG_BINARY {
            return Ok(Recipe::Native);
        }
        return Ok(Recipe::Content(Content::Data(value.bytes)));
    }
    Ok(Recipe::Missing)
}

fn resolve_template(name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    if path.is_absolute() {
        return Ok(path.to_owned());
    }
    // Reject drive-relative names and parent traversal before looking in the
    // template locations documented by Windows. Never use the process cwd.
    if path.components().any(|part| {
        !matches!(
            part,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    }) {
        return Err(anyhow!("the document template path is not fully qualified"));
    }
    for id in [&FOLDERID_CommonTemplates, &FOLDERID_Templates] {
        if let Ok(folder) = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None) } {
            let root = unsafe { folder.to_string() };
            unsafe { CoTaskMemFree(Some(folder.0.cast())) };
            if let Ok(root) = root {
                let candidate = Path::new(&root).join(path);
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
    }
    if let Some(root) = std::env::var_os("SystemRoot") {
        let candidate = Path::new(&root).join("ShellNew").join(path);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "the registered document template was not found",
    )
    .into())
}

#[cfg(test)]
mod tests {
    use super::super::tests::folder;
    use super::*;

    fn string(text: &str) -> Value {
        Value {
            kind: REG_SZ,
            bytes: wide(text).into_iter().flat_map(u16::to_le_bytes).collect(),
        }
    }

    #[test]
    fn documented_recipe_precedence_keeps_empty_templates_and_binary_data_distinct() {
        let resolve = |values: Vec<(&str, Value)>| {
            let mut values: std::collections::HashMap<_, _> = values.into_iter().collect();
            recipe(|name| Ok(values.remove(name))).unwrap()
        };
        assert!(matches!(
            resolve(vec![
                ("NullFile", string("")),
                ("FileName", string("File01.bin")),
                (
                    "Data",
                    Value {
                        kind: REG_BINARY,
                        bytes: vec![1, 2]
                    }
                ),
            ]),
            Recipe::Content(Content::Empty)
        ));
        assert!(matches!(resolve(vec![
            ("FileName", string("File01.bin")),
            ("Data", Value { kind: REG_BINARY, bytes: vec![1, 2] }),
        ]), Recipe::Content(Content::File(name)) if name == "File01.bin"));
        assert!(matches!(resolve(vec![
            ("Data", Value { kind: REG_BINARY, bytes: vec![0, 1, 255] }),
        ]), Recipe::Content(Content::Data(bytes)) if bytes == [0, 1, 255]));
        assert!(matches!(
            resolve(vec![("Data", string("File01"))]),
            Recipe::Native
        ));
        assert!(matches!(resolve(vec![]), Recipe::Missing));
    }

    #[test]
    fn plain_text_uses_the_file_dialog_without_overriding_registered_creation_rules() {
        let folder = folder();
        let fallback = resolve_extension(".txt", |_, _| Ok(None)).unwrap().unwrap();
        let created = fallback.create(&folder.0, "File01.txt").unwrap();
        assert!(std::fs::read(created).unwrap().is_empty());
        assert!(
            resolve_extension(".bin", |_, _| Ok(None))
                .unwrap()
                .is_none()
        );
        for field in ["Command", "Handler", "Directory", "FileName", "Data"] {
            let native = resolve_extension(".txt", |key, name| {
                Ok((key == ".txt\\ShellNew" && name == field).then(|| string("")))
            })
            .unwrap();
            assert!(
                native.is_none(),
                "an explicit {field} rule must not become an empty text file"
            );
        }
        let registered = resolve_extension(".txt", |key, name| {
            Ok((key == ".txt\\ShellNew" && name == "Data").then(|| Value {
                kind: REG_BINARY,
                bytes: b"File01".to_vec(),
            }))
        })
        .unwrap()
        .unwrap();
        let created = registered.create(&folder.0, "File02.txt").unwrap();
        assert_eq!(std::fs::read(created).unwrap(), b"File01");
    }

    #[test]
    fn a_named_document_keeps_template_content_and_never_replaces_an_entry() {
        let folder = folder();
        let template_path = folder.0.join("File01.bin");
        std::fs::write(&template_path, b"File01\0\xFF").unwrap();
        let template = DocumentTemplate {
            extension: ".bin".into(),
            content: Content::File(template_path.display().to_string()),
        };
        let created = template.create(&folder.0, "File02.bin").unwrap();
        assert_eq!(std::fs::read(&created).unwrap(), b"File01\0\xFF");
        assert!(template.create(&folder.0, "File02.bin").is_err());
        assert_eq!(std::fs::read(&created).unwrap(), b"File01\0\xFF");
        std::fs::create_dir(folder.0.join("folder01")).unwrap();
        assert!(template.create(&folder.0, "folder01").is_err());
        assert!(folder.0.join("folder01").is_dir());
        assert!(template.create(&folder.0, "../File03.bin").is_err());
        assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 3);
    }

    #[test]
    fn inline_data_is_preserved_and_a_missing_template_leaves_no_partial_file() {
        let folder = folder();
        let template = DocumentTemplate {
            extension: ".bin".into(),
            content: Content::Data(vec![0, 2, 255]),
        };
        let created = template.create(&folder.0, "File01.bin").unwrap();
        assert_eq!(std::fs::read(created).unwrap(), [0, 2, 255]);
        let missing = DocumentTemplate {
            extension: ".bin".into(),
            content: Content::File(folder.0.join("File03.bin").display().to_string()),
        };
        assert!(missing.create(&folder.0, "File02.bin").is_err());
        assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 1);
        let invalid = Value {
            kind: REG_SZ,
            bytes: vec![1],
        };
        assert!(invalid.string().is_err());
    }

    #[test]
    fn ambiguous_template_paths_do_not_search_the_working_directory() {
        for name in ["../File01.bin", "C:File01.bin", "\\File01.bin"] {
            assert!(resolve_template(name).is_err());
        }
        for extension in ["", "txt", ".", ".txt\\folder01", ".txt:File01"] {
            assert!(for_extension(extension).unwrap().is_none());
        }
    }
}
