//! Event-driven retries for failed views and unavailable favorites.

use super::*;

const VIEW_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const FAVORITE_RETRY_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(super) struct State {
    running: Cell<bool>,
    requested: Cell<bool>,
    last_started: Cell<Option<Instant>>,
    timer: slint::Timer,
    deliveries: Arc<Mutex<Vec<RecoveredFavorite>>>,
}

#[derive(Clone)]
struct FavoriteTarget {
    id: String,
    path: String,
}

struct RecoveredFavorite {
    target: FavoriteTarget,
    is_dir: bool,
    icon: Option<(Vec<u8>, u32, u32)>,
}

pub(super) fn can_retry(panel: &Panel, now: Instant) -> bool {
    panel.listing_failed
        && panel.tabs.group_page().is_none()
        && !panel.pending_initial
        && !panel.pending_listing
        && panel.search.is_none()
        && panel
            .last_recovery
            .is_none_or(|last| now.duration_since(last) >= VIEW_RETRY_INTERVAL)
}

pub(super) fn retry_view(window: &MainWindow, state: &AppState, index: usize, authenticate: bool) {
    if shutdown::stopping(state) || window.get_popup_overlay_open() {
        return;
    }
    let now = Instant::now();
    let path = {
        let mut panels = state.panels.borrow_mut();
        let Some(panel) = panels.get_mut(index) else {
            return;
        };
        if !can_retry(panel, now) {
            return;
        }
        panel.last_recovery = Some(now);
        panel.tabs.tabs[panel.tabs.active].current_path.clone()
    };
    let filter = if index == *state.active_panel.borrow() {
        state.filter.borrow().clone()
    } else {
        String::new()
    };
    // Recovery never probes the path on the UI thread, including a mount
    // whose network filesystem is not yet registered by the OS.
    request_async_panel_listing(window, state, index, &path, filter, authenticate);
}

fn targets(model: &ModelRc<FavNode>) -> Vec<FavoriteTarget> {
    model
        .iter()
        .filter(needs_recovery)
        .map(|row| FavoriteTarget {
            id: row.id.to_string(),
            path: row.path.to_string(),
        })
        .collect()
}

fn needs_recovery(row: &FavNode) -> bool {
    !row.is_container && !row.available && !row.path.is_empty()
}

fn matches_target(row: &FavNode, target: &FavoriteTarget) -> bool {
    !row.is_container && !row.available && row.id == target.id && row.path == target.path
}

/// One worker at a time; events received during a scan become one follow-up.
/// No timer probes the filesystem without an actual recovery event.
pub(super) fn request_favorites(window: &MainWindow, state: &AppState) {
    if shutdown::stopping(state)
        || !window
            .get_fav_nodes()
            .iter()
            .any(|row| needs_recovery(&row))
    {
        return;
    }
    let recovery = &state.recovery;
    recovery.requested.set(true);
    if recovery.running.get() || recovery.timer.running() {
        return;
    }
    let wait = recovery
        .last_started
        .get()
        .map(|last| FAVORITE_RETRY_INTERVAL.saturating_sub(last.elapsed()))
        .unwrap_or_default();
    if !wait.is_zero() {
        let weak = window.as_weak();
        let st = state.clone();
        recovery
            .timer
            .start(slint::TimerMode::SingleShot, wait, move || {
                if let Some(w) = weak.upgrade() {
                    start_favorites(&w, &st);
                }
            });
    } else {
        start_favorites(window, state);
    }
}

fn start_favorites(window: &MainWindow, state: &AppState) {
    let recovery = &state.recovery;
    recovery.requested.set(false);
    if shutdown::stopping(state) {
        return;
    }
    let candidates = targets(&window.get_fav_nodes());
    if candidates.is_empty() {
        return;
    }
    recovery.running.set(true);
    recovery.last_started.set(Some(Instant::now()));
    let deliveries = recovery.deliveries.clone();
    let weak = window.as_weak();
    if let Err(error) = std::thread::Builder::new()
        .name("favorite-recovery".into())
        .spawn(move || {
            #[cfg(windows)]
            let apartment = crate::winutil::ComApartment::sta();
            let recovered: Vec<_> = candidates
                .into_iter()
                .filter_map(|target| {
                    let metadata = std::fs::metadata(&target.path).ok()?;
                    let is_dir = metadata.is_dir();
                    #[cfg(windows)]
                    let can_extract = apartment.is_some();
                    #[cfg(not(windows))]
                    let can_extract = true;
                    let icon = if !is_dir
                        && can_extract
                        && crate::cloud_content_guard::may_read_automatically(Path::new(
                            &target.path,
                        )) {
                        openwith::icon_rgba_for_path(&target.path, false)
                    } else {
                        None
                    };
                    Some(RecoveredFavorite {
                        target,
                        is_dir,
                        icon,
                    })
                })
                .collect();
            *deliveries.lock().unwrap_or_else(|p| p.into_inner()) = recovered;
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(w) = weak.upgrade() {
                    w.invoke_recovery_drain();
                }
            });
        })
    {
        recovery.running.set(false);
        error!(%error, "cannot start favorite recovery worker");
    }
}

