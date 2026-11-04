use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

slint::slint! {
    import { TabItem, TabBarEntry } from "src/ui/tabs.slint";
    import { TabGroupEditor } from "src/ui/tab_groups.slint";
    export component TabInputHarness inherits Window {
        width: 240px; height: 100px;
        in property <bool> vertical;
        in property <bool> on-left: true;
        in-out property <bool> drag-active;
        out property <int> clicks;
        out property <int> modifiers;
        out property <int> moves;
        out property <int> drops;
        out property <int> closes;
        out property <int> contexts;
        TabItem {
            x: 10px; y: 10px;
            title: "folder01";
            vertical: root.vertical;
            indicator-right: root.on-left;
            tab-width: root.vertical ? 26px : 180px;
            cross-len: 180px;
            tab-drag-active: root.drag-active;
            clicked(modifiers) => { root.clicks += 1; root.modifiers = modifiers; }
            close-clicked => { root.closes += 1; }
            right-clicked => { root.contexts += 1; }
            drag-progress => { root.moves += 1; root.drag-active = true; }
            drag-completed => { root.drops += 1; root.drag-active = false; }
        }
    }

    export component GroupInputHarness inherits Window {
        width: 240px; height: 110px;
        in property <bool> vertical;
        in property <bool> on-left: true;
        in-out property <bool> collapsed;
        in-out property <bool> group-highlighted;
        in-out property <bool> dragging;
        out property <string> dragged-group;
        out property <int> drops;
        out property <int> toggles;
        out property <int> contexts;
        out property <int> clicked-index: -1;
        TabBarEntry {
            x: 10px; y: 10px;
            tab-drag-active: root.dragging;
            vertical: root.vertical; cross-len: 180px; indicator-right: root.on-left;
            entry: { index: -1, is-group: true, title: "Group 01", group-id: "g1",
                count: 3, collapsed: root.collapsed, group-highlighted: root.group-highlighted,
                width: root.vertical ? 26px : 180px };
            group-toggle(id) => {
                root.group-highlighted = true;
                root.toggles += 1; root.collapsed = !root.collapsed;
            }
            group-context(id, x, y) => { root.contexts += 1; }
            drag-progress(i, id, x, y) => { root.dragged-group = id; root.dragging = true; }
            drag-completed => { root.drops += 1; root.dragging = false; }
        }
        TabBarEntry {
            x: 10px; y: 50px;
            vertical: root.vertical; cross-len: 180px; indicator-right: root.on-left;
            entry: { index: 5, grouped: true, title: "folder01", group-id: "g1",
                width: root.vertical ? 26px : 180px };
            clicked(i, modifiers) => { root.clicked-index = i; }
        }
    }

    export component GroupEditorHarness inherits Window {
        width: 480px; height: 300px;
        in-out property <int> chosen-color;
        in-out property <string> name: "Group 01";
        out property <int> submits;
        out property <int> cancels;
        public function focus-name() { editor.focus-name(); }
        editor := TabGroupEditor {
            x: 10px; y: 10px;
            name <=> root.name; color-slot <=> root.chosen-color;
            submit => { root.submits += 1; }
            cancel => { root.cancels += 1; }
        }
    }
}

#[test]
fn group_headers_toggle_without_navigation_and_members_keep_logical_indices() {
    crate::test_support::run(|| {
        for mode in 0..=2 {
            let ui = GroupInputHarness::new().unwrap();
            ui.set_vertical(mode != 0);
            ui.set_on_left(mode != 2);
            ui.show().unwrap();
            let position = LogicalPosition::new(35.0, 22.0);
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
            ui.window().dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
            assert_eq!(ui.get_toggles(), 1);
            assert!(ui.get_collapsed());
            assert_eq!(ui.get_clicked_index(), -1);
            ui.window().dispatch_event(WindowEvent::KeyPressed {
                text: Key::Space.into(),
            });
            assert_eq!(ui.get_toggles(), 2);
            ui.window().dispatch_event(WindowEvent::KeyReleased {
                text: Key::Space.into(),
            });
            ui.window().dispatch_event(WindowEvent::KeyPressed {
                text: Key::Menu.into(),
            });
            assert_eq!(ui.get_contexts(), 1);
            ui.window().dispatch_event(WindowEvent::KeyReleased {
                text: Key::Menu.into(),
            });
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
            ui.window().dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(60.0, 22.0),
            });
            ui.window().dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
            assert_eq!(ui.get_toggles(), 2);
            assert_eq!(ui.get_dragged_group(), "g1");
            assert_eq!(ui.get_drops(), 1);
            let position = LogicalPosition::new(35.0, 62.0);
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
            ui.window().dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
            assert_eq!(ui.get_clicked_index(), 5);
            ui.hide().unwrap();
        }
    });
}

#[test]
fn group_editor_keyboard_changes_a_draft_and_keeps_palette_enter_out_of_submit() {
    crate::test_support::run(|| {
        let ui = GroupEditorHarness::new().unwrap();
        ui.show().unwrap();
        ui.invoke_focus_name();
        let key = |text: slint::SharedString| {
            ui.window()
                .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            ui.window()
                .dispatch_event(WindowEvent::KeyReleased { text });
        };
        key("group02".into());
        assert_eq!(ui.get_name(), "group02");
        key(Key::Tab.into());
        key(Key::RightArrow.into());
        key(Key::Return.into());
        assert_eq!(ui.get_chosen_color(), 1);
        assert_eq!(ui.get_submits(), 0);
        key(Key::Tab.into());
        key(Key::Tab.into());
        key(Key::Return.into());
        assert_eq!(ui.get_submits(), 1);
        key(Key::Escape.into());
        assert_eq!(ui.get_cancels(), 1);
        ui.hide().unwrap();
    });
}

#[test]
fn group_menus_support_keyboard_commands_and_escape_without_file_state() {
    use std::{cell::RefCell, rc::Rc};
    crate::test_support::run(|| {
        let ui = crate::MainWindow::new().unwrap();
        let commands = Rc::new(RefCell::new(Vec::new()));
        let captured = commands.clone();
        ui.on_group_command(move |action, id| {
            captured.borrow_mut().push((action, id.to_string()));
        });
        ui.set_group_choices(slint::ModelRc::new(slint::VecModel::from(vec![
            crate::TabGroupChoice {
                id: "g1".into(),
                name: "Group 01".into(),
                slot: 0,
                checked: false,
            },
        ])));
        ui.show().unwrap();
        let key = |text: slint::SharedString| {
            slint::platform::update_timers_and_animations();
            ui.window()
                .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            slint::platform::update_timers_and_animations();
            ui.window()
                .dispatch_event(WindowEvent::KeyReleased { text });
        };
        ui.set_group_menu_open(true);
        key(Key::DownArrow.into());
        key(Key::Return.into());
        assert_eq!(*commands.borrow(), [(3, String::new())]);
        assert!(!ui.get_group_menu_open());
        ui.set_tab_menu_open(true);
        key(Key::UpArrow.into());
        key(Key::RightArrow.into());
        key(Key::Return.into());
        assert_eq!(commands.borrow().last(), Some(&(0, String::new())));
        ui.set_group_menu_open(true);
        key(Key::Escape.into());
        assert!(!ui.get_group_menu_open());
        ui.hide().unwrap();
    });
}

