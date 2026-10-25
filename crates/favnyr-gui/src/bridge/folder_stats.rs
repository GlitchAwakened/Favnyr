//! Shared background folder statistics and in-memory date sorting.
use super::*;

struct Task {
    path: PathBuf,
    row: usize,
    mtime_depth: u32,
    size_depth: u32,
}

pub(super) struct Job {
    generation: u64,
    panel: usize,
    directory: PathBuf,
    tasks: Vec<Task>,
}

struct Value {
    path: PathBuf,
    row: usize,
    mtime: Option<i64>,
    size: Option<u64>,
}

struct Delivery {
    generation: u64,
    panel: usize,
    directory: PathBuf,
    value: Value,
    finished: bool,
}

pub(super) fn remember_entries(state: &AppState, panel: &mut Panel, entries: &[Entry]) {
    panel.folder_stat_entries = if state.config.borrow().recursive_mtime_depth > 0
        && date_sort(panel.tabs.tabs[panel.tabs.active].sort)
        && !favnyr_core::places::is_network_path(&panel.tabs.tabs[panel.tabs.active].current_path)
    {
        entries.to_vec()
    } else {
        Vec::new()
    };
}

fn apply_mtime(row: &mut FileRow, mtime: i64, now: i64, lang: Lang) {
    row.modified = rfs::format_mtime(mtime, mtime_offset()).into();
    row.age = rfs::format_age(mtime, now, i18n::age_units(lang)).into();
    row.age_bucket = rfs::age_bucket(mtime, now);
}

/// The generation is checked on receipt as well as on the worker. A completed
/// walk can otherwise arrive after a refresh or after the option was disabled.
fn delivery_matches_panel(panel: &Panel, delivery: &Delivery, generation: u64) -> bool {
    delivery.generation == generation
        && panel.search.is_none()
        && !panel.pending_listing
        && !panel.pending_initial
        && panel.displayed_path == delivery.directory
        && panel.tabs.tabs[panel.tabs.active].current_path == delivery.directory
}

fn apply_delivery(state: &AppState, delivery: &Delivery) -> bool {
    if delivery.generation != state.rmtime_gen.load(Ordering::Relaxed) {
        return false;
    }
    let panels = state.panels.borrow();
    let Some(panel) = panels.get(delivery.panel) else {
        return false;
    };
    if !delivery_matches_panel(panel, delivery, state.rmtime_gen.load(Ordering::Relaxed)) {
        return false;
    }
    let config = state.config.borrow();
    let mut mtimes = state.rmtime_cache.borrow_mut();
    let mut sizes = state.size_cache.borrow_mut();
    let value = &delivery.value;
    let key = value.path.to_string_lossy().into_owned();
    if config.recursive_mtime_depth > 0
        && let Some(mtime) = value.mtime
    {
        mtimes.insert(key.clone(), mtime);
    }
    if config.recursive_size_depth > 0
        && let Some(size) = value.size
    {
        sizes.insert(key, size);
    }
    // Most results keep their row index. Check the path, and fall back to a
    // lookup only if an in-memory reorder moved the folder during the walk.
    let matches = |row: &FileRow| row.is_dir && row_path(row) == value.path;
    let index = if panel
        .rows_model
        .row_data(value.row)
        .as_ref()
        .is_some_and(matches)
    {
        Some(value.row)
    } else {
        panel.rows_model.iter().position(|row| matches(&row))
    };
    if let Some(index) = index
        && let Some(mut row) = panel.rows_model.row_data(index)
    {
        if config.recursive_mtime_depth > 0
            && let Some(mtime) = value.mtime
        {
            apply_mtime(&mut row, mtime, now_unix(), config.language);
        }
        if config.recursive_size_depth > 0
            && let Some(size) = value.size
        {
            row.size = rfs::format_size(size, i18n::size_units(config.language)).into();
        }
        panel.rows_model.set_row_data(index, row);
    }
    true
}

