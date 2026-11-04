//! Tab state and mutations, independent of the rendered file listing.

use favnyr_core::tab_groups::{GroupedTab, TabGroups};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{
    Config, GRID_DEFAULT_ZOOM, GroupMode, ItemArrangement, LIST_DEFAULT_ZOOM, MAX_GRID_ZOOM,
    MAX_THUMBNAIL_ZOOM, MAX_ZOOM, MIN_GRID_ZOOM, MIN_THUMBNAIL_ZOOM, MIN_ZOOM, SortState,
    THUMB_DEFAULT_ZOOM, THUMB_ZOOM, TabState, ops, resolve_restored_path,
};

pub(super) mod batch;
pub(super) mod drag;
pub(super) mod groups;
pub(super) mod ipc;
pub(super) mod projection;
mod selection;
pub(super) mod transfer;
pub(super) mod ui;

// ---------- Navigation history ----------

#[derive(Debug, Default)]
pub(super) struct NavHistory {
    pub(super) stack: Vec<PathBuf>,
    cursor: usize,
}

impl NavHistory {
    pub(super) fn current(&self) -> Option<&Path> {
        self.stack.get(self.cursor).map(|p| p.as_path())
    }

    /// Pushes a new path, truncating any "forward" entries.
    pub(super) fn push(&mut self, path: PathBuf) {
        if self.current().map(|c| c == path).unwrap_or(false) {
            return;
        }
        if !self.stack.is_empty() {
            self.stack.truncate(self.cursor + 1);
        }
        self.stack.push(path);
        self.cursor = self.stack.len() - 1;
    }

    pub(super) fn can_back(&self) -> bool {
        self.cursor > 0
    }
    pub(super) fn can_forward(&self) -> bool {
        self.cursor + 1 < self.stack.len()
    }
    pub(super) fn back(&mut self) -> Option<PathBuf> {
        if !self.can_back() {
            return None;
        }
        self.cursor -= 1;
        Some(self.stack[self.cursor].clone())
    }
    pub(super) fn forward(&mut self) -> Option<PathBuf> {
        if !self.can_forward() {
            return None;
        }
        self.cursor += 1;
        Some(self.stack[self.cursor].clone())
    }

    pub(super) fn relocate(&mut self, from: &Path, to: &Path) -> bool {
        let mut changed = false;
        for path in &mut self.stack {
            if let Some(relocated) = ops::relocated_path(path, from, to) {
                *path = relocated;
                changed = true;
            }
        }
        changed
    }
}

/// A session identity, independent of paths and positions in a tab bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct TabId(u64);

impl TabId {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self(
            NEXT_ID
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .expect("tab identity space exhausted"),
        )
    }
}

/// A tab groups a current path, its navigation history, and its
/// sort order. Selection is not persisted between tabs; it is,
/// however, preserved when refreshing the same folder via the
/// `same-dir` mechanism of `refresh_listing`.
#[derive(Debug)]
pub(super) struct Tab {
    pub(super) tab_group_id: Option<String>,
    pub(super) id: TabId,
    pub(super) current_path: PathBuf,
    pub(super) history: NavHistory,
    pub(super) sort: SortState,
    /// Selection anchor for Shift+click. `-1` if none.
    pub(super) selection_anchor: i32,
    /// Content thumbnails, derived from list zoom only while using the list.
    /// Grid thumbnails are independent of either size setting.
    pub(super) preview: bool,
    /// Zoom level of entries (Ctrl+wheel). Mapped to a row height by
    /// `zoom_to_height`, and persisted per tab in the workspace TOML: a view
    /// left at a chosen thumbnail size reopens at that size, not at the
    /// default for its mode.
    pub(super) zoom: i32,
    pub(super) item_arrangement: ItemArrangement,
    /// None preserves the absence of a grid preference in older workspaces.
    /// The grid renderer supplies its default when the user first opens it.
    pub(super) grid_zoom: Option<i32>,
    /// Whether hidden files are shown (dotfiles + Windows HIDDEN attribute).
    /// `false` by default. Persisted per tab in the workspace TOML.
    pub(super) show_hidden: bool,
    /// Grouping mode (folders, coloured folders or files first, or mixed).
    /// Persisted per tab in the workspace TOML.
    pub(super) group_mode: GroupMode,
    /// "Cursor" row (head of arrow-key keyboard navigation). `-1`
    /// = none. Session only (not persisted).
    pub(super) cursor: i32,
    /// Counter incremented on each keyboard cursor movement → triggers
    /// scroll-into-view on the Slint side (without doing so on other refreshes).
    pub(super) scroll_gen: i32,
    /// Extension filter: bar enabled from the columns menu.
    /// `ext_filter_on` = bar shown; `ext_filter` = free text ("jpg, png",
    /// loose syntax). Session only (not persisted in the workspace).
    pub(super) ext_filter_on: bool,
    pub(super) ext_filter: String,
}