pub(super) fn window_with_grouped_views(mode: u8) -> crate::MainWindow {
    use slint::{ModelRc, VecModel};
    let ui = crate::MainWindow::new().unwrap();
    ui.window().set_size(slint::LogicalSize::new(1200.0, 800.0));
    let mut books = Vec::new();
    let mut presentations = Vec::new();
    for _ in 0..2 {
        let mut book = super::tests::sample(2, 0);
        book.groups
            .create(&mut book.tabs, &mut book.active, &[0, 1], "Group 01", 1)
            .unwrap();
        let projection = super::projection::Presentation::default();
        projection.update(
            &book,
            mode,
            &favnyr_core::annotations::AnnotationStore::default(),
        );
        books.push(book);
        presentations.push(projection);
    }
    ui.set_tab_bars(ModelRc::new(VecModel::from(
        books
            .iter()
            .zip(&presentations)
            .map(|(book, projection)| projection.state(book))
            .collect::<Vec<_>>(),
    )));
    ui.set_panels(ModelRc::new(VecModel::from(
        presentations
            .iter()
            .map(|p| crate::PanelView {
                tabs: ModelRc::from(p.model.clone()),
                tab_bar_mode: mode as i32,
                vbar_user_w: 130.0,
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_panel_boxes(ModelRc::new(VecModel::from(vec![
        crate::PanelBox {
            fx: 0.0,
            fy: 0.0,
            fw: 0.5,
            fh: 1.0,
        },
        crate::PanelBox {
            fx: 0.5,
            fy: 0.0,
            fw: 0.5,
            fh: 1.0,
        },
    ])));
    ui.on_tab_drop_gap(move |panel, position, joins| {
        let book = &books[panel as usize];
        let presentation = &presentations[panel as usize];
        if joins {
            presentation.destination(book, position, None)
        } else {
            presentation.gap(book, position, None)
        }
    });
    ui
}

fn interactive_grouped_view(
    mode: u8,
) -> (
    crate::MainWindow,
    std::rc::Rc<std::cell::RefCell<super::TabBook>>,
) {
    use slint::{Model, ModelRc};
    use std::{cell::RefCell, rc::Rc};
    let ui = window_with_grouped_views(mode);
    ui.window().set_size(slint::LogicalSize::new(1600.0, 800.0));
    let mut book = super::tests::sample(5, 4);
    for (indices, name) in [([0, 1], "Group 01"), ([2, 3], "Group 02")] {
        book.groups
            .create(&mut book.tabs, &mut book.active, &indices, name, 1)
            .unwrap();
    }
    let book = Rc::new(RefCell::new(book));
    let projection = Rc::new(super::projection::Presentation::default());
    let panels = ui.get_panels();
    let mut panel = panels.row_data(0).unwrap();
    panel.tabs = ModelRc::from(projection.model.clone());
    panels.set_row_data(0, panel);
    let state = book.clone();
    let visible = projection.clone();
    let weak = ui.as_weak();
    let publish = Rc::new(move || {
        let book = state.borrow();
        visible.update(&book, mode, &Default::default());
        weak.upgrade()
            .unwrap()
            .get_tab_bars()
            .set_row_data(0, visible.state(&book));
    });
    publish();
    let state = book.clone();
    let refresh = publish.clone();
    ui.on_group_toggle(move |panel, id| {
        assert_eq!(panel, 0);
        {
            let mut book = state.borrow_mut();
            let collapsed = !book.groups.get(&id).unwrap().collapsed;
            book.groups.set_collapsed(&id, collapsed).unwrap();
            book.reconcile_selection();
            book.focus_group(&id);
        }
        refresh();
    });
    let state = book.clone();
    let refresh = publish.clone();
    ui.on_group_clicked(move |panel, id, modifiers| {
        assert_eq!(panel, 0);
        state
            .borrow_mut()
            .click_group(&id, modifiers & 1 != 0, modifiers & 2 != 0);
        refresh();
    });
    let state = book.clone();
    let refresh = publish.clone();
    ui.on_tab_clicked(move |index, modifiers| {
        state
            .borrow_mut()
            .click(index as usize, modifiers & 1 != 0, modifiers & 2 != 0);
        refresh();
    });
    let state = book.clone();
    let refresh = publish.clone();
    ui.on_tab_wheel_selected(move |index| {
        state.borrow_mut().select_from_wheel(index as usize);
        refresh();
    });
    let state = book.clone();
    ui.on_panel_tab_step(move |panel, forward| {
        assert_eq!(panel, 0);
        projection.step(&state.borrow(), forward)
    });
    let state = book.clone();
    ui.on_reset_panel_tab_selection(move |panel| {
        assert_eq!(panel, 0);
        let changed = state.borrow_mut().reset_selection();
        publish();
        changed
    });
    (ui, book)
}

#[test]
fn group_folding_keeps_keyboard_commands_after_rebuilding_visible_members() {
    use slint::Model;
    use std::rc::Rc;
    crate::test_support::run(|| {
        for mode in 0..=2 {
            let (ui, book) = interactive_grouped_view(mode);
            let group = book.borrow().groups.entries()[0].id.clone();
            let saves = Rc::new(std::cell::Cell::new(0));
            let counted = saves.clone();
            ui.on_ws_save_current(move || counted.set(counted.get() + 1));
            ui.on_match_action(|key, ctrl, _, _, _| {
                if (ctrl && key.eq_ignore_ascii_case("s")) || key == "F8" {
                    "save-workspace".into()
                } else {
                    "".into()
                }
            });
            ui.show().unwrap();
            let panel_w = ui.get_panels_cont_w() / 2.0;
            let header = ui
                .get_panels()
                .row_data(0)
                .unwrap()
                .tabs
                .row_data(0)
                .unwrap();
            let position = LogicalPosition::new(
                ui.get_panels_cont_x()
                    + if mode == 0 {
                        header.width / 2.0
                    } else if mode == 1 {
                        60.0
                    } else {
                        panel_w - 70.0
                    },
                ui.get_panels_cont_y() + if mode == 0 { 20.0 } else { 60.0 },
            );
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
            ui.window().dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
            assert!(
                book.borrow().groups.get(&group).unwrap().collapsed,
                "mode {mode}"
            );
            for turn in 0..3 {
                for text in [Key::Control.into(), "s".into()] {
                    ui.window().dispatch_event(WindowEvent::KeyPressed { text });
                }
                for text in ["s".into(), Key::Control.into()] {
                    ui.window()
                        .dispatch_event(WindowEvent::KeyReleased { text });
                }
                assert_eq!(saves.get(), turn + 1, "mode {mode}, turn {turn}");
                ui.window().dispatch_event(WindowEvent::KeyPressed {
                    text: Key::Space.into(),
                });
                ui.window().dispatch_event(WindowEvent::KeyReleased {
                    text: Key::Space.into(),
                });
                assert_eq!(
                    book.borrow().groups.get(&group).unwrap().collapsed,
                    turn % 2 != 0,
                    "mode {mode}, turn {turn}"
                );
            }
            ui.window().dispatch_event(WindowEvent::KeyPressed {
                text: Key::F8.into(),
            });
            assert_eq!(saves.get(), 4, "mode {mode}");
            ui.hide().unwrap();
        }
    });
}

#[test]
fn mixed_selection_uses_modified_header_clicks_and_drags_in_every_orientation() {
    use slint::Model;
    use std::{cell::RefCell, rc::Rc};
    crate::test_support::run(|| {
        for mode in 0..=2 {
            let (ui, book) = interactive_grouped_view(mode);
            let groups: Vec<_> = book
                .borrow()
                .groups
                .entries()
                .iter()
                .map(|g| g.id.clone())
                .collect();
            ui.invoke_group_toggle(0, groups[0].as_str().into());
            ui.invoke_tab_clicked(4, 0);
            let captures = Rc::new(RefCell::new(Vec::new()));
            let log = captures.clone();
            let source = book.clone();
            ui.on_tab_drag_started(move |panel, index, group| {
                assert_eq!(panel, 0);
                let batch = super::batch::Batch::capture(
                    &source.borrow(),
                    index as usize,
                    (!group.is_empty()).then_some(group.as_str()),
                )
                .unwrap();
                log.borrow_mut().push(batch.ids);
                true
            });
            ui.show().unwrap();
            ui.window().set_size(slint::LogicalSize::new(2400.0, 800.0));
            let header_position = |id: &str| {
                let row = ui
                    .get_panels()
                    .row_data(0)
                    .unwrap()
                    .tabs
                    .iter()
                    .find(|row| row.is_group && row.group_id == id)
                    .unwrap();
                LogicalPosition::new(
                    ui.get_panels_cont_x()
                        + if mode == 0 {
                            row.offset + row.width / 2.0
                        } else if mode == 1 {
                            60.0
                        } else {
                            ui.get_panels_cont_w() / 2.0 - 70.0
                        },
                    ui.get_panels_cont_y() + if mode == 0 { 20.0 } else { 60.0 + row.offset },
                )
            };
            let click = |position| {
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
            };
            ui.window().dispatch_event(WindowEvent::KeyPressed {
                text: Key::Control.into(),
            });
            click(header_position(&groups[0]));
            click(header_position(&groups[1]));
            ui.window().dispatch_event(WindowEvent::KeyReleased {
                text: Key::Control.into(),
            });
            assert_eq!(book.borrow().selection_len(), 5, "mode {mode}");
            assert!(book.borrow().groups.get(&groups[0]).unwrap().collapsed);
            assert!(!book.borrow().groups.get(&groups[1]).unwrap().collapsed);
            assert_eq!(book.borrow().active, 4);
            assert!(book.borrow().group_page().is_none());
            let expected = book.borrow().selected_ids();
            let position = header_position(&groups[0]);
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
            let moved = LogicalPosition::new(position.x + 15.0, position.y + 10.0);
            ui.window()
                .dispatch_event(WindowEvent::PointerMoved { position: moved });
            ui.window().dispatch_event(WindowEvent::PointerReleased {
                position: moved,
                button: PointerEventButton::Left,
            });
            assert_eq!(*captures.borrow(), [expected], "mode {mode}");
            assert_eq!(book.borrow().selection_len(), 5);
            // A blank click releases the batch but keeps the ordinary active tab.
            click(LogicalPosition::new(
                ui.get_panels_cont_x() + ui.get_panels_cont_w() * 0.25,
                ui.get_panels_cont_y() + ui.get_panels_cont_h() * 0.7,
            ));
            assert_eq!(book.borrow().selection_len(), 1, "mode {mode}");
            assert_eq!(book.borrow().active, 4);
            ui.window().dispatch_event(WindowEvent::KeyPressed {
                text: Key::Shift.into(),
            });
            click(header_position(&groups[0]));
            ui.window().dispatch_event(WindowEvent::KeyReleased {
                text: Key::Shift.into(),
            });
            assert_eq!(book.borrow().selection_len(), 5, "mode {mode}");
            assert!(book.borrow().group_selected(&groups[0]));
            assert!(book.borrow().group_selected(&groups[1]));
            assert_eq!(book.borrow().active, 4);
            ui.hide().unwrap();
        }
    });
}

#[test]
fn focused_group_modified_neighbor_clicks_drag_the_complete_batch_in_every_orientation() {
    use slint::Model;
    use std::{cell::RefCell, rc::Rc};
    crate::test_support::run(|| {
        for mode in 0..=2 {
            for collapsed in [false, true] {
                for neighbor in [1, 4] {
                    for grab_header in [false, true] {
                        for modifier in [Key::Control, Key::Shift] {
                            let (ui, book) = interactive_grouped_view(mode);
                            let group = {
                                let mut borrowed = book.borrow_mut();
                                let book = &mut *borrowed;
                                let first = book.groups.entries()[0].id.clone();
                                book.groups.dissolve(&mut book.tabs, &first).unwrap();
                                let group = book.groups.entries()[0].id.clone();
                                book.groups.set_collapsed(&group, !collapsed).unwrap();
                                group
                            };
                            ui.invoke_tab_clicked(neighbor, 0);
                            let captures = Rc::new(RefCell::new(Vec::new()));
                            let captured = captures.clone();
                            let source = book.clone();
                            ui.on_tab_drag_started(move |panel, index, group| {
                                assert_eq!(panel, 0);
                                let batch = super::batch::Batch::capture(
                                    &source.borrow(),
                                    index as usize,
                                    (!group.is_empty()).then_some(group.as_str()),
                                )
                                .unwrap();
                                captured.borrow_mut().push(batch.ids);
                                true
                            });
                            ui.show().unwrap();
                            ui.window().set_size(slint::LogicalSize::new(2400.0, 800.0));
                            let position_of = |header: bool| {
                                let rows = ui.get_panels().row_data(0).unwrap().tabs;
                                let row = rows
                                    .iter()
                                    .find(|row| {
                                        if header {
                                            row.is_group && row.group_id == group
                                        } else {
                                            !row.is_group && row.index == neighbor
                                        }
                                    })
                                    .unwrap();
                                LogicalPosition::new(
                                    ui.get_panels_cont_x()
                                        + if mode == 0 {
                                            row.offset + row.width / 2.0
                                        } else if mode == 1 {
                                            60.0
                                        } else {
                                            ui.get_panels_cont_w() / 2.0 - 70.0
                                        },
                                    ui.get_panels_cont_y()
                                        + if mode == 0 { 20.0 } else { 60.0 + row.offset },
                                )
                            };
                            let click = |position| {
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
                            };
                            click(position_of(true));
                            assert_eq!(book.borrow().group_page().unwrap().id, group);
                            assert_eq!(
                                book.borrow().groups.get(&group).unwrap().collapsed,
                                collapsed
                            );
                            assert!(book.borrow().selected_ids().is_empty());
                            ui.window().dispatch_event(WindowEvent::KeyPressed {
                                text: modifier.into(),
                            });
                            click(position_of(false));
                            ui.window().dispatch_event(WindowEvent::KeyReleased {
                                text: modifier.into(),
                            });
                            let expected: Vec<_> = {
                                let book = book.borrow();
                                assert!(
                                    book.group_selected(&group),
                                    "mode {mode}, collapsed {collapsed}, neighbor {neighbor}, modifier {modifier:?}"
                                );
                                assert_eq!(book.selection_len(), 3);
                                book.tabs
                                    .iter()
                                    .enumerate()
                                    .filter(|(index, _)| [2, 3, neighbor as usize].contains(index))
                                    .map(|(_, tab)| tab.id)
                                    .collect()
                            };
                            let tabs = ui.get_panels().row_data(0).unwrap().tabs;
                            assert!(
                                tabs.iter()
                                    .find(|row| row.index == neighbor)
                                    .unwrap()
                                    .selection_marked
                            );
                            assert!(
                                tabs.iter()
                                    .find(|row| row.is_group)
                                    .unwrap()
                                    .selection_marked
                            );
                            let position = position_of(grab_header);
                            ui.window().dispatch_event(WindowEvent::PointerPressed {
                                position,
                                button: PointerEventButton::Left,
                            });
                            let moved = LogicalPosition::new(position.x + 15.0, position.y + 10.0);
                            ui.window()
                                .dispatch_event(WindowEvent::PointerMoved { position: moved });
                            ui.window().dispatch_event(WindowEvent::PointerReleased {
                                position: moved,
                                button: PointerEventButton::Left,
                            });
                            assert_eq!(*captures.borrow(), [expected]);
                            assert_eq!(book.borrow().selection_len(), 3);
                            assert_eq!(book.borrow().group_page().unwrap().id, group);
                            ui.hide().unwrap();
                        }
                    }
                }
            }
        }
    });
}

#[test]
fn group_outlines_follow_navigation_and_persist_on_blank_or_modified_clicks() {
    use slint::Model;
    crate::test_support::run(|| {
        for mode in 0..=2 {
            let (ui, book) = interactive_grouped_view(mode);
            ui.show().unwrap();
            ui.window().set_size(slint::LogicalSize::new(2400.0, 800.0));
            let groups: Vec<_> = book
                .borrow()
                .groups
                .entries()
                .iter()
                .map(|group| group.id.clone())
                .collect();
            let check = |expected: Option<&str>| {
                let marked: Vec<_> = ui
                    .get_panels()
                    .row_data(0)
                    .unwrap()
                    .tabs
                    .iter()
                    .filter(|row| row.group_highlighted)
                    .map(|row| row.group_id)
                    .collect();
                assert!(marked.len() <= 1, "mode {mode}");
                assert_eq!(
                    marked.first().map(|id| id.as_str()),
                    expected,
                    "mode {mode}"
                );
            };
            let click_at = |position| {
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
            };
            let position_of = |header: bool, index: usize| {
                let rows = ui.get_panels().row_data(0).unwrap().tabs;
                let row = if header {
                    rows.iter().filter(|row| row.is_group).nth(index).unwrap()
                } else {
                    rows.iter()
                        .find(|row| !row.is_group && row.index == index as i32)
                        .unwrap()
                };
                let panel_w = ui.get_panels_cont_w() / 2.0;
                LogicalPosition::new(
                    ui.get_panels_cont_x()
                        + if mode == 0 {
                            row.offset + row.width / 2.0
                        } else if mode == 1 {
                            60.0
                        } else {
                            panel_w - 70.0
                        },
                    ui.get_panels_cont_y() + if mode == 0 { 20.0 } else { 60.0 + row.offset },
                )
            };
            let wheel = || {
                ui.window().dispatch_event(WindowEvent::PointerScrolled {
                    position: position_of(true, 0),
                    delta_x: 0.0,
                    delta_y: -40.0,
                });
            };
            check(None);
            click_at(position_of(true, 0));
            check(Some(&groups[0]));
            assert_eq!(book.borrow().active, 4);
            click_at(position_of(false, 4));
            check(None);
            click_at(position_of(true, 0));
            click_at(position_of(false, 1));
            check(Some(&groups[0]));
            wheel();
            check(Some(&groups[1]));
            assert_eq!(book.borrow().active, 2);

            let rows = ui.get_panels().row_data(0).unwrap().tabs;
            let last = rows.row_data(rows.row_count() - 1).unwrap();
            let end = last.offset + last.width;
            let panel_w = ui.get_panels_cont_w() / 2.0;
            let blank = LogicalPosition::new(
                ui.get_panels_cont_x()
                    + if mode == 0 {
                        (end + panel_w - 80.0) / 2.0
                    } else if mode == 1 {
                        60.0
                    } else {
                        panel_w - 70.0
                    },
                ui.get_panels_cont_y()
                    + if mode == 0 {
                        20.0
                    } else {
                        ui.get_panels_cont_h() * 0.7
                    },
            );
            if mode == 0 {
                assert!(
                    end + 8.0 < panel_w - 80.0,
                    "entries end at {end}, panel width {panel_w}"
                );
            }
            click_at(blank);
            check(Some(&groups[1]));
            assert_eq!(book.borrow().active, 2);
            ui.window().dispatch_event(WindowEvent::KeyPressed {
                text: Key::Space.into(),
            });
            ui.window().dispatch_event(WindowEvent::KeyReleased {
                text: Key::Space.into(),
            });
            assert!(
                book.borrow()
                    .groups
                    .entries()
                    .iter()
                    .all(|group| !group.collapsed)
            );
            wheel();
            check(Some(&groups[1]));
            click_at(position_of(true, 1));
            wheel();
            check(None);
            assert_eq!(book.borrow().active, 4);
            wheel();
            check(Some(&groups[0]));
            assert_eq!(book.borrow().active, 0);
            ui.window().dispatch_event(WindowEvent::KeyPressed {
                text: Key::Control.into(),
            });
            click_at(position_of(false, 1));
            ui.window().dispatch_event(WindowEvent::KeyReleased {
                text: Key::Control.into(),
            });
            check(Some(&groups[0]));
            assert_eq!(book.borrow().active, 0);
            assert_eq!(book.borrow().selection_len(), 2);
            click_at(blank);
            check(Some(&groups[0]));
            assert_eq!(book.borrow().selection_len(), 1);
            let view_body = LogicalPosition::new(
                ui.get_panels_cont_x() + panel_w * 0.5,
                ui.get_panels_cont_y() + ui.get_panels_cont_h() * 0.7,
            );
            click_at(view_body);
            check(Some(&groups[0]));
            assert_eq!(book.borrow().active, 0);
            click_at(position_of(false, 0));
            check(Some(&groups[0]));
            ui.hide().unwrap();
        }
    });
}

#[test]
fn wheel_activation_uses_its_own_selection_path_in_every_bar_orientation() {
    use std::{cell::RefCell, rc::Rc};
    crate::test_support::run(|| {
        for mode in 0..=2 {
            let ui = window_with_grouped_views(mode);
            let directions = Rc::new(RefCell::new(Vec::new()));
            let captured = directions.clone();
            ui.on_panel_tab_step(move |panel, forward| {
                assert_eq!(panel, 0);
                captured.borrow_mut().push(forward);
                1
            });
            let selections = Rc::new(RefCell::new(Vec::new()));
            let captured = selections.clone();
            ui.on_tab_wheel_selected(move |index| captured.borrow_mut().push(index));
            ui.on_tab_clicked(|_, _| panic!("wheel used ordinary tab selection"));
            ui.show().unwrap();
            let panel_w = ui.get_panels_cont_w() / 2.0;
            let position = LogicalPosition::new(
                ui.get_panels_cont_x()
                    + if mode == 0 || mode == 1 {
                        60.0
                    } else {
                        panel_w - 70.0
                    },
                ui.get_panels_cont_y() + if mode == 0 { 20.0 } else { 60.0 },
            );
            for delta in [-40.0, 40.0] {
                ui.window().dispatch_event(WindowEvent::PointerScrolled {
                    position,
                    delta_x: 0.0,
                    delta_y: delta,
                });
            }
            assert_eq!(*directions.borrow(), [true, false], "mode {mode}");
            assert_eq!(*selections.borrow(), [1, 1], "mode {mode}");
            ui.set_ctx_menu_open(true);
            ui.window().dispatch_event(WindowEvent::PointerScrolled {
                position,
                delta_x: 0.0,
                delta_y: -40.0,
            });
            assert_eq!(selections.borrow().len(), 2, "mode {mode}");
            ui.hide().unwrap();
        }
    });
}

#[test]
fn clicking_off_the_tabs_releases_a_tab_selection_in_every_orientation() {
    use slint::Model;
    use std::{cell::Cell, rc::Rc};
    crate::test_support::run(|| {
        for mode in 0..=2 {
            let ui = window_with_grouped_views(mode);
            let resets = Rc::new(Cell::new(0));
            let counted = resets.clone();
            ui.on_reset_panel_tab_selection(move |panel| {
                assert_eq!(panel, 0);
                counted.set(counted.get() + 1);
                false
            });
            ui.show().unwrap();
            let tabs = ui.get_panels().row_data(0).unwrap().tabs;
            let last = tabs.row_data(tabs.row_count() - 1).unwrap();
            let end = last.offset + last.width;
            let panel_w = ui.get_panels_cont_w() / 2.0;
            let position = if mode == 0 {
                // Past the last tab, and clear of the fixed cluster the strip
                // reserves on its right.
                let free = panel_w - 80.0;
                assert!(end + 8.0 < free, "no empty strip: {end} in {panel_w}");
                LogicalPosition::new(
                    ui.get_panels_cont_x() + (end + free) / 2.0,
                    ui.get_panels_cont_y() + 20.0,
                )
            } else {
                // Below the last entry of the vertical bar, on whichever side
                // of the panel it is docked.
                LogicalPosition::new(
                    ui.get_panels_cont_x() + if mode == 1 { 60.0 } else { panel_w - 70.0 },
                    ui.get_panels_cont_y() + ui.get_panels_cont_h() * 0.6,
                )
            };
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
            assert_eq!(resets.get(), 1, "mode {mode}");
            // The blank area of the listing underneath picks no row and no
            // tab either, so it releases the selection the same way.
            let listing = LogicalPosition::new(
                ui.get_panels_cont_x() + panel_w * 0.5,
                ui.get_panels_cont_y() + ui.get_panels_cont_h() * 0.5,
            );
            ui.window()
                .dispatch_event(WindowEvent::PointerMoved { position: listing });
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position: listing,
                button: PointerEventButton::Left,
            });
            ui.window().dispatch_event(WindowEvent::PointerReleased {
                position: listing,
                button: PointerEventButton::Left,
            });
            assert_eq!(resets.get(), 2, "mode {mode}");
            // A group header picks the group, not a batch of tabs, so it
            // releases the selection as well. The horizontal strip stands for
            // the three orientations here: they share this one callback path.
            if mode == 0 {
                let header = tabs.row_data(0).unwrap();
                assert!(header.is_group);
                let on_header = LogicalPosition::new(
                    ui.get_panels_cont_x() + header.offset + header.width / 2.0,
                    ui.get_panels_cont_y() + 20.0,
                );
                ui.window().dispatch_event(WindowEvent::PointerMoved {
                    position: on_header,
                });
                ui.window().dispatch_event(WindowEvent::PointerPressed {
                    position: on_header,
                    button: PointerEventButton::Left,
                });
                ui.window().dispatch_event(WindowEvent::PointerReleased {
                    position: on_header,
                    button: PointerEventButton::Left,
                });
                assert_eq!(resets.get(), 3, "mode {mode}");
            }
            ui.hide().unwrap();
        }
    });
}

#[test]
fn clicking_unavailable_view_content_releases_tab_selection() {
    use slint::Model;
    use std::{cell::Cell, rc::Rc};
    crate::test_support::run(|| {
        let ui = window_with_grouped_views(0);
        let panels = ui.get_panels();
        let mut panel = panels.row_data(0).unwrap();
        panel.unavailable = true;
        panel.current_path = "folder01/missing".into();
        panels.set_row_data(0, panel);
        let tabs = panels.row_data(0).unwrap().tabs;
        let mut valid_tab = tabs.row_data(2).unwrap();
        assert!(!valid_tab.is_group);
        valid_tab.title = "folder02".into();
        valid_tab.selection_marked = true;
        tabs.set_row_data(2, valid_tab);

        let selected = Rc::new(Cell::new(true));
        let recovery_saw_selection = Rc::new(Cell::new(false));
        let selected_for_retry = selected.clone();
        let saw_for_retry = recovery_saw_selection.clone();
        let retry_panels = panels.clone();
        ui.on_panel_clicked(move |_| {
            saw_for_retry.set(selected_for_retry.get());
            let mut panel = retry_panels.row_data(0).unwrap();
            panel.unavailable = false;
            retry_panels.set_row_data(0, panel);
        });

        let resets = Rc::new(Cell::new(0));
        let counted = resets.clone();
        let selected_for_reset = selected.clone();
        let tabs_for_reset = tabs.clone();
        ui.on_reset_panel_tab_selection(move |panel| {
            assert_eq!(panel, 0);
            counted.set(counted.get() + 1);
            selected_for_reset.set(false);
            let mut valid_tab = tabs_for_reset.row_data(2).unwrap();
            valid_tab.selection_marked = false;
            tabs_for_reset.set_row_data(2, valid_tab);
            false
        });
        ui.show().unwrap();
        let position = LogicalPosition::new(
            ui.get_panels_cont_x() + 40.0,
            ui.get_panels_cont_y() + ui.get_panels_cont_h() * 0.6,
        );
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
        assert_eq!(resets.get(), 1);
        assert!(!recovery_saw_selection.get());
        assert!(!tabs.row_data(2).unwrap().selection_marked);
        ui.hide().unwrap();
    });
}

#[test]
fn unavailable_view_releases_selection_before_tab_retry_or_address_edit() {
    use slint::Model;
    use std::{cell::Cell, rc::Rc};
    crate::test_support::run(|| {
        for address in [false, true] {
            let ui = window_with_grouped_views(0);
            let panels = ui.get_panels();
            let mut panel = panels.row_data(0).unwrap();
            panel.unavailable = true;
            panel.current_path = "folder01/missing".into();
            panels.set_row_data(0, panel);
            let tabs = panels.row_data(0).unwrap().tabs;
            let mut marked = tabs.row_data(2).unwrap();
            marked.selection_marked = true;
            tabs.set_row_data(2, marked);

            let selected = Rc::new(Cell::new(true));
            let recovery_saw_selection = Rc::new(Cell::new(false));
            let selected_for_retry = selected.clone();
            let saw_for_retry = recovery_saw_selection.clone();
            let retry_panels = panels.clone();
            ui.on_panel_clicked(move |_| {
                saw_for_retry.set(selected_for_retry.get());
                let panel = retry_panels.row_data(0).unwrap();
                retry_panels.set_row_data(0, panel);
            });

            let selected_for_reset = selected.clone();
            let tabs_for_reset = tabs.clone();
            ui.on_reset_panel_tab_selection(move |panel| {
                assert_eq!(panel, 0);
                selected_for_reset.set(false);
                let mut row = tabs_for_reset.row_data(2).unwrap();
                row.selection_marked = false;
                tabs_for_reset.set_row_data(2, row);
                true
            });
            let selected_for_tab = selected.clone();
            let tabs_for_tab = tabs.clone();
            ui.on_tab_clicked(move |index, modifiers| {
                assert_eq!((index, modifiers), (0, 0));
                selected_for_tab.set(false);
                let mut row = tabs_for_tab.row_data(2).unwrap();
                row.selection_marked = false;
                tabs_for_tab.set_row_data(2, row);
            });

            ui.show().unwrap();
            let x = ui.get_panels_cont_x();
            let y = ui.get_panels_cont_y();
            let position = if address {
                LogicalPosition::new(x + 380.0, y + 65.0)
            } else {
                let tab = tabs.row_data(1).unwrap();
                LogicalPosition::new(x + 13.0 + tab.offset + tab.width / 2.0, y + 20.0)
            };
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
            assert!(!selected.get(), "address edit: {address}");
            assert!(!recovery_saw_selection.get(), "address edit: {address}");
            assert!(!tabs.row_data(2).unwrap().selection_marked);
            ui.hide().unwrap();
        }
    });
}

#[test]
fn inactive_unavailable_view_clears_its_own_selection_before_activation() {
    use slint::Model;
    use std::{cell::Cell, rc::Rc};
    crate::test_support::run(|| {
        let ui = window_with_grouped_views(0);
        let panels = ui.get_panels();
        let mut panel = panels.row_data(1).unwrap();
        panel.unavailable = true;
        panel.current_path = "folder02/missing".into();
        panels.set_row_data(1, panel);
        let tabs = panels.row_data(1).unwrap().tabs;
        let mut marked = tabs.row_data(2).unwrap();
        marked.selection_marked = true;
        tabs.set_row_data(2, marked);

        let released = Rc::new(Cell::new(false));
        let released_for_reset = released.clone();
        let tabs_for_reset = tabs.clone();
        ui.on_reset_panel_tab_selection(move |index| {
            assert_eq!(index, 1);
            released_for_reset.set(true);
            let mut row = tabs_for_reset.row_data(2).unwrap();
            row.selection_marked = false;
            tabs_for_reset.set_row_data(2, row);
            true
        });
        let released_for_activate = released.clone();
        let retry_panels = panels.clone();
        ui.on_panel_clicked(move |index| {
            assert_eq!(index, 1);
            assert!(released_for_activate.get());
            let panel = retry_panels.row_data(1).unwrap();
            retry_panels.set_row_data(1, panel);
        });

        ui.show().unwrap();
        let position = LogicalPosition::new(
            ui.get_panels_cont_x() + ui.get_panels_cont_w() * 0.75,
            ui.get_panels_cont_y() + ui.get_panels_cont_h() * 0.6,
        );
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
        assert!(released.get());
        assert!(!tabs.row_data(2).unwrap().selection_marked);
        ui.hide().unwrap();
    });
}

#[test]
fn view_controls_release_tab_selection_without_touching_tab_clicks() {
    use slint::{Model, ModelRc, VecModel};
    use std::{cell::Cell, rc::Rc};
    crate::test_support::run(|| {
        let ui = window_with_grouped_views(0);
        ui.set_left_panel(0);
        let panels = ui.get_panels();
        let mut panel = panels.row_data(0).unwrap();
        panel.can_go_back = true;
        panel.current_path = "folder01/folder02".into();
        panel.crumbs = ModelRc::new(VecModel::from(vec![crate::Crumb {
            label: "folder02".into(),
            path: "folder01/folder02".into(),
        }]));
        panel.columns = ModelRc::new(VecModel::from(vec![crate::ColumnInfo {
            id: "name".into(),
            label: "Name".into(),
            visible: true,
            ..Default::default()
        }]));
        panel.sort_column = "name".into();
        panel.col_name_w = 180.0;
        panel.content_width = 180.0;
        panels.set_row_data(0, panel);

        let selected = Rc::new(Cell::new(true));
        let captured = selected.clone();
        ui.on_reset_panel_tab_selection(move |panel| {
            assert_eq!(panel, 0);
            captured.replace(false)
        });
        ui.show().unwrap();
        slint::platform::update_timers_and_animations();
        let x = ui.get_panels_cont_x();
        let y = ui.get_panels_cont_y();
        let click = |position: LogicalPosition| {
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
        };

        let tabs = ui.get_panels().row_data(0).unwrap().tabs;
        let member = tabs.row_data(1).unwrap();
        assert!(!member.is_group);
        click(LogicalPosition::new(
            x + 13.0 + member.offset + member.width / 2.0,
            y + 20.0,
        ));
        assert!(
            selected.get(),
            "clicking a tab must preserve the selection gesture"
        );

        for (label, position) in [
            ("back", LogicalPosition::new(x + 30.0, y + 65.0)),
            ("address", LogicalPosition::new(x + 380.0, y + 65.0)),
            ("column", LogicalPosition::new(x + 60.0, y + 108.0)),
        ] {
            selected.set(true);
            click(position);
            assert!(!selected.get(), "{label} did not release the tab selection");
        }

        let mut panel = panels.row_data(0).unwrap();
        panel.grid_mode = true;
        panels.set_row_data(0, panel);
        slint::platform::update_timers_and_animations();
        selected.set(true);
        click(LogicalPosition::new(x + 60.0, y + 108.0));
        assert!(
            !selected.get(),
            "grid header did not release the tab selection"
        );

        selected.set(true);
        click(LogicalPosition::new(x + 174.0, y + 65.0));
        assert!(
            !selected.get(),
            "display options did not release the tab selection"
        );
        ui.hide().unwrap();
    });
}

#[cfg(windows)]
#[test]
fn native_file_drag_releases_pointer_before_external_drop_menu() {
    use std::{cell::RefCell, rc::Rc};
    crate::test_support::run(|| {
        let ui = window_with_grouped_views(0);
        ui.show().unwrap();
        let listing = LogicalPosition::new(
            ui.get_panels_cont_x() + ui.get_panels_cont_w() * 0.25,
            ui.get_panels_cont_y() + ui.get_panels_cont_h() * 0.5,
        );
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position: listing });
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: listing,
            button: PointerEventButton::Left,
        });
        assert!(ui.get_rows_pointer_busy());

        // The native drag's modal loop can consume the matching pointer release.
        ui.set_native_drag_active(true);
        crate::bridge::release_native_drag_pointer(&ui, -10.0, -10.0);
        ui.set_native_drag_active(false);
        assert!(!ui.get_rows_pointer_busy());

        let actions = Rc::new(RefCell::new(Vec::new()));
        let received = actions.clone();
        ui.on_file_drop_action(move |action| received.borrow_mut().push(action));
        ui.invoke_prepare_external_file_menu();
        ui.set_file_drop_menu_x(100.0);
        ui.set_file_drop_menu_y(120.0);
        ui.set_file_drop_menu_open(true);
        let choice = LogicalPosition::new(150.0, 137.0);
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position: choice });
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: choice,
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position: choice,
            button: PointerEventButton::Left,
        });
        assert_eq!(*actions.borrow(), [0]);
        ui.hide().unwrap();
    });
}

