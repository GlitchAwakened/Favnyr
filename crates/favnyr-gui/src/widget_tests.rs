//! Input and layout contracts for the shared themeable controls.

use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, SharedString};

slint::slint! {
    import { Button, CheckBox, ComboBox, LineEdit, SpinBox, ScrollView } from "src/ui/widgets.slint";
    import { EditLineEdit } from "src/ui/controls.slint";
    import { Theme } from "src/ui/theme.slint";
    import { ChoiceList } from "src/ui/widgets/choice.slint";
    import { FileItemName, GridItemName, FileItemView } from "src/ui/file_list.slint";
    import { NewItemMenu } from "src/ui/new_items.slint";
    import { ShellCtxEntry } from "src/ui/types.slint";
    import { CtxNav } from "src/ui/globals.slint";
    import { Button as ReferenceButton, LineEdit as ReferenceEdit, CheckBox as ReferenceCheck,
        ComboBox as ReferenceCombo, SpinBox as ReferenceSpin, ScrollView as ReferenceScroll } from "std-widgets.slint";
    export { Theme }
    export component NewItemMenuHarness inherits Window {
        width: 320px; height: 260px;
        in property <bool> windows: true;
        in property <[ShellCtxEntry]> entries;
        in-out property <int> created-kind: -1;
        in-out property <int> created-document: -1;
        out property <length> menu-height: menu.height;
        out property <length> navigation-y: CtxNav.sub-y;
        public function begin-navigation() {
            CtxNav.disarm(); CtxNav.level = 2;
        }
        public function next() { CtxNav.step(1); }
        public function activate() { CtxNav.activate += 1; }
        menu := NewItemMenu {
            x: 12px; y: 12px; windows: root.windows;
            documents: root.entries; nav-level: 2; available-height: 236px;
            create(kind) => { root.created-kind = kind; }
            document(id) => { root.created-document = id; }
        }
    }
    export component WidgetHarness inherits Window {
        width: 400px; height: 500px;
        in-out property <bool> enabled: true;
        in-out property <bool> read-only;
        in-out property <bool> checked <=> check.checked;
        in-out property <int> index <=> combo.current-index;
        in-out property <int> value <=> spin.value;
        in-out property <string> text <=> edit.text;
        in-out property <length> scroll-y <=> scroll.content-y;
        out property <length> scroll-x: scroll.content-x;
        out property <length> content-height: scroll.content-height;
        out property <length> measured-height: measured.content-height;
        out property <brush> button-background: button.background;
        out property <brush> edit-background: edit.background;
        in-out property <int> clicks;
        in-out property <int> selections;
        in-out property <int> opens;
        in-out property <int> edits;
        out property <bool> input-focused: edit.has-focus;
        out property <bool> text-aligned-left: edit.horizontal-alignment == TextHorizontalAlignment.left;
        out property <bool> combo-expanded: combo.accessible-expanded;
        public function focus-button() { button.focus(); }
        public function focus-check() { check.focus(); }
        public function focus-combo() { combo.focus(); }
        public function focus-edit() { edit.focus(); }
        public function focus-spin() { spin.focus(); }
        public function select-text() { edit.select-all(); }
        public function select-tail() { edit.set-selection-offsets(3, 99999); }
        button := Button {
            x: 10px; y: 10px; width: 180px; height: 32px;
            text: "Action"; enabled: root.enabled;
            clicked => { root.clicks += 1; }
        }
        check := CheckBox {
            x: 10px; y: 60px; width: 180px; height: 32px;
            text: "Option"; enabled: root.enabled;
        }
        combo := ComboBox {
            x: 10px; y: 110px; width: 180px; height: 32px;
            model: ["Option 1", "Option 2", "Option 3", "Option 4", "Option 5", "Option 6", "Option 7", "Option 8"];
            enabled: root.enabled;
            selected => { root.selections += 1; }
            opened => { root.opens += 1; }
        }
        edit := EditLineEdit {
            x: 10px; y: 160px; width: 180px; height: 32px;
            enabled: root.enabled; read-only: root.read-only;
            edited => { root.edits += 1; }
        }
        spin := SpinBox {
            x: 10px; y: 210px; width: 96px; height: 32px;
            minimum: 1; maximum: 8; enabled: root.enabled;
        }
        scroll := ScrollView {
            x: 10px; y: 260px; width: 180px; height: 100px;
            content-height: 600px;
            content-width: 500px;
            Rectangle { height: 600px; width: 500px; }
        }
        measured := ScrollView {
            x: 210px; y: 260px; width: 180px; height: 100px;
            VerticalLayout {
                for i in 10: Rectangle { height: 32px; }
            }
        }
    }
    export component PlacementHarness inherits Window {
        width: 300px; height: 300px; background: #000000;
        in property <length> control-width: 180px;
        in property <int> selected-index;
        ChoiceList {
            x: 10px; y: 10px; width: root.control-width;
            model: ["Option 1", "Option 2", "Option 3"];
            current-index: root.selected-index;
        }
        CheckBox {
            x: 10px; y: 160px; width: root.control-width; height: 32px;
            checked: true; text: "Option";
        }
        ScrollView {
            x: 10px; y: 210px; width: root.control-width; height: 70px;
            content-width: 600px; content-height: 600px;
        }
    }
    export component FileNameHarness inherits Window {
        width: 360px; height: 80px; background: #000000;
        in property <string> base: "File01";
        in property <string> extension: ".txt";
        in property <length> label-width: 100px;
        in property <length> label-height: 40px;
        in property <length> text-size: 13px;
        in property <bool> wrap-name;
        if !root.wrap-name : FileItemName {
            x: 10px; y: 10px; width: root.label-width; height: root.label-height;
            base: root.base; extension: root.extension;
            foreground: #00ff00; extension-foreground: #ff0000;
            font-size: root.text-size;
        }
        if root.wrap-name : GridItemName {
            x: 10px; y: 10px; width: root.label-width; height: root.label-height;
            base: root.base; extension: root.extension;
            foreground: #00ff00; extension-foreground: #ff0000;
            font-size: root.text-size;
        }
    }
    export component FileItemHarness inherits Window {
        width: 360px; height: 360px; background: #000000;
        in property <string> base: "File01";
        in property <string> extension: ".txt";
        in property <length> item-width: 160px;
        in property <bool> grid-mode;
        FileItemView {
            x: 10px; y: 10px; width: root.item-width;
            height: self.row-h;
            row-h: root.grid-mode ? root.item-width + 44px : 32px;
            name: root.base + root.extension; name-base: root.base; name-ext: root.extension;
            grid-mode: root.grid-mode;
            col-name-width: root.item-width;
            content-width: root.item-width;
            columns: [{ id: "name", label: "Name", visible: true, offset: 0px }];
        }
    }
    export component GeometryHarness inherits Window {
        width: 600px; height: 600px;
        in property <length> text-size: 12px;
        in property <string> check-label: "Option";
        default-font-size: root.text-size;
        out property <[float]> actual: [button.min-width / 1px, button.min-height / 1px,
            edit.min-width / 1px, edit.min-height / 1px, check.min-width / 1px, check.min-height / 1px,
            combo.min-width / 1px, combo.min-height / 1px, spin.min-width / 1px, spin.min-height / 1px,
            scroll.min-width / 1px, scroll.min-height / 1px];
        out property <[float]> expected: [ref-button.min-width / 1px, ref-button.min-height / 1px,
            ref-edit.min-width / 1px, ref-edit.min-height / 1px, ref-check.min-width / 1px, ref-check.min-height / 1px,
            ref-combo.min-width / 1px, ref-combo.min-height / 1px, ref-spin.min-width / 1px, ref-spin.min-height / 1px,
            ref-scroll.min-width / 1px, ref-scroll.min-height / 1px];
        button := Button { text: "Action"; }
        ref-button := ReferenceButton { text: "Action"; }
        edit := LineEdit { placeholder-text: "Field"; }
        ref-edit := ReferenceEdit { placeholder-text: "Field"; }
        check := CheckBox { text: root.check-label; }
        ref-check := ReferenceCheck { text: root.check-label; }
        combo := ComboBox { model: ["Option 1", "Option 2"]; }
        ref-combo := ReferenceCombo { model: ["Option 1", "Option 2"]; }
        spin := SpinBox { }
        ref-spin := ReferenceSpin { }
        scroll := ScrollView { }
        ref-scroll := ReferenceScroll { }
    }
}

