//! Per-path annotations: a colour slot, and a free-text note.
//!
//! The data belongs to a PATH, not to a window arrangement, so it is global
//! and immediate — exactly like the favorites tree. A folder keeps its colour
//! across every workspace, and across sessions where no workspace is named at
//! all. Writing happens the moment something changes, so there is nothing for
//! the user to remember to save.
//!
//! This is deliberately outside the workspace "modified" signature: nothing
//! about the panel layout changed when a folder is recoloured, and marking the
//! workspace dirty would light a star that saving could never clear.
//!
//! **A colour is stored as an INDEX, never as a literal colour.** The two
//! themes need different values for the same slot — a mark light enough to
//! hold contrast on a dark row is too light on a pale one — so the index is
//! resolved on the GUI side, per theme. It also makes the file readable and
//! keeps it valid when the palette is retuned.
//!
//! Known limitation: an annotation is keyed by path, so renaming or moving an
//! item outside Favnyr leaves its annotation behind under the old key. The
//! favorites tree has the same property.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::error::Error;

/// Highest assignable colour slot. Slot 0 is the untouched default and is
/// never stored — an item carrying it has no entry at all.
pub const MAX_COLOR_SLOT: u8 = 7;

/// Longest note kept. A note is a label, not a document: the bubble that
/// displays it is a preview, and an unbounded string would make both the file
/// and that bubble grow without limit.
pub const MAX_NOTE_CHARS: usize = 240;

/// What one path carries. Both fields are optional, and an entry holding
/// neither is dropped rather than written empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    /// Colour slot, 1..=[`MAX_COLOR_SLOT`]. Absent means the default.
    #[serde(default, skip_serializing_if = "is_default_slot")]
    pub color: u8,
    /// Free text attached to the item. Absent means none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

fn is_default_slot(slot: &u8) -> bool {
    *slot == 0
}

impl Annotation {
    /// Does this entry still say anything? An entry that does not is removed,
    /// so the file never accumulates rows that carry no information.
    fn is_meaningful(&self) -> bool {
        self.color != 0 || !self.note.is_empty()
    }
}

/// An annotation whose item is gone: where it pointed, and what it carries.
///
/// What a cleanup takes away is this pair — a colour and a note — never a
/// file: the file left long ago.
#[derive(Debug, Clone, PartialEq)]
pub struct Orphan {
    pub path: String,
    pub color: u8,
    pub note: String,
}

/// Every annotation, keyed by the item's path.
///
/// A `BTreeMap` rather than a `HashMap`: the file is written on every change,
/// so a stable order keeps it readable and its diffs quiet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotationStore {
    #[serde(default, rename = "entry")]
    entries: BTreeMap<String, Annotation>,
}

impl AnnotationStore {
    /// Loads from `path`; an empty store if the file is missing, unreadable or
    /// corrupted — an annotation must never prevent the app from starting.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => match toml::from_str::<Self>(&content) {
                Ok(mut store) => {
                    store.refold_keys();
                    store
                }
                Err(err) => {
                    // The file is there but cannot be read back. Starting from
                    // an empty store keeps the application usable; letting the
                    // next save write that emptiness over the user's only copy
                    // does not, so the file is kept aside first.
                    tracing::warn!(
                        error = %err,
                        path = %path.display(),
                        "unreadable store"
                    );
                    crate::paths::preserve_unreadable(path);
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    /// Writes the store as TOML (creating the parent folder if needed).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|err| Error::Workspace(format!("annotations serialization: {err}")))?;
        crate::paths::write_atomic(path, &text)?;
        Ok(())
    }

    /// Brings every key under the current folding rule.
    ///
    /// A file written before the rule existed holds paths as they were spelled
    /// then. Re-keying on load keeps those annotations findable instead of
    /// leaving them quietly unreachable — two keys that fold to the same one
    /// describe the same file, so merging them loses nothing.
    fn refold_keys(&mut self) {
        if self
            .entries
            .keys()
            .all(|key| key_of(Path::new(key)) == *key)
        {
            return; // already folded: the usual case, and it costs one pass
        }
        self.entries = std::mem::take(&mut self.entries)
            .into_iter()
            .map(|(key, entry)| (key_of(Path::new(&key)), entry))
            .collect();
    }

    /// Colour slot of `path`, `0` when it carries none.
    pub fn color_of(&self, path: &Path) -> u8 {
        self.entries
            .get(&key_of(path))
            .map_or(0, |entry| entry.color)
    }

