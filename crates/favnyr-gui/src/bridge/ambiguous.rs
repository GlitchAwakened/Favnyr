//! Lazy content classification, sharing thumbnail workers and invalidation.

use super::*;
use rfs::ambiguous::{PROBE_BYTES, Rule, rule_for_extension};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Stamp {
    size: Option<u64>,
    modified: Option<i64>,
}

impl Stamp {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        Self {
            size: Some(metadata.len()),
            modified: metadata.modified().ok().and_then(|time| {
                time.duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|duration| duration.as_secs() as i64)
            }),
        }
    }

    fn from_entry(entry: &Entry) -> Self {
        Self {
            size: entry.size_bytes,
            modified: entry.mtime_unix,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Resolution {
    stamp: Stamp,
    kind: FileKind,
    remote: bool,
}

impl Default for Resolution {
    fn default() -> Self {
        Self {
            stamp: Stamp::default(),
            kind: FileKind::File,
            remote: false,
        }
    }
}

impl Resolution {
    #[cfg(windows)]
    pub(super) fn kind(self) -> FileKind {
        self.kind
    }
}

/// Both recognized and generic results have their own bounded session budget.
pub(super) struct Cache {
    map: HashMap<PathBuf, Resolution>,
    order: VecDeque<PathBuf>,
    cap: usize,
}

impl Cache {
    pub(super) fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    pub(super) fn remove(&mut self, path: &Path) {
        if self.map.remove(path).is_some()
            && let Some(index) = self.order.iter().position(|key| key == path)
        {
            self.order.remove(index);
        }
    }

    fn insert(&mut self, path: PathBuf, resolution: Resolution) {
        if self.cap == 0 {
            return;
        }
        self.remove(&path);
        if self.map.len() >= self.cap
            && let Some(old) = self.order.pop_front()
        {
            self.map.remove(&old);
        }
        self.order.push_back(path.clone());
        self.map.insert(path, resolution);
    }

    pub(super) fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.map.keys()
    }

    fn kind(&self, path: &Path) -> Option<FileKind> {
        self.map.get(path).map(|value| value.kind)
    }
}

/// A normal file never enters this module's cache or worker-read path.
pub(super) fn rule(row: &FileRow) -> Option<Rule> {
    if row.is_dir || !matches!(row.kind, 1 | 7) {
        return None;
    }
    rule_for_extension(row.ext.as_str())
}

/// Rebuilt rows reuse a result only for the size and date already listed.
/// A changed version also loses its old pixels and cached decode failure.
pub(super) fn entry_kind(entry: &Entry, cache: &RefCell<ThumbLru>) -> FileKind {
    let mut cache = cache.borrow_mut();
    let Some(resolution) = cache.kinds.map.get(&entry.path).copied() else {
        return if !entry.content_remote && cache.contains(entry.path.to_string_lossy().as_ref()) {
            FileKind::Video
        } else {
            entry.kind
        };
    };
    if entry.content_remote || resolution.stamp != Stamp::from_entry(entry) {
        cache.remove_path(&entry.path);
        return entry.kind;
    }
    resolution.kind
}

/// Only confirmed thumbnail pixels reserve extra space for an ambiguous file.
pub(super) fn can_promote(row: &FileRow) -> bool {
    row.kind == FileKind::Image.as_i32()
        || row.kind == FileKind::Video.as_i32() && rule(row).is_some()
}

fn push_probe(
    requests: &mut HashMap<PathBuf, ThumbRequest>,
    path: PathBuf,
    format: Rule,
    location: ThumbLocation,
) {
    if let Some(request) = requests.get_mut(&path) {
        if !request.locations.contains(&location) {
            request.locations.push(location);
        }
        return;
    }
    requests.insert(
        path.clone(),
        ThumbRequest {
            job: ThumbJob {
                path,
                kind: FileKind::File,
                svg: false,
                probe: Some(format),
                #[cfg(windows)]
                link_target: None,
            },
            locations: vec![location],
        },
    );
}

/// Returns true while the row must not enter a video decoder. Callers pass
/// the visible-range decision, so even a very large source tree stays lazy.
pub(super) fn prepare(
    cache: &RefCell<ThumbLru>,
    row: &mut FileRow,
    visible: bool,
    location: ThumbLocation,
    requests: &mut HashMap<PathBuf, ThumbRequest>,
) -> bool {
    let Some(format) = rule(row) else {
        return false;
    };
    if row.content_remote {
        return true;
    }
    let path = row_path(row);
    let kind = {
        let cache = cache.borrow();
        cache.kinds.kind(&path).or_else(|| {
            cache
                .contains(path.to_string_lossy().as_ref())
                .then_some(FileKind::Video)
        })
    };
    match kind {
        Some(kind) => {
            row.kind = kind.as_i32();
            if kind != FileKind::Video {
                row.thumbnail = Image::default();
                row.preview_capable = false;
            }
            kind != FileKind::Video
        }
        None => {
            if visible {
                push_probe(requests, path, format, location);
            }
            true
        }
    }
}

/// The only content read for classification. No shell provider, process or
/// unbounded parsing is used, and special files are never opened knowingly.
pub(super) fn probe(path: &Path, format: Rule) -> Resolution {
    use std::io::Read;
    match crate::cloud_content_guard::automatic_content_status(path) {
        #[cfg(windows)]
        crate::cloud_content_guard::ContentStatus::Remote => {
            return Resolution {
                remote: true,
                ..Default::default()
            };
        }
        #[cfg(windows)]
        crate::cloud_content_guard::ContentStatus::Unavailable => return Resolution::default(),
        crate::cloud_content_guard::ContentStatus::Local => {}
    }
    let Ok(metadata) = std::fs::metadata(path) else {
        return Resolution::default();
    };
    let mut result = Resolution {
        stamp: Stamp::from_metadata(&metadata),
        ..Default::default()
    };
    if !metadata.is_file() || metadata.len() == 0 {
        return result;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return result;
    };
    let Ok(before) = file.metadata() else {
        return result;
    };
    if !before.is_file() {
        return result;
    }
    result.stamp = Stamp::from_metadata(&before);
    let mut prefix = [0; PROBE_BYTES];
    let limit = usize::try_from(before.len())
        .unwrap_or(usize::MAX)
        .min(prefix.len());
    let mut read = 0;
    while read < limit {
        match file.read(&mut prefix[read..limit]) {
            Ok(0) => break,
            Ok(count) => read += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return result,
        }
    }
    let Ok(after) = file.metadata() else {
        return result;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if rfs::content_is_remote_attributes(after.file_attributes()) {
            return Resolution {
                remote: true,
                ..Default::default()
            };
        }
    }
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Resolution::default();
    }
    result.kind = format.classify(&prefix[..read]);
    result
}

