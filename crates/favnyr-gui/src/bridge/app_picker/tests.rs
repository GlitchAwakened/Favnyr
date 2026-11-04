use super::*;
use crate::Strings;

fn handler(key: &str, name: &str, exe: Option<&str>) -> openwith::AppHandler {
    openwith::AppHandler {
        key: key.into(),
        name: name.into(),
        exe: exe.map(str::to_owned),
        recommended: false,
    }
}

fn context() -> Context {
    let mut context = Context::new(vec![PathBuf::from("folder01/file01.txt")]);
    context.replace_system(
        vec![
            handler("app01", "App01", Some("folder02/app01")),
            handler("app02", "App02", None),
        ],
        |_| Image::default(),
    );
    context
}

#[test]
fn selection_is_provisional_and_follows_the_visible_identity() {
    let mut context = context();
    assert_eq!(context.visible("").1, -1);
    assert!(context.prepare("").is_none());
    assert_eq!(context.select("system:app02", ""), 1);
    assert_eq!(context.visible("  APP02 ").1, 0);
    assert_eq!(context.selected.as_deref(), Some("system:app02"));
    assert_eq!(context.visible("App01").1, -1);
    assert!(context.prepare("App01").is_none());
    assert_eq!(context.visible("").1, -1);
    assert_eq!(context.select("system:app02", "App01"), -1);
}

#[test]
fn changing_discovery_scope_keeps_only_visible_selections_and_reuses_icons() {
    let mut context = context();
    context.select("system:app02", "");
    context.replace_system(vec![handler("app02", "App03", None)], |_| {
        panic!("icon should be cached")
    });
    let (items, selected) = context.visible("");
    assert_eq!(selected, 0);
    assert_eq!(items[0].label, "App03");
    context.replace_system(Vec::new(), |_| Image::default());
    assert_eq!(context.visible("").1, -1);
}

#[test]
fn browsed_program_is_provisional_survives_scope_changes_and_does_not_duplicate() {
    let store = openers::OpenerStore::default();
    let mut context = context();
    context.stage_program("folder02/app03".into(), |_| Image::default());
    let (items, selected) = context.visible("");
    assert_eq!(selected, 2);
    assert_eq!(items[selected as usize].label, "app03");
    context.replace_system(Vec::new(), |_| panic!("icon should be cached"));
    assert_eq!(context.visible("").1, 0);
    context.stage_program("folder02/app03".into(), |_| {
        panic!("program should be reused")
    });
    assert_eq!(context.items.len(), 1);
    let candidate = context.prepare("").unwrap();
    assert_eq!(candidate.origin, Origin::Program);
    let opener = program_opener(&store, &candidate.handler);
    assert_eq!(opener.program, "folder02/app03");
    assert_eq!(opener.args, ["{file}"]);
    assert!(store.openers.is_empty());
}

#[test]
fn browsing_an_existing_system_program_keeps_the_explicit_command_route() {
    let mut context = context();
    context.stage_program("folder02/app01".into(), |_| Image::default());
    assert_eq!(context.visible("").1, 2);
    assert_eq!(context.items.len(), 3);
    assert_eq!(context.prepare("").unwrap().origin, Origin::Program);
}

#[test]
fn launch_snapshot_prevents_reentry_and_retries_the_same_choice_after_failure() {
    let mut context = context();
    context.select("system:app01", "");
    let first = context.prepare("").unwrap();
    assert_eq!(first.handler.key, "app01");
    assert_eq!(context.ext, "txt");
    assert_eq!(context.paths, [PathBuf::from("folder01/file01.txt")]);
    context.select("system:app02", "");
    context.stage_program("folder02/app03".into(), |_| panic!("launch is in progress"));
    assert!(context.prepare("").is_none());
    context.launching = false;
    assert_eq!(context.prepare("").unwrap().id, first.id);
    assert!(Context::new(Vec::new()).prepare("").is_none());
}

#[test]
fn successful_confirmation_preserves_custom_arguments_and_only_sets_an_explicit_default() {
    let mut store = openers::OpenerStore::default();
    let id = store.add(
        "App01",
        "folder02/app01",
        vec!["--option".into(), "{file}".into()],
    );
    let handler = handler("app01", "App01", Some("folder02/app01"));
    let prepared = program_opener(&store, &handler);
    assert_eq!(prepared.args, ["--option", "{file}"]);
    assert_eq!(store.get(&id).unwrap().use_count, 0);
    record_success(&mut store, &handler, "txt", false);
    assert_eq!(store.openers.len(), 1);
    assert_eq!(store.get(&id).unwrap().use_count, 1);
    assert!(store.get(&id).unwrap().default_exts.is_empty());
    record_success(&mut store, &handler, "txt", true);
    assert_eq!(store.get(&id).unwrap().default_exts, ["txt"]);
    assert_eq!(store.get(&id).unwrap().args, ["--option", "{file}"]);
}