#[test]
fn new_item_menu_keeps_all_registered_types_reachable_in_a_short_window() {
    use slint::{ModelRc, VecModel};
    crate::test_support::run(|| {
        let ui = NewItemMenuHarness::new().unwrap();
        ui.set_entries(ModelRc::new(VecModel::from(
            (0..40)
                .map(|id| ShellCtxEntry {
                    id,
                    label: format!("File type {id}").into(),
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
        )));
        ui.show().unwrap();
        assert_eq!(ui.get_menu_height(), 236.0);
        ui.invoke_begin_navigation();
        for _ in 0..43 {
            ui.invoke_next();
            slint::platform::update_timers_and_animations();
        }
        ui.invoke_activate();
        slint::platform::update_timers_and_animations();
        assert_eq!(ui.get_created_document(), 39);
        ui.set_created_document(-1);
        let position = LogicalPosition::new(70.0, 231.0);
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
        assert_eq!(ui.get_created_document(), 39);
        ui.hide().unwrap();
    });
}

#[test]
fn new_item_menu_accepts_late_choices_without_losing_keyboard_selection() {
    use slint::{ModelRc, VecModel};
    crate::test_support::run_with_windows(|windows| {
        let ui = NewItemMenuHarness::new().unwrap();
        ui.window().set_size(slint::LogicalSize::new(320., 260.));
        ui.show().unwrap();
        let adapter = windows.borrow()[0].clone();
        let mut pixels = vec![slint::Rgb8Pixel::default(); 320 * 260];
        let mut render = || {
            ui.window().request_redraw();
            adapter.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, 320);
            });
            slint::platform::update_timers_and_animations();
        };
        render();
        ui.invoke_begin_navigation();
        ui.invoke_next();
        slint::platform::update_timers_and_animations();
        ui.invoke_next();
        slint::platform::update_timers_and_animations();
        ui.invoke_activate();
        slint::platform::update_timers_and_animations();
        assert_eq!(ui.get_created_kind(), 1);
        assert_eq!(ui.get_navigation_y(), 26.0);
        ui.set_created_kind(-1);
        ui.set_entries(ModelRc::new(VecModel::from(vec![ShellCtxEntry {
            id: 0,
            label: "File01".into(),
            ..Default::default()
        }])));
        render();
        render();
        assert_eq!(ui.get_menu_height(), 136.0);
        assert_eq!(ui.get_created_document(), -1);
        ui.invoke_activate();
        slint::platform::update_timers_and_animations();
        assert_eq!(ui.get_created_kind(), 1);
        assert_eq!(ui.get_navigation_y(), 26.0);
        ui.invoke_next();
        slint::platform::update_timers_and_animations();
        assert_eq!(ui.get_navigation_y(), 52.0);
        ui.invoke_next();
        slint::platform::update_timers_and_animations();
        assert_eq!(ui.get_navigation_y(), 102.0);
        ui.invoke_activate();
        slint::platform::update_timers_and_animations();
        assert_eq!(ui.get_created_document(), 0);
        ui.hide().unwrap();
    });
}

