//! Serialization / deserialization of the workspace state.
//!
//! The workspace captures the **session layout**: the panel tree
//! (with their width ratios), the tabs open in each one
//! (current path + sort order), the active tab and panel, as well as
//! the collapsed state of the left sidebar sections. Their order is
//! a global preference kept in `config.toml`.
//!
//! Stored as TOML in `$XDG_DATA_HOME/favnyr/workspace.toml` (see
//! `paths::workspace_path`). The window size, meanwhile, stays in
//! `config.toml` (a cosmetic preference persisted by `persist_window_size`).
//!
//! The **selection** and **history** (back/forward) are NOT persisted:
//! they are volatile session states, with no restoration value.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::fs::{GroupMode, SortColumn, SortOrder};
use crate::layout::LayoutNode;

/// Maximum number of closed tabs kept per workspace, oldest to most
/// recent. The last element is the one "Reopen" will restore.
pub const MAX_CLOSED_TABS: usize = 20;

/// Item placement inside a tab, independent of thumbnail representation.
/// Unknown string values fall back locally without discarding the workspace.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemArrangement {
    Grid,
    #[default]
    #[serde(other)]
    List,
}

impl ItemArrangement {
    pub fn code(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Grid => "grid",
        }
    }

    pub fn from_code(code: &str) -> Self {
        match code {
            "grid" => Self::Grid,
            _ => Self::List,
        }
    }

    fn is_list(&self) -> bool {
        *self == Self::List
    }
}

/// Serializable state of a tab: current path + sort + display mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabState {
    pub path: String,
    #[serde(default = "default_sort_column")]
    pub sort_column: SortColumn,
    #[serde(default = "default_sort_order")]
    pub sort_order: SortOrder,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_sort: Option<crate::fs::SecondarySort>,
    /// Whether content thumbnails are enabled. An absent field keeps icons.
    #[serde(default)]
    pub preview: bool,
    /// Display of hidden files for this tab. When the field is
    /// absent, `#[serde(default)]` hides hidden files.
    #[serde(default)]
    pub show_hidden: bool,
    /// Grouping mode for this tab. When the field is absent, the default value
    /// is "folders first".
    #[serde(default = "default_group_mode")]
    pub group_mode: GroupMode,
    /// Thumbnail/row zoom level for this tab (Ctrl+wheel).
    ///
    /// `None` when the field is absent, which is what every workspace saved
    /// before this was persisted looks like. The caller then derives a level
    /// from `preview`, so an older file reopens exactly as it used to instead
    /// of snapping to a zoom it never chose. The valid range belongs to the
    /// interface, which clamps on restore — the core only carries the value.
    #[serde(default)]
    pub zoom: Option<i32>,
    /// Missing from older workspaces, whose items always used the list.
    #[serde(default, skip_serializing_if = "ItemArrangement::is_list")]
    pub item_arrangement: ItemArrangement,
    /// Independent grid size. The interface supplies and clamps its default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid_zoom: Option<i32>,
}

fn default_group_mode() -> GroupMode {
    GroupMode::FoldersFirst
}

fn default_sort_column() -> SortColumn {
    SortColumn::Name
}
fn default_sort_order() -> SortOrder {
    SortOrder::Asc
}

/// Serializable state of a panel: width ratio, active tab, tabs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewState {
    /// Width ratio relative to the other panels (see `panel_stretches`).
    #[serde(default = "default_stretch")]
    pub stretch: f32,
    #[serde(default)]
    pub active_tab: usize,
    pub tabs: Vec<TabState>,
    /// Columns of the view (order + visibility + width). When the field is
    /// absent, the full set is restored.
    #[serde(default = "crate::columns::default_columns")]
    pub columns: Vec<crate::columns::ColumnSpec>,
    /// Position of the view's tab bar: 0 = horizontal at the top,
    /// 1 = vertical on the left, 2 = vertical on the right. The default value is 0.
    #[serde(default)]
    pub tab_bar_mode: u8,
    /// USER width of the vertical tab bar (logical px), set by the
    /// resize handle. `0` = automatic (clamped 40% formula). Persisted per
    /// view.
    #[serde(default)]
    pub vbar_width: f32,
}

fn default_stretch() -> f32 {
    1.0
}

impl ViewState {
    /// Normalizes presentation and active-tab bounds without removing tabs.
    /// Workspace loading and detached-window startup share these rules.
    pub fn sanitize(&mut self) {
        self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
        if !(self.stretch.is_finite() && self.stretch > 0.0) {
            self.stretch = 1.0;
        }
        if self.tab_bar_mode > 2 {
            self.tab_bar_mode = 0;
        }
        if !self.vbar_width.is_finite() || self.vbar_width < 0.0 {
            self.vbar_width = 0.0;
        }
        self.columns = crate::columns::sanitize(std::mem::take(&mut self.columns));
    }
}

/// State of the four collapsible sections of the left sidebar. All
/// fields default to `false` so that an old workspace without this table
/// keeps its sections expanded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarSectionsState {
    #[serde(default)]
    pub shortcuts_collapsed: bool,
    #[serde(default)]
    pub favorites_collapsed: bool,
    #[serde(default)]
    pub drives_collapsed: bool,
    #[serde(default)]
    pub network_collapsed: bool,
}

impl SidebarSectionsState {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Complete state of the workspace.
///
/// `panels` is the **flat** list of panel contents (each with its
/// tabs); `layout` describes their **arrangement** as a tree of splits
/// (leaves reference an index in `panels`). If `layout` is absent or
/// invalid, it is rebuilt as a horizontal chain covering all
/// panels (see [`Self::layout_tree`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceState {
    #[serde(default)]
    pub active_panel: usize,
    pub panels: Vec<ViewState>,
    /// Layout tree. `None` triggers its reconstruction on load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutNode>,
    /// Name of the named workspace this state comes from. Shown in the
    /// window title ("{workspace} — Favnyr"). `None` = ad hoc state,
    /// never attached to a saved workspace → title "Favnyr".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    /// History of tabs actually closed by the user. When the field is
    /// absent from the TOML, the list is empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub closed_tabs: Vec<TabState>,
    /// Collapse state of the sidebar sections. The table is omitted when
    /// everything is expanded; `default` keeps old files perfectly readable.
    #[serde(default, skip_serializing_if = "SidebarSectionsState::is_default")]
    pub sidebar_sections: SidebarSectionsState,
}

