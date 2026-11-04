//! Registered document creation through the folder's native New menu.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::{
            Shell::{
                BHID_SFObject, CMF_SYNCCASCADEMENU, IContextMenu, IContextMenu2, IShellFolder,
                IShellItem, SHCreateItemFromParsingName,
            },
            WindowsAndMessaging::{
                CreatePopupMenu, GetMenuItemCount, GetMenuItemInfoW, MENUITEMINFOW, MIIM_ID,
                MIIM_SUBMENU, WM_INITMENUPOPUP,
            },
        },
    },
    core::{Interface, PCWSTR},
};

use super::imp::{ID_FIRST, ID_LAST, ShellEntry, ShellMenuSession, canonical_verb, walk_documents};

mod template;
pub use template::DocumentTemplate;

/// Owned display data, safe to deliver after the native apartment is released.
pub struct DocumentChoice {
    pub extension: String,
    pub label: String,
    pub icon: Option<(Vec<u8>, u32, u32)>,
}

/// Commands belong to one captured folder and one native menu. The apartment
/// outlives the COM objects, including when a command opens an application.
pub struct DocumentMenuSession {
    menu: ShellMenuSession,
    commands: Vec<u32>,
    _apartment: crate::winutil::ComApartment,
}

impl DocumentMenuSession {
    pub fn choices(&self, entries: &[ShellEntry]) -> Result<Vec<DocumentChoice>> {
        entries
            .iter()
            .map(|entry| {
                Ok(DocumentChoice {
                    extension: self.extension(entry.id)?,
                    label: entry.label.clone(),
                    icon: entry.icon.clone(),
                })
            })
            .collect()
    }

    pub fn extension(&self, command: u32) -> Result<String> {
        if !self.commands.contains(&command) {
            return Err(anyhow!("the document command is not part of this menu"));
        }
        Ok(unsafe { canonical_verb(&self.menu.icm, command) })
    }

    pub fn invoke(&self, command: u32) -> Result<()> {
        if !self.commands.contains(&command) {
            return Err(anyhow!("the document command is not part of this menu"));
        }
        super::imp::invoke(&self.menu, command)
    }
}

/// Windows supplies the registered labels, templates, collision handling and
/// application-specific creation rules. Only the New submenu is exposed.
pub fn build_for_folder(folder: &Path) -> Result<(DocumentMenuSession, Vec<ShellEntry>)> {
    let owner = crate::winmsg::self_hwnd()
        .map_or(HWND::default(), |raw| HWND(raw as *mut core::ffi::c_void));
    build_with_owner(folder, owner)
}

/// Discovery owns its apartment and never lends the UI window to shell handlers.
pub fn discover_documents(folder: &Path) -> Result<Vec<DocumentChoice>> {
    let (menu, entries) = build_with_owner(folder, HWND::default())?;
    menu.choices(&entries)
}