#[test]
fn new_item_menu_preserves_the_basic_creation_actions_on_both_platforms() {
    crate::test_support::run(|| {
        for windows in [false, true] {
            let ui = NewItemMenuHarness::new().unwrap();
            ui.set_windows(windows);
            ui.show().unwrap();
            assert_eq!(ui.get_menu_height(), if windows { 86.0 } else { 60.0 });
            ui.invoke_begin_navigation();
            for kind in 0..if windows { 3 } else { 2 } {
                ui.invoke_next();
                slint::platform::update_timers_and_animations();
                ui.invoke_activate();
                slint::platform::update_timers_and_animations();
                assert_eq!(ui.get_created_kind(), kind);
            }
            ui.hide().unwrap();
        }
    });
}

#[test]
fn fresh_window_preferences_match_new_install_config() {
    use crate::MainWindow;
    use favnyr_core::{Config, workspace::ItemArrangement};

    crate::test_support::run(|| {
        let config = Config::default();
        let ui = MainWindow::new().unwrap();
        assert_eq!(ui.get_clock_utc_pref(), i32::from(config.clock_utc));
        assert_eq!(
            ui.get_default_view_mode_index(),
            i32::from(config.default_preview_mode)
        );
        assert_eq!(
            ui.get_default_item_arrangement_index(),
            i32::from(config.default_item_arrangement == ItemArrangement::Grid)
        );
        assert_eq!(ui.get_default_show_hidden(), config.default_show_hidden);
        assert_eq!(ui.get_tab_tooltip_enabled(), config.tab_path_tooltip);
        assert_eq!(ui.get_grid_name_tooltip_enabled(), config.grid_name_tooltip);
        assert_eq!(
            ui.get_search_path_tooltip_enabled(),
            config.search_path_tooltip
        );
        assert_eq!(
            ui.get_ws_panel_save_index(),
            config.panel_save_policy().index()
        );
        assert_eq!(
            ui.get_ws_shortcut_save_index(),
            config.workspace_shortcut_save.index()
        );
    });
}

#[test]
fn comment_editor_restores_the_note_on_each_open() {
    use slint::platform::Key;
    use std::{cell::RefCell, rc::Rc};

    crate::test_support::run_with_windows(|windows| {
        let ui = crate::MainWindow::new().unwrap();
        ui.window().set_size(slint::LogicalSize::new(900.0, 720.0));
        let saved = Rc::new(RefCell::new(Vec::<String>::new()));
        let received = saved.clone();
        ui.on_comment_confirmed(move |text| received.borrow_mut().push(text.to_string()));
        ui.show().unwrap();
        ui.window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        let adapter = windows.borrow()[0].clone();
        let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 720];

        for note in ["Note01", "Note02", ""] {
            ui.set_comment_text(note.into());
            ui.set_comment_popup_open(true);
            for _ in 0..2 {
                ui.window().request_redraw();
                adapter.draw_if_needed(|renderer| {
                    renderer.render(&mut pixels, 900);
                });
                std::thread::sleep(std::time::Duration::from_millis(10));
                slint::platform::update_timers_and_animations();
            }
            let key: slint::SharedString = Key::Return.into();
            ui.window()
                .dispatch_event(WindowEvent::KeyPressed { text: key.clone() });
            ui.window()
                .dispatch_event(WindowEvent::KeyReleased { text: key });
            assert_eq!(saved.borrow().last().map(String::as_str), Some(note));
            assert!(!ui.get_comment_popup_open());
        }
    });
}