    /// Note attached to `path`, empty when it carries none.
    pub fn note_of(&self, path: &Path) -> &str {
        self.entries
            .get(&key_of(path))
            .map_or("", |entry| entry.note.as_str())
    }

    /// Assigns a colour slot. Slot `0` — or anything past the palette —
    /// clears it instead, which is what makes the first swatch a reset.
    pub fn set_color(&mut self, path: &Path, slot: u8) {
        let slot = if slot > MAX_COLOR_SLOT { 0 } else { slot };
        self.update(path, |entry| entry.color = slot);
    }

    /// Attaches a note. Surrounding blank space is dropped and the text is
    /// bounded, so an empty or whitespace-only note clears the annotation.
    pub fn set_note(&mut self, path: &Path, note: &str) {
        let note: String = note.trim().chars().take(MAX_NOTE_CHARS).collect();
        self.update(path, |entry| entry.note = note);
    }

    /// Follows an item that moved, so its annotation moves with it.
    ///
    /// Renaming or moving a FOLDER also moves everything annotated inside it:
    /// those paths changed too. Matching is done component by component, so a
    /// sibling whose name merely starts with the same letters is left alone.
    ///
    /// Anything already sitting at the destination is overwritten, which is
    /// what the filesystem did to the item itself.
    pub fn rename(&mut self, from: &Path, to: &Path) {
        let folded_from = fold(from);
        let moved: Vec<(String, String, Annotation)> = self
            .entries
            .iter()
            .filter_map(|(key, entry)| {
                let rest = Path::new(key).strip_prefix(&folded_from).ok()?;
                let destination = if rest.as_os_str().is_empty() {
                    to.to_path_buf()
                } else {
                    to.join(rest)
                };
                Some((key.clone(), key_of(&destination), entry.clone()))
            })
            .collect();
        for (old, new, entry) in moved {
            self.entries.remove(&old);
            self.entries.insert(new, entry);
        }
    }

    /// Drops the annotation of an item that is gone for good, and those of
    /// everything that was inside it.
    ///
    /// Reserved for a deletion nothing can undo. An item sent to the trash
    /// keeps its annotation: it can come back, and losing a note to a mistaken
    /// delete is worse than carrying an entry that may never be claimed.
    pub fn forget(&mut self, path: &Path) {
        let folded = fold(path);
        self.entries
            .retain(|key, _| Path::new(key).strip_prefix(&folded).is_err());
    }

    /// The annotations pointing at an item that is gone, in path order.
    ///
    /// Listed rather than merely counted, so the interface can show WHAT a
    /// cleanup would take. What is lost is a colour and a note — not a file:
    /// the file left long ago, and showing only its path would hide the very
    /// thing the user is being asked about.
    ///
    /// The store is ordered by path, so entries of the same folder come out
    /// next to each other for free.
    pub fn orphans(&self) -> Vec<Orphan> {
        self.orphans_where(&|path: &Path| path.exists())
    }

    fn orphans_where(&self, exists: &impl Fn(&Path) -> bool) -> Vec<Orphan> {
        self.entries
            .iter()
            .filter(|(key, _)| is_orphan(Path::new(key), exists))
            .map(|(key, entry)| Orphan {
                path: key.clone(),
                color: entry.color,
                note: entry.note.clone(),
            })
            .collect()
    }

    /// Drops the annotations named by `keys`, re-checking each one first, and
    /// returns how many actually went.
    ///
    /// The re-check is the whole point: the list the user answered was a
    /// snapshot. An item restored from the trash while the question was on
    /// screen is silently kept, because the answer given no longer applies to
    /// it. A key that is unknown, or whose item is back, is skipped rather than
    /// reported.
    pub fn remove_selected(&mut self, keys: &[String]) -> usize {
        self.remove_selected_where(keys, &|path: &Path| path.exists())
    }

    fn remove_selected_where(&mut self, keys: &[String], exists: &impl Fn(&Path) -> bool) -> usize {
        let mut removed = 0;
        for key in keys {
            if self.entries.contains_key(key) && is_orphan(Path::new(key), exists) {
                self.entries.remove(key);
                removed += 1;
            }
        }
        removed
    }

    /// Applies `change`, then drops the entry if it no longer says anything.
    fn update(&mut self, path: &Path, change: impl FnOnce(&mut Annotation)) {
        let key = key_of(path);
        let entry = self.entries.entry(key.clone()).or_default();
        change(entry);
        if !entry.is_meaningful() {
            self.entries.remove(&key);
        }
    }