impl WorkspaceState {
    /// Loads from `path`. Returns `Ok(None)` if the file does not exist
    /// (first launch) — the caller will then use a default workspace.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let ws: WorkspaceState = toml::from_str(&content).map_err(|e| {
                    // A default workspace takes over and will be saved in its
                    // place: the unreadable one is kept aside so the panels and
                    // tabs it described are not lost with it.
                    crate::paths::preserve_unreadable(path);
                    crate::error::Error::Workspace(format!("parse {}: {e}", path.display()))
                })?;
                // Guard: a workspace without a panel is invalid.
                if ws.panels.is_empty() {
                    return Ok(None);
                }
                Ok(Some(ws.sanitized()))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    /// Writes the workspace to disk (creates parent folders as needed).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| {
            crate::error::Error::Workspace(format!("serialize {}: {e}", path.display()))
        })?;
        crate::paths::write_atomic(path, &text)?;
        Ok(())
    }

    /// Returns the validated layout tree: the stored tree if it covers
    /// exactly `0..panels.len()`, otherwise a horizontal chain rebuilt
    /// from the available `stretch` values.
    pub fn layout_tree(&self) -> LayoutNode {
        if let Some(node) = &self.layout
            && node.is_valid_for(self.panels.len())
        {
            return node.clone();
        }
        let stretches: Vec<f32> = self.panels.iter().map(|p| p.stretch).collect();
        LayoutNode::row_chain(&stretches)
    }

    /// Normalizes out-of-bounds indices (robustness against a hand-edited
    /// or corrupted file): clamps `active_panel` and each `active_tab`,
    /// removes empty panels, and guarantees a `layout` consistent with the
    /// remaining panels.
    pub fn sanitized(mut self) -> Self {
        if self.closed_tabs.len() > MAX_CLOSED_TABS {
            self.closed_tabs
                .drain(..self.closed_tabs.len() - MAX_CLOSED_TABS);
        }
        self.panels.retain(|p| !p.tabs.is_empty());
        if self.panels.is_empty() {
            // Rebuild a minimal panel — the caller will replace the empty
            // path with $HOME via `effective_*`.
            self.panels.push(ViewState {
                stretch: 1.0,
                active_tab: 0,
                tabs: vec![TabState {
                    path: String::new(),
                    sort_column: SortColumn::Name,
                    sort_order: SortOrder::Asc,
                    secondary_sort: None,
                    preview: false,
                    show_hidden: false,
                    group_mode: GroupMode::FoldersFirst,
                    zoom: None,
                    item_arrangement: ItemArrangement::List,
                    grid_zoom: None,
                }],
                columns: crate::columns::default_columns(),
                tab_bar_mode: 0,
                vbar_width: 0.0,
            });
        }
        for p in &mut self.panels {
            p.sanitize();
        }
        if self.active_panel >= self.panels.len() {
            self.active_panel = self.panels.len() - 1;
        }
        // Guarantees a valid tree for the remaining panels (the removals
        // above may have reindexed the panels).
        let tree = self.layout_tree();
        self.layout = Some(tree);
        self
    }
}

// Named workspaces ----------
//
// Each named workspace is a `<id>.toml` file in `paths::workspaces_dir()`.
// `id` is a stable generated identifier (timestamp), distinct from the
// displayed `name` (freely editable by the user). The file contains the
// `name` + the complete state (`WorkspaceState`).

use std::path::PathBuf;

/// Serialized content of a named workspace.
///
/// `saved_at` = nanoseconds since the UNIX epoch at the last save
/// (creation **or** update), used to sort the list by recency. When the
/// field is absent, `#[serde(default)]` produces 0 and the sort then falls
/// back to the file's modification date.
/// TOML note: scalar fields (`name`, `saved_at`) MUST precede the
/// `state` table, otherwise they would get swallowed into `[state]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct NamedWorkspaceFile {
    name: String,
    #[serde(default)]
    saved_at: u64,
    /// Keyboard slot, 1 to 9, that loads this workspace directly. Absent for
    /// the many workspaces that carry none, and omitted from the file then, so
    /// an existing collection reads back unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slot: Option<u8>,
    /// Theme this workspace imposes while it is loaded, absent to follow the
    /// application's own choice. It sits beside the slot rather than inside
    /// the state because it is an attribute of the workspace, not part of the
    /// arrangement of views and tabs: setting it is therefore not a change to
    /// save, and it never makes the workspace read as modified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    state: WorkspaceState,
}

/// Highest keyboard slot a workspace can hold. The slots mirror the digit row,
/// so there is no tenth.
pub const MAX_WORKSPACE_SLOT: u8 = 9;

/// Is `slot` one a workspace may actually hold?
fn slot_is_valid(slot: u8) -> bool {
    (1..=MAX_WORKSPACE_SLOT).contains(&slot)
}

/// Lightweight metadata of a named workspace (for list display).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedWorkspaceMeta {
    /// Stable identifier = file name without extension.
    pub id: String,
    /// Displayed name, editable by the user.
    pub name: String,
    /// Number of views (for the metadata shown in the list).
    pub views: usize,
    /// Total number of tabs (sum over all views).
    pub tabs: usize,
    /// Date of the last save (epoch nanos) — sort key for recency.
    pub saved_at: u64,
    /// Keyboard slot that loads it, when it holds one.
    pub slot: Option<u8>,
    /// Theme it imposes while loaded, when it imposes one.
    pub theme: Option<String>,
}