#[test]
fn mixed_progress_cards_keep_separate_close_targets_with_long_errors() {
    use slint::{ModelRc, VecModel};
    use std::{cell::RefCell, rc::Rc};
    crate::test_support::run(|| {
        let ui = crate::MainWindow::new().unwrap();
        ui.window().set_size(slint::LogicalSize::new(900.0, 720.0));
        let detail = "folder01/".repeat(80) + "my_file.bin: access denied";
        ui.set_ops(ModelRc::new(VecModel::from(vec![
            crate::OpProgress {
                id: 1,
                state: 4,
                detail: detail.clone().into(),
                ..Default::default()
            },
            crate::OpProgress {
                id: 2,
                state: 4,
                detail: detail.into(),
                ..Default::default()
            },
            crate::OpProgress {
                id: 3,
                state: 4,
                ..Default::default()
            },
        ])));
        let dismissed = Rc::new(RefCell::new(Vec::new()));
        let calls = dismissed.clone();
        ui.on_op_dismiss(move |id| calls.borrow_mut().push(id));
        ui.show().unwrap();
        slint::platform::update_timers_and_animations();
        for y in [563.0, 395.0, 299.0] {
            let position = LogicalPosition::new(864.0, y);
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
        assert_eq!(*dismissed.borrow(), [1, 2, 3]);
        dismissed.borrow_mut().clear();
        ui.window().set_size(slint::LogicalSize::new(900.0, 420.0));
        slint::platform::update_timers_and_animations();
        for y in [312.0, 194.0, 98.0] {
            let position = LogicalPosition::new(864.0, y);
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
        assert_eq!(*dismissed.borrow(), [1, 2, 3]);
        ui.hide().unwrap();
    });
}

#[test]
fn external_drop_menu_accepts_the_first_click_after_window_activation() {
    use std::{cell::RefCell, rc::Rc};
    crate::test_support::run(|| {
        let ui = crate::MainWindow::new().unwrap();
        ui.window().set_size(slint::LogicalSize::new(900.0, 720.0));
        let actions = Rc::new(RefCell::new(Vec::new()));
        let calls = actions.clone();
        ui.on_file_drop_action(move |action| calls.borrow_mut().push(action));
        ui.show().unwrap();
        for action in 0..3 {
            ui.window()
                .dispatch_event(WindowEvent::WindowActiveChanged(false));
            ui.invoke_prepare_external_file_menu();
            ui.set_file_drop_menu_x(100.0);
            ui.set_file_drop_menu_y(120.0);
            ui.set_file_drop_menu_open(true);
            ui.window()
                .dispatch_event(WindowEvent::WindowActiveChanged(true));
            slint::platform::update_timers_and_animations();
            let position = LogicalPosition::new(150.0, 137.0 + action as f32 * 28.0);
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
            assert!(!ui.get_file_drop_menu_open());
            assert_eq!(actions.borrow().last(), Some(&action));
        }
        ui.hide().unwrap();
    });
}

#[test]
fn completed_native_drag_does_not_block_progressive_rows() {
    crate::test_support::run(|| {
        let ui = crate::MainWindow::new().unwrap();
        assert!(!ui.get_rows_interaction_busy());
        // The one-shot latch remains set until a new internal drag starts.
        ui.set_native_drag_armed(true);
        assert!(!ui.get_rows_interaction_busy());
        ui.set_native_drag_active(true);
        assert!(ui.get_rows_interaction_busy());
        ui.set_native_drag_active(false);
        assert!(!ui.get_rows_interaction_busy());
        ui.set_rows_pointer_busy(true);
        assert!(ui.get_rows_interaction_busy());
        ui.set_rows_pointer_busy(false);
        assert!(!ui.get_rows_interaction_busy());
    });
}

#[test]
fn themed_widgets_keep_decoration_positions_when_resized() {
    use slint::Rgb8Pixel;

    crate::test_support::run_with_windows(|windows| {
        let ui = PlacementHarness::new().unwrap();
        let colors = ui.global::<Theme>();
        colors.set_custom_palette(true);
        colors.set_accent_background(slint::Color::from_rgb_u8(255, 0, 255));
        colors.set_border(slint::Color::from_rgb_u8(255, 0, 0));
        ui.window().set_size(slint::LogicalSize::new(300., 300.));
        ui.show().unwrap();
        let adapter = windows.borrow()[0].clone();

        // Render only synthetic controls into memory, without capturing a screen or saving an image.
        for width in [180, 260] {
            ui.set_control_width(width as f32);
            for selected in [0, 2] {
                ui.set_selected_index(selected);
                let mut pixels = vec![Rgb8Pixel::default(); 300 * 300];
                adapter.draw_if_needed(|renderer| {
                    renderer.render(&mut pixels, 300);
                });
                let bounds = |left: usize,
                              top: usize,
                              right: usize,
                              bottom: usize,
                              color: (u8, u8, u8)| {
                    let mut found = None;
                    for y in top..bottom {
                        for x in left..right {
                            let p = pixels[y * 300 + x];
                            if (p.r, p.g, p.b) == color {
                                let (min_x, min_y, max_x, max_y) = found.unwrap_or((x, y, x, y));
                                found =
                                    Some((min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y)));
                            }
                        }
                    }
                    found.expect("expected decoration pixels")
                };
                let marker = bounds(10, 10, width + 10, 138, (255, 0, 255));
                assert_eq!(
                    (marker.0, marker.2),
                    (17, 19),
                    "selection marker must stay at the left edge"
                );
                let row_top = 14 + selected as usize * 40;
                assert!(marker.1 >= row_top && marker.3 < row_top + 40);
                let check = bounds(10, 160, width + 10, 192, (255, 0, 255));
                assert_eq!(
                    (check.0, check.2),
                    (10, 27),
                    "checkbox must stay left of its label"
                );
                let vertical = bounds(width - 4, 210, width + 10, 266, (255, 0, 0));
                // The rounded cap may antialias its first row or column.
                assert!(
                    (226..=227).contains(&vertical.1),
                    "vertical scrollbar must start at the top"
                );
                let horizontal = bounds(10, 266, width - 4, 280, (255, 0, 0));
                assert!(
                    (26..=27).contains(&horizontal.0),
                    "horizontal scrollbar must start at the left"
                );
            }
        }
    });
}

#[test]
fn file_names_keep_their_prefix_at_the_left_and_the_extension_next_to_the_stem() {
    crate::test_support::run_with_windows(|windows| {
        let ui = FileNameHarness::new().unwrap();
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(360., 80.));
        let adapter = windows.borrow()[0].clone();
        // Render the list label into memory, without screen capture or image output.
        let render = || {
            let mut pixels = vec![slint::Rgb8Pixel::default(); 360 * 80];
            ui.window().request_redraw();
            adapter.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, 360);
            });
            let mut stem = None;
            let mut suffix = None;
            for y in 0..80 {
                for x in 0..360 {
                    let pixel = pixels[y * 360 + x];
                    let bounds = if pixel.g > 32 && pixel.r == 0 && pixel.b == 0 {
                        &mut stem
                    } else if pixel.r > 32 && pixel.g == 0 && pixel.b == 0 {
                        &mut suffix
                    } else {
                        continue;
                    };
                    let (left, right) = bounds.unwrap_or((x, x));
                    *bounds = Some((left.min(x), right.max(x)));
                }
            }
            (stem, suffix)
        };
        ui.set_base("File01".into());
        ui.set_extension(".txt".into());
        let mut reference = None;
        for width in [96., 180., 280.] {
            ui.set_label_width(width);
            let (stem, suffix) = render();
            let stem = stem.expect("stem must be visible");
            let suffix = suffix.expect("extension must be visible");
            assert!((10..=12).contains(&stem.0), "stem bounds: {stem:?}");
            assert!(suffix.0 > stem.1 && suffix.0 <= stem.1 + 6);
            if let Some(previous) = reference {
                assert_eq!((stem, suffix), previous, "width: {width}");
            }
            reference = Some((stem, suffix));
        }
        let prefix = "File01";
        ui.set_label_width(96.);
        ui.set_base(prefix.into());
        let left = render().0.unwrap().0;
        ui.set_base(format!("{prefix} {}", "File02 ".repeat(20)).into());
        let (stem, suffix) = render();
        let stem = stem.expect("long name must retain visible prefix text");
        let suffix = suffix.expect("long name must retain its extension");
        assert_eq!(stem.0, left);
        assert!(stem.1 < suffix.0 && suffix.1 < 106);
        ui.set_extension("".into());
        let (stem, suffix) = render();
        assert_eq!(stem.unwrap().0, left);
        assert!(stem.unwrap().1 < 106);
        assert_eq!(suffix, None);
    });
}