#[derive(Clone, Copy)]
pub(super) struct TabDisplayDefaults {
    pub(super) preview: bool,
    pub(super) thumbnail_zoom: i32,
    pub(super) arrangement: ItemArrangement,
    pub(super) show_hidden: bool,
}

impl From<&Config> for TabDisplayDefaults {
    fn from(config: &Config) -> Self {
        Self {
            preview: config.default_preview_mode,
            thumbnail_zoom: config
                .default_thumbnail_zoom
                .clamp(MIN_THUMBNAIL_ZOOM, MAX_THUMBNAIL_ZOOM),
            arrangement: config.default_item_arrangement,
            show_hidden: config.default_show_hidden,
        }
    }
}

#[cfg(test)]
impl From<bool> for TabDisplayDefaults {
    fn from(preview: bool) -> Self {
        Self {
            preview,
            arrangement: ItemArrangement::List,
            thumbnail_zoom: THUMB_DEFAULT_ZOOM,
            show_hidden: false,
        }
    }
}

impl Tab {
    pub(super) fn new(initial: PathBuf, display: impl Into<TabDisplayDefaults>) -> Self {
        let display = display.into();
        let preview = display.preview;
        let mut h = NavHistory::default();
        h.push(initial.clone());
        let zoom = if preview {
            display.thumbnail_zoom
        } else {
            LIST_DEFAULT_ZOOM
        };
        Self {
            tab_group_id: None,
            id: TabId::new(),
            current_path: initial,
            history: h,
            sort: SortState::default(),
            selection_anchor: -1,
            preview,
            zoom,
            item_arrangement: display.arrangement,
            grid_zoom: (display.thumbnail_zoom != GRID_DEFAULT_ZOOM)
                .then_some(display.thumbnail_zoom),
            show_hidden: display.show_hidden,
            group_mode: GroupMode::FoldersFirst,
            cursor: -1,
            scroll_gen: 0,
            ext_filter_on: false,
            ext_filter: String::new(),
        }
    }

    /// Builds a tab restored from a workspace: path + sort + display
    /// mode. History restarts with a single entry (back/forward not
    /// persisted).
    pub(super) fn restored(path: PathBuf, saved: &TabState) -> Self {
        let mut h = NavHistory::default();
        h.push(path.clone());
        // A workspace saved before the level was persisted carries none: the
        // tab reopens at the default for the mode it was in, exactly as it
        // used to. A hand-edited file is clamped rather than trusted.
        let zoom = match saved.zoom {
            Some(z) => z.clamp(MIN_ZOOM, MAX_ZOOM),
            None if saved.preview => THUMB_DEFAULT_ZOOM,
            None => LIST_DEFAULT_ZOOM,
        };
        Self {
            tab_group_id: saved.tab_group_id.clone(),
            id: TabId::new(),
            current_path: path,
            history: h,
            sort: SortState {
                column: saved.sort_column,
                order: saved.sort_order,
                secondary: saved
                    .secondary_sort
                    .filter(|s| !s.column.same_field(saved.sort_column)),
            },
            selection_anchor: -1,
            // Keep the historical list precedence, including hand-edited
            // workspaces. Grid representation has its own explicit choice.
            preview: match saved.item_arrangement {
                ItemArrangement::List => zoom >= THUMB_ZOOM,
                ItemArrangement::Grid => saved.preview,
            },
            zoom,
            item_arrangement: saved.item_arrangement,
            grid_zoom: saved
                .grid_zoom
                .map(|zoom| zoom.clamp(MIN_GRID_ZOOM, MAX_GRID_ZOOM)),
            show_hidden: saved.show_hidden,
            group_mode: saved.group_mode,
            cursor: -1,
            scroll_gen: 0,
            ext_filter_on: false,
            ext_filter: String::new(),
        }
    }
}