#[test]
fn native_associations_keep_the_os_key_and_extensionless_files_get_no_default() {
    let mut store = openers::OpenerStore::default();
    let handler = handler("app02", "App02", None);
    record_success(&mut store, &handler, "", true);
    assert_eq!(store.openers.len(), 1);
    assert_eq!(store.openers[0].assoc.as_deref(), Some("app02"));
    assert!(store.openers[0].default_exts.is_empty());
    record_success(&mut store, &handler, "txt", true);
    assert_eq!(store.openers.len(), 1);
    assert_eq!(store.openers[0].default_exts, ["txt"]);
}

fn show_rows(ui: &MainWindow, context: &mut Context) {
    let (items, index) = context.visible(ui.get_ow_picker_search_text().as_str());
    ui.set_ow_picker_selected_index(-1);
    ui.set_ow_picker_handlers(ModelRc::new(VecModel::from(items)));
    ui.set_ow_picker_selected_index(index);
}

type Confirmations = Rc<RefCell<Vec<(String, bool)>>>;

fn picker_window(context: Rc<RefCell<Context>>) -> (MainWindow, Confirmations) {
    let ui = MainWindow::new().unwrap();
    let catalog: toml::Table = toml::from_str(include_str!("../../../i18n/en.toml")).unwrap();
    let label = |key: &str| -> SharedString { catalog[key].as_str().unwrap().into() };
    let strings = ui.global::<Strings>();
    strings.set_ow_choose(label("ow_choose"));
    strings.set_ow_pick_browse(label("ow_pick_browse"));
    strings.set_ow_run(label("ow_run"));
    strings.set_ow_set_default(label("ow_set_default"));
    strings.set_ow_search_placeholder(label("ow_search_placeholder"));
    strings.set_ow_empty(label("ow_empty"));
    strings.set_ow_no_results(label("ow_no_results"));
    strings.set_ow_display_other_apps(label("ow_display_other_apps"));
    strings.set_btn_cancel(label("btn_cancel"));
    ui.window().set_size(slint::LogicalSize::new(900., 720.));
    let weak = ui.as_weak();
    let selected = context.clone();
    ui.on_ow_picker_select(move |id| {
        let ui = weak.unwrap();
        let index = selected
            .borrow_mut()
            .select(&id, &ui.get_ow_picker_search_text());
        ui.set_ow_picker_selected_index(index);
    });
    let weak = ui.as_weak();
    let filtered = context.clone();
    ui.on_ow_picker_search(move |_| show_rows(&weak.unwrap(), &mut filtered.borrow_mut()));
    let weak = ui.as_weak();
    let browsed = context.clone();
    ui.on_ow_pick_browse(move || {
        let ui = weak.unwrap();
        browsed
            .borrow_mut()
            .stage_program("folder02/app03".into(), |_| Image::default());
        ui.set_ow_picker_search_text("".into());
        show_rows(&ui, &mut browsed.borrow_mut());
    });
    let weak = ui.as_weak();
    let confirmations = Confirmations::default();
    let confirmed = confirmations.clone();
    let chosen = context.clone();
    ui.on_ow_pick(move || {
        let ui = weak.unwrap();
        let mut context = chosen.borrow_mut();
        if let Some(candidate) = context.prepare(&ui.get_ow_picker_search_text()) {
            confirmed
                .borrow_mut()
                .push((candidate.id, ui.get_ow_picker_set_default()));
            context.launching = false;
        }
    });
    show_rows(&ui, &mut context.borrow_mut());
    ui.set_ow_picker_open(true);
    ui.show().unwrap();
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
    (ui, confirmations)
}

fn key(ui: &MainWindow, key: impl Into<SharedString>) {
    use slint::platform::WindowEvent;
    let text = key.into();
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn click(ui: &MainWindow, x: f32, y: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(x, y);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

fn settle(ui: &MainWindow, adapter: &slint::platform::software_renderer::MinimalSoftwareWindow) {
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 720];
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        ui.window().request_redraw();
        adapter.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 900);
        });
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn picker_clicks_select_then_confirm_with_the_current_default_choice() {
    use slint::platform::Key;
    crate::test_support::run_with_windows(|windows| {
        let context = Rc::new(RefCell::new(context()));
        let (ui, confirmations) = picker_window(context);
        let adapter = windows.borrow()[0].clone();
        settle(&ui, &adapter);
        click(&ui, 625., 536.);
        assert!(confirmations.borrow().is_empty());
        click(&ui, 300., 260.);
        assert_eq!(ui.get_ow_picker_selected_index(), 0);
        click(&ui, 300., 260.);
        assert!(confirmations.borrow().is_empty());
        assert!(ui.get_ow_picker_open());
        click(&ui, 260., 500.);
        assert!(ui.get_ow_picker_set_default());
        key(&ui, Key::DownArrow);
        assert_eq!(ui.get_ow_picker_selected_index(), 1);
        key(&ui, Key::UpArrow);
        assert_eq!(ui.get_ow_picker_selected_index(), 0);
        key(&ui, Key::Return);
        assert_eq!(&*confirmations.borrow(), &[("system:app01".into(), true)]);
        click(&ui, 625., 536.);
        assert_eq!(confirmations.borrow().len(), 2);
        ui.set_ow_picker_busy(true);
        key(&ui, Key::Return);
        key(&ui, Key::Escape);
        click(&ui, 625., 536.);
        click(&ui, 100., 100.);
        assert!(ui.get_ow_picker_open());
        assert_eq!(confirmations.borrow().len(), 2);
        ui.set_ow_picker_busy(false);
        key(&ui, Key::Escape);
        assert!(!ui.get_ow_picker_open());
    });
}