#[test]
fn wrapped_file_names_keep_their_prefix_and_only_two_visible_lines() {
    crate::test_support::run_with_windows(|windows| {
        let ui = FileNameHarness::new().unwrap();
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(360., 80.));
        let adapter = windows.borrow()[0].clone();
        let reference = FileNameHarness::new().unwrap();
        reference.show().unwrap();
        reference
            .window()
            .set_size(slint::LogicalSize::new(360., 80.));
        let reference_adapter = windows.borrow()[1].clone();
        let render_prefix = |ui: &FileNameHarness, adapter: &MinimalSoftwareWindow| {
            let mut pixels = vec![slint::Rgb8Pixel::default(); 360 * 80];
            for _ in 0..2 {
                ui.window().request_redraw();
                adapter.draw_if_needed(|renderer| {
                    renderer.render(&mut pixels, 360);
                });
                slint::platform::update_timers_and_animations();
            }
            let is_text = |x: usize, y: usize| {
                let pixel = pixels[y * 360 + x];
                pixel.g > 64 && pixel.r == 0 && pixel.b == 0
            };
            let top = (10..80)
                .find(|&y| (10..360).any(|x| is_text(x, y)))
                .expect("prefix text must be visible");
            let bottom = (top + 1..80)
                .find(|&y| (10..360).all(|x| !is_text(x, y)))
                .unwrap_or(80);
            let left = (10..360)
                .find(|&x| (top..bottom).any(|y| is_text(x, y)))
                .unwrap();
            let mut ink = Vec::new();
            for y in top..bottom {
                for x in 10..360 {
                    if is_text(x, y) {
                        ink.push((x - left, y - top));
                    }
                }
            }
            let mut line_count = 0;
            let mut previous_row_has_ink = false;
            for y in 10..80 {
                let row_has_ink = (10..360).any(|x| is_text(x, y));
                if row_has_ink && !previous_row_has_ink {
                    line_count += 1;
                }
                previous_row_has_ink = row_has_ink;
            }
            (ink, line_count)
        };
        ui.set_wrap_name(true);
        reference.set_extension("".into());
        for font_size in [13., 16., 24.] {
            ui.set_text_size(font_size);
            ui.set_label_height(font_size * 2.8);
            reference.set_text_size(font_size);
            reference.set_label_height(font_size * 2.8);
            for width in [font_size * 8., font_size * 10.] {
                ui.set_label_width(width);
                reference.set_label_width(width);
                for prefix in ["Folder01", "_File01_", "<File01>", "File&01"] {
                    // The reserved tail can wrap the rest of a wide first word.
                    // Its first two characters must still lead the rendered text.
                    reference.set_base(prefix.chars().take(2).collect::<String>().into());
                    let expected = render_prefix(&reference, &reference_adapter).0;
                    let prefix_width = expected.iter().map(|&(x, _)| x).max().unwrap() + 1;
                    ui.set_base(format!("{prefix} {}", "File02 ".repeat(20)).into());
                    let (ink, line_count) = render_prefix(&ui, &adapter);
                    let actual: Vec<_> =
                        ink.into_iter().filter(|&(x, _)| x < prefix_width).collect();
                    // Centered text may land between pixels. Compare glyph
                    // shapes with one-pixel rasterization tolerance in both directions.
                    let matches = |source: &[(usize, usize)], target: &[(usize, usize)]| {
                        source
                            .iter()
                            .filter(|&&(x, y)| {
                                target
                                    .iter()
                                    .any(|&(tx, ty)| x.abs_diff(tx) <= 1 && y.abs_diff(ty) <= 1)
                            })
                            .count()
                    };
                    let expected_matches = matches(&expected, &actual);
                    let actual_matches = matches(&actual, &expected);
                    assert!(
                        expected_matches * 100 >= expected.len() * 98
                            && actual_matches * 100 >= actual.len() * 98,
                        "the visible first line must contain the original prefix, size: {font_size}, width: {width}, prefix: {prefix}, expected pixels: {}, actual pixels: {}, missing: {:?}, unexpected: {:?}",
                        expected.len(),
                        actual.len(),
                        expected
                            .iter()
                            .filter(|pixel| !actual.contains(pixel))
                            .take(8)
                            .collect::<Vec<_>>(),
                        actual
                            .iter()
                            .filter(|pixel| !expected.contains(pixel))
                            .take(8)
                            .collect::<Vec<_>>()
                    );
                    if prefix == "Folder01" {
                        assert_eq!(line_count, 2, "size: {font_size}, width: {width}");
                    }
                }
            }
        }
    });
}

