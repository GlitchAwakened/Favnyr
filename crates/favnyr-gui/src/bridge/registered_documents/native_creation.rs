//! Short-lived, non-destructive selection after a native document command.

use super::*;
use notify::event::{CreateKind, ModifyKind, RenameMode};

const SETTLE: Duration = Duration::from_millis(350);
const DEADLINE: Duration = Duration::from_secs(60);
const MAX_CANDIDATES: usize = 16;

#[derive(Default)]
struct Events {
    paths: Vec<PathBuf>,
    failed: bool,
}

impl Events {
    fn forget(&mut self, removed: &[PathBuf]) -> bool {
        let before = self.paths.len();
        self.paths.retain(|path| {
            !removed
                .iter()
                .any(|removed| ops::paths_equal(path, removed))
        });
        before != self.paths.len()
    }

    fn record(&mut self, event: &Event, folder: &Path, extensions: &[String]) -> bool {
        let relevant = |path: &Path| {
            path.parent()
                .is_some_and(|parent| ops::paths_equal(parent, folder))
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        let name = name.to_lowercase();
                        extensions.iter().any(|extension| name.ends_with(extension))
                    })
        };
        let mut changed = false;
        let mut add = None;
        match event.kind {
            EventKind::Create(CreateKind::File | CreateKind::Any) => add = event.paths.last(),
            EventKind::Modify(ModifyKind::Name(RenameMode::To | RenameMode::Both)) => {
                if matches!(
                    event.kind,
                    EventKind::Modify(ModifyKind::Name(RenameMode::Both))
                ) && let Some(from) = event.paths.first()
                {
                    changed |= self.forget(std::slice::from_ref(from));
                }
                add = event.paths.last();
            }
            EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                changed |= self.forget(&event.paths);
            }
            EventKind::Modify(_) => {
                return event.paths.iter().any(|changed| {
                    self.paths
                        .iter()
                        .any(|path| ops::paths_equal(path, changed))
                });
            }
            _ => {}
        }
        if let Some(path) = add.filter(|path| relevant(path))
            && !self
                .paths
                .iter()
                .any(|existing| ops::paths_equal(existing, path))
        {
            if self.paths.len() >= MAX_CANDIDATES {
                self.failed = true;
            } else {
                self.paths.push(path.to_owned());
                changed = true;
            }
        }
        self.failed || changed
    }
}

pub(super) struct Observation {
    _watcher: RecommendedWatcher,
    events: Arc<Mutex<Events>>,
    active: Arc<AtomicBool>,
    queued: Arc<AtomicBool>,
}

impl Observation {
    pub(super) fn start(window: &MainWindow, folder: &Path, extension: &str) -> Option<Self> {
        // Skip known network paths to avoid an extra synchronous share handle.
        if rfs::is_unc_path(folder) || favnyr_core::places::is_network_path(folder) {
            return None;
        }
        let extensions: Vec<_> = extension.split(';').map(str::to_lowercase).collect();
        if extensions.is_empty()
            || extensions.len() > 16
            || extensions.iter().any(|extension| {
                !extension.starts_with('.')
                    || extension.len() < 2
                    || ops::check_file_name(&format!("File01{extension}")).is_err()
            })
        {
            return None;
        }
        let events = Arc::new(Mutex::new(Events::default()));
        let active = Arc::new(AtomicBool::new(true));
        let queued = Arc::new(AtomicBool::new(false));
        let (output, running, dispatched) = (events.clone(), active.clone(), queued.clone());
        let weak = window.as_weak();
        let parent = folder.to_owned();
        let mut watcher = RecommendedWatcher::new(
            move |result: notify::Result<Event>| {
                if !running.load(Ordering::Relaxed) {
                    return;
                }
                let changed = {
                    let mut output = output.lock().unwrap_or_else(|error| error.into_inner());
                    match result {
                        Ok(event) if event.need_rescan() => {
                            output.failed = true;
                            true
                        }
                        Ok(event) => output.record(&event, &parent, &extensions),
                        Err(error) => {
                            debug!(%error, "native document selection watcher failed");
                            output.failed = true;
                            true
                        }
                    }
                };
                if changed && !dispatched.swap(true, Ordering::Relaxed) {
                    let weak = weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = weak.upgrade() {
                            window.invoke_document_created_changed();
                        }
                    });
                }
            },
            notify::Config::default(),
        )
        .ok()?;
        watcher.watch(folder, RecursiveMode::NonRecursive).ok()?;
        Some(Self {
            _watcher: watcher,
            events,
            active,
            queued,
        })
    }
}

