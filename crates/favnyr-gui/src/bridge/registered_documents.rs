//! Cached Windows document choices bind to the current folder only on invocation.

use super::*;
use crate::shellmenu::{DocumentMenuSession, DocumentTemplate, build_for_folder};

mod catalog;
mod discovery;
mod native_creation;

use catalog::{Cache, Catalog};

struct Location {
    book: tabs::BookId,
    tab: tabs::TabId,
    folder: PathBuf,
}

impl Location {
    fn capture(book: &TabBook) -> Self {
        Self {
            book: book.identity,
            tab: book.tabs[book.active].id,
            folder: book.tabs[book.active].current_path.clone(),
        }
    }

    fn matches(&self, book: &TabBook) -> bool {
        book.identity == self.book
            && book.group_page().is_none()
            && book.tabs[book.active].id == self.tab
            && ops::paths_equal(&book.tabs[book.active].current_path, &self.folder)
    }

    fn is_current(&self, state: &AppState) -> bool {
        documents_enabled(&state.config.borrow())
            && !search::active(state)
            && state.with_tabs(|book| self.matches(book))
    }
}

struct Context {
    location: Location,
    catalog: Rc<Catalog>,
}

struct Pending {
    location: Location,
    template: DocumentTemplate,
}

fn documents_enabled(config: &Config) -> bool {
    config.shell_ctx_menu && config.registered_file_types
}

