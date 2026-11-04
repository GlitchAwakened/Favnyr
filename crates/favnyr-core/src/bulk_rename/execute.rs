use super::{Issue, Plan, key, platform, validate};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub struct Failure {
    pub path: PathBuf,
    pub kind: io::ErrorKind,
    pub issue: Option<Issue>,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub cancelled: bool,
    pub failure: Option<Failure>,
    /// Actual chronological moves, including recovery, for path-owned UI state.
    pub steps: Vec<(PathBuf, PathBuf)>,
    /// Original and current paths that differ after execution or recovery.
    pub locations: Vec<(PathBuf, PathBuf)>,
}

/// Stage siblings before assigning final names. A failure restores what can
/// safely be restored, recording any remaining locations rather than deleting them.
pub fn execute(
    plan: &Plan,
    cancelled: impl Fn() -> bool,
    progress: impl FnMut(usize, usize),
) -> Outcome {
    execute_with(plan, cancelled, progress, platform::rename)
}

fn execute_with(
    plan: &Plan,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(usize, usize),
    mut rename: impl FnMut(&std::path::Path, &std::path::Path) -> io::Result<()>,
) -> Outcome {
    let mut outcome = Outcome::default();
    if cancelled() {
        outcome.cancelled = true;
        return outcome;
    }
    let mut checked = plan.clone();
    for entry in &mut checked.entries {
        if cancelled() {
            outcome.cancelled = true;
            return outcome;
        }
        match platform::identity(&entry.source) {
            Err(error) => entry.issue = Some(Issue::Unavailable(error.kind())),
            Ok(identity) if Some(&identity) != entry.identity.as_ref() => {
                entry.issue = Some(Issue::Changed)
            }
            _ => {}
        }
        if entry.source.parent() != entry.target.parent()
            || entry
                .target
                .file_name()
                .and_then(|name| name.to_str())
                .is_none_or(|name| crate::fs::ops::check_file_name(name).is_err())
        {
            entry.issue = Some(Issue::InvalidOptions);
        }
    }
    if validate(&mut checked.entries, &cancelled).is_none() {
        outcome.cancelled = true;
        return outcome;
    }
    if !checked.valid() {
        outcome.failure = Some(Failure {
            path: checked
                .entries
                .iter()
                .find(|e| e.issue.is_some())
                .map(|e| e.source.clone())
                .unwrap_or_default(),
            kind: io::ErrorKind::InvalidInput,
            issue: checked.entries.iter().find_map(|e| e.issue),
        });
        return outcome;
    }
    let entries: Vec<_> = checked.entries.iter().filter(|e| e.changes()).collect();
    let mut locations: Vec<_> = entries.iter().map(|e| e.source.clone()).collect();
    let mut temporary: Vec<Option<PathBuf>> = vec![None; entries.len()];
    let forbidden: std::collections::HashSet<_> = checked
        .entries
        .iter()
        .flat_map(|e| [key(&e.source), key(&e.target)])
        .collect();
    let total = entries.len() * 2;
    let mut move_entry = |index: usize,
                          to: &std::path::Path,
                          locations: &mut Vec<PathBuf>,
                          outcome: &mut Outcome|
     -> io::Result<()> {
        if platform::identity(&locations[index]).ok() != entries[index].identity {
            return Err(io::ErrorKind::InvalidData.into());
        }
        rename(&locations[index], to)?;
        outcome
            .steps
            .push((locations[index].clone(), to.to_owned()));
        locations[index] = to.to_owned();
        Ok(())
    };
    for (index, entry) in entries.iter().enumerate() {
        if cancelled() {
            outcome.cancelled = true;
            break;
        }
        let mut result = Err(io::Error::from(io::ErrorKind::AlreadyExists));
        for _ in 0..64 {
            static SERIAL: AtomicU64 = AtomicU64::new(0);
            let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
            let path = entry
                .source
                .with_file_name(format!(".favnyr-rename-{}-{serial}", std::process::id()));
            if forbidden.contains(&key(&path)) {
                continue;
            }
            result = move_entry(index, &path, &mut locations, &mut outcome);
            if result.is_ok() {
                temporary[index] = Some(path);
                break;
            }
            if result
                .as_ref()
                .err()
                .is_none_or(|e| e.kind() != io::ErrorKind::AlreadyExists)
            {
                break;
            }
        }
        if let Err(error) = result {
            outcome.failure = Some(Failure {
                path: entry.source.clone(),
                kind: error.kind(),
                issue: None,
            });
            break;
        }
        progress(index + 1, total);
    }
    if outcome.failure.is_none() && !outcome.cancelled {
        for (index, entry) in entries.iter().enumerate() {
            if cancelled() {
                outcome.cancelled = true;
                break;
            }
            if let Err(error) = move_entry(index, &entry.target, &mut locations, &mut outcome) {
                outcome.failure = Some(Failure {
                    path: entry.target.clone(),
                    kind: error.kind(),
                    issue: None,
                });
                break;
            }
            progress(entries.len() + index + 1, total);
        }
    }
    if outcome.failure.is_some() || outcome.cancelled {
        // Free all final names before restoring originals, including cycles.
        for (index, entry) in entries.iter().enumerate() {
            if locations[index] == entry.target
                && let Some(path) = &temporary[index]
            {
                let _ = move_entry(index, path, &mut locations, &mut outcome);
            }
        }
        for (index, entry) in entries.iter().enumerate().rev() {
            if locations[index] != entry.source {
                let _ = move_entry(index, &entry.source, &mut locations, &mut outcome);
            }
        }
    }
    outcome.locations = entries
        .iter()
        .zip(locations)
        .filter_map(|(entry, location)| {
            (entry.source != location).then(|| (entry.source.clone(), location))
        })
        .collect();
    outcome
}

#[cfg(test)]
pub(super) fn fail_commit(plan: &Plan, fail_recovery: bool) -> Outcome {
    let mut calls = 0;
    let stages = plan.changes();
    execute_with(
        plan,
        || false,
        |_, _| {},
        |from, to| {
            calls += 1;
            if calls == stages + 2 || (fail_recovery && calls > stages + 2) {
                Err(io::ErrorKind::PermissionDenied.into())
            } else {
                platform::rename(from, to)
            }
        },
    )
}