pub(super) fn connect(window: &MainWindow, state: &AppState) {
    let Some(rx) = state.rmtime_rx.borrow_mut().take() else {
        return;
    };
    let (delivery_tx, delivery_rx) = mpsc::channel::<Delivery>();
    let weak = window.as_weak();
    let st = state.clone();
    window.on_folder_stats_drain(move || {
        let Some(window) = weak.upgrade() else { return };
        while let Ok(delivery) = delivery_rx.try_recv() {
            if apply_delivery(&st, &delivery) && delivery.finished {
                finish_sort(&window, &st, delivery.panel, delivery.generation);
            }
        }
    });
    let generation = state.rmtime_gen.clone();
    let weak = window.as_weak();
    std::thread::spawn(move || {
        while let Ok(job) = rx.recv() {
            let count = job.tasks.len();
            for (index, task) in job.tasks.into_iter().enumerate() {
                if job.generation != generation.load(Ordering::Relaxed) {
                    break;
                }
                let (mtime, size) =
                    rfs::recursive_folder_stats(&task.path, task.mtime_depth, task.size_depth);
                if job.generation != generation.load(Ordering::Relaxed) {
                    break;
                }
                // Publish each folder promptly: a slow later walk must not
                // hide completed values. Only the final delivery may sort.
                if delivery_tx
                    .send(Delivery {
                        generation: job.generation,
                        panel: job.panel,
                        directory: job.directory.clone(),
                        value: Value {
                            path: task.path,
                            row: task.row,
                            mtime,
                            size,
                        },
                        finished: index + 1 == count,
                    })
                    .is_err()
                {
                    return;
                }
                let weak = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = weak.upgrade() {
                        window.invoke_folder_stats_drain();
                    }
                });
            }
        }
    });
}

fn date_sort(sort: SortState) -> bool {
    matches!(sort.column, SortColumn::Modified | SortColumn::Age)
        || sort.secondary.is_some_and(|secondary| {
            matches!(secondary.column, SortColumn::Modified | SortColumn::Age)
        })
}

/// Reuses the core sorter, including secondary criteria and date grouping.
/// Only row order changes: images, selection and computed values stay intact.
fn resort_panel(
    panel: &mut Panel,
    cache: &HashMap<String, i64>,
    manual_order: bool,
    config: &Config,
) -> bool {
    let tab = &panel.tabs.tabs[panel.tabs.active];
    if panel.search.is_some()
        || panel.pending_listing
        || panel.pending_initial
        || panel.displayed_path != tab.current_path
        || panel.folder_stat_entries.is_empty()
        || !date_sort(tab.sort)
        || manual_order
    {
        return false;
    }
    let mut entries = panel.folder_stat_entries.clone();
    for entry in &mut entries {
        if entry.is_dir
            && let Some(&mtime) = cache.get(entry.path.to_string_lossy().as_ref())
        {
            entry.mtime_unix = Some(mtime);
        }
    }
    rfs::sort_with_secondary(
        &mut entries,
        tab.sort.column,
        tab.sort.order,
        tab.group_mode,
        tab.sort.secondary,
        mtime_offset(),
    );
    let mut rows_by_path: HashMap<_, _> = panel
        .rows_model
        .iter()
        .map(|row| (row_path(&row), row))
        .collect();
    // Do not apply an order from a different listing or a changed filter.
    if entries.len() != rows_by_path.len()
        || entries
            .iter()
            .any(|entry| !rows_by_path.contains_key(&entry.path))
    {
        return false;
    }
    let mut rows: Vec<_> = entries
        .iter()
        .filter_map(|entry| rows_by_path.remove(&entry.path))
        .collect();
    if tab.group_mode == GroupMode::ColoredFoldersFirst {
        group_colored_folder_rows(&mut rows);
    }
    replace_reordered_rows(
        panel,
        rows,
        config.compact_icon_rows_in_preview,
        config.view_item_spacing,
    )
}

