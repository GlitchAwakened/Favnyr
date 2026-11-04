//! Session-only display data, independent of a native menu's command offsets.

use super::*;
use crate::shellmenu::{DocumentChoice, ShellEntry};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Key {
    pub verb: String,
    pub label: String,
}

pub(super) struct Catalog {
    keys: Vec<Key>,
    pub rows: ModelRc<ShellCtxEntry>,
}

impl Catalog {
    pub fn read(menu: &DocumentMenuSession, entries: &[ShellEntry]) -> anyhow::Result<Self> {
        Ok(Self::from_choices(menu.choices(entries)?))
    }

    pub fn from_choices(entries: Vec<DocumentChoice>) -> Self {
        let keys = entries
            .iter()
            .map(|entry| Key {
                verb: entry.extension.clone(),
                label: entry.label.clone(),
            })
            .collect();
        let rows = ModelRc::new(VecModel::from(
            entries
                .iter()
                .enumerate()
                .map(|(index, entry)| ShellCtxEntry {
                    // These are catalogue indices, never Windows command offsets.
                    id: index as i32,
                    label: entry.label.as_str().into(),
                    icon: shell_icon(&entry.icon),
                    monochrome: shell_icon_monochrome(&entry.icon),
                    has_sub: false,
                    sub: -1,
                })
                .collect::<Vec<_>>(),
        ));
        Self { keys, rows }
    }

    pub fn key(&self, index: i32) -> Option<&Key> {
        self.keys.get(usize::try_from(index).ok()?)
    }

    /// Resolve a displayed type against a fresh native menu. Never fall back
    /// to an old position, offset or an ambiguous caption.
    pub fn resolve(&self, index: i32, current: &Self) -> Option<usize> {
        let key = self.key(index)?;
        if key.verb.is_empty()
            && self
                .keys
                .iter()
                .filter(|candidate| *candidate == key)
                .count()
                != 1
        {
            return None;
        }
        let repeated_verb = self
            .keys
            .iter()
            .filter(|candidate| candidate.verb == key.verb)
            .take(2)
            .count()
            > 1;
        let mut candidates = current.keys.iter().enumerate().filter(|(_, candidate)| {
            candidate.verb == key.verb
                && ((!key.verb.is_empty() && !repeated_verb) || candidate.label == key.label)
        });
        let first = candidates.next()?;
        if candidates.next().is_none() {
            return Some(first.0);
        }
        let mut exact = current
            .keys
            .iter()
            .enumerate()
            .filter(|(_, candidate)| *candidate == key);
        let first = exact.next()?;
        exact.next().is_none().then_some(first.0)
    }
}

#[derive(Default)]
pub(super) struct Cache {
    catalog: RefCell<Option<Rc<Catalog>>>,
    loading: Cell<Option<u64>>,
    generation: Cell<u64>,
}

impl Cache {
    pub fn clear(&self) {
        self.catalog.borrow_mut().take();
        self.generation.set(self.generation.get().wrapping_add(1));
    }

    pub fn replace(&self, catalog: Catalog) {
        self.clear();
        *self.catalog.borrow_mut() = Some(Rc::new(catalog));
    }

    pub fn get(&self) -> Option<Rc<Catalog>> {
        self.catalog.borrow().clone()
    }

    pub fn begin(&self) -> Option<u64> {
        if self.catalog.borrow().is_some() || self.loading.get().is_some() {
            return None;
        }
        let generation = self.generation.get();
        self.loading.set(Some(generation));
        Some(generation)
    }