#[test]
fn group_choices_pointer_stays_active_past_close_delay() {
    use std::{cell::RefCell, rc::Rc, time::Duration};
    crate::test_support::run(|| {
        for (count, item_y) in [(1, 227.0), (2, 143.0)] {
            let ui = window_with_grouped_views(0);
            let commands = Rc::new(RefCell::new(Vec::new()));
            let calls = commands.clone();
            ui.on_group_command(move |action, id| {
                calls.borrow_mut().push((action, id.to_string()))
            });
            ui.on_tab_context_selection(move |_, _| count);
            ui.show().unwrap();
            ui.invoke_prepare_external_file_menu();
            let tab = LogicalPosition::new(
                ui.get_panels_cont_x() + 180.0,
                ui.get_panels_cont_y() + 30.0,
            );
            ui.window()
                .dispatch_event(WindowEvent::PointerMoved { position: tab });
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position: tab,
                button: PointerEventButton::Right,
            });
            ui.window().dispatch_event(WindowEvent::PointerReleased {
                position: tab,
                button: PointerEventButton::Right,
            });
            assert!(ui.get_tab_menu_open());
            ui.set_tab_menu_x(100.0);
            ui.set_tab_menu_y(100.0);
            for key in [Key::UpArrow, Key::RightArrow] {
                slint::platform::update_timers_and_animations();
                ui.window()
                    .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
                slint::platform::update_timers_and_animations();
                ui.window()
                    .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
            }
            let position = LogicalPosition::new(410.0, item_y);
            ui.window()
                .dispatch_event(WindowEvent::PointerMoved { position });
            slint::platform::update_timers_and_animations();
            std::thread::sleep(Duration::from_millis(320));
            slint::platform::update_timers_and_animations();
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
            ui.window().dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
            assert_eq!(
                *commands.borrow(),
                [(0, String::new())],
                "selection count {count}"
            );
            ui.hide().unwrap();
        }
    });
}