/// A drag or context action may still reference row indices. Defer only the
/// reorder until it finishes; the cached values can be displayed immediately.
fn finish_sort(window: &MainWindow, state: &AppState, panel_index: usize, generation: u64) {
    if generation != state.rmtime_gen.load(Ordering::Relaxed)
        || state.config.borrow().recursive_mtime_depth == 0
    {
        return;
    }
    let needs_sort = state.panels.borrow().get(panel_index).is_some_and(|panel| {
        date_sort(panel.tabs.tabs[panel.tabs.active].sort) && !panel.folder_stat_entries.is_empty()
    });
    if !needs_sort {
        return;
    }
    if window.get_rows_interaction_busy() {
        let weak = window.as_weak();
        let st = state.clone();
        slint::Timer::single_shot(Duration::from_millis(100), move || {
            if let Some(window) = weak.upgrade() {
                finish_sort(&window, &st, panel_index, generation);
            }
        });
        return;
    }
    let changed = {
        let mut panels = state.panels.borrow_mut();
        panels.get_mut(panel_index).is_some_and(|panel| {
            let manual =
                folder_state_now(state).is_manual(&panel.tabs.tabs[panel.tabs.active].current_path);
            resort_panel(
                panel,
                &state.rmtime_cache.borrow(),
                manual,
                &state.config.borrow(),
            )
        })
    };
    if changed {
        update_panels_ui(window, state);
        request_thumbnails(state);
        request_imgmeta(state);
    }
}