pub(super) fn install(window: &MainWindow, state: &AppState) {
    let context = Rc::new(RefCell::new(None::<Context>));
    let pending = Rc::new(RefCell::new(None::<Pending>));
    let cache = Rc::new(Cache::default());
    let discovery = discovery::Discovery::install(window, state, cache.clone(), context.clone());
    let epoch = Rc::new(Cell::new(0u64));
    let followup = native_creation::Followup::install(window, state);
    let slot = context.clone();
    let st = state.clone();
    let weak = window.as_weak();
    let stop_followup = followup.clone();
    let cached = cache.clone();
    let serial = epoch.clone();
    let loader = discovery.clone();
    window.on_document_new_prepare(move || {
        let request = serial.get().wrapping_add(1);
        serial.set(request);
        stop_followup.cancel();
        let previous = slot.borrow_mut().take();
        drop(previous);
        loader.release();
        let Some(window) = weak.upgrade() else { return };
        window.set_new_documents(ModelRc::default());
        if !documents_enabled(&st.config.borrow())
            || shutdown::stopping(&st)
            || search::active(&st)
            || st.with_tabs(|book| book.group_page().is_some())
        {
            return;
        }
        let location = st.with_tabs(Location::capture);
        match cached.get() {
            Some(catalog) => {
                if serial.get() != request
                    || !window.get_new_menu_open()
                    || shutdown::stopping(&st)
                    || !location.is_current(&st)
                {
                    return;
                }
                let rows = catalog.rows.clone();
                let previous = slot.borrow_mut().replace(Context { location, catalog });
                drop(previous);
                window.set_new_documents(rows);
            }
            None => loader.request(location),
        }
    });

    let slot = context.clone();
    let weak = window.as_weak();
    let serial = epoch.clone();
    let loader = discovery.clone();
    window.on_document_new_release(move || {
        loader.release();
        serial.set(serial.get().wrapping_add(1));
        let previous = slot.borrow_mut().take();
        drop(previous);
        if let Some(window) = weak.upgrade() {
            window.set_new_documents(ModelRc::default());
        }
    });

    let cached = cache.clone();
    let slot = context.clone();
    let draft = pending.clone();
    let stop_followup = followup.clone();
    let st = state.clone();
    let weak = window.as_weak();
    let serial = epoch.clone();
    window.on_registered_types_changed(move |enabled| {
        st.persist_config(|config| config.registered_file_types = enabled);
        info!(enabled, "registered document types toggled");
        cached.clear();
        discovery.release();
        serial.set(serial.get().wrapping_add(1));
        slot.borrow_mut().take();
        if !enabled {
            stop_followup.cancel();
            draft.borrow_mut().take();
        }
        if let Some(window) = weak.upgrade() {
            window.set_new_documents(ModelRc::default());
            if !enabled && window.get_create_kind() == 4 {
                window.set_create_popup_open(false);
            }
        }
    });

    let st = state.clone();
    let weak = window.as_weak();
    let draft = pending.clone();
    let serial = epoch;
    window.on_document_new_run(move |command| {
        followup.cancel();
        // Closing a menu releases its context. Keep the displayed catalogue,
        // and hold no RefCell borrow while Windows pumps its own messages.
        let selected = context.borrow_mut().take();
        let Some(window) = weak.upgrade() else { return };
        window.invoke_ctx_close();
        let request = serial.get();
        let Some(context) = selected else { return };
        if shutdown::stopping(&st) || !context.location.is_current(&st) {
            return;
        }
        let Some(key) = context.catalog.key(command) else {
            return;
        };
        match DocumentTemplate::for_extension(&key.verb) {
            Ok(Some(template)) => {
                if serial.get() != request
                    || shutdown::stopping(&st)
                    || !context.location.is_current(&st)
                {
                    return;
                }
                let base = i18n::tr(st.config.borrow().language, "ctx_new_file");
                let name =
                    proposed_name(
                        &base,
                        &template.extension,
                        |name| match std::fs::symlink_metadata(context.location.folder.join(name)) {
                            Ok(_) => Some(false),
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                Some(!st.ops.is_reserved(&context.location.folder.join(name)))
                            }
                            Err(_) => None,
                        },
                    );
                *draft.borrow_mut() = Some(Pending {
                    location: context.location,
                    template,
                });
                window.set_create_kind(4);
                window.set_create_name(name.into());
                window.set_create_target(SharedString::new());
                window.set_create_popup_open(true);
                return;
            }
            Ok(None) => {}
            Err(error) => {
                debug!(%error, command, "document recipe unavailable; using native command")
            }
        }
        if serial.get() != request || shutdown::stopping(&st) || !context.location.is_current(&st) {
            return;
        }
        let (menu, entries) = match build_for_folder(&context.location.folder) {
            Ok(menu) => menu,
            Err(error) => {
                error!(%error, "registered document menu could not be rebuilt");
                show_file_failure(
                    &window,
                    st.config.borrow().language,
                    "create_failed",
                    &context.location.folder,
                    error.as_ref(),
                );
                return;
            }
        };
        if serial.get() != request || shutdown::stopping(&st) || !context.location.is_current(&st) {
            return;
        }
        let current = match Catalog::read(&menu, &entries) {
            Ok(catalog) => catalog,
            Err(error) => {
                error!(%error, "registered document commands could not be identified");
                show_file_failure(
                    &window,
                    st.config.borrow().language,
                    "create_failed",
                    &context.location.folder,
                    error.as_ref(),
                );
                return;
            }
        };
        if serial.get() != request || shutdown::stopping(&st) || !context.location.is_current(&st) {
            return;
        }
        let Some(index) = context.catalog.resolve(command, &current) else {
            cache.replace(current);
            show_notice(
                &window,
                i18n::tr(st.config.borrow().language, "new_documents_changed"),
            );
            return;
        };
        let command = entries[index].id;
        let selected_before = native_creation::Selection::capture(&st);
        let observation = if window.get_op_busy() {
            None
        } else {
            menu.extension(command).ok().and_then(|extension| {
                native_creation::Observation::start(&window, &context.location.folder, &extension)
            })
        };
        if serial.get() != request || shutdown::stopping(&st) || !context.location.is_current(&st) {
            return;
        }
        match menu.invoke(command) {
            Ok(()) => {
                info!(command, "registered document command invoked");
                if let Some(observation) = observation {
                    followup.start(&window, &st, context.location, selected_before, observation);
                }
            }
            Err(error) if cancelled(&error) => {
                debug!(command, "registered document creation cancelled")
            }
            Err(error) => {
                error!(%error, command, "registered document creation failed");
                let lang = st.config.borrow().language;
                show_notice(
                    &window,
                    i18n::file_operation_failed(
                        lang,
                        "create_failed",
                        &key.label,
                        &i18n::file_error_reason(lang, file_error_kind(error.as_ref())),
                    ),
                );
            }
        }
        // Windows handles templates, unique names and optional application UI.
        // A later application-created file is also picked up by the watcher.
        window.invoke_refresh_all();
    });

    let draft = pending.clone();
    window.on_document_create_release(move || {
        draft.borrow_mut().take();
    });

    let st = state.clone();
    let weak = window.as_weak();
    window.on_document_create_confirmed(move |name| {
        let Some(window) = weak.upgrade() else {
            return false;
        };
        let Some(draft) = pending.borrow_mut().take() else {
            return false;
        };
        if shutdown::stopping(&st) || !draft.location.is_current(&st) {
            window.set_create_popup_open(false);
            return false;
        }
        let name = name.trim();
        let lang = st.config.borrow().language;
        let availability = entry_name_availability_now(&st, &draft.location.folder, name);
        if availability != EntryNameAvailability::Available {
            push_create_name_status(&window, lang, name, availability);
            *pending.borrow_mut() = Some(draft);
            return false;
        }
        match draft.template.create(&draft.location.folder, name) {
            Ok(path) => {
                info!(path = %path.display(), "registered document created");
                reveal_created_entry(&window, &st, &draft.location.folder, name);
                true
            }
            Err(error) => {
                error!(%error, "registered document creation failed");
                let availability = entry_name_availability_now(&st, &draft.location.folder, name);
                if availability != EntryNameAvailability::Available {
                    push_create_name_status(&window, lang, name, availability);
                } else {
                    show_file_failure(
                        &window,
                        lang,
                        "create_failed",
                        &draft.location.folder.join(name),
                        error.as_ref(),
                    );
                }
                *pending.borrow_mut() = Some(draft);
                false
            }
        }
    });
}