fn build_with_owner(folder: &Path, owner: HWND) -> Result<(DocumentMenuSession, Vec<ShellEntry>)> {
    let apartment = crate::winutil::ComApartment::sta()
        .ok_or_else(|| anyhow!("initializing the document menu apartment failed"))?;
    unsafe {
        let dir = crate::winutil::wide(&folder.to_string_lossy());
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(dir.as_ptr()), None)
            .context("opening the destination shell item")?;
        let folder: IShellFolder = item
            .BindToHandler(None, &BHID_SFObject)
            .context("binding the destination shell folder")?;
        let icm: IContextMenu = folder
            .CreateViewObject(owner)
            .context("creating the folder background menu")?;
        let hmenu = CreatePopupMenu()?;
        let session = ShellMenuSession { icm, hmenu, dir };
        session
            .icm
            .QueryContextMenu(hmenu, 0, ID_FIRST, ID_LAST, CMF_SYNCCASCADEMENU)
            .ok()
            .context("building the folder background menu")?;
        let icm2: Option<IContextMenu2> = session.icm.cast().ok();
        if let Some(handler) = &icm2 {
            let _ = handler.HandleMenuMsg(WM_INITMENUPOPUP, WPARAM(hmenu.0 as usize), LPARAM(0));
        }
        for position in 0..GetMenuItemCount(Some(hmenu)).max(0) as u32 {
            let mut info = MENUITEMINFOW {
                cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_ID | MIIM_SUBMENU,
                ..Default::default()
            };
            GetMenuItemInfoW(hmenu, position, true, &mut info)?;
            if !info.hSubMenu.is_invalid()
                && (ID_FIRST..=ID_LAST).contains(&info.wID)
                && canonical_verb(&session.icm, info.wID - ID_FIRST) == "new"
            {
                if let Some(handler) = &icm2 {
                    let _ = handler.HandleMenuMsg(
                        WM_INITMENUPOPUP,
                        WPARAM(info.hSubMenu.0 as usize),
                        LPARAM(position as isize),
                    );
                }
                let entries = walk_documents(info.hSubMenu, &session.icm, &icm2);
                let commands = entries.iter().map(|entry| entry.id).collect();
                return Ok((
                    DocumentMenuSession {
                        menu: session,
                        commands,
                        _apartment: apartment,
                    },
                    entries,
                ));
            }
        }
        Err(anyhow!("the folder has no native New menu"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestFolder;
    use std::collections::HashSet;

    pub(super) fn folder() -> TestFolder {
        TestFolder::new()
    }

    #[test]
    #[ignore = "queries installed Windows shell extensions"]
    fn background_document_discovery_returns_owned_data_without_creating_files() {
        let folder = folder();
        let path = folder.0.clone();
        let choices = std::thread::spawn(move || discover_documents(&path))
            .join()
            .unwrap()
            .unwrap();
        assert!(!choices.is_empty());
        assert!(choices.iter().all(|choice| !choice.label.is_empty()));
        for choice in choices {
            if let Some((pixels, width, height)) = choice.icon {
                assert_eq!(pixels.len(), width as usize * height as usize * 4);
            }
        }
        assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 0);
    }

    #[test]
    #[ignore = "requires Windows text and bitmap document registrations"]
    fn native_documents_create_distinct_files_without_overwriting() {
        let folder = folder();
        assert!(build_for_folder(&folder.0.join("folder02")).is_err());
        let (session, entries) = build_for_folder(&folder.0).unwrap();
        assert!(session.invoke(u32::MAX).is_err());
        for extension in [".txt", ".bmp"] {
            let entry = entries
                .iter()
                .find(|entry| unsafe { canonical_verb(&session.menu.icm, entry.id) == extension })
                .expect("the document type must be registered on this machine");
            let before: HashSet<_> = std::fs::read_dir(&folder.0)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            session.invoke(entry.id).unwrap();
            let mut added: Vec<_> = std::fs::read_dir(&folder.0)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| !before.contains(path))
                .collect();
            assert_eq!(added.len(), 1);
            let created = added.pop().unwrap();
            assert_eq!(
                created.extension().unwrap().to_str().unwrap(),
                &extension[1..]
            );
            std::fs::write(&created, b"File01").unwrap();
            session.invoke(entry.id).unwrap();
            assert_eq!(std::fs::read(&created).unwrap(), b"File01");
            assert_eq!(
                std::fs::read_dir(&folder.0).unwrap().count(),
                before.len() + 2
            );
        }
    }

    #[test]
    #[ignore = "requires a Windows text document registration"]
    fn native_document_recipe_resolves_or_preserves_native_fallback() {
        let folder = folder();
        let (session, entries) = build_for_folder(&folder.0).unwrap();
        let entry = entries
            .iter()
            .find(|entry| unsafe { canonical_verb(&session.menu.icm, entry.id) == ".txt" })
            .unwrap();
        let template =
            DocumentTemplate::for_extension(&session.extension(entry.id).unwrap()).unwrap();
        assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 0);
        let created = if let Some(template) = template {
            assert_eq!(template.extension, ".txt");
            let created = template.create(&folder.0, "File01.txt").unwrap();
            assert_eq!(created, folder.0.join("File01.txt"));
            created
        } else {
            // Some Windows installations expose types whose creation recipes
            // are not available through the documented ShellNew registry keys.
            session.invoke(entry.id).unwrap();
            std::fs::read_dir(&folder.0)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path()
        };
        std::fs::write(&created, b"File01").unwrap();
        if let Some(template) =
            DocumentTemplate::for_extension(&session.extension(entry.id).unwrap()).unwrap()
        {
            assert!(template.create(&folder.0, "File01.txt").is_err());
        }
        assert_eq!(std::fs::read(&created).unwrap(), b"File01");
        assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 1);
        assert!(session.extension(u32::MAX).is_err());
    }
}