/// Called only after the scheduler accepted the generation. Classification
/// and thumbnail failures are separate: generic content must not block a
/// subsequent video preview after the file is edited or replaced.
pub(super) fn apply(
    panels: &[Panel],
    cache: &RefCell<ThumbLru>,
    config: &Config,
    path: &Path,
    locations: &[ThumbLocation],
    resolution: Resolution,
) -> Vec<ThumbRequest> {
    if resolution.remote || resolution.kind != FileKind::Video {
        cache.borrow_mut().remove_path(path);
    }
    if !resolution.remote {
        cache
            .borrow_mut()
            .kinds
            .insert(path.to_path_buf(), resolution);
    }
    let mut requests = HashMap::new();
    let mut relayout = HashSet::new();
    for location in locations {
        let Some(panel) = panels.get(location.panel) else {
            continue;
        };
        let Some(mut row) = panel.rows_model.row_data(location.row) else {
            continue;
        };
        if row_path(&row) != path || rule(&row).is_none() {
            continue;
        }
        let original = row.clone();
        if resolution.remote {
            row.content_remote = true;
            row.thumbnail = Image::default();
            row.preview_capable = false;
        } else if !row.content_remote {
            row.kind = resolution.kind.as_i32();
            if resolution.kind == FileKind::Video {
                row.drop_runnable = false;
                if row.rendered
                    && panel.tabs.tabs[panel.tabs.active].preview
                    && !cache
                        .borrow()
                        .failed
                        .contains(path.to_string_lossy().as_ref())
                {
                    push_thumbnail_request(
                        &mut requests,
                        path.to_path_buf(),
                        FileKind::Video,
                        false,
                        None,
                        *location,
                    );
                }
            } else {
                row.thumbnail = Image::default();
                row.preview_capable = false;
            }
        }
        if row != original {
            if row.preview_capable != original.preview_capable {
                relayout.insert(location.panel);
            }
            panel.rows_model.set_row_data(location.row, row);
        }
    }
    for index in relayout {
        relayout_changed_previews(&panels[index], config);
    }
    requests.into_values().collect()
}

#[cfg(test)]
mod tests;