fn proposed_name(
    base: &str,
    extension: &str,
    mut available: impl FnMut(&str) -> Option<bool>,
) -> String {
    let plain = format!("{base}{extension}");
    if ops::check_file_name(&plain).is_err() || available(&plain) != Some(false) {
        return plain;
    }
    // Keep probing bounded on unavailable folders. The normal live validation
    // still rejects the final proposal if no available name can be found.
    for index in 2..=1000 {
        let candidate = format!("{base} ({index}){extension}");
        match available(&candidate) {
            Some(true) => return candidate,
            Some(false) => {}
            None => return plain,
        }
    }
    plain
}

fn cancelled(error: &anyhow::Error) -> bool {
    use windows::Win32::Foundation::{E_ABORT, ERROR_CANCELLED};
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<windows::core::Error>())
        .any(|error| {
            error.code() == E_ABORT
                || error.code() == windows::core::HRESULT::from_win32(ERROR_CANCELLED.0)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_types_respect_both_the_dedicated_and_master_preferences() {
        for master in [false, true] {
            for enabled in [false, true] {
                let config = Config {
                    shell_ctx_menu: master,
                    registered_file_types: enabled,
                    ..Config::default()
                };
                assert_eq!(documents_enabled(&config), master && enabled);
            }
        }
    }

    #[test]
    fn a_proposed_document_name_avoids_collisions_without_repeated_failed_queries() {
        let occupied = ["File01.bin", "File01 (2).bin"];
        assert_eq!(
            proposed_name("File01", ".bin", |name| Some(!occupied.contains(&name))),
            "File01 (3).bin"
        );
        let mut queries = 0;
        assert_eq!(
            proposed_name("File01", ".bin", |_| {
                queries += 1;
                None
            }),
            "File01.bin"
        );
        assert_eq!(queries, 1);
    }

    #[test]
    fn the_registered_document_popup_keeps_caret_before_extension_and_releases_cancelled_drafts() {
        use slint::platform::{Key, WindowEvent};
        crate::test_support::run_with_windows(|windows| {
            let window = MainWindow::new().unwrap();
            window
                .window()
                .set_size(slint::LogicalSize::new(900., 720.));
            window
                .global::<crate::Strings>()
                .set_ctx_new_file("New file".into());
            window
                .global::<crate::TextMetrics>()
                .on_rename_caret_offset(|name, is_dir| {
                    filename_caret_offset(name.as_str(), is_dir)
                });
            window.on_create_check(|_, _| {});
            let releases = Rc::new(Cell::new(0));
            let count = releases.clone();
            window.on_document_create_release(move || count.set(count.get() + 1));
            window.show().unwrap();
            window
                .window()
                .dispatch_event(WindowEvent::WindowActiveChanged(true));
            let adapter = windows.borrow()[0].clone();
            let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 720];
            let press = |text: SharedString| {
                window
                    .window()
                    .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
                window
                    .window()
                    .dispatch_event(WindowEvent::KeyReleased { text });
            };
            for (index, (kind, initial, edited)) in [
                (0, "folder01", "folder01_02"),
                (1, "File01é.txt", "File01é_02.txt"),
                (4, "File01é.txt", "File01é_02.txt"),
            ]
            .into_iter()
            .enumerate()
            {
                window.set_create_kind(kind);
                window.set_create_name(initial.into());
                window.set_create_name_valid(true);
                window.set_create_popup_open(true);
                for _ in 0..2 {
                    window.window().request_redraw();
                    adapter.draw_if_needed(|renderer| {
                        renderer.render(&mut pixels, 900);
                    });
                    std::thread::sleep(Duration::from_millis(10));
                    slint::platform::update_timers_and_animations();
                }
                press("_02".into());
                assert_eq!(window.get_create_name(), edited);
                press(Key::Escape.into());
                slint::platform::update_timers_and_animations();
                assert!(!window.get_create_popup_open());
                assert_eq!(releases.get(), index + 1);
            }
            window.hide().unwrap();
        });
    }

    fn book() -> TabBook {
        TabBook {
            identity: Default::default(),
            selection: Default::default(),
            focused_group: None,
            active_group: None,
            groups: Default::default(),
            tabs: vec![
                Tab::new("folder01".into(), false),
                Tab::new("folder01".into(), false),
            ],
            active: 0,
        }
    }

    #[test]
    fn a_captured_document_location_rejects_other_tabs_views_paths_and_group_pages() {
        let mut tabs = book();
        let location = Location::capture(&tabs);
        assert!(location.matches(&tabs));
        tabs.active = 1;
        assert!(!location.matches(&tabs));
        tabs.active = 0;
        tabs.tabs[0].current_path = "folder02".into();
        assert!(!location.matches(&tabs));
        tabs.tabs[0].current_path = "folder01".into();
        let group = tabs
            .groups
            .create(&mut tabs.tabs, &mut tabs.active, &[0, 1], "Group 01", 0)
            .unwrap();
        tabs.active_group = Some(group);
        assert!(!location.matches(&tabs));
        tabs.active_group = None;
        assert!(location.matches(&tabs));
        assert!(!location.matches(&book()));
    }

    #[test]
    fn native_cancellation_preserves_its_identity_through_error_context() {
        use windows::Win32::Foundation::{E_ABORT, E_ACCESSDENIED, ERROR_CANCELLED};
        for code in [
            E_ABORT,
            windows::core::HRESULT::from_win32(ERROR_CANCELLED.0),
        ] {
            let error = anyhow::Error::new(windows::core::Error::from_hresult(code))
                .context("native command failed");
            assert!(cancelled(&error));
        }
        let error = anyhow::Error::new(windows::core::Error::from_hresult(E_ACCESSDENIED));
        assert!(!cancelled(&error));
        assert_eq!(
            file_error_kind(error.as_ref()),
            Some(std::io::ErrorKind::PermissionDenied)
        );
    }
}