impl Drop for Observation {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Relaxed);
    }
}

struct Pending {
    location: Location,
    before: Selection,
    observation: Observation,
}

#[derive(PartialEq)]
pub(super) struct Selection {
    paths: Vec<PathBuf>,
    filter: String,
}

impl Selection {
    pub(super) fn capture(state: &AppState) -> Self {
        Self {
            paths: selected_paths(state),
            filter: state.filter.borrow().clone(),
        }
    }
}

#[derive(Default)]
pub(super) struct Followup {
    pending: RefCell<Option<Pending>>,
    settle: slint::Timer,
    expires: slint::Timer,
}

impl Followup {
    pub(super) fn install(window: &MainWindow, state: &AppState) -> Rc<Self> {
        let followup = Rc::new(Self::default());
        let target = followup.clone();
        let weak = window.as_weak();
        let state = state.clone();
        window.on_document_created_changed(move || {
            if let Some(window) = weak.upgrade() {
                target.schedule(&window, &state);
            } else {
                target.cancel();
            }
        });
        followup
    }

    pub(super) fn cancel(&self) {
        self.settle.stop();
        self.expires.stop();
        self.pending.borrow_mut().take();
    }

    pub(super) fn start(
        self: &Rc<Self>,
        window: &MainWindow,
        state: &AppState,
        location: Location,
        before: Selection,
        observation: Observation,
    ) {
        self.cancel();
        *self.pending.borrow_mut() = Some(Pending {
            location,
            before,
            observation,
        });
        let target = Rc::downgrade(self);
        self.expires
            .start(slint::TimerMode::SingleShot, DEADLINE, move || {
                if let Some(target) = target.upgrade() {
                    target.cancel();
                }
            });
        self.schedule(window, state);
    }

    fn schedule(self: &Rc<Self>, window: &MainWindow, state: &AppState) {
        let pending = self.pending.borrow();
        let Some(pending) = pending.as_ref() else {
            return;
        };
        pending.observation.queued.store(false, Ordering::Relaxed);
        let target = Rc::downgrade(self);
        let weak = window.as_weak();
        let state = state.clone();
        self.settle
            .start(slint::TimerMode::SingleShot, SETTLE, move || {
                let Some(target) = target.upgrade() else {
                    return;
                };
                let Some(window) = weak.upgrade() else {
                    target.cancel();
                    return;
                };
                target.finish(&window, &state);
            });
    }

