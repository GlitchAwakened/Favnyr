//! Previewable filename transformations and non-overwriting batch execution.

mod execute;
mod platform;
#[cfg(test)]
mod tests;
mod transform;

use crate::fs::ops;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub use execute::{Outcome, execute};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Case {
    #[default]
    Unchanged,
    Lower,
    Upper,
}

#[derive(Clone, Debug)]
pub struct Replacement {
    pub find: String,
    pub with: String,
    pub case_sensitive: bool,
    pub all: bool,
}

#[derive(Clone, Debug)]
pub struct Numbering {
    pub start: u64,
    pub step: u64,
    pub digits: usize,
    pub before: bool,
    pub separator: String,
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub common: Option<String>,
    pub replacement: Option<Replacement>,
    pub affixes: Option<(String, String)>,
    pub numbering: Option<Numbering>,
    pub case: Case,
    pub extension: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue {
    InvalidName(ops::NameRejection),
    InvalidOptions,
    InvalidEncoding,
    Unavailable(std::io::ErrorKind),
    Duplicate,
    Occupied,
    NestedSelection,
    Changed,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub source: PathBuf,
    pub target: PathBuf,
    pub issue: Option<Issue>,
    identity: Option<platform::Identity>,
}

impl Entry {
    pub fn changes(&self) -> bool {
        self.source != self.target
    }
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub entries: Vec<Entry>,
}

impl Plan {
    pub fn changes(&self) -> usize {
        self.entries.iter().filter(|entry| entry.changes()).count()
    }
    pub fn valid(&self) -> bool {
        self.changes() > 0 && self.entries.iter().all(|entry| entry.issue.is_none())
    }
}

fn key(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(path.as_os_str().to_string_lossy().to_lowercase())
    }
    #[cfg(not(windows))]
    {
        path.to_owned()
    }
}

/// Captures identities and validates names without reading file contents.
/// Sources retain the caller's display order, including unchanged entries.
pub fn preview(
    sources: &[PathBuf],
    options: &Options,
    cancelled: impl Fn() -> bool,
) -> Option<Plan> {
    let mut entries = Vec::with_capacity(sources.len());
    for (index, source) in sources.iter().enumerate() {
        if cancelled() {
            return None;
        }
        let identity = platform::identity(source);
        let unavailable = identity
            .as_ref()
            .err()
            .map(|error| Issue::Unavailable(error.kind()));
        let identity = identity.ok();
        let transformed = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(Issue::InvalidEncoding)
            .and_then(|name| {
                transform::name(
                    name,
                    identity.as_ref().is_some_and(|id| id.directory),
                    index,
                    options,
                )
            });
        let (target, issue) = match transformed {
            Ok(name) => (source.with_file_name(name), None),
            Err(issue) => (source.clone(), Some(issue)),
        };
        entries.push(Entry {
            source: source.clone(),
            target,
            issue: issue.or(unavailable),
            identity,
        });
    }
    validate(&mut entries, &cancelled)?;
    Some(Plan { entries })
}

fn validate(entries: &mut [Entry], cancelled: &impl Fn() -> bool) -> Option<()> {
    let sources: HashSet<_> = entries.iter().map(|e| key(&e.source)).collect();
    let mut targets = HashMap::<PathBuf, Vec<usize>>::new();
    let mut nested = HashSet::new();
    for (index, entry) in entries.iter_mut().enumerate() {
        if cancelled() {
            return None;
        }
        for parent in entry.source.ancestors().skip(1) {
            let parent = key(parent);
            if sources.contains(&parent) {
                nested.insert(parent);
                nested.insert(key(&entry.source));
            }
        }
        targets.entry(key(&entry.target)).or_default().push(index);
        if entry.issue.is_none() && entry.changes() {
            match std::fs::symlink_metadata(&entry.target) {
                Ok(_) if !sources.contains(&key(&entry.target)) => {
                    entry.issue = Some(Issue::Occupied)
                }
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    entry.issue = Some(Issue::Unavailable(error.kind()))
                }
                _ => {}
            }
        }
    }
    for indexes in targets.values().filter(|indexes| indexes.len() > 1) {
        for &index in indexes {
            entries[index].issue = Some(Issue::Duplicate);
        }
    }
    for entry in entries {
        if nested.contains(&key(&entry.source)) {
            entry.issue = Some(Issue::NestedSelection);
        }
    }
    Some(())
}