#[test]
fn picker_search_browse_and_cancellation_never_launch_implicitly() {
    use slint::platform::Key;
    crate::test_support::run_with_windows(|windows| {
        let context = Rc::new(RefCell::new(context()));
        let (ui, confirmations) = picker_window(context.clone());
        let adapter = windows.borrow()[0].clone();
        settle(&ui, &adapter);
        key(&ui, Key::DownArrow);
        assert_eq!(ui.get_ow_picker_selected_index(), 0);
        key(&ui, "App02");
        assert_eq!(ui.get_ow_picker_search_text(), "App02");
        assert_eq!(ui.get_ow_picker_selected_index(), -1);
        key(&ui, Key::Return);
        assert!(confirmations.borrow().is_empty());
        key(&ui, Key::DownArrow);
        assert_eq!(ui.get_ow_picker_selected_index(), 0);
        assert_eq!(context.borrow().selected.as_deref(), Some("system:app02"));
        click(&ui, 280., 536.);
        assert_eq!(ui.get_ow_picker_search_text(), "");
        assert_eq!(ui.get_ow_picker_selected_index(), 2);
        assert!(confirmations.borrow().is_empty());
        key(&ui, Key::Return);
        assert_eq!(
            &*confirmations.borrow(),
            &[("program:folder02/app03".into(), false)]
        );
        click(&ui, 555., 536.);
        assert!(!ui.get_ow_picker_open());
        assert_eq!(confirmations.borrow().len(), 1);
    });
}

#[test]
fn picker_tab_navigation_keeps_the_default_choice_and_list_keys_accessible() {
    use slint::platform::{Key, WindowEvent};
    crate::test_support::run_with_windows(|windows| {
        for linux in [false, true] {
            let context = Rc::new(RefCell::new(context()));
            let (ui, confirmations) = picker_window(context);
            ui.set_platform_linux(linux);
            let adapter = windows.borrow().last().unwrap().clone();
            settle(&ui, &adapter);
            // With no selection, the disabled Run button must not enter the cycle.
            for _ in 0..if linux { 5 } else { 4 } {
                key(&ui, Key::Tab);
            }
            key(&ui, "App0");
            assert_eq!(ui.get_ow_picker_search_text(), "App0");
            key(&ui, Key::DownArrow);
            key(&ui, Key::Tab);
            key(&ui, Key::Space);
            assert!(ui.get_ow_picker_set_default());
            key(&ui, Key::Space);
            for reverse in [false, true] {
                for _ in 0..12 {
                    if reverse {
                        ui.window().dispatch_event(WindowEvent::KeyPressed {
                            text: Key::Shift.into(),
                        });
                    }
                    key(&ui, Key::Tab);
                    if reverse {
                        ui.window().dispatch_event(WindowEvent::KeyReleased {
                            text: Key::Shift.into(),
                        });
                    }
                    key(&ui, Key::DownArrow);
                    assert_eq!(ui.get_ow_picker_selected_index(), 1);
                    key(&ui, Key::UpArrow);
                    assert_eq!(ui.get_ow_picker_selected_index(), 0);
                }
            }
            assert!(confirmations.borrow().is_empty());
            key(&ui, Key::Escape);
            assert!(!ui.get_ow_picker_open());
            ui.hide().unwrap();
        }
    });
}

#[test]
fn picker_keyboard_navigation_reveals_rows_and_filtering_clamps_the_scroll() {
    use slint::platform::Key;
    crate::test_support::run_with_windows(|windows| {
        let mut context = context();
        context.replace_system(
            (0..40)
                .map(|index| handler(&format!("app{index:02}"), &format!("App{index:02}"), None))
                .collect(),
            |_| Image::default(),
        );
        let context = Rc::new(RefCell::new(context));
        let (ui, confirmations) = picker_window(context.clone());
        let adapter = windows.borrow()[0].clone();
        settle(&ui, &adapter);
        for _ in 0..45 {
            key(&ui, Key::DownArrow);
            slint::platform::update_timers_and_animations();
        }
        assert_eq!(ui.get_ow_picker_selected_index(), 39);
        settle(&ui, &adapter);
        // The last row must be hittable at the bottom of the scrolled viewport.
        ui.set_ow_picker_selected_index(-1);
        click(&ui, 300., 470.);
        assert_eq!(context.borrow().selected.as_deref(), Some("system:app39"));
        click(&ui, 300., 212.);
        key(&ui, "App00");
        settle(&ui, &adapter);
        assert_eq!(ui.get_ow_picker_selected_index(), -1);
        click(&ui, 300., 260.);
        assert_eq!(context.borrow().selected.as_deref(), Some("system:app00"));
        assert!(confirmations.borrow().is_empty());
        click(&ui, 100., 100.);
        assert!(!ui.get_ow_picker_open());
    });
}