#[test]
fn truncated_grid_names_preserve_the_complete_extension_pixels() {
    crate::test_support::run_with_windows(|windows| {
        let ui = FileNameHarness::new().unwrap();
        let reference = FileNameHarness::new().unwrap();
        ui.set_wrap_name(true);
        reference.set_base("".into());
        for window in [&ui, &reference] {
            window.show().unwrap();
            window.window().set_size(slint::LogicalSize::new(360., 80.));
        }
        let adapter = windows.borrow()[0].clone();
        let reference_adapter = windows.borrow()[1].clone();
        let render = |window: &FileNameHarness, adapter: &MinimalSoftwareWindow| {
            let mut pixels = vec![slint::Rgb8Pixel::default(); 360 * 80];
            for _ in 0..2 {
                window.window().request_redraw();
                adapter.draw_if_needed(|renderer| {
                    renderer.render(&mut pixels, 360);
                });
                slint::platform::update_timers_and_animations();
            }
            let mut ink = Vec::new();
            for y in 0..80 {
                for x in 0..360 {
                    let pixel = pixels[y * 360 + x];
                    if pixel.r > 64 && pixel.g == 0 && pixel.b == 0 {
                        ink.push((x as i32, y as i32));
                    }
                }
            }
            let right = ink
                .iter()
                .map(|&(x, _)| x)
                .max()
                .expect("extension must be visible");
            let bottom = ink.iter().map(|&(_, y)| y).max().unwrap();
            ink.into_iter()
                .map(|(x, y)| (x - right, y - bottom))
                .collect::<Vec<_>>()
        };
        for font_size in [13.0, 16.0, 24.0] {
            for window in [&ui, &reference] {
                window.set_text_size(font_size);
                window.set_label_height(font_size * 2.8);
            }
            for extension in [".txt", ".png"] {
                ui.set_extension(extension.into());
                reference.set_extension(extension.into());
                reference.set_label_width(280.0);
                let expected = render(&reference, &reference_adapter);
                let left = expected.iter().map(|&(x, _)| x).min().unwrap();
                for width in [84.0, 104.0, 160.0, 280.0] {
                    ui.set_label_width(width);
                    let prefix = "File01";
                    for separator in ["", " "] {
                        ui.set_base(format!("{prefix}{separator}").repeat(24).into());
                        let actual: Vec<_> = render(&ui, &adapter)
                            .into_iter()
                            .filter(|&(x, _)| x >= left)
                            .collect();
                        let matches = |a: &[(i32, i32)], b: &[(i32, i32)]| {
                            a.iter()
                                .filter(|&&(x, y)| {
                                    b.iter()
                                        .any(|&(tx, ty)| (x - tx).abs() <= 1 && (y - ty).abs() <= 1)
                                })
                                .count()
                        };
                        assert!(
                            matches(&expected, &actual) * 100 >= expected.len() * 98
                                && matches(&actual, &expected) * 100 >= actual.len() * 98,
                            "extension pixels, font: {font_size}, width: {width}, extension: {extension}, prefix: {prefix}, separator: {separator:?}"
                        );
                    }
                }
            }
        }
        ui.hide().unwrap();
        reference.hide().unwrap();
    });
}