    /// Is anything annotated at all? Lets the caller skip writing a file that
    /// would hold nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many items carry an annotation.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Is this annotation pointing at something that is gone for good?
///
/// A missing item is NOT enough on its own. An external drive simply unplugged
/// makes every path on it vanish, and sweeping those annotations away would
/// destroy data the user never asked to lose. The parent folder must still be
/// there: that is what tells "this item was removed" apart from "this whole
/// branch is currently out of reach".
///
/// A path with no parent at all — a filesystem root — is never swept: there is
/// nothing above it to corroborate its absence.
fn is_orphan(path: &Path, exists: &impl Fn(&Path) -> bool) -> bool {
    if exists(path) {
        return false;
    }
    path.parent().is_some_and(exists)
}

/// The path a key is derived from, under the platform's own rules.
///
/// Windows treats two spellings of one path as the same file, and Favnyr can
/// legitimately reach the same folder under either — the address bar keeps
/// what the user typed, and a path can also arrive from another application.
/// Comparing the two as plain text would mean annotating through one spelling
/// and finding nothing through the other, then losing the colour and the note
/// for good at the next rename, which strips the prefix it was given.
#[cfg(windows)]
fn fold(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

#[cfg(not(windows))]
fn fold(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// Key under which a path is stored. Lossy conversion is deliberate: a path
/// the platform cannot render as text cannot be annotated either, and the
/// alternative is refusing to load the whole file over one odd entry.
fn key_of(path: &Path) -> String {
    fold(path).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn item(name: &str) -> PathBuf {
        PathBuf::from("/somewhere").join(name)
    }

    #[test]
    fn an_unannotated_path_carries_nothing() {
        let store = AnnotationStore::default();
        assert_eq!(store.color_of(&item("folder01")), 0);
        assert_eq!(store.note_of(&item("folder01")), "");
    }

    #[test]
    fn a_colour_is_kept_and_read_back() {
        let mut store = AnnotationStore::default();
        store.set_color(&item("folder01"), 3);
        assert_eq!(store.color_of(&item("folder01")), 3);
        // And it belongs to that path alone.
        assert_eq!(store.color_of(&item("folder02")), 0);
    }

    #[test]
    fn the_default_slot_clears_rather_than_stores() {
        let mut store = AnnotationStore::default();
        store.set_color(&item("folder01"), 3);
        store.set_color(&item("folder01"), 0);
        assert_eq!(store.color_of(&item("folder01")), 0);
        // The entry is gone, not merely zeroed: the file must not accumulate
        // rows that say nothing.
        assert!(store.is_empty());
    }

    #[test]
    fn a_slot_past_the_palette_clears_too() {
        let mut store = AnnotationStore::default();
        store.set_color(&item("folder01"), MAX_COLOR_SLOT + 1);
        assert!(store.is_empty());
    }

    #[test]
    fn a_note_is_trimmed_and_bounded() {
        let mut store = AnnotationStore::default();
        store.set_note(&item("my_file.pdf"), "   kept for later   ");
        assert_eq!(store.note_of(&item("my_file.pdf")), "kept for later");

        store.set_note(&item("my_file.pdf"), &"x".repeat(MAX_NOTE_CHARS + 50));
        assert_eq!(
            store.note_of(&item("my_file.pdf")).chars().count(),
            MAX_NOTE_CHARS
        );
    }

    #[test]
    fn a_blank_note_clears_the_entry() {
        let mut store = AnnotationStore::default();
        store.set_note(&item("my_file.pdf"), "something");
        store.set_note(&item("my_file.pdf"), "   ");
        assert!(store.is_empty());
    }

    #[test]
    fn clearing_one_field_keeps_the_other() {
        let mut store = AnnotationStore::default();
        let path = item("folder01");
        store.set_color(&path, 5);
        store.set_note(&path, "a note");

        store.set_note(&path, "");
        assert_eq!(store.color_of(&path), 5, "the colour must survive");
        assert_eq!(store.len(), 1);

        store.set_color(&path, 0);
        assert!(store.is_empty(), "the entry goes only once it says nothing");
    }

    #[test]
    fn a_renamed_item_keeps_its_annotation() {
        let mut store = AnnotationStore::default();
        store.set_color(&item("folder01"), 3);
        store.rename(&item("folder01"), &item("folder02"));
        assert_eq!(store.color_of(&item("folder01")), 0, "the old key is gone");
        assert_eq!(store.color_of(&item("folder02")), 3);
    }

    /// The behaviour a rename-with-replacement relies on: what was at the
    /// destination is gone from the disk, so its annotation must not stay
    /// behind and end up describing the item that took its place.
    /// Reaching the same folder under two spellings is ordinary on Windows —
    /// the address bar keeps whatever was typed. The annotation must be the
    /// same one either way, or it looks lost and the next rename really does
    /// lose it.
    #[cfg(windows)]
    #[test]
    fn a_path_spelled_differently_finds_the_same_annotation() {
        let mut store = AnnotationStore::default();
        store.set_note(&PathBuf::from(r"C:\Folder01\my_file.txt"), "kept");

        assert_eq!(
            store.note_of(&PathBuf::from(r"c:\folder01\MY_FILE.TXT")),
            "kept",
            "the spelling does not decide which file this is"
        );
        // And a rename issued under the other spelling still carries it.
        store.rename(
            &PathBuf::from(r"c:\folder01"),
            &PathBuf::from(r"C:\Folder02"),
        );
        assert_eq!(
            store.note_of(&PathBuf::from(r"C:\Folder02\my_file.txt")),
            "kept"
        );
    }

    /// A file written before the folding rule existed must not become
    /// unreachable: its keys are brought over on load.
    #[cfg(windows)]
    #[test]
    fn keys_written_before_the_folding_rule_are_migrated() {
        let mut store = AnnotationStore::default();
        store.entries.insert(
            r"C:\Legacy\Item.txt".to_owned(),
            Annotation {
                color: 4,
                note: "from an older file".to_owned(),
            },
        );

        store.refold_keys();

        assert_eq!(store.color_of(&PathBuf::from(r"C:\Legacy\Item.txt")), 4);
        assert_eq!(
            store.note_of(&PathBuf::from(r"c:\legacy\item.txt")),
            "from an older file"
        );
    }

    #[test]
    fn renaming_onto_an_annotated_item_replaces_its_annotation() {
        let mut store = AnnotationStore::default();
        store.set_note(&item("victim.txt"), "belonged to the replaced file");
        store.set_note(&item("winner.txt"), "belonged to the renamed file");

        store.rename(&item("winner.txt"), &item("victim.txt"));

        assert_eq!(
            store.note_of(&item("victim.txt")),
            "belonged to the renamed file",
            "the destination wears the annotation of what now occupies it"
        );
        assert_eq!(
            store.note_of(&item("winner.txt")),
            "",
            "the old key is gone"
        );
    }

    #[test]
    fn renaming_a_folder_carries_what_was_annotated_inside_it() {
        let mut store = AnnotationStore::default();
        store.set_color(&item("folder01"), 3);
        store.set_note(&item("folder01").join("my_file.pdf"), "kept for later");
        // A sibling whose name merely starts the same must NOT be dragged along.
        store.set_note(&item("folder01_backup").join("other.pdf"), "untouched");

        store.rename(&item("folder01"), &item("folder02"));

        assert_eq!(store.color_of(&item("folder02")), 3);
        assert_eq!(
            store.note_of(&item("folder02").join("my_file.pdf")),
            "kept for later"
        );
        assert_eq!(
            store.note_of(&item("folder01_backup").join("other.pdf")),
            "untouched",
            "component-wise matching, not a raw string prefix"
        );
    }

    #[test]
    fn forgetting_an_item_drops_it_and_everything_under_it() {
        let mut store = AnnotationStore::default();
        store.set_color(&item("folder01"), 3);
        store.set_note(&item("folder01").join("my_file.pdf"), "kept for later");
        store.set_note(&item("folder01_backup").join("other.pdf"), "untouched");

        store.forget(&item("folder01"));

        assert_eq!(store.len(), 1, "only the unrelated sibling survives");
        assert_eq!(
            store.note_of(&item("folder01_backup").join("other.pdf")),
            "untouched"
        );
    }

    /// Pretends the shared PARENT and the listed items exist. Without the
    /// parent nothing is an orphan — that is the rule itself — so this is the
    /// helper to reach for whenever a test is about orphans rather than about
    /// unreachable branches.
    fn present(names: &[&str]) -> impl Fn(&Path) -> bool + use<> {
        let present: Vec<PathBuf> = names.iter().map(|p| item(p)).collect();
        move |path: &Path| path == Path::new("/somewhere") || present.iter().any(|p| p == path)
    }

    /// Pretends only the listed paths exist.
    fn only(present: &[&str]) -> impl Fn(&Path) -> bool + use<> {
        let present: Vec<PathBuf> = present.iter().map(|p| item(p)).collect();
        move |path: &Path| present.iter().any(|p| p == path)
    }

    #[test]
    fn an_annotation_whose_item_is_gone_is_listed() {
        let mut store = AnnotationStore::default();
        store.set_color(&item("folder01"), 3);
        // The parent is still there, so the item really was removed.
        assert_eq!(store.orphans_where(&present(&[])).len(), 1);
    }

    #[test]
    fn an_annotation_on_an_unreachable_branch_is_kept() {
        let mut store = AnnotationStore::default();
        store.set_note(&item("folder01").join("my_file.pdf"), "kept for later");
        // Nothing exists: the whole branch is out of reach — an unplugged
        // drive, not a deletion. Offering it for removal would lose real data.
        assert!(store.orphans_where(&only(&[])).is_empty());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn an_annotation_whose_item_is_still_there_is_kept() {
        let mut store = AnnotationStore::default();
        store.set_color(&item("folder01"), 3);
        assert!(store.orphans_where(&present(&["folder01"])).is_empty());
        assert_eq!(store.color_of(&item("folder01")), 3);
    }

    #[test]
    fn a_store_survives_a_round_trip_through_toml() {
        let mut store = AnnotationStore::default();
        store.set_color(&item("folder01"), 2);
        store.set_note(&item("my_file.pdf"), "kept for later");

        let text = toml::to_string_pretty(&store).unwrap();
        let back: AnnotationStore = toml::from_str(&text).unwrap();

        assert_eq!(back, store);
        assert_eq!(back.color_of(&item("folder01")), 2);
        assert_eq!(back.note_of(&item("my_file.pdf")), "kept for later");
    }

    #[test]
    fn a_corrupted_file_yields_an_empty_store_rather_than_failing() {
        let path = std::env::temp_dir().join(format!(
            "favnyr-annotations-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, "this is not valid TOML {{{").unwrap();
        assert!(AnnotationStore::load(&path).is_empty());
        std::fs::remove_file(&path).ok();
    }

    /// The list carries what would be LOST — the colour and the note — not
    /// just the path of something that is already gone.
    #[test]
    fn orphans_list_what_would_be_lost() {
        let mut store = AnnotationStore::default();
        store.set_color(&item("folder01"), 3);
        store.set_note(&item("folder01"), "kept for later");
        store.set_color(&item("folder02"), 5);
        let found = store.orphans_where(&present(&["folder02"]));
        assert_eq!(
            found,
            vec![Orphan {
                path: item("folder01").to_string_lossy().into_owned(),
                color: 3,
                note: "kept for later".to_string(),
            }]
        );
    }

    /// A whole branch out of reach — an unplugged drive — is never listed, so
    /// it can never be offered for removal either.
    #[test]
    fn orphans_never_list_an_unreachable_branch() {
        let mut store = AnnotationStore::default();
        store.set_color(&item("folder01").join("my_file.pdf"), 2);
        assert!(store.orphans_where(&only(&[])).is_empty());
    }

    /// The answer was given on a snapshot: an item restored from the trash
    /// while the question was on screen keeps its annotation.
    #[test]
    fn remove_selected_keeps_an_item_that_came_back() {
        let mut store = AnnotationStore::default();
        store.set_color(&item("folder01"), 3);
        store.set_color(&item("folder02"), 4);
        let keys: Vec<String> = store
            .orphans_where(&present(&[]))
            .into_iter()
            .map(|orphan| orphan.path)
            .collect();
        assert_eq!(keys.len(), 2, "both look gone when the list is taken");
        // folder01 is back by the time the user confirms.
        assert_eq!(
            store.remove_selected_where(&keys, &present(&["folder01"])),
            1
        );
        assert_eq!(store.color_of(&item("folder01")), 3);
        assert_eq!(store.color_of(&item("folder02")), 0);
    }

    /// Only what was checked goes; the rest stays, however dead it looks.
    #[test]
    fn remove_selected_touches_only_the_keys_it_was_given() {
        let mut store = AnnotationStore::default();
        store.set_color(&item("folder01"), 3);
        store.set_color(&item("folder02"), 4);
        let chosen = vec![item("folder01").to_string_lossy().into_owned()];
        assert_eq!(store.remove_selected_where(&chosen, &present(&[])), 1);
        assert_eq!(store.color_of(&item("folder02")), 4);
    }

    /// An unknown key is skipped rather than counted: the figure reported back
    /// to the user is what really left.
    #[test]
    fn remove_selected_reports_what_actually_went() {
        let mut store = AnnotationStore::default();
        store.set_color(&item("folder01"), 3);
        let keys = vec![
            item("folder01").to_string_lossy().into_owned(),
            item("never_annotated").to_string_lossy().into_owned(),
        ];
        assert_eq!(store.remove_selected_where(&keys, &present(&[])), 1);
    }
}