#[test]
fn group_page_clicks_clear_tab_batches_and_keep_global_shortcuts() {
    use slint::{Model, ModelRc, VecModel};
    use std::{cell::Cell, rc::Rc};
    crate::test_support::run(|| {
        for mode in 0..=2 {
            for unavailable in [false, true] {
                let (ui, book) = interactive_grouped_view(mode);
                let id = book.borrow().groups.entries()[0].id.clone();
                book.borrow_mut().tabs[2].current_path = "folder01/missing".into();
                let panels = ui.get_panels();
                let mut panel = panels.row_data(0).unwrap();
                panel.unavailable = unavailable;
                panel.rows = ModelRc::new(VecModel::from(vec![crate::FileRow {
                    name: "File01.txt".into(),
                    selected: true,
                    visual_h: 2000.0,
                    ..Default::default()
                }]));
                panels.set_row_data(0, panel);
                let weak = ui.as_weak();
                ui.on_panel_clicked(move |index| {
                    weak.upgrade().unwrap().set_active_panel_idx(index)
                });
                let saves = Rc::new(Cell::new(0));
                let counted = saves.clone();
                ui.on_ws_save_current(move || counted.set(counted.get() + 1));
                ui.on_action_paste(|| panic!("group page used a retained folder for paste"));
                ui.on_action_delete(|| panic!("group page used a retained file selection"));
                ui.on_match_action(|key, ctrl, _, _, _| {
                    match (key.as_str(), ctrl) {
                        ("s", true) => "save-workspace",
                        ("v", true) => "paste",
                        ("l", true) => "focus-address",
                        ("F6", _) => "delete",
                        _ => "",
                    }
                    .into()
                });
                ui.show().unwrap();
                ui.window().set_size(slint::LogicalSize::new(1600., 800.));
                ui.invoke_group_toggle(0, id.as_str().into());
                assert!(ui.get_tab_bars().row_data(0).unwrap().group_page);
                assert_eq!(book.borrow().active, 4);
                let panel_w = ui.get_panels_cont_w() / 2.;
                for x in [
                    panel_w * 0.5,
                    if mode == 1 { 150. } else { 20. },
                    panel_w - if mode == 2 { 150. } else { 20. },
                ] {
                    ui.invoke_tab_clicked(2, 1);
                    ui.invoke_tab_clicked(3, 1);
                    assert_eq!(book.borrow().selection_len(), 4);
                    assert!(book.borrow().group_selected(&id));
                    ui.set_active_panel_idx(1);
                    let position = LogicalPosition::new(
                        ui.get_panels_cont_x() + x,
                        ui.get_panels_cont_y() + ui.get_panels_cont_h() * 0.7,
                    );
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
                    assert_eq!(ui.get_active_panel_idx(), 0, "mode: {mode}");
                    assert_eq!(book.borrow().selection_len(), 0);
                    assert_eq!(book.borrow().group_page().unwrap().id, id);
                    assert_eq!(book.borrow().active, 4);
                    ui.window().dispatch_event(WindowEvent::KeyPressed {
                        text: Key::Control.into(),
                    });
                    for text in ["s", "v", "l"] {
                        ui.window()
                            .dispatch_event(WindowEvent::KeyPressed { text: text.into() });
                        ui.window()
                            .dispatch_event(WindowEvent::KeyReleased { text: text.into() });
                    }
                    ui.window().dispatch_event(WindowEvent::KeyReleased {
                        text: Key::Control.into(),
                    });
                    ui.window().dispatch_event(WindowEvent::KeyPressed {
                        text: Key::F6.into(),
                    });
                    ui.window().dispatch_event(WindowEvent::KeyReleased {
                        text: Key::F6.into(),
                    });
                    assert!(ui.get_url_edit_text().is_empty());
                }
                assert_eq!(saves.get(), 3);
                ui.hide().unwrap();
            }
        }
    });
}

