use std::fs::File;
use std::io::Read;
use std::path::Path;

use super::{MAX_THEME_BYTES, ThemePalettes};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    pub errors: Vec<String>,
    /// Reuses the selected file's validation result instead of reading it twice.
    pub selected: Option<Result<ThemePalettes, String>>,
}

/// A persisted identifier can only address a direct, portable filename.
pub fn user_file_name(id: &str) -> Option<&str> {
    let name = id.strip_prefix("user:")?;
    if name.len() <= 5
        || !name.ends_with(".toml")
        || name.starts_with('.')
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
    {
        return None;
    }
    Some(name)
}

/// Is `id` one a stored choice may name, a built-in family or a theme file?
/// A value answering neither, which only a hand-edited file can hold, is read
/// as no choice at all rather than as an error, so the file stays loadable.
pub fn id_is_known(id: &str) -> bool {
    super::BuiltinTheme::from_id(Some(id)).is_some() || user_file_name(id).is_some()
}

pub fn load(dir: &Path, id: &str) -> Result<ThemePalettes, String> {
    let name = user_file_name(id).ok_or("invalid theme identifier")?;
    let path = dir.join(name);
    let metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_THEME_BYTES as u64 {
        return Err("theme must be an ordinary file within the size limit".into());
    }
    let file = File::open(&path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_THEME_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    super::parse(text)
}

pub fn discover(dir: &Path, selected: Option<&str>) -> Catalog {
    let selected = selected.filter(|id| super::BuiltinTheme::from_id(Some(id)).is_none());
    let mut catalog = Catalog::default();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => Some(entries),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            catalog.errors.push(e.to_string());
            None
        }
    };
    for entry in entries.into_iter().flatten() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                catalog.errors.push(e.to_string());
                continue;
            }
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let id = format!("user:{name}");
        if user_file_name(&id).is_none() {
            continue;
        }
        let result = load(dir, &id);
        match &result {
            Ok(_) => catalog.entries.push(Entry {
                id: id.clone(),
                name: name.strip_suffix(".toml").unwrap_or(name).to_owned(),
            }),
            Err(e) => catalog.errors.push(format!("{name}: {e}")),
        }
        if selected == Some(id.as_str()) {
            catalog.selected = Some(result);
        }
    }
    if catalog.selected.is_none() {
        catalog.selected = selected.map(|id| load(dir, id));
    }
    catalog.entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    catalog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_cannot_escape_the_theme_directory() {
        for id in [
            "user:../my_theme.toml",
            "user:folder01/my_theme.toml",
            "user:folder01\\my_theme.toml",
            "user:C:my_theme.toml",
            "user:.toml",
            "builtin:unknown",
            "user:my_theme.toml\n",
        ] {
            assert!(user_file_name(id).is_none(), "{id}");
        }
        assert_eq!(user_file_name("user:my theme.toml"), Some("my theme.toml"));
    }

    /// A stored choice names a family or a file, and the base family must be
    /// nameable: a workspace override reads an absent value as "no override",
    /// so the family whose own identifier is absent needs its alias.
    #[test]
    fn every_family_has_a_nameable_identifier_and_unknown_ones_are_rejected() {
        use super::super::BuiltinTheme;
        for family in BuiltinTheme::ALL {
            let stored = family.stored_id();
            assert!(!stored.is_empty(), "{family:?}");
            assert!(id_is_known(stored), "{stored}");
            assert_eq!(
                BuiltinTheme::from_id(Some(stored)),
                Some(*family),
                "{stored}"
            );
        }
        assert_eq!(BuiltinTheme::Nord.stored_id(), "builtin:base");
        assert!(id_is_known("user:my_theme.toml"));
        for id in ["", "builtin:unknown", "user:../my_theme.toml", "my_theme"] {
            assert!(!id_is_known(id), "{id}");
        }
    }

    // Also covers the documented way of writing a personal theme: copy an
    // embedded palette file under a name of your own.
    #[test]
    fn a_copied_built_in_file_is_discovered_and_loads_identically() {
        let family = super::super::BuiltinTheme::Atom;
        let dir = std::env::temp_dir().join(format!(
            "favnyr-theme-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("theme01.toml"), family.source()).unwrap();
        std::fs::write(dir.join("theme02.toml"), "broken").unwrap();
        std::fs::write(dir.join("theme03.txt"), "ignored").unwrap();
        let catalog = discover(&dir, Some("user:theme01.toml"));
        assert_eq!(catalog.entries.len(), 1);
        assert_eq!(catalog.entries[0].name, "theme01");
        assert_eq!(catalog.errors.len(), 1);
        assert_eq!(catalog.selected.unwrap().unwrap(), *family.palettes());
        assert!(
            discover(&dir, Some("user:theme02.toml"))
                .selected
                .unwrap()
                .is_err()
        );
        assert!(discover(&dir, Some("builtin:base")).selected.is_none());
        assert_eq!(load(&dir, "user:theme01.toml").unwrap(), *family.palettes());
        std::fs::remove_file(dir.join("theme01.toml")).unwrap();
        assert!(
            discover(&dir, Some("user:theme01.toml"))
                .selected
                .unwrap()
                .is_err()
        );
        assert!(load(&dir, "user:theme01.toml").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