/// One job per panel computes its missing statistics, publishing progressively
/// and sorting once on completion. Date and size share one walk per folder.
pub(super) fn request_folder_stats(window: &MainWindow, state: &AppState) {
    // Invalidate deliveries even when both options have just been disabled.
    let generation = state.rmtime_gen.fetch_add(1, Ordering::Relaxed) + 1;
    let (mtime_depth, size_depth, lang) = {
        let config = state.config.borrow();
        (
            config.recursive_mtime_depth.max(0) as u32,
            config.recursive_size_depth.max(0) as u32,
            config.language,
        )
    };
    if mtime_depth == 0 {
        for panel in state.panels.borrow_mut().iter_mut() {
            panel.folder_stat_entries.clear();
        }
    }
    if mtime_depth == 0 && size_depth == 0 {
        return;
    }
    let now = now_unix();
    let mut cached_panels = Vec::new();
    {
        let panels = state.panels.borrow();
        let mtimes = state.rmtime_cache.borrow();
        let sizes = state.size_cache.borrow();
        for (panel_index, panel) in panels.iter().enumerate() {
            let directory = &panel.tabs.tabs[panel.tabs.active].current_path;
            if panel.search.is_some()
                || panel.pending_initial
                || panel.pending_listing
                || panel.displayed_path != *directory
                || favnyr_core::places::is_network_path(directory)
            {
                continue;
            }
            let mut tasks = Vec::new();
            for index in 0..panel.rows_model.row_count() {
                let Some(mut row) = panel.rows_model.row_data(index) else {
                    continue;
                };
                if !row.is_dir {
                    continue;
                }
                let path = row_path(&row);
                let key = path.to_string_lossy();
                let mtime = (mtime_depth > 0)
                    .then(|| mtimes.get(key.as_ref()))
                    .flatten();
                let size = (size_depth > 0).then(|| sizes.get(key.as_ref())).flatten();
                if let Some(&mtime) = mtime {
                    apply_mtime(&mut row, mtime, now, lang);
                }
                if let Some(&size) = size {
                    row.size = rfs::format_size(size, i18n::size_units(lang)).into();
                }
                if mtime.is_some() || size.is_some() {
                    panel.rows_model.set_row_data(index, row);
                }
                let missing_mtime = if mtime.is_none() { mtime_depth } else { 0 };
                let missing_size = if size.is_none() { size_depth } else { 0 };
                if missing_mtime > 0 || missing_size > 0 {
                    tasks.push(Task {
                        path,
                        row: index,
                        mtime_depth: missing_mtime,
                        size_depth: missing_size,
                    });
                }
            }
            if tasks.is_empty() {
                cached_panels.push(panel_index);
            } else {
                let _ = state.rmtime_tx.send(Job {
                    generation,
                    panel: panel_index,
                    directory: directory.clone(),
                    tasks,
                });
            }
        }
    }
    for panel in cached_panels {
        finish_sort(window, state, panel, generation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Panel, HashMap<String, i64>, Config) {
        let directory = PathBuf::from("folder00");
        let mut panel = Panel::with_mode(directory.clone(), columns::default_columns(), 0, false);
        panel.tabs.tabs[0].group_mode = GroupMode::Mixed;
        panel.tabs.tabs[0].sort = SortState {
            column: SortColumn::Modified,
            order: SortOrder::Asc,
            secondary: None,
        };
        let entries: Vec<_> = [
            ("folder01", true, 300),
            ("folder02", true, 100),
            ("my_file.txt", false, 20),
        ]
        .into_iter()
        .map(|(name, is_dir, mtime)| Entry {
            name: name.into(),
            path: directory.join(name),
            size_bytes: (!is_dir).then_some(4),
            mtime_unix: Some(mtime),
            created_unix: Some(1),
            is_dir,
            kind: if is_dir {
                FileKind::Folder
            } else {
                FileKind::File
            },
            hidden: false,
            is_symlink: false,
            is_share: false,
            executable: false,
        })
        .collect();
        let rows = entries
            .iter()
            .map(|entry| FileRow {
                name: entry.name.clone().into(),
                path: "folder00".into(),
                is_dir: entry.is_dir,
                selected: entry.name == "folder02",
                cut: !entry.is_dir,
                modified: "computed value".into(),
                ..FileRow::default()
            })
            .collect();
        panel.replace_rows(rows);
        panel.tabs.tabs[0].selection_anchor = 1;
        panel.tabs.tabs[0].cursor = 2;
        let cache = [("folder01", 10), ("folder02", 30)]
            .into_iter()
            .map(|(name, date)| (directory.join(name).to_string_lossy().into_owned(), date))
            .collect();
        panel.folder_stat_entries = entries;
        (panel, cache, Config::default())
    }

    fn names(panel: &Panel) -> Vec<String> {
        panel
            .rows_model
            .iter()
            .map(|row| row.name.to_string())
            .collect()
    }

    #[test]
    fn recursive_date_display_can_replace_a_newer_directory_date() {
        let mut row = FileRow {
            is_dir: true,
            ..FileRow::default()
        };
        let now = 1_700_000_000;
        for timestamp in [now, 1_200_000_000, 1_000_000_000] {
            apply_mtime(&mut row, timestamp, now, Lang::En);
            assert_eq!(
                row.modified.as_str(),
                rfs::format_mtime(timestamp, mtime_offset())
            );
            assert_eq!(
                row.age.as_str(),
                rfs::format_age(timestamp, now, i18n::age_units(Lang::En))
            );
            assert_eq!(row.age_bucket, rfs::age_bucket(timestamp, now));
        }
    }

    #[test]
    fn recursive_date_sort_preserves_selection_cursor_and_computed_rows() {
        for column in [SortColumn::Modified, SortColumn::Age] {
            let (mut panel, cache, config) = fixture();
            panel.tabs.tabs[0].sort.column = column;
            assert!(resort_panel(&mut panel, &cache, false, &config));
            assert_eq!(names(&panel), ["folder01", "my_file.txt", "folder02"]);
            assert_eq!(panel.tabs.tabs[0].selection_anchor, 2);
            assert_eq!(panel.tabs.tabs[0].cursor, 1);
            assert!(panel.rows_model.row_data(2).unwrap().selected);
            assert!(panel.rows_model.row_data(1).unwrap().cut);
            assert_eq!(
                panel.rows_model.row_data(2).unwrap().modified,
                "computed value"
            );
            assert!(!resort_panel(&mut panel, &cache, false, &config));
            panel.tabs.tabs[0].sort.order = SortOrder::Desc;
            assert!(resort_panel(&mut panel, &cache, false, &config));
            assert_eq!(names(&panel), ["folder02", "my_file.txt", "folder01"]);
        }
    }

    #[test]
    fn recursive_date_sort_reuses_secondary_sort_and_day_grouping() {
        let (mut panel, mut cache, config) = fixture();
        panel.tabs.tabs[0].sort = SortState {
            column: SortColumn::Created,
            order: SortOrder::Asc,
            secondary: Some(SecondarySort {
                column: SortColumn::Age,
                order: SortOrder::Desc,
                group_dates_by_day: false,
            }),
        };
        assert!(resort_panel(&mut panel, &cache, false, &config));
        assert_eq!(names(&panel), ["folder02", "my_file.txt", "folder01"]);
        for entry in &mut panel.folder_stat_entries {
            entry.mtime_unix = Some(1_000_000_000);
            if entry.is_dir {
                cache.insert(entry.path.to_string_lossy().into_owned(), 1_000_000_001);
            }
        }
        panel.tabs.tabs[0].sort = SortState {
            column: SortColumn::Modified,
            order: SortOrder::Asc,
            secondary: Some(SecondarySort {
                column: SortColumn::Name,
                order: SortOrder::Desc,
                group_dates_by_day: true,
            }),
        };
        assert!(resort_panel(&mut panel, &cache, false, &config));
        assert_eq!(names(&panel), ["my_file.txt", "folder02", "folder01"]);
    }

    #[test]
    fn recursive_date_sort_respects_manual_order_colours_and_listing_identity() {
        let (mut panel, cache, config) = fixture();
        let original = names(&panel);
        assert!(!resort_panel(&mut panel, &cache, true, &config));
        assert_eq!(names(&panel), original);
        panel.tabs.tabs[0].group_mode = GroupMode::ColoredFoldersFirst;
        let mut row = panel.rows_model.row_data(1).unwrap();
        row.folder_slot = 2;
        panel.rows_model.set_row_data(1, row);
        assert!(resort_panel(&mut panel, &cache, false, &config));
        assert_eq!(names(&panel), ["folder02", "folder01", "my_file.txt"]);
        panel.folder_stat_entries.pop();
        assert!(!resort_panel(&mut panel, &cache, false, &config));
    }

    #[test]
    fn folder_stat_deliveries_require_current_generation_and_directory() {
        let (mut panel, _, _) = fixture();
        let delivery = Delivery {
            generation: 4,
            panel: 0,
            directory: PathBuf::from("folder00"),
            value: Value {
                path: PathBuf::from("folder00/folder01"),
                row: 0,
                mtime: Some(10),
                size: None,
            },
            finished: true,
        };
        assert!(delivery_matches_panel(&panel, &delivery, 4));
        assert!(!delivery_matches_panel(&panel, &delivery, 5));
        panel.pending_listing = true;
        assert!(!delivery_matches_panel(&panel, &delivery, 4));
        panel.pending_listing = false;
        panel.pending_initial = true;
        assert!(!delivery_matches_panel(&panel, &delivery, 4));
        panel.pending_initial = false;
        panel.tabs.tabs[0].current_path = PathBuf::from("folder03");
        assert!(!delivery_matches_panel(&panel, &delivery, 4));
        panel.tabs.tabs[0].current_path = PathBuf::from("folder00");
        panel.displayed_path = PathBuf::from("folder03");
        assert!(!delivery_matches_panel(&panel, &delivery, 4));
    }
}