#[test]
fn group_pages_keep_keyboard_navigation_with_visible_and_hidden_tab_bars() {
    use slint::Model;
    crate::test_support::run(|| {
        for mode in 0..=3 {
            let (ui, book) = interactive_grouped_view(mode);
            let id = book.borrow().groups.entries()[0].id.clone();
            ui.on_match_action(|key, _, _, _, _| {
                match key.as_str() {
                    "F6" => "tab-next",
                    "F7" => "tab-prev",
                    _ => "",
                }
                .into()
            });
            ui.show().unwrap();
            ui.invoke_group_toggle(0, id.as_str().into());
            ui.invoke_prepare_external_file_menu();
            let press = |key: Key| {
                ui.window()
                    .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
                ui.window()
                    .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
            };
            press(Key::F6);
            assert!(book.borrow().group_page().is_none(), "mode {mode}");
            assert_eq!(book.borrow().active, if mode == 3 { 0 } else { 2 });
            press(Key::F7);
            if mode == 3 {
                assert!(book.borrow().group_page().is_none());
                assert_eq!(book.borrow().active, 4);
                assert!(!book.borrow().groups.get(&id).unwrap().collapsed);
            } else {
                assert_eq!(book.borrow().group_page().unwrap().id, id);
                assert_eq!(book.borrow().active, 2);
                ui.invoke_group_toggle(0, id.as_str().into());
                assert!(ui.get_tab_bars().row_data(0).unwrap().group_page);
                assert!(!book.borrow().groups.get(&id).unwrap().collapsed);
                press(Key::F6);
                assert!(book.borrow().group_page().is_none());
                assert_eq!(book.borrow().active, 0);
            }
            ui.hide().unwrap();
        }
    });
}