    /// A cleared cache keeps its outstanding worker until it finishes. This
    /// prevents repeated settings changes from spawning overlapping shell calls.
    pub fn finish(&self, generation: u64, catalog: Option<Catalog>) -> bool {
        if self.loading.get() != Some(generation) {
            return false;
        }
        self.loading.set(None);
        if generation != self.generation.get() {
            return false;
        }
        *self.catalog.borrow_mut() = catalog.map(Rc::new);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(keys: &[(&str, &str)]) -> Catalog {
        Catalog {
            keys: keys
                .iter()
                .map(|&(verb, label)| Key {
                    verb: verb.into(),
                    label: label.into(),
                })
                .collect(),
            rows: ModelRc::default(),
        }
    }

    #[test]
    fn display_cache_loads_once_including_an_empty_catalog_and_retries_failures() {
        for empty in [false, true] {
            let cache = Cache::default();
            let failed = cache.begin().unwrap();
            assert!(cache.begin().is_none());
            assert!(cache.finish(failed, None));
            assert!(cache.get().is_none());
            let request = cache.begin().unwrap();
            assert!(cache.finish(
                request,
                Some(catalog(if empty { &[] } else { &[(".bin", "File01")] }))
            ));
            let first = cache.get().unwrap();
            assert!(cache.begin().is_none());
            let second = cache.get().unwrap();
            assert!(Rc::ptr_eq(&first, &second));
            cache.clear();
            assert!(cache.get().is_none());
            assert!(cache.begin().is_some());
        }
    }

    #[test]
    fn reentrant_loading_and_disabling_cannot_publish_a_stale_catalog() {
        let cache = Cache::default();
        let first = cache.begin().unwrap();
        for _ in 0..10 {
            cache.clear();
            assert!(cache.begin().is_none());
        }
        assert!(!cache.finish(first, Some(catalog(&[(".bin", "File01")]))));
        assert!(cache.get().is_none());
        let current = cache.begin().unwrap();
        assert!(!cache.finish(first, Some(catalog(&[]))));
        assert!(cache.begin().is_none());
        assert!(cache.finish(current, Some(catalog(&[]))));
        assert!(cache.get().unwrap().keys.is_empty());
    }

    #[test]
    fn replacement_catalog_cannot_be_overwritten_by_an_old_worker() {
        let cache = Cache::default();
        let first = cache.begin().unwrap();
        cache.replace(catalog(&[(".bin", "File01")]));
        assert!(!cache.finish(first, Some(catalog(&[]))));
        assert_eq!(cache.get().unwrap().keys.len(), 1);
        assert!(cache.begin().is_none());
    }

    #[test]
    fn native_resolution_follows_identity_instead_of_old_menu_positions() {
        let old = catalog(&[(".bin", "File01"), ("", "File02"), (".dat", "File03")]);
        let current = catalog(&[(".dat", "File03"), (".bin", "File04"), ("", "File02")]);
        assert_eq!(old.resolve(0, &current), Some(1));
        assert_eq!(old.resolve(1, &current), Some(2));
        assert_eq!(old.resolve(2, &current), Some(0));
        assert_eq!(old.resolve(-1, &current), None);
        assert_eq!(old.resolve(9, &current), None);
        assert_eq!(old.resolve(0, &catalog(&[])), None);
        assert_eq!(
            old.resolve(1, &catalog(&[("", "File02"), ("", "File02")])),
            None
        );
        assert_eq!(
            catalog(&[("", "File02"), ("", "File02")]).resolve(0, &current),
            None
        );
        assert_eq!(
            old.resolve(0, &catalog(&[(".bin", "File01"), (".bin", "File01")])),
            None
        );
        assert_eq!(
            old.resolve(0, &catalog(&[(".bin", "File02"), (".bin", "File01")])),
            Some(1)
        );
        assert_eq!(old.resolve(1, &catalog(&[("", "File03")])), None);
        let repeated = catalog(&[(".bin", "File01"), (".bin", "File02")]);
        assert_eq!(repeated.resolve(0, &catalog(&[(".bin", "File02")])), None);
        assert_eq!(
            repeated.resolve(1, &catalog(&[(".bin", "File02")])),
            Some(0)
        );
    }

    #[test]
    #[ignore = "requires a Windows text document registration"]
    fn cached_native_choices_rebind_to_another_folder_without_reusing_offsets() {
        let first = crate::test_support::TestFolder::new();
        let second = crate::test_support::TestFolder::new();
        let cache = Cache::default();
        let request = cache.begin().unwrap();
        let choices = crate::shellmenu::discover_documents(&first.0).unwrap();
        assert!(cache.finish(request, Some(Catalog::from_choices(choices))));
        let old = cache.get().unwrap();
        let selected = old.keys.iter().position(|key| key.verb == ".txt").unwrap() as i32;
        assert!(cache.begin().is_none());
        let cached = cache.get().unwrap();
        assert!(Rc::ptr_eq(&old, &cached));
        assert!(
            !cached
                .rows
                .row_data(selected as usize)
                .unwrap()
                .label
                .is_empty()
        );
        let (menu, entries) = build_for_folder(&second.0).unwrap();
        let current = Catalog::read(&menu, &entries).unwrap();
        let selected = old.resolve(selected, &current).unwrap();
        menu.invoke(entries[selected].id).unwrap();
        assert_eq!(std::fs::read_dir(&first.0).unwrap().count(), 0);
        let files: Vec<_> = std::fs::read_dir(&second.0)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].extension().unwrap(), "txt");
    }
}