impl GroupedTab for Tab {
    fn tab_group_id(&self) -> Option<&str> {
        self.tab_group_id.as_deref()
    }
    fn set_tab_group_id(&mut self, id: Option<String>) {
        self.tab_group_id = id;
    }
}

pub(super) fn tab_to_state(tab: &Tab) -> TabState {
    TabState {
        tab_group_id: tab.tab_group_id.clone(),
        path: tab.current_path.display().to_string(),
        sort_column: tab.sort.column,
        sort_order: tab.sort.order,
        secondary_sort: tab.sort.secondary,
        preview: tab.preview,
        show_hidden: tab.show_hidden,
        group_mode: tab.group_mode,
        zoom: Some(tab.zoom),
        item_arrangement: tab.item_arrangement,
        grid_zoom: tab.grid_zoom,
    }
}

pub(super) fn tab_from_state(tab: TabState) -> Tab {
    Tab::restored(resolve_restored_path(&tab.path), &tab)
}

#[derive(Debug)]
pub(super) struct TabBook {
    pub(super) identity: BookId,
    pub(super) selection: selection::Selection,
    /// Group focus survives batch-selection resets and is never persisted.
    pub(super) focused_group: Option<String>,
    /// A group page has no active folder. `active` retains the last real tab.
    pub(super) active_group: Option<String>,
    pub(super) groups: TabGroups,
    pub(super) tabs: Vec<Tab>,
    pub(super) active: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BookId(TabId);

impl Default for BookId {
    fn default() -> Self {
        Self(TabId::new())
    }
}

impl TabBook {
    pub(in crate::bridge) fn group_page(&self) -> Option<&favnyr_core::tab_groups::TabGroupState> {
        self.active_group
            .as_deref()
            .and_then(|id| self.groups.get(id))
    }

    pub(in crate::bridge) fn focus_group(&mut self, id: &str) -> bool {
        let Some(group) = self.groups.get(id) else {
            return false;
        };
        let changed = self.active_group.as_deref() != Some(id);
        self.active_group = Some(group.id.clone());
        self.highlight_group(id);
        self.reset_selection();
        changed
    }

    pub(in crate::bridge) fn group_highlighted(&self, id: &str) -> bool {
        self.focused_group.as_deref() == Some(id) && self.groups.get(id).is_some()
    }

    pub(in crate::bridge) fn highlight_group(&mut self, id: &str) {
        if !self.group_highlighted(id) {
            self.focused_group = self.groups.get(id).map(|group| group.id.clone());
            if self.active_group.is_some() {
                self.active_group = self.focused_group.clone();
            }
        }
    }

    pub(in crate::bridge) fn highlight_active_group(&mut self) {
        let group = self
            .tabs
            .get(self.active)
            .and_then(|tab| tab.tab_group_id.as_deref())
            .and_then(|id| self.groups.get(id));
        self.focused_group = group.map(|group| group.id.clone());
        self.active_group = group
            .filter(|group| group.collapsed)
            .map(|group| group.id.clone());
    }

    /// Inserts an ungrouped tab at a logical gap and activates it. A gap
    /// inside a group moves to its end so that existing members stay together.
    pub(super) fn insert_tab_at(&mut self, at: usize, mut tab: Tab) -> usize {
        let at = self.groups.outside_gap(&self.tabs, at);
        tab.tab_group_id = None;
        self.tabs.insert(at, tab);
        self.active = at;
        self.highlight_active_group();
        self.reset_selection();
        at
    }

    pub(super) fn open(&mut self, path: PathBuf, display: impl Into<TabDisplayDefaults>) -> usize {
        self.insert_tab_at(self.tabs.len(), Tab::new(path, display))
    }

    /// Inserts `tab` right after tab `idx` and activates the new entry.
    /// Common primitive for contextual openings and duplication.
    pub(super) fn insert_after(&mut self, idx: usize, mut tab: Tab) -> Option<usize> {
        if idx >= self.tabs.len() {
            return None;
        }
        tab.tab_group_id = self.tabs[idx].tab_group_id.clone();
        if let Some(id) = &tab.tab_group_id {
            let _ = self.groups.set_collapsed(id, false);
        }
        self.tabs.insert(idx + 1, tab);
        self.active = idx + 1;
        self.highlight_active_group();
        self.reset_selection();
        Some(self.active)
    }