#[test]
fn hidden_tab_bar_keeps_shortcuts_and_body_drop_zones() {
    use std::{cell::RefCell, rc::Rc};
    crate::test_support::run(|| {
        let ui = window_with_grouped_views(3);
        ui.set_active_panel_idx(1);
        let selected = Rc::new(RefCell::new(Vec::new()));
        let calls = selected.clone();
        ui.on_tab_clicked(move |index, _| calls.borrow_mut().push(index));
        let modes = Rc::new(RefCell::new(Vec::new()));
        let calls = modes.clone();
        ui.on_set_tab_bar_mode(move |panel, mode| calls.borrow_mut().push((panel, mode)));
        ui.on_match_action(|key, _, _, _, _| {
            match key.as_str() {
                "F6" => "tab-next",
                "F7" => "tab-prev",
                "F8" => "tab-bar-toggle",
                _ => "",
            }
            .into()
        });
        ui.show().unwrap();
        ui.invoke_prepare_external_file_menu();
        for key in [Key::F6, Key::F7, Key::F8] {
            ui.window()
                .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
            ui.window()
                .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
        }
        assert_eq!(*selected.borrow(), [1, 1]);
        assert_eq!(*modes.borrow(), [(1, -1)]);

        let targets = Rc::new(RefCell::new(Vec::new()));
        let calls = targets.clone();
        ui.on_external_tab_drop_resolved(move |_, panel, zone, gap, group| {
            calls
                .borrow_mut()
                .push((panel, zone, gap, group.to_string()));
        });
        ui.set_drag_source_panel(-1);
        ui.set_drag_active(true);
        ui.set_tab_drag_count(1);
        let x = ui.get_panels_cont_x();
        let y = ui.get_panels_cont_y();
        let half = ui.get_panels_cont_w() / 2.0;
        let height = ui.get_panels_cont_h();
        for (query, px, py, zone) in [
            (1, half / 2.0, height * 0.57, 1),
            (2, 20.0, height * 0.57, 2),
            (3, half - 20.0, height * 0.57, 3),
            (4, half / 2.0, 165.0, 4),
            (5, half / 2.0, height - 30.0, 5),
        ] {
            targets.borrow_mut().clear();
            ui.set_drag_abs_x(x + half + px);
            ui.set_drag_abs_y(y + py);
            ui.set_external_tab_drop_query(query);
            slint::platform::update_timers_and_animations();
            assert_eq!(*targets.borrow(), [(1, zone, -1, String::new())]);
        }
        ui.hide().unwrap();
    });
}