fn apply_favorites(window: &MainWindow, state: &AppState) {
    let recovery = &state.recovery;
    let recovered = std::mem::take(
        &mut *recovery
            .deliveries
            .lock()
            .unwrap_or_else(|p| p.into_inner()),
    );
    recovery.running.set(false);
    let model = window.get_fav_nodes();
    if !recovered.is_empty() {
        apply_recovered(&model, recovered, &annotations_now(state));
    }
    if recovery.requested.get() {
        request_favorites(window, state);
    }
}

fn apply_recovered(
    model: &ModelRc<FavNode>,
    recovered: Vec<RecoveredFavorite>,
    annotations: &favnyr_core::annotations::AnnotationStore,
) {
    for result in recovered {
        let index = (0..model.row_count()).find(|&index| {
            model
                .row_data(index)
                .is_some_and(|row| matches_target(&row, &result.target))
        });
        let Some(index) = index else { continue };
        let Some(mut row) = model.row_data(index) else {
            continue;
        };
        let path = Path::new(&result.target.path);
        row.available = true;
        row.is_dir = result.is_dir;
        let icon = result.icon.map(|(rgba, width, height)| {
            image_from_thumb(&Thumbnail {
                rgba,
                width,
                height,
            })
        });
        // Forget stale failures for both sizes, then prime the small icon.
        // Even a failed extraction is cached so application never reopens the
        // remote executable while applying this worker's result.
        SELF_ICON_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            cache.remove(&(result.target.path.clone(), true));
            cache.insert((result.target.path.clone(), false), icon);
        });
        row.icon = if result.is_dir {
            Image::default()
        } else {
            self_icon(path, &ext_of(path), false)
        };
        row.folder_slot = favorite_folder_slot(&row, annotations);
        model.set_row_data(index, row);
    }
}

pub(super) fn install(window: &MainWindow, state: &AppState) {
    let st = state.clone();
    let weak = window.as_weak();
    window.on_recovery_activated(move || {
        if let Some(w) = weak.upgrade() {
            request_favorites(&w, &st);
        }
    });
    let st = state.clone();
    let weak = window.as_weak();
    window.on_recovery_drain(move || {
        if let Some(w) = weak.upgrade() {
            apply_favorites(&w, &st);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_view_retry_skips_pending_and_repeated_requests() {
        let mut panel = Panel::with_mode(
            PathBuf::from("folder01"),
            columns::default_columns(),
            0,
            false,
        );
        let now = Instant::now();
        assert!(!can_retry(&panel, now));
        panel.listing_failed = true;
        assert!(can_retry(&panel, now));
        panel.pending_listing = true;
        assert!(!can_retry(&panel, now));
        panel.pending_listing = false;
        panel.pending_initial = true;
        assert!(!can_retry(&panel, now));
        panel.pending_initial = false;
        panel.last_recovery = Some(now);
        assert!(!can_retry(&panel, now + Duration::from_millis(500)));
        assert!(can_retry(&panel, now + VIEW_RETRY_INTERVAL));
    }

    #[test]
    fn favorite_recovery_ignores_removed_replaced_or_already_recovered_rows() {
        let mut row = FavNode {
            id: "favorite01".into(),
            path: "folder01/my_file.exe".into(),
            ..Default::default()
        };
        let target = FavoriteTarget {
            id: row.id.to_string(),
            path: row.path.to_string(),
        };
        assert!(matches_target(&row, &target));
        row.available = true;
        assert!(!matches_target(&row, &target));
        row.available = false;
        row.path = "folder02/my_file.exe".into();
        assert!(!matches_target(&row, &target));
        row.path = target.path.as_str().into();
        row.id = "favorite02".into();
        assert!(!matches_target(&row, &target));
    }

    #[test]
    fn recovered_file_updates_the_current_row_and_replaces_a_cached_icon_failure() {
        let target = FavoriteTarget {
            id: "favorite01".into(),
            path: "folder01/my_file.exe".into(),
        };
        let rows = Rc::new(VecModel::from(vec![
            FavNode {
                id: "favorite02".into(),
                label: "Folder".into(),
                available: true,
                ..Default::default()
            },
            FavNode {
                id: target.id.as_str().into(),
                path: target.path.as_str().into(),
                label: "Application".into(),
                depth: 2,
                ..Default::default()
            },
        ]));
        let model: ModelRc<FavNode> = rows.clone().into();
        SELF_ICON_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .insert((target.path.clone(), false), None);
            cache.borrow_mut().insert((target.path.clone(), true), None);
        });
        apply_recovered(
            &model,
            vec![RecoveredFavorite {
                target: target.clone(),
                is_dir: false,
                icon: Some((vec![80, 160, 40, 255], 1, 1)),
            }],
            &Default::default(),
        );
        let row = rows.row_data(1).unwrap();
        assert!(row.available);
        assert!(!row.is_dir);
        assert_eq!(row.label, "Application");
        assert_eq!(row.depth, 2);
        assert_eq!(row.icon.size().width, 1);
        assert_eq!(rows.row_data(0).unwrap().id, "favorite02");
        SELF_ICON_CACHE.with(|cache| {
            assert!(!cache.borrow().contains_key(&(target.path.clone(), true)));
            cache.borrow_mut().remove(&(target.path, false));
        });
    }
}