    /// Opens a target right after the active tab. A `TabBook` always has
    /// at least one tab; the fallback at the end nonetheless protects this invariant.
    pub(super) fn open_after_active(
        &mut self,
        path: PathBuf,
        display: impl Into<TabDisplayDefaults>,
    ) -> usize {
        if self.active >= self.tabs.len() {
            return self.open(path, display);
        }
        self.insert_after(self.active, Tab::new(path, display))
            .expect("active tab was validated")
    }

    /// Duplicates tab `idx`: inserts a copy (same path + view settings)
    /// RIGHT AFTER it and activates it. `false` if the index is out of bounds.
    pub(super) fn duplicate(&mut self, idx: usize) -> bool {
        let Some(src) = self.tabs.get(idx) else {
            return false;
        };
        let dup = Tab::restored(src.current_path.clone(), &tab_to_state(src));
        self.insert_after(idx, dup).is_some()
    }

    /// Closes tab `idx` and returns it. Refuses to close the last one.
    pub(super) fn close(&mut self, idx: usize) -> Option<Tab> {
        if self.tabs.len() <= 1 {
            return None;
        }
        self.take_tab(idx)
    }

    /// Extracts an entry for either closure or transfer, retaining the active
    /// identity when it survives. The caller must prune an emptied view.
    pub(super) fn take_tab(&mut self, idx: usize) -> Option<Tab> {
        if idx >= self.tabs.len() {
            return None;
        }
        let previous_active = self.tabs.get(self.active).map(|tab| tab.id);
        let mut closed = self.tabs.remove(idx);
        closed.tab_group_id = None;
        if idx < self.active {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        self.groups.normalize(&mut self.tabs);
        self.reconcile_selection();
        if self.group_page().is_none()
            && self.tabs.get(self.active).map(|tab| tab.id) != previous_active
        {
            self.highlight_active_group();
        }
        Some(closed)
    }

    pub(super) fn select(&mut self, idx: usize) -> bool {
        self.select_tab(idx, true)
    }

    pub(super) fn select_from_wheel(&mut self, idx: usize) -> bool {
        if let Some(id) = self
            .tabs
            .get(idx)
            .and_then(|tab| tab.tab_group_id.as_deref())
            .and_then(|id| self.groups.get(id))
            .filter(|group| group.collapsed)
            .map(|group| group.id.clone())
        {
            return self.focus_group(&id);
        }
        self.select_tab(idx, false)
    }

    fn select_tab(&mut self, idx: usize, expand_group: bool) -> bool {
        if idx >= self.tabs.len() {
            return false;
        }
        let changed = idx != self.active || self.group_page().is_some();
        self.active = idx;
        if expand_group && let Some(id) = &self.tabs[idx].tab_group_id {
            let _ = self.groups.set_collapsed(id, false);
        }
        self.highlight_active_group();
        self.reset_selection();
        changed
    }

    /// Moves one tab through the same stable operation used for a selection.
    #[cfg(test)]
    pub(super) fn move_tab(&mut self, from: usize, to_pre: usize) -> bool {
        let Some(tab) = self.tabs.get(from) else {
            return false;
        };
        self.move_tabs(&[tab.id], to_pre)
    }

    /// Resolves the complete selection before touching the order. A stale
    /// identity rejects the whole operation, even when paths are identical.
    pub(super) fn move_tabs(&mut self, ids: &[TabId], to_pre: usize) -> bool {
        let requested: HashSet<_> = ids.iter().copied().collect();
        if requested.len() != ids.len() {
            return false;
        }
        let indices: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(i, tab)| requested.contains(&tab.id).then_some(i))
            .collect();
        if indices.len() != ids.len() {
            return false;
        }
        let changed = self
            .groups
            .move_members(&mut self.tabs, &mut self.active, &indices, to_pre)
            .unwrap_or(false);
        self.reconcile_selection();
        changed
    }
}

#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod selection_tests;

#[cfg(test)]
mod input_tests;

#[cfg(test)]
mod drop_tests;