#[test]
fn window_drag_captures_a_group_and_delivers_its_destination_before_clearing_feedback() {
    use std::{cell::RefCell, rc::Rc};
    crate::test_support::run(|| {
        for mode in 0..=2 {
            let ui = window_with_grouped_views(mode);
            let starts = Rc::new(RefCell::new(Vec::new()));
            let capture = starts.clone();
            let weak = ui.as_weak();
            ui.on_tab_drag_started(move |panel, tab, group| {
                capture.borrow_mut().push((panel, tab, group.to_string()));
                weak.unwrap().set_tab_drag_count(2);
                true
            });
            let drops = Rc::new(RefCell::new(Vec::new()));
            let capture = drops.clone();
            ui.on_tab_drag_completed(move |panel, zone, gap, group, _, _| {
                capture
                    .borrow_mut()
                    .push((panel, zone, gap, group.to_string()));
            });
            ui.show().unwrap();
            slint::platform::update_timers_and_animations();
            let origin = ui.get_panels_cont_x();
            let half = ui.get_panels_cont_w() / 2.0;
            let x = origin
                + match mode {
                    0 => 52.0,
                    1 => 36.0,
                    _ => half - 116.0,
                };
            let y = ui.get_panels_cont_y() + if mode == 0 { 30.0 } else { 61.0 };
            let start = LogicalPosition::new(x, y);
            let end = LogicalPosition::new(x + half, y);
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position: start,
                button: PointerEventButton::Left,
            });
            ui.window().dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(x + 12.0, y),
            });
            ui.window()
                .dispatch_event(WindowEvent::PointerMoved { position: end });
            slint::platform::update_timers_and_animations();
            assert_eq!(*starts.borrow(), [(0, -1, "g1".into())], "mode {mode}");
            assert_eq!(ui.get_drag_target_group(), "g1", "mode {mode}");
            ui.window().dispatch_event(WindowEvent::PointerReleased {
                position: end,
                button: PointerEventButton::Left,
            });
            assert_eq!(*drops.borrow(), [(1, 0, 2, "g1".into())], "mode {mode}");
            assert!(!ui.get_drag_active());
            assert!(ui.get_drag_target_group().is_empty());
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position: start,
                button: PointerEventButton::Left,
            });
            ui.window()
                .dispatch_event(WindowEvent::PointerMoved { position: end });
            ui.window().dispatch_event(WindowEvent::KeyPressed {
                text: Key::Escape.into(),
            });
            slint::platform::update_timers_and_animations();
            ui.window().dispatch_event(WindowEvent::PointerReleased {
                position: end,
                button: PointerEventButton::Left,
            });
            assert_eq!(drops.borrow().len(), 1, "Escape must not drop, mode {mode}");
            assert!(!ui.get_drag_active());
            ui.hide().unwrap();
        }
    });
}