#[test]
fn file_item_captions_center_grid_labels_and_keep_list_left_edges() {
    crate::test_support::run_with_windows(|windows| {
        let ui = FileItemHarness::new().unwrap();
        let colors = ui.global::<Theme>();
        colors.set_custom_palette(true);
        colors.set_foreground(slint::Color::from_rgb_u8(0, 255, 0));
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(360., 360.));
        let adapter = windows.borrow()[0].clone();
        for grid in [false, true] {
            ui.set_grid_mode(grid);
            let mut reference = None;
            for width in [96, 160, 280] {
                ui.set_item_width(width as f32);
                for extension in ["", ".txt"] {
                    ui.set_extension(extension.into());
                    for base in ["File01".to_owned(), "File01 ".repeat(20)] {
                        ui.set_base(base.into());
                        let mut pixels = vec![slint::Rgb8Pixel::default(); 360 * 360];
                        ui.window().request_redraw();
                        adapter.draw_if_needed(|renderer| {
                            renderer.render(&mut pixels, 360);
                        });
                        let top = if grid { 12 + width } else { 10 };
                        let bottom = if grid { 10 + width + 44 } else { 42 };
                        let mut left = None;
                        let mut right = 0;
                        for y in top..bottom {
                            for x in 0..360 {
                                let pixel = pixels[y * 360 + x];
                                if pixel.g > 32 && pixel.r == 0 && pixel.b == 0 {
                                    left = Some(left.unwrap_or(x).min(x));
                                    right = right.max(x);
                                }
                            }
                        }
                        let left = left.expect("item caption must be visible");
                        if grid {
                            let center = (left + right) as f32 / 2.0;
                            assert!(
                                (center - (10.0 + width as f32 / 2.0)).abs() <= 2.0,
                                "caption bounds: {left}..{right}, width: {width}"
                            );
                            assert!(left >= 16 && right < 10 + width - 6);
                        } else if let Some(previous) = reference {
                            assert!(left.abs_diff(previous) <= 2, "caption left: {left}");
                        } else {
                            reference = Some(left);
                        }
                    }
                }
            }
        }
    });
}

#[test]
fn themed_widgets_preserve_standard_control_dimensions() {
    use slint::Model;
    crate::test_support::run(|| {
        let ui = GeometryHarness::new().unwrap();
        for font_size in [12., 18., 28.] {
            ui.set_text_size(font_size);
            for label in ["Option", ""] {
                ui.set_check_label(label.into());
                let actual: Vec<_> = ui.get_actual().iter().collect();
                let expected: Vec<_> = ui.get_expected().iter().collect();
                assert_eq!(actual, expected, "font size: {font_size}, label: {label:?}");
            }
        }
    });
}

