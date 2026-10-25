//! Input and layout contracts for the shared themeable controls.

use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, SharedString};

slint::slint! {
    import { Button, CheckBox, ComboBox, LineEdit, SpinBox, ScrollView } from "src/ui/widgets.slint";
    import { EditLineEdit } from "src/ui/controls.slint";
    import { Theme } from "src/ui/theme.slint";
    import { ChoiceList } from "src/ui/widgets/choice.slint";
    import { Button as ReferenceButton, LineEdit as ReferenceEdit, CheckBox as ReferenceCheck,
        ComboBox as ReferenceCombo, SpinBox as ReferenceSpin, ScrollView as ReferenceScroll } from "std-widgets.slint";
    export { Theme }
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