#[test]
fn external_drop_queries_use_the_final_point_in_all_bar_orientations() {
    use std::{cell::RefCell, rc::Rc};
    crate::test_support::run(|| {
        for mode in 0..=2 {
            let ui = window_with_grouped_views(mode);
            let drops = Rc::new(RefCell::new(Vec::new()));
            let capture = drops.clone();
            ui.on_external_tab_drop_resolved(move |query, panel, zone, gap, group| {
                capture
                    .borrow_mut()
                    .push((query, panel, zone, gap, group.to_string()));
            });
            ui.show().unwrap();
            slint::platform::update_timers_and_animations();
            let x = ui.get_panels_cont_x();
            let y = ui.get_panels_cont_y();
            let half = ui.get_panels_cont_w() / 2.0;
            let h = ui.get_panels_cont_h();
            let header_x = match mode {
                0 => 52.0,
                1 => 36.0,
                _ => half - 116.0,
            };
            let header_y = if mode == 0 { 30.0 } else { 61.0 };
            ui.set_drag_source_panel(-1);
            ui.set_drag_active(true);
            ui.set_tab_drag_count(2);
            // The last hover belongs to the first view. No update is pumped
            // between changing the drop coordinates and requesting resolution.
            ui.set_drag_abs_x(x + header_x);
            ui.set_drag_abs_y(y + header_y);
            slint::platform::update_timers_and_animations();
            ui.set_drag_abs_x(x + half + header_x);
            ui.set_external_tab_drop_query(1);
            slint::platform::update_timers_and_animations();
            assert_eq!(*drops.borrow(), [(1, 1, 1, 2, "g1".into())], "mode {mode}");
            drops.borrow_mut().clear();
            ui.set_external_tab_legacy(true);
            ui.set_external_tab_drop_query(2);
            slint::platform::update_timers_and_animations();
            assert_eq!(drops.borrow().len(), 1);
            assert!(drops.borrow()[0].4.is_empty());
            ui.set_external_tab_legacy(false);
            // Keep away from side tab bars when targeting the file viewport.
            let left = if mode == 1 { 160.0 } else { 20.0 };
            let right = half - if mode == 2 { 160.0 } else { 20.0 };
            let mid = (left + right) / 2.0;
            for (i, (px, py, zone)) in [
                (mid, h * 0.57, 1),
                (left, h * 0.57, 2),
                (right, h * 0.57, 3),
                (mid, 165.0, 4),
                (mid, h - 30.0, 5),
            ]
            .into_iter()
            .enumerate()
            {
                drops.borrow_mut().clear();
                ui.set_drag_abs_x(x + half + px);
                ui.set_drag_abs_y(y + py);
                ui.set_external_tab_drop_query(3 + i as i32);
                slint::platform::update_timers_and_animations();
                assert_eq!(drops.borrow().len(), 1, "mode {mode}, zone {zone}");
                let actual = &drops.borrow()[0];
                assert_eq!((actual.1, actual.2, actual.3), (1, zone, -1), "mode {mode}");
            }
            drops.borrow_mut().clear();
            ui.set_drag_abs_x(-20.0);
            ui.set_external_tab_drop_query(10);
            slint::platform::update_timers_and_animations();
            assert!(drops.borrow().is_empty());
            ui.hide().unwrap();
        }
    });
}

fn pointer(ui: &TabInputHarness, down: bool, button: PointerEventButton) {
    let position = LogicalPosition::new(35.0, 22.0);
    ui.window().dispatch_event(if down {
        WindowEvent::PointerPressed { position, button }
    } else {
        WindowEvent::PointerReleased { position, button }
    });
}

fn harness(mode: u8) -> TabInputHarness {
    let ui = TabInputHarness::new().unwrap();
    ui.set_vertical(mode != 0);
    ui.set_on_left(mode != 2);
    ui.show().unwrap();
    ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(35.0, 22.0),
    });
    ui
}

#[test]
fn tab_pointer_retains_pressed_modifiers_until_release_in_all_orientations() {
    crate::test_support::run(|| {
        for mode in 0..=2 {
            for (ctrl, shift, expected) in [(true, false, 1), (false, true, 2), (true, true, 3)] {
                let ui = harness(mode);
                if ctrl {
                    ui.window().dispatch_event(WindowEvent::KeyPressed {
                        text: Key::Control.into(),
                    });
                }
                if shift {
                    ui.window().dispatch_event(WindowEvent::KeyPressed {
                        text: Key::Shift.into(),
                    });
                }
                pointer(&ui, true, PointerEventButton::Left);
                assert_eq!(ui.get_clicks(), 0);
                if ctrl {
                    ui.window().dispatch_event(WindowEvent::KeyReleased {
                        text: Key::Control.into(),
                    });
                }
                if shift {
                    ui.window().dispatch_event(WindowEvent::KeyReleased {
                        text: Key::Shift.into(),
                    });
                }
                pointer(&ui, false, PointerEventButton::Left);
                assert_eq!(ui.get_clicks(), 1);
                assert_eq!(ui.get_modifiers(), expected);
                assert_eq!(ui.get_moves(), 0);
                ui.hide().unwrap();
            }
        }
    });
}

#[test]
fn tab_pointer_never_turns_a_blocked_or_cancelled_drag_into_a_click() {
    crate::test_support::run(|| {
        for mode in 0..=2 {
            for modified in [false, true] {
                let ui = harness(mode);
                if modified {
                    ui.window().dispatch_event(WindowEvent::KeyPressed {
                        text: Key::Control.into(),
                    });
                }
                pointer(&ui, true, PointerEventButton::Left);
                ui.window().dispatch_event(WindowEvent::PointerMoved {
                    position: LogicalPosition::new(50.0, 22.0),
                });
                if !modified {
                    assert!(ui.get_moves() > 0);
                    ui.set_drag_active(false);
                    slint::platform::update_timers_and_animations();
                }
                let moves = ui.get_moves();
                ui.window().dispatch_event(WindowEvent::PointerMoved {
                    position: LogicalPosition::new(65.0, 22.0),
                });
                pointer(&ui, false, PointerEventButton::Left);
                assert_eq!(ui.get_moves(), moves);
                assert_eq!(ui.get_drops(), 0);
                assert_eq!(ui.get_clicks(), 0);
                if modified {
                    ui.window().dispatch_event(WindowEvent::KeyReleased {
                        text: Key::Control.into(),
                    });
                }
                ui.hide().unwrap();
            }
        }
    });
}

#[test]
fn tab_pointer_keeps_single_drag_middle_close_and_context_actions() {
    crate::test_support::run(|| {
        for mode in 0..=2 {
            let ui = harness(mode);
            pointer(&ui, true, PointerEventButton::Left);
            ui.window().dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(35.0, 40.0),
            });
            pointer(&ui, false, PointerEventButton::Left);
            assert!(ui.get_moves() > 0);
            assert_eq!(ui.get_drops(), 1);
            assert_eq!(ui.get_clicks(), 0);
            pointer(&ui, true, PointerEventButton::Middle);
            pointer(&ui, false, PointerEventButton::Middle);
            assert_eq!(ui.get_closes(), 1);
            pointer(&ui, true, PointerEventButton::Right);
            pointer(&ui, false, PointerEventButton::Right);
            assert_eq!(ui.get_contexts(), 1);
            assert_eq!(ui.get_clicks(), 0);
            ui.hide().unwrap();
        }
    });
}