fn press(ui: &WidgetHarness, key: impl Into<SharedString>) {
    let text = key.into();
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn click(ui: &WidgetHarness, x: f32, y: f32, button: PointerEventButton) {
    let position = LogicalPosition::new(x, y);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    ui.window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
}

fn window() -> WidgetHarness {
    let ui = WidgetHarness::new().unwrap();
    // The headless adapter needs a physical surface size for popup placement.
    ui.window().set_size(slint::LogicalSize::new(400., 500.));
    ui.show().unwrap();
    ui.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    ui
}

#[test]
fn themed_widgets_keep_keyboard_activation_and_disabled_guards() {
    crate::test_support::run(|| {
        let ui = window();
        ui.invoke_focus_button();
        press(&ui, Key::Space);
        assert_eq!(ui.get_clicks(), 1);
        press(&ui, Key::Tab);
        press(&ui, Key::Space);
        assert!(ui.get_checked());
        click(&ui, 50., 25., PointerEventButton::Left);
        assert_eq!(ui.get_clicks(), 2);
        ui.invoke_focus_check();
        press(&ui, Key::Space);
        assert!(!ui.get_checked());
        ui.set_enabled(false);
        press(&ui, Key::Space);
        click(&ui, 50., 25., PointerEventButton::Left);
        assert_eq!(ui.get_clicks(), 2);
        assert!(!ui.get_checked());
        // A disabled selector announces no opening, so whatever work is
        // attached to that event is not started either.
        click(&ui, 50., 125., PointerEventButton::Left);
        assert!(!ui.get_combo_expanded());
        assert_eq!(ui.get_opens(), 0);
    });
}

#[test]
fn themed_widgets_keep_text_selection_read_only_and_live_palette() {
    crate::test_support::run(|| {
        let ui = window();
        ui.set_text("my_file.txt".into());
        assert!(ui.get_text_aligned_left());
        ui.invoke_focus_edit();
        assert!(ui.get_input_focused());
        ui.invoke_select_tail();
        press(&ui, "item.png");
        assert_eq!(ui.get_text(), "my_item.png");
        ui.invoke_select_text();
        press(&ui, "é");
        assert_eq!(ui.get_text(), "é");
        ui.set_read_only(true);
        press(&ui, "x");
        assert_eq!(ui.get_text(), "é");
        ui.set_read_only(false);
        let surface = slint::Color::from_rgb_u8(30, 40, 50);
        ui.global::<Theme>().set_custom_palette(true);
        ui.global::<Theme>().set_surface(surface);
        assert_eq!(
            ui.get_button_background(),
            slint::Brush::SolidColor(surface)
        );
        assert_eq!(ui.get_edit_background(), slint::Brush::SolidColor(surface));
        assert!(ui.get_input_focused());
        press(&ui, "x");
        assert_eq!(ui.get_text(), "éx");
    });
}

#[test]
fn themed_widgets_keep_selector_popup_and_bounded_numbers() {
    crate::test_support::run(|| {
        let ui = window();
        ui.invoke_focus_combo();
        press(&ui, Key::DownArrow);
        assert_eq!(ui.get_index(), 1);
        press(&ui, Key::Return);
        assert!(ui.get_combo_expanded());
        assert_eq!(ui.get_opens(), 1);
        press(&ui, Key::End);
        assert_eq!(ui.get_index(), 7);
        press(&ui, Key::UpArrow);
        assert_eq!(ui.get_index(), 6);
        press(&ui, Key::Escape);
        assert!(!ui.get_combo_expanded());
        assert_eq!(ui.get_opens(), 1, "closing the list is not an opening");
        ui.invoke_focus_spin();
        press(&ui, Key::UpArrow);
        assert_eq!(ui.get_value(), 2);
        ui.set_value(8);
        press(&ui, Key::UpArrow);
        assert_eq!(ui.get_value(), 8);
        press(&ui, Key::DownArrow);
        assert_eq!(ui.get_value(), 7);
    });
}

#[test]
fn themed_widgets_keep_popup_mouse_selection_and_outside_dismissal() {
    crate::test_support::run(|| {
        let ui = window();
        click(&ui, 50., 125., PointerEventButton::Left);
        assert!(ui.get_combo_expanded());
        click(&ui, 50., 205., PointerEventButton::Left);
        assert_eq!(
            ui.get_index(),
            2,
            "expanded={}, selections={}",
            ui.get_combo_expanded(),
            ui.get_selections()
        );
        assert!(!ui.get_combo_expanded());
        assert_eq!(ui.get_selections(), 1);
        click(&ui, 50., 125., PointerEventButton::Left);
        click(&ui, 350., 450., PointerEventButton::Left);
        assert!(!ui.get_combo_expanded());
        assert_eq!(ui.get_selections(), 1);
        // One per opening, by pointer as by keyboard, and none for a
        // selection or a dismissal.
        assert_eq!(ui.get_opens(), 2);
    });
}

#[test]
fn themed_widgets_keep_text_context_menu_and_numeric_commit() {
    crate::test_support::run(|| {
        let ui = window();
        ui.set_text("my_file.txt".into());
        click(&ui, 50., 175., PointerEventButton::Right);
        press(&ui, Key::End);
        press(&ui, Key::Return);
        press(&ui, "new_file.txt");
        assert_eq!(ui.get_text(), "new_file.txt");

        ui.invoke_focus_spin();
        ui.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
        press(&ui, "a");
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
        press(&ui, "99");
        assert_eq!(ui.get_value(), 1);
        press(&ui, Key::Return);
        press(&ui, Key::UpArrow);
        assert_eq!(ui.get_value(), 2);
    });
}

#[test]
fn themed_widgets_keep_measured_content_and_wheel_scroll() {
    crate::test_support::run(|| {
        let ui = window();
        assert_eq!(ui.get_content_height(), 600.);
        assert!(ui.get_measured_height() >= 320.);
        ui.window().dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(50., 280.),
            delta_x: 0.,
            delta_y: -60.,
        });
        assert!(ui.get_scroll_y() < 0.);
        ui.set_scroll_y(-120.);
        assert_eq!(ui.get_scroll_y(), -120.);
        click(&ui, 183., 339., PointerEventButton::Left);
        assert!(ui.get_scroll_y() < -120.);
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: LogicalPosition::new(182., 294.),
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(182., 330.),
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position: LogicalPosition::new(182., 330.),
            button: PointerEventButton::Left,
        });
        assert!((-500. ..-130.).contains(&ui.get_scroll_y()));
        ui.window().dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(50., 280.),
            delta_x: -60.,
            delta_y: 0.,
        });
        assert!(ui.get_scroll_x() < 0.);
    });
}