    fn finish(&self, window: &MainWindow, state: &AppState) {
        let Some(pending) = self.pending.borrow_mut().take() else {
            return;
        };
        let (failed, paths) = {
            let events = pending
                .observation
                .events
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            (events.failed, events.paths.clone())
        };
        let current = !failed
            && !shutdown::stopping(state)
            && pending.location.is_current(state)
            && !window.get_op_busy()
            && !window.get_file_drag_active()
            && !window.get_create_popup_open()
            && pending.before.filter == *state.filter.borrow();
        if !current || paths.len() > 1 {
            self.cancel();
            return;
        }
        if paths.is_empty() {
            *self.pending.borrow_mut() = Some(pending);
            return;
        }
        self.expires.stop();
        let path = &paths[0];
        if Selection::capture(state) != pending.before
            || !std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
        {
            return;
        }
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            // Candidate identification only changes selection. It never names,
            // renames, deletes or rewrites any file reported by another process.
            reveal_created_entry(window, state, &pending.location.folder, name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a Windows text document registration"]
    fn a_real_native_creation_reports_the_actual_file_to_the_selection_tracker() {
        crate::test_support::run(|| {
            let folder = crate::test_support::TestFolder::new();
            let window = MainWindow::new().unwrap();
            let observation = Observation::start(&window, &folder.0, ".txt").unwrap();
            let (menu, entries) = crate::shellmenu::build_for_folder(&folder.0).unwrap();
            let entry = entries
                .iter()
                .find(|entry| menu.extension(entry.id).unwrap() == ".txt")
                .unwrap();
            menu.invoke(entry.id).unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while observation.events.lock().unwrap().paths.is_empty() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            let events = observation.events.lock().unwrap();
            assert!(!events.failed);
            let actual: Vec<_> = std::fs::read_dir(&folder.0)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            assert_eq!(actual.len(), 1);
            assert_eq!(events.paths.len(), 1);
            assert!(ops::paths_equal(&events.paths[0], &actual[0]));
            drop(events);
            drop(observation);
            assert!(Observation::start(&window, &folder.0, "invalid").is_none());
        });
    }

    fn event(kind: EventKind, paths: &[&str]) -> Event {
        let mut event = Event::new(kind);
        for path in paths {
            event.paths.push(PathBuf::from(path));
        }
        event
    }

    #[test]
    fn creation_tracking_ignores_content_changes_and_unrelated_files() {
        let mut events = Events::default();
        let folder = Path::new("folder01");
        let types = vec![".bin".into()];
        for unrelated in [
            event(EventKind::Modify(ModifyKind::Any), &["folder01/File01.bin"]),
            event(
                EventKind::Create(CreateKind::Folder),
                &["folder01/folder02.bin"],
            ),
            event(
                EventKind::Create(CreateKind::File),
                &["folder02/File01.bin"],
            ),
            event(
                EventKind::Create(CreateKind::File),
                &["folder01/File01.txt"],
            ),
        ] {
            assert!(!events.record(&unrelated, folder, &types));
        }
        let created = event(
            EventKind::Create(CreateKind::File),
            &["folder01/File01.BIN"],
        );
        assert!(events.record(&created, folder, &types));
        assert!(!events.record(&created, folder, &types));
        assert_eq!(events.paths.len(), 1);
    }

    #[test]
    fn creation_tracking_follows_temporary_renames_and_preserves_ambiguity() {
        let mut events = Events::default();
        let folder = Path::new("folder01");
        let types = vec![".bin".into()];
        events.record(
            &event(
                EventKind::Create(CreateKind::File),
                &["folder01/File01.bin"],
            ),
            folder,
            &types,
        );
        events.record(
            &event(
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                &["folder01/File01.bin", "folder01/File02.bin"],
            ),
            folder,
            &types,
        );
        assert_eq!(events.paths, [PathBuf::from("folder01/File02.bin")]);
        events.record(
            &event(
                EventKind::Create(CreateKind::File),
                &["folder01/File03.bin"],
            ),
            folder,
            &types,
        );
        assert_eq!(events.paths.len(), 2);
        events.record(
            &event(
                EventKind::Remove(notify::event::RemoveKind::File),
                &["folder01/File02.bin"],
            ),
            folder,
            &types,
        );
        assert_eq!(events.paths, [PathBuf::from("folder01/File03.bin")]);
    }

    #[test]
    fn excessive_candidates_disable_selection_instead_of_growing_the_event_buffer() {
        let mut events = Events::default();
        for index in 0..=MAX_CANDIDATES {
            let path = format!("folder01/File{index:02}.bin");
            events.record(
                &event(EventKind::Create(CreateKind::File), &[&path]),
                Path::new("folder01"),
                &[".bin".into()],
            );
        }
        assert!(events.failed);
        assert_eq!(events.paths.len(), MAX_CANDIDATES);
    }
}