/// Validates that an `id` is safe as a file name (anti path-traversal).
fn id_is_safe(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn workspace_file_path(dir: &Path, id: &str) -> Option<PathBuf> {
    if id_is_safe(id) {
        Some(dir.join(format!("{id}.toml")))
    } else {
        None
    }
}

/// Lists the named workspaces present in `dir`, sorted by **recency**
/// (last saved first). Ties are broken by name (case-insensitive).
/// Unreadable/corrupted files are ignored.
/// The caller (GUI) can reverse the order to offer "oldest first".
pub fn list_named_workspaces(dir: &Path) -> Vec<NamedWorkspaceMeta> {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Vec::new(), // missing folder = no workspace
    };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !id_is_safe(id) {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path)
            && let Ok(file) = toml::from_str::<NamedWorkspaceFile>(&content)
        {
            let panels = file.state.panels.len();
            let tabs = file.state.panels.iter().map(|p| p.tabs.len()).sum();
            // Recency: `saved_at`, falling back to the file's
            // modification date when the field is 0.
            let saved_at = if file.saved_at != 0 {
                file.saved_at
            } else {
                entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0)
            };
            out.push(NamedWorkspaceMeta {
                id: id.to_string(),
                name: file.name,
                views: panels,
                tabs,
                saved_at,
                // A value written by hand outside the range is read as no slot
                // rather than as an error: the collection stays loadable, and
                // the workspace simply answers to no shortcut.
                slot: file.slot.filter(|s| slot_is_valid(*s)),
                // Same reading for a theme naming neither a built-in family
                // nor a theme file: the workspace simply imposes none.
                theme: file.theme.filter(|id| crate::theme::id_is_known(id)),
            });
        }
    }
    // Most recent first; ties → name (case-insensitive).
    out.sort_by(|a, b| {
        b.saved_at
            .cmp(&a.saved_at)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// Nanoseconds (u64) since the UNIX epoch. Precision sufficient to
/// distinguish two close saves and sort by recency.
fn now_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Generates a stable identifier based on the clock (nanos since the epoch).
fn generate_id() -> String {
    format!("ws-{}", now_nanos())
}

/// Creates a new named workspace from `state`. Returns its `id`.
pub fn save_named_workspace(dir: &Path, name: &str, state: &WorkspaceState) -> Result<String> {
    std::fs::create_dir_all(dir)?;
    let id = generate_id();
    let file = NamedWorkspaceFile {
        name: name.trim().to_string(),
        saved_at: now_nanos(),
        slot: None,
        // A new workspace imposes no theme. Copying the one on screen would
        // create a choice nobody asked for, and after a few saves the
        // application's own theme would look as though it did nothing.
        theme: None,
        state: state.clone(),
    };
    write_named(dir, &id, &file)?;
    Ok(id)
}

/// Overwrites the state of an existing named workspace (keeps its `name`).
pub fn overwrite_named_workspace(dir: &Path, id: &str, state: &WorkspaceState) -> Result<()> {
    let mut file = read_named(dir, id)?;
    file.state = state.clone();
    file.saved_at = now_nanos(); // update → moves to the top of the list
    write_named(dir, id, &file)
}

/// Loads a named workspace. Returns `(name, sanitized state)`.
pub fn load_named_workspace(dir: &Path, id: &str) -> Result<(String, WorkspaceState)> {
    let file = read_named(dir, id)?;
    Ok((file.name, file.state.sanitized()))
}

/// Gives `id` the keyboard slot `slot`, or takes its slot away with `None`.
/// Returns the workspace that had to give the slot up, when there was one.
///
/// A slot addresses exactly one workspace, so handing it over takes it from
/// whoever held it: a shortcut with two answers would have to pick one of them
/// silently, and the list would show the same digit twice.
///
/// The previous holder is cleared BEFORE the new one is written. Should the
/// second write fail, the slot ends up held by nobody, which is visible in the
/// list and one click away from being fixed. The reverse order would leave two
/// holders, an inconsistency the rest of the code would then have to arbitrate.
///
/// Neither save date moves: assigning a shortcut is not a save, and the list is
/// ordered by recency.
pub fn set_workspace_slot(dir: &Path, id: &str, slot: Option<u8>) -> Result<Option<String>> {
    if let Some(value) = slot
        && !slot_is_valid(value)
    {
        return Err(crate::error::Error::Workspace(format!(
            "workspace slot out of range: {value}"
        )));
    }
    // Read first, so an unknown id fails before any other file is touched.
    let mut file = read_named(dir, id)?;
    let mut previous = None;
    if let Some(value) = slot {
        for meta in list_named_workspaces(dir) {
            if meta.slot != Some(value) || meta.id == id {
                continue;
            }
            let mut other = read_named(dir, &meta.id)?;
            other.slot = None;
            write_named(dir, &meta.id, &other)?;
            previous = Some(meta.id);
        }
    }
    file.slot = slot;
    write_named(dir, id, &file)?;
    Ok(previous)
}

/// Identifier of the workspace holding `slot`, if any.
pub fn workspace_with_slot(dir: &Path, slot: u8) -> Option<String> {
    list_named_workspaces(dir)
        .into_iter()
        .find(|meta| meta.slot == Some(slot))
        .map(|meta| meta.id)
}

/// Gives `id` the theme it imposes while loaded, or `None` to let it follow
/// the application's own choice.
///
/// Unlike a keyboard slot, a theme is held by as many workspaces as want it,
/// so nothing has to be taken from another holder. Neither save date moves:
/// choosing a theme is not a save, and the list is ordered by recency.
///
/// An unknown identifier is refused here rather than written and ignored at
/// the next read, which would look like the choice had been silently dropped.
pub fn set_workspace_theme(dir: &Path, id: &str, theme: Option<&str>) -> Result<()> {
    if let Some(value) = theme
        && !crate::theme::id_is_known(value)
    {
        return Err(crate::error::Error::Workspace(format!(
            "unknown workspace theme: {value:?}"
        )));
    }
    let mut file = read_named(dir, id)?;
    file.theme = theme.map(str::to_owned);
    write_named(dir, id, &file)
}

/// Renames a workspace (only changes the `name` field, keeps the `id`/file).
pub fn rename_named_workspace(dir: &Path, id: &str, new_name: &str) -> Result<()> {
    let mut file = read_named(dir, id)?;
    file.name = new_name.trim().to_string();
    write_named(dir, id, &file)
}

/// Deletes a named workspace's file.
pub fn delete_named_workspace(dir: &Path, id: &str) -> Result<()> {
    let path = workspace_file_path(dir, id)
        .ok_or_else(|| crate::error::Error::Workspace(format!("invalid workspace id: {id:?}")))?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn read_named(dir: &Path, id: &str) -> Result<NamedWorkspaceFile> {
    let path = workspace_file_path(dir, id)
        .ok_or_else(|| crate::error::Error::Workspace(format!("invalid workspace id: {id:?}")))?;
    let content = std::fs::read_to_string(&path)?;
    toml::from_str(&content)
        .map_err(|e| crate::error::Error::Workspace(format!("parse {}: {e}", path.display())))
}

fn write_named(dir: &Path, id: &str, file: &NamedWorkspaceFile) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = workspace_file_path(dir, id)
        .ok_or_else(|| crate::error::Error::Workspace(format!("invalid workspace id: {id:?}")))?;
    let text = toml::to_string_pretty(file)
        .map_err(|e| crate::error::Error::Workspace(format!("serialize: {e}")))?;
    crate::paths::write_atomic(&path, &text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> WorkspaceState {
        WorkspaceState {
            active_panel: 1,
            panels: vec![
                ViewState {
                    stretch: 1.5,
                    active_tab: 0,
                    tabs: vec![TabState {
                        path: "/home/u/Documents".into(),
                        sort_column: SortColumn::Size,
                        sort_order: SortOrder::Desc,
                        secondary_sort: None,
                        preview: false,
                        show_hidden: false,
                        group_mode: GroupMode::FoldersFirst,
                        zoom: None,
                        item_arrangement: ItemArrangement::List,
                        grid_zoom: None,
                    }],
                    columns: crate::columns::default_columns(),
                    tab_bar_mode: 0,
                    vbar_width: 0.0,
                },
                ViewState {
                    stretch: 1.0,
                    active_tab: 1,
                    tabs: vec![
                        TabState {
                            path: "/tmp".into(),
                            sort_column: SortColumn::Name,
                            sort_order: SortOrder::Asc,
                            secondary_sort: None,
                            preview: true,
                            show_hidden: false,
                            group_mode: GroupMode::FoldersFirst,
                            zoom: None,
                            item_arrangement: ItemArrangement::List,
                            grid_zoom: None,
                        },
                        TabState {
                            path: "/etc".into(),
                            sort_column: SortColumn::Modified,
                            sort_order: SortOrder::Desc,
                            secondary_sort: None,
                            preview: false,
                            show_hidden: false,
                            group_mode: GroupMode::FoldersFirst,
                            zoom: None,
                            item_arrangement: ItemArrangement::List,
                            grid_zoom: None,
                        },
                    ],
                    columns: crate::columns::default_columns(),
                    tab_bar_mode: 0,
                    vbar_width: 0.0,
                },
            ],
            layout: None,
            workspace_name: None,
            closed_tabs: Vec::new(),
            sidebar_sections: SidebarSectionsState::default(),
        }
    }

    #[test]
    fn round_trip_toml() {
        let ws = sample();
        let s = toml::to_string_pretty(&ws).unwrap();
        let back: WorkspaceState = toml::from_str(&s).unwrap();
        assert_eq!(back, ws);
    }

    #[test]
    fn secondary_sort_is_optional_and_round_trips_without_changing_legacy_keys() {
        let mut ws = sample();
        let old = toml::to_string_pretty(&ws).unwrap();
        assert!(!old.contains("secondary_sort"));
        let restored: WorkspaceState = toml::from_str(&old).unwrap();
        assert_eq!(restored.panels[0].tabs[0].secondary_sort, None);

        ws.panels[0].tabs[0].secondary_sort = Some(crate::fs::SecondarySort {
            column: SortColumn::Created,
            order: SortOrder::Desc,
            group_dates_by_day: true,
        });
        let written = toml::to_string_pretty(&ws).unwrap();
        let loaded: WorkspaceState = toml::from_str(&written).unwrap();
        assert_eq!(
            loaded.panels[0].tabs[0].secondary_sort,
            ws.panels[0].tabs[0].secondary_sort
        );
        assert!(written.contains("sort_column"));
        assert!(written.contains("sort_order"));
    }

    #[test]
    fn grid_state_is_optional_and_keeps_all_legacy_tab_keys() {
        let old = r#"path = "folder01"
preview = true
zoom = 5
"#;
        let saved: TabState = toml::from_str(old).unwrap();
        assert_eq!(saved.item_arrangement, ItemArrangement::List);
        assert_eq!(saved.grid_zoom, None);
        let old_written = toml::to_string(&saved).unwrap();
        assert!(!old_written.contains("item_arrangement"));
        assert!(!old_written.contains("grid_zoom"));

        let mut current = saved.clone();
        current.item_arrangement = ItemArrangement::Grid;
        current.grid_zoom = Some(3);
        let value = toml::Value::try_from(&current).unwrap();
        let table = value.as_table().unwrap();
        let expected: std::collections::BTreeSet<_> = [
            "path",
            "sort_column",
            "sort_order",
            "preview",
            "show_hidden",
            "group_mode",
            "zoom",
            "item_arrangement",
            "grid_zoom",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            table
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            expected
        );
        assert_eq!(table["path"].as_str(), Some("folder01"));
        assert_eq!(table["preview"].as_bool(), Some(true));
        assert_eq!(table["zoom"].as_integer(), Some(5));
        assert_eq!(table["item_arrangement"].as_str(), Some("grid"));
        assert_eq!(table["grid_zoom"].as_integer(), Some(3));
    }

    #[test]
    fn grid_state_round_trips_open_closed_and_named_workspaces() {
        for item_arrangement in [ItemArrangement::List, ItemArrangement::Grid] {
            for preview in [false, true] {
                let mut state = sample();
                let tab = &mut state.panels[0].tabs[0];
                tab.item_arrangement = item_arrangement;
                tab.preview = preview;
                tab.zoom = Some(5);
                tab.grid_zoom = Some(2);
                state.closed_tabs.push(tab.clone());
                let named = NamedWorkspaceFile {
                    name: "workspace01".into(),
                    saved_at: 1,
                    slot: None,
                    theme: None,
                    state,
                };
                let encoded = toml::to_string(&named).unwrap();
                let restored: NamedWorkspaceFile = toml::from_str(&encoded).unwrap();
                assert_eq!(restored, named);
            }
        }
    }

    #[test]
    fn grid_state_unknown_arrangement_only_falls_back_for_that_field() {
        let saved: TabState = toml::from_str(
            r#"
path = "folder01"
preview = true
zoom = 6
grid_zoom = 4
item_arrangement = "future"
"#,
        )
        .unwrap();
        assert_eq!(saved.item_arrangement, ItemArrangement::List);
        assert_eq!(saved.zoom, Some(6));
        assert_eq!(saved.grid_zoom, Some(4));
        assert!(saved.preview);
        assert!(toml::from_str::<TabState>("path = 'folder01'\nitem_arrangement = 42").is_err());
        assert_eq!(ItemArrangement::from_code("future"), ItemArrangement::List);
        for value in [ItemArrangement::List, ItemArrangement::Grid] {
            assert_eq!(ItemArrangement::from_code(value.code()), value);
        }
    }

    #[test]
    fn a_workspace_saved_before_the_zoom_existed_still_loads() {
        // Exactly what older files contain: a tab with no `zoom` key at all.
        // It must parse, and the absence has to stay distinguishable from a
        // stored level so the interface can fall back to the default for the
        // mode instead of snapping the view to some arbitrary size.
        let older = r#"
            active_panel = 0
            [[panels]]
            stretch = 1.0
            active_tab = 0
            [[panels.tabs]]
            path = "/tmp"
            sort_column = "name"
            sort_order = "asc"
            preview = true
            show_hidden = false
            group_mode = "foldersfirst"
        "#;
        let ws: WorkspaceState = toml::from_str(older).unwrap();
        assert_eq!(ws.panels[0].tabs[0].zoom, None);
        assert!(ws.panels[0].tabs[0].preview);

        // A file written today keeps the level through a full round trip.
        let mut current = ws.clone();
        current.panels[0].tabs[0].zoom = Some(5);
        let text = toml::to_string_pretty(&current).unwrap();
        let back: WorkspaceState = toml::from_str(&text).unwrap();
        assert_eq!(back.panels[0].tabs[0].zoom, Some(5));
    }

    /// The names a workspace file carries are a compatibility contract, not a
    /// matter of taste: a reader that cannot find `panels` fails outright,
    /// since the field has no default, and the user loses the arrangement and
    /// every named workspace.
    ///
    /// The vocabulary moved to "view" everywhere a user can see it, and some
    /// Rust names followed. These did not, and this test is what keeps them
    /// from following later by accident.
    #[test]
    fn the_persisted_names_are_pinned_whatever_the_rust_names_become() {
        let mut ws = sample();
        ws.layout = Some(LayoutNode::row_chain(&[1.0, 1.0]));
        let text = toml::to_string_pretty(&ws).unwrap();

        for name in ["active_panel", "[[panels]]", "[[panels.tabs]]", "panel = 0"] {
            assert!(
                text.contains(name),
                "the file no longer writes {name}; existing workspaces and \
                 older builds expect it:\n{text}"
            );
        }
        // And what it writes, it reads back unchanged.
        let back: WorkspaceState = toml::from_str(&text).unwrap();
        assert_eq!(back.active_panel, ws.active_panel);
        assert_eq!(back.panels.len(), ws.panels.len());
        assert_eq!(back.layout, ws.layout);
    }

    #[test]
    fn colored_folder_grouping_is_persisted() {
        let mut workspace = sample();
        workspace.panels[0].tabs[0].group_mode = GroupMode::ColoredFoldersFirst;

        let serialized = toml::to_string_pretty(&workspace).unwrap();
        assert!(serialized.contains("group_mode = \"coloredfoldersfirst\""));

        let restored: WorkspaceState = toml::from_str(&serialized).unwrap();
        assert_eq!(
            restored.panels[0].tabs[0].group_mode,
            GroupMode::ColoredFoldersFirst
        );
    }

    #[test]
    fn closed_tabs_are_backward_compatible_persisted_and_capped() {
        let serialized_without_history = toml::to_string_pretty(&sample()).unwrap();
        assert!(!serialized_without_history.contains("closed_tabs"));
        let restored: WorkspaceState = toml::from_str(&serialized_without_history).unwrap();
        assert!(restored.closed_tabs.is_empty());

        let mut ws = sample();
        ws.closed_tabs = (0..25)
            .map(|index| TabState {
                path: format!("/closed/{index}"),
                sort_column: SortColumn::Name,
                sort_order: SortOrder::Asc,
                secondary_sort: None,
                preview: false,
                show_hidden: false,
                group_mode: GroupMode::FoldersFirst,
                zoom: None,
                item_arrangement: ItemArrangement::List,
                grid_zoom: None,
            })
            .collect();

        let ws = ws.sanitized();
        assert_eq!(ws.closed_tabs.len(), MAX_CLOSED_TABS);
        assert_eq!(ws.closed_tabs.first().unwrap().path, "/closed/5");
        assert_eq!(ws.closed_tabs.last().unwrap().path, "/closed/24");

        let serialized = toml::to_string_pretty(&ws).unwrap();
        let restored: WorkspaceState = toml::from_str(&serialized).unwrap();
        assert_eq!(restored.closed_tabs, ws.closed_tabs);
    }

    #[test]
    fn sidebar_sections_are_backward_compatible_and_persisted() {
        // The historical "all expanded" state stays absent from the TOML and
        // comes back by default: no old workspace needs migration.
        let without_sidebar = toml::to_string_pretty(&sample()).unwrap();
        assert!(!without_sidebar.contains("sidebar_sections"));
        let restored: WorkspaceState = toml::from_str(&without_sidebar).unwrap();
        assert_eq!(restored.sidebar_sections, SidebarSectionsState::default());

        let mut ws = sample();
        ws.sidebar_sections = SidebarSectionsState {
            shortcuts_collapsed: true,
            favorites_collapsed: false,
            drives_collapsed: true,
            network_collapsed: true,
        };
        let serialized = toml::to_string_pretty(&ws).unwrap();
        assert!(serialized.contains("sidebar_sections"));
        assert!(!serialized.contains("\norder ="));
        let restored: WorkspaceState = toml::from_str(&serialized).unwrap();
        assert_eq!(restored.sidebar_sections, ws.sidebar_sections);

        // Compatibility with the short-lived version that persisted the order
        // per workspace: Serde ignores this old field, the collapse states remain intact.
        let legacy = serialized.replacen(
            "[sidebar_sections]\n",
            "[sidebar_sections]\norder = [\"network\", \"drives\", \"shortcuts\", \"favorites\"]\n",
            1,
        );
        let restored_legacy: WorkspaceState = toml::from_str(&legacy).unwrap();
        assert_eq!(restored_legacy.sidebar_sections, ws.sidebar_sections);

        // Same guarantee through the real envelope of a named workspace
        // (`name` + `saved_at` + `[state]`), whose TOML structure is more
        // nested than the session file.
        let dir = temp_dir("sidebar-sections");
        let id = save_named_workspace(&dir, "Sidebar", &ws).unwrap();
        let (_, restored) = load_named_workspace(&dir, &id).unwrap();
        assert_eq!(restored.sidebar_sections, ws.sidebar_sections);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn save_load_file() {
        let dir = std::env::temp_dir().join(format!(
            "favnyr-ws-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("workspace.toml");

        // Missing file → None.
        assert!(WorkspaceState::load(&path).unwrap().is_none());

        sample().save(&path).unwrap();
        let loaded = WorkspaceState::load(&path).unwrap().unwrap();
        // `load` sanitizes, which materializes the migrated `layout`.
        assert_eq!(loaded, sample().sanitized());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sanitized_clamps_out_of_bounds() {
        let ws = WorkspaceState {
            active_panel: 99,
            panels: vec![ViewState {
                stretch: -3.0,  // invalid
                active_tab: 42, // out of bounds
                tabs: vec![TabState {
                    path: "/x".into(),
                    sort_column: SortColumn::Name,
                    sort_order: SortOrder::Asc,
                    secondary_sort: None,
                    preview: false,
                    show_hidden: false,
                    group_mode: GroupMode::FoldersFirst,
                    zoom: None,
                    item_arrangement: ItemArrangement::List,
                    grid_zoom: None,
                }],
                columns: crate::columns::default_columns(),
                tab_bar_mode: 0,
                vbar_width: 0.0,
            }],
            layout: None,
            workspace_name: None,
            closed_tabs: Vec::new(),
            sidebar_sections: SidebarSectionsState::default(),
        }
        .sanitized();
        assert_eq!(ws.active_panel, 0);
        assert_eq!(ws.panels[0].active_tab, 0);
        assert_eq!(ws.panels[0].stretch, 1.0);
    }

    #[test]
    fn sanitized_drops_empty_panels() {
        let ws = WorkspaceState {
            active_panel: 0,
            panels: vec![
                ViewState {
                    stretch: 1.0,
                    active_tab: 0,
                    tabs: vec![],
                    columns: crate::columns::default_columns(),
                    tab_bar_mode: 0,
                    vbar_width: 0.0,
                },
                ViewState {
                    stretch: 1.0,
                    active_tab: 0,
                    tabs: vec![TabState {
                        path: "/y".into(),
                        sort_column: SortColumn::Name,
                        sort_order: SortOrder::Asc,
                        secondary_sort: None,
                        preview: false,
                        show_hidden: false,
                        group_mode: GroupMode::FoldersFirst,
                        zoom: None,
                        item_arrangement: ItemArrangement::List,
                        grid_zoom: None,
                    }],
                    columns: crate::columns::default_columns(),
                    tab_bar_mode: 0,
                    vbar_width: 0.0,
                },
            ],
            layout: None,
            workspace_name: None,
            closed_tabs: Vec::new(),
            sidebar_sections: SidebarSectionsState::default(),
        }
        .sanitized();
        assert_eq!(ws.panels.len(), 1);
        assert_eq!(ws.panels[0].tabs[0].path, "/y");
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "favnyr-named-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_keyboard_slot_belongs_to_one_workspace_and_survives_a_save() {
        let dir = temp_dir("slot");
        let first = save_named_workspace(&dir, "First", &sample()).unwrap();
        let second = save_named_workspace(&dir, "Second", &sample()).unwrap();

        // A fresh workspace answers to no shortcut.
        let slot_of = |dir: &std::path::Path, id: &str| {
            list_named_workspaces(dir)
                .into_iter()
                .find(|meta| meta.id == id)
                .and_then(|meta| meta.slot)
        };
        assert_eq!(slot_of(&dir, &first), None);

        assert_eq!(set_workspace_slot(&dir, &first, Some(3)).unwrap(), None);
        assert_eq!(slot_of(&dir, &first), Some(3));
        assert_eq!(
            workspace_with_slot(&dir, 3).as_deref(),
            Some(first.as_str())
        );

        // Saving the workspace again must not cost it its shortcut: the two are
        // independent, which is the whole point of storing the slot per file.
        overwrite_named_workspace(&dir, &first, &sample()).unwrap();
        assert_eq!(slot_of(&dir, &first), Some(3));
        rename_named_workspace(&dir, &first, "Renamed").unwrap();
        assert_eq!(slot_of(&dir, &first), Some(3));

        // Handing the slot over takes it from its holder, and says so.
        assert_eq!(
            set_workspace_slot(&dir, &second, Some(3))
                .unwrap()
                .as_deref(),
            Some(first.as_str())
        );
        assert_eq!(slot_of(&dir, &first), None);
        assert_eq!(slot_of(&dir, &second), Some(3));
        assert_eq!(
            workspace_with_slot(&dir, 3).as_deref(),
            Some(second.as_str())
        );

        // Clearing leaves the shortcut answering nobody.
        assert_eq!(set_workspace_slot(&dir, &second, None).unwrap(), None);
        assert_eq!(slot_of(&dir, &second), None);
        assert_eq!(workspace_with_slot(&dir, 3), None);

        // Out of range is refused rather than stored.
        assert!(set_workspace_slot(&dir, &first, Some(0)).is_err());
        assert!(set_workspace_slot(&dir, &first, Some(10)).is_err());
        assert_eq!(slot_of(&dir, &first), None);

        // A file written without the field, and one written with a value out
        // of range, both read back as no shortcut instead of failing.
        let path = dir.join(format!("{first}.toml"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("slot"));
        std::fs::write(
            &path,
            text.replace(
                "name =",
                "slot = 42
name =",
            ),
        )
        .unwrap();
        assert_eq!(slot_of(&dir, &first), None);

        let _ = std::fs::remove_dir_all(dir);
    }

    /// A theme sits beside the slot and behaves like it: independent of the
    /// arrangement, untouched by a save, and held by as many workspaces as
    /// want it, since nothing makes it exclusive.
    #[test]
    fn a_workspace_theme_is_independent_of_its_arrangement() {
        let dir = temp_dir("theme");
        let first = save_named_workspace(&dir, "First", &sample()).unwrap();
        let second = save_named_workspace(&dir, "Second", &sample()).unwrap();
        let theme_of = |dir: &std::path::Path, id: &str| {
            list_named_workspaces(dir)
                .into_iter()
                .find(|meta| meta.id == id)
                .and_then(|meta| meta.theme)
        };

        // A fresh workspace imposes nothing, and the field is omitted from its
        // file, so a collection written before this existed reads back as is.
        assert_eq!(theme_of(&dir, &first), None);
        let path = dir.join(format!("{first}.toml"));
        assert!(!std::fs::read_to_string(&path).unwrap().contains("theme"));

        // Both kinds of identifier are storable, the base family included:
        // its own identifier is absent, which is why it needs its alias.
        for id in ["builtin:base", "builtin:atom", "user:my_theme.toml"] {
            set_workspace_theme(&dir, &first, Some(id)).unwrap();
            assert_eq!(theme_of(&dir, &first).as_deref(), Some(id));
        }

        // Saving and renaming leave it alone: it is not part of the state.
        overwrite_named_workspace(&dir, &first, &sample()).unwrap();
        assert_eq!(
            theme_of(&dir, &first).as_deref(),
            Some("user:my_theme.toml")
        );
        rename_named_workspace(&dir, &first, "Renamed").unwrap();
        assert_eq!(
            theme_of(&dir, &first).as_deref(),
            Some("user:my_theme.toml")
        );

        // Nothing is exclusive: a second workspace takes the same theme
        // without the first losing it.
        set_workspace_theme(&dir, &second, Some("user:my_theme.toml")).unwrap();
        assert_eq!(
            theme_of(&dir, &first).as_deref(),
            Some("user:my_theme.toml")
        );
        assert_eq!(
            theme_of(&dir, &second).as_deref(),
            Some("user:my_theme.toml")
        );

        // Clearing returns the workspace to the application's own choice.
        set_workspace_theme(&dir, &first, None).unwrap();
        assert_eq!(theme_of(&dir, &first), None);

        // An unknown identifier is refused rather than stored and then
        // ignored, which would look like the choice had been dropped.
        for id in ["builtin:unknown", "user:../my_theme.toml", "my_theme"] {
            assert!(set_workspace_theme(&dir, &first, Some(id)).is_err(), "{id}");
            assert_eq!(theme_of(&dir, &first), None);
        }

        // A value written by hand that names nothing reads back as no theme,
        // leaving the workspace loadable.
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            text.replace("name =", "theme = \"builtin:nonsense\"\nname ="),
        )
        .unwrap();
        assert_eq!(theme_of(&dir, &first), None);
        assert!(load_named_workspace(&dir, &first).is_ok());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn named_workspace_crud() {
        let dir = temp_dir("crud");

        // Empty list at the start.
        assert!(list_named_workspaces(&dir).is_empty());

        // Save → appears in the list.
        let id = save_named_workspace(&dir, "  My workspace  ", &sample()).unwrap();
        let list = list_named_workspaces(&dir);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "My workspace"); // trim applied
        assert_eq!(list[0].id, id);
        // Counters: sample() = 2 panels, 1+2 = 3 tabs.
        assert_eq!(list[0].views, 2);
        assert_eq!(list[0].tabs, 3);

        // Load → name + state returned (sanitized: migrated layout materialized).
        let (name, state) = load_named_workspace(&dir, &id).unwrap();
        assert_eq!(name, "My workspace");
        assert_eq!(state, sample().sanitized());

        // Rename → name changed, id stable.
        rename_named_workspace(&dir, &id, "Renamed").unwrap();
        let list = list_named_workspaces(&dir);
        assert_eq!(list[0].name, "Renamed");
        assert_eq!(list[0].id, id);

        // Overwrite → state replaced, name kept.
        let other = WorkspaceState {
            active_panel: 0,
            panels: vec![ViewState {
                stretch: 1.0,
                active_tab: 0,
                tabs: vec![TabState {
                    path: "/var".into(),
                    sort_column: SortColumn::Name,
                    sort_order: SortOrder::Asc,
                    secondary_sort: None,
                    preview: false,
                    show_hidden: false,
                    group_mode: GroupMode::FoldersFirst,
                    zoom: None,
                    item_arrangement: ItemArrangement::List,
                    grid_zoom: None,
                }],
                columns: crate::columns::default_columns(),
                tab_bar_mode: 0,
                vbar_width: 0.0,
            }],
            layout: None,
            workspace_name: None,
            closed_tabs: Vec::new(),
            sidebar_sections: SidebarSectionsState::default(),
        };
        overwrite_named_workspace(&dir, &id, &other).unwrap();
        let (name, state) = load_named_workspace(&dir, &id).unwrap();
        assert_eq!(name, "Renamed"); // name preserved
        assert_eq!(state.panels.len(), 1);
        assert_eq!(state.panels[0].tabs[0].path, "/var");

        // Delete → empty list.
        delete_named_workspace(&dir, &id).unwrap();
        assert!(list_named_workspaces(&dir).is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Was this refused BECAUSE the id is unusable, rather than merely failing
    /// because nothing was there?
    ///
    /// The distinction carries the test. The paths these ids build do not
    /// exist, so a plain `is_err()` holds just as well with the guard removed —
    /// it would then be reading "file not found" and calling it a rejection.
    /// Only the cause tells the two apart: the guard answers `Workspace`, a
    /// missing file answers `Io`.
    fn refused_as_invalid_id<T>(result: crate::error::Result<T>) -> bool {
        match result {
            Err(crate::error::Error::Workspace(message)) => {
                message.starts_with("invalid workspace id")
            }
            _ => false,
        }
    }

    #[test]
    fn rejects_unsafe_id() {
        let dir = temp_dir("unsafe");
        // An id becomes a file path, so one that walks out of the folder — or
        // simply carries a separator — must be turned away before it reaches
        // the filesystem.
        assert!(refused_as_invalid_id(load_named_workspace(
            &dir,
            "../outside"
        )));
        assert!(refused_as_invalid_id(delete_named_workspace(&dir, "a/b")));
        assert!(!id_is_safe(""));
        assert!(!id_is_safe("a.b"));
        assert!(id_is_safe("ws-12345"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_sorted_newest_first() {
        let dir = temp_dir("sort");
        // Small sleep between each save → strictly increasing `saved_at`
        // (deterministic, no nanosecond collision).
        save_named_workspace(&dir, "Zeta", &sample()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        save_named_workspace(&dir, "alpha", &sample()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        let beta_id = save_named_workspace(&dir, "Beta", &sample()).unwrap();
        // By default: the most recently saved comes first.
        let names: Vec<_> = list_named_workspaces(&dir)
            .into_iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(names, ["Beta", "alpha", "Zeta"]);

        // An update (overwrite) moves the workspace to the top.
        std::thread::sleep(std::time::Duration::from_millis(3));
        overwrite_named_workspace(&dir, &beta_id, &sample()).unwrap(); // Beta already at the top
        save_named_workspace(&dir, "Gamma", &sample()).unwrap();
        let top = list_named_workspaces(&dir)
            .into_iter()
            .next()
            .map(|m| m.name)
            .unwrap();
        assert_eq!(top, "Gamma");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_rejects_empty_panels_as_none() {
        let dir = std::env::temp_dir().join(format!(
            "favnyr-ws-empty-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("workspace.toml");
        std::fs::write(&path, "active_panel = 0\npanels = []\n").unwrap();
        assert!(WorkspaceState::load(&path).unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn layout_migrated_from_flat_panels() {
        // A workspace without `layout` produces a valid chain covering all
        // the panels.
        let ws = sample(); // layout: None, 2 panels
        let tree = ws.layout_tree();
        assert!(tree.is_valid_for(2));
        assert_eq!(tree.leaf_indices(), vec![0, 1]);

        // After sanitize, the layout is materialized and valid.
        let s = ws.sanitized();
        assert!(s.layout.is_some());
        assert!(s.layout.as_ref().unwrap().is_valid_for(s.panels.len()));
    }

    #[test]
    fn stored_valid_layout_survives_round_trip() {
        let mut ws = sample();
        ws.layout = Some(LayoutNode::Split {
            dir: crate::layout::SplitDir::Column,
            ratio: 0.4,
            first: Box::new(LayoutNode::Leaf { panel: 0 }),
            second: Box::new(LayoutNode::Leaf { panel: 1 }),
        });
        let text = toml::to_string_pretty(&ws).unwrap();
        assert!(text.contains("layout"));
        let back: WorkspaceState = toml::from_str(&text).unwrap();
        assert_eq!(back.layout, ws.layout);
        // The stored tree (Column) is kept by layout_tree (it is valid).
        assert!(matches!(
            back.layout_tree(),
            LayoutNode::Split {
                dir: crate::layout::SplitDir::Column,
                ..
            }
        ));
    }

    #[test]
    fn invalid_stored_layout_falls_back_to_chain() {
        let mut ws = sample(); // 2 panels
        // Layout referencing only panel 0 → invalid for 2 panels.
        ws.layout = Some(LayoutNode::Leaf { panel: 0 });
        let tree = ws.layout_tree();
        assert!(tree.is_valid_for(2)); // rebuilt into a valid chain
        assert_eq!(tree.leaf_indices(), vec![0, 1]);
    }
}
