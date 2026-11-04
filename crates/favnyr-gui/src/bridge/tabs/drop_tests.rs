use super::{Tab, TabBook, TabId, batch::Batch, transfer};
use crate::bridge::{
    LayoutNode, MAX_PANELS, Panel, SplitDir, add_favorite_entries, panel_to_state,
};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc, VecModel};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

fn panel(count: usize, active: usize, mode: u8) -> Panel {
    let mut book = super::tests::sample(count, active);
    for (index, tab) in book.tabs.iter_mut().enumerate() {
        *tab = Tab::new(PathBuf::from(format!("folder{:02}", index + 1)), false);
    }
    Panel::from_book(book, favnyr_core::columns::default_columns(), mode, 137.0)
}

fn mixed_panel(mode: u8, active: usize, all: bool) -> Panel {
    let mut panel = panel(7, active, mode);
    panel.tab_bar_restore_mode = 2;
    let book = &mut panel.tabs;
    for (indices, name, slot, collapsed) in [
        ([0, 1], "Group 01", 1, true),
        ([3, 4], "Group 02", 2, false),
    ] {
        let id = book
            .groups
            .create(&mut book.tabs, &mut book.active, &indices, name, slot)
            .unwrap();
        book.groups.set_collapsed(&id, collapsed).unwrap();
    }
    book.highlight_active_group();
    book.select_ids(&[]);
    let groups: Vec<_> = book
        .groups
        .entries()
        .iter()
        .map(|group| group.id.clone())
        .collect();
    for id in groups {
        book.click_group(&id, true, false);
    }
    book.click(2, true, false);
    if all {
        book.click(5, true, false);
        book.click(6, true, false);
    }
    panel
}

fn capture(book: &TabBook) -> Batch {
    Batch::capture(book, usize::MAX, Some(&book.groups.entries()[0].id)).unwrap()
}

fn ids(panel: &Panel) -> Vec<TabId> {
    panel.tabs.tabs.iter().map(|tab| tab.id).collect()
}

fn assert_arrival(panel: &Panel, batch: &Batch, mode: u8) {
    assert_eq!(ids(panel), batch.ids);
    assert_eq!(panel.tabs.selected_ids(), batch.ids);
    assert_eq!(panel.tabs.tabs[panel.tabs.active].id, batch.active);
    assert_eq!(panel.tab_bar_mode, mode);
    assert_eq!(panel.tab_bar_restore_mode, 2);
    for (name, slot, collapsed, range) in
        [("Group 01", 1, true, 0..2), ("Group 02", 2, false, 3..5)]
    {
        let group = panel
            .tabs
            .groups
            .entries()
            .iter()
            .find(|group| group.name == name)
            .unwrap();
        assert_eq!((group.color_slot, group.collapsed), (slot, collapsed));
        assert!(panel.tabs.group_selected(&group.id));
        let members: Vec<_> = panel
            .tabs
            .tabs
            .iter()
            .filter(|tab| tab.tab_group_id.as_deref() == Some(&group.id))
            .map(|tab| tab.id)
            .collect();
        assert_eq!(members, batch.ids[range]);
    }
}

#[test]
fn mixed_split_preserves_every_member_selection_and_view_settings_on_both_axes() {
    for mode in 0..=3 {
        for active in [0, 2, 6] {
            for target in [0, 1] {
                for (dir, first) in [
                    (SplitDir::Row, true),
                    (SplitDir::Row, false),
                    (SplitDir::Column, true),
                    (SplitDir::Column, false),
                ] {
                    let mut panels = vec![mixed_panel(mode, active, false), panel(2, 0, 0)];
                    let original = ids(&panels[0]);
                    let columns = panels[0].columns.clone();
                    let batch = capture(&panels[0].tabs);
                    let mut layout = LayoutNode::row_chain(&[1.0, 1.0]);
                    let arrival =
                        transfer::split_batch(&mut panels, &mut layout, &batch, target, dir, first)
                            .unwrap();
                    assert_eq!(arrival, 2);
                    assert_eq!(panels.len(), 3);
                    assert!(layout.is_valid_for(3));
                    assert_arrival(&panels[arrival], &batch, mode);
                    assert_eq!(panels[arrival].columns, columns);
                    assert_eq!(panels[arrival].vbar_user_w, 137.0);
                    assert_eq!(ids(&panels[0]), original[5..]);
                    assert!(panels[0].tabs.groups.is_empty());
                    assert_eq!(
                        panels[0].tabs.tabs[panels[0].tabs.active].id,
                        original[if active == 6 { 6 } else { 5 }]
                    );
                    assert_eq!(panels[arrival].tabs.group_page().is_some(), active != 2);
                }
            }
        }
    }
}

#[test]
fn whole_mixed_view_moves_without_losing_models_and_partial_splits_respect_limits() {
    for all in [false, true] {
        let mut panels: Vec<_> = (0..MAX_PANELS).map(|_| panel(1, 0, 0)).collect();
        panels[0] = mixed_panel(2, 2, all);
        let batch = capture(&panels[0].tabs);
        let before: Vec<_> = panels.iter().map(panel_to_state).collect();
        let model = panels[0].tab_presentation.model.clone();
        let selected = panels[0].tabs.selected_ids();
        let mut layout = LayoutNode::row_chain(&[1.0; MAX_PANELS]);
        let before_layout = layout.clone();
        let arrival =
            transfer::split_batch(&mut panels, &mut layout, &batch, 1, SplitDir::Column, true);
        assert_eq!(arrival, all.then_some(0));
        assert_eq!(
            panels.iter().map(panel_to_state).collect::<Vec<_>>(),
            before
        );
        assert_eq!(panels[0].tabs.selected_ids(), selected);
        assert!(Rc::ptr_eq(&model, &panels[0].tab_presentation.model));
        assert!(layout.is_valid_for(MAX_PANELS));
        if all {
            assert_arrival(&panels[0], &batch, 2);
        } else {
            assert_eq!(layout, before_layout);
        }
    }
    let mut panels = vec![mixed_panel(0, 2, true)];
    let batch = capture(&panels[0].tabs);
    let mut layout = LayoutNode::Leaf { panel: 0 };
    let before = panel_to_state(&panels[0]);
    assert!(
        transfer::split_batch(&mut panels, &mut layout, &batch, 0, SplitDir::Row, false).is_none()
    );
    assert_eq!(panel_to_state(&panels[0]), before);
    assert_eq!(panels[0].tabs.selected_ids(), batch.ids);
}

struct DragWindow {
    ui: crate::MainWindow,
    panels: Rc<RefCell<Vec<Panel>>>,
    batch: Rc<RefCell<Option<Batch>>>,
}

impl DragWindow {
    fn new(mode: u8) -> Self {
        let ui = super::input_tests::window_with_grouped_views(mode);
        ui.window().set_size(slint::LogicalSize::new(1600.0, 800.0));
        let panels = Rc::new(RefCell::new(vec![
            mixed_panel(mode, 2, false),
            panel(2, 0, mode),
        ]));
        for panel in panels.borrow().iter() {
            panel
                .tab_presentation
                .update(&panel.tabs, mode, &Default::default());
        }
        ui.set_panels(ModelRc::new(VecModel::from(
            panels
                .borrow()
                .iter()
                .map(|panel| crate::PanelView {
                    tabs: ModelRc::from(panel.tab_presentation.model.clone()),
                    tab_bar_mode: mode.into(),
                    vbar_user_w: 137.0,
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
        )));
        ui.set_tab_bars(ModelRc::new(VecModel::from(
            panels
                .borrow()
                .iter()
                .map(|panel| panel.tab_presentation.state(&panel.tabs))
                .collect::<Vec<_>>(),
        )));
        let batch = Rc::new(RefCell::new(None));
        let source = panels.clone();
        let frozen = batch.clone();
        let weak = ui.as_weak();
        ui.on_tab_drag_started(move |panel, tab, group| {
            let captured = Batch::capture(
                &source.borrow()[panel as usize].tabs,
                tab as usize,
                (!group.is_empty()).then_some(group.as_str()),
            )
            .unwrap();
            weak.unwrap().set_tab_drag_count(captured.ids.len() as i32);
            weak.unwrap().set_tab_drag_can_detach(true);
            *frozen.borrow_mut() = Some(captured);
            true
        });
        let frozen = batch.clone();
        ui.on_tab_drag_cancelled(move || {
            frozen.borrow_mut().take();
        });
        let source = panels.clone();
        let frozen = batch.clone();
        ui.on_tab_drop_gap(move |index, position, joins| {
            let panels = source.borrow();
            let panel = &panels[index as usize];
            if joins {
                panel
                    .tab_presentation
                    .destination(&panel.tabs, position, frozen.borrow().as_ref())
            } else {
                panel.tab_presentation.gap(&panel.tabs, position, None)
            }
        });
        ui.show().unwrap();
        Self { ui, panels, batch }
    }

    fn start(&self, mode: u8) {
        let header = self
            .ui
            .get_panels()
            .row_data(0)
            .unwrap()
            .tabs
            .row_data(0)
            .unwrap();
        let x = self.ui.get_panels_cont_x()
            + match mode {
                0 => header.offset + header.width / 2.0,
                1 => 60.0,
                _ => self.ui.get_panels_cont_w() / 2.0 - 70.0,
            };
        let y = self.ui.get_panels_cont_y() + if mode == 0 { 20.0 } else { 60.0 };
        self.ui
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position: LogicalPosition::new(x, y),
                button: PointerEventButton::Left,
            });
        self.move_to(LogicalPosition::new(x + 12.0, y));
        assert_eq!(self.batch.borrow().as_ref().unwrap().ids.len(), 5);
    }

    fn move_to(&self, position: LogicalPosition) {
        self.ui
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        slint::platform::update_timers_and_animations();
    }

    fn release(&self, position: LogicalPosition) {
        self.ui
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
        assert!(!self.ui.get_drag_active());
    }
}

#[test]
fn pointer_mixed_drag_creates_one_split_at_each_view_edge_in_every_bar_orientation() {
    crate::test_support::run(|| {
        for mode in 0..=2 {
            for zone in 2..=5 {
                let window = DragWindow::new(mode);
                let layout = Rc::new(RefCell::new(LayoutNode::row_chain(&[1.0, 1.0])));
                let tree = layout.clone();
                let panels = window.panels.clone();
                let frozen = window.batch.clone();
                window
                    .ui
                    .on_tab_to_favorite(|_| panic!("view edge routed to favorites"));
                window
                    .ui
                    .on_tab_drag_completed(move |target, actual_zone, gap, group, _, _| {
                        assert_eq!((target, actual_zone, gap), (1, zone, -1));
                        assert!(group.is_empty());
                        let batch = frozen.borrow_mut().take().unwrap();
                        let mut panels = panels.borrow_mut();
                        let index = transfer::split_batch(
                            &mut panels,
                            &mut tree.borrow_mut(),
                            &batch,
                            target as usize,
                            if zone >= 4 {
                                SplitDir::Column
                            } else {
                                SplitDir::Row
                            },
                            zone == 2 || zone == 4,
                        )
                        .unwrap();
                        assert_arrival(&panels[index], &batch, mode);
                    });
                window.start(mode);
                let half = window.ui.get_panels_cont_w() / 2.0;
                let h = window.ui.get_panels_cont_h();
                let left = if mode == 1 { 167.0 } else { 20.0 };
                let right = half - if mode == 2 { 167.0 } else { 20.0 };
                let (x, y) = match zone {
                    2 => (left, h * 0.57),
                    3 => (right, h * 0.57),
                    4 => ((left + right) / 2.0, 165.0),
                    _ => ((left + right) / 2.0, h - 30.0),
                };
                let position = LogicalPosition::new(
                    window.ui.get_panels_cont_x() + half + x,
                    window.ui.get_panels_cont_y() + y,
                );
                window.move_to(position);
                window.release(position);
                assert_eq!(window.panels.borrow().len(), 3, "mode {mode}, zone {zone}");
                assert!(layout.borrow().is_valid_for(3));
                assert!(window.batch.borrow().is_none());
                window.ui.hide().unwrap();
            }
        }
    });
}

#[test]
fn pointer_mixed_drag_bookmarks_every_path_and_never_removes_source_tabs() {
    crate::test_support::run(|| {
        for mode in 0..=2 {
            for nested in [false, true] {
                for cancel in [false, true] {
                    let window = DragWindow::new(mode);
                    // Two distinct tabs may represent the same folder.
                    let duplicate = window.panels.borrow()[0].tabs.tabs[0].current_path.clone();
                    window.panels.borrow_mut()[0].tabs.tabs[3].current_path = duplicate;
                    let source_before = panel_to_state(&window.panels.borrow()[0]);
                    let selected_before = window.panels.borrow()[0].tabs.selected_ids();
                    let mut store = favnyr_core::favorites::FavStore::default();
                    let container = if nested {
                        store.add_container("", "Folder01").unwrap()
                    } else {
                        String::new()
                    };
                    let existing = window.panels.borrow()[0].tabs.tabs[1]
                        .current_path
                        .display()
                        .to_string();
                    store
                        .add_favorite(&container, "Folder02", &existing)
                        .unwrap();
                    if nested {
                        store
                            .nodes
                            .iter_mut()
                            .find(|node| node.id == container)
                            .unwrap()
                            .expanded = false;
                    }
                    window.ui.set_fav_nodes(ModelRc::new(VecModel::from(
                        store
                            .flatten()
                            .into_iter()
                            .map(|node| crate::FavNode {
                                id: node.id.into(),
                                label: node.name.into(),
                                is_container: node.is_container,
                                expanded: false,
                                has_children: node.has_children,
                                depth: node.depth,
                                available: true,
                                is_dir: true,
                                path: node.path.into(),
                                ..Default::default()
                            })
                            .collect::<Vec<_>>(),
                    )));
                    let store_before = store.clone();
                    let store = Rc::new(RefCell::new(store));
                    let saved = store.clone();
                    let source = window.panels.clone();
                    let frozen = window.batch.clone();
                    let expected_container = container.clone();
                    window.ui.on_tab_drag_completed(|_, _, _, _, _, _| {
                        panic!("favorites drop routed to a view")
                    });
                    window.ui.on_tab_to_favorite(move |target| {
                        assert_eq!(target.as_str(), expected_container);
                        let batch = frozen.borrow_mut().take().unwrap();
                        let paths = batch.paths(&source.borrow()[0].tabs).unwrap();
                        assert_eq!(paths.len(), 5);
                        assert_eq!(
                            add_favorite_entries(&mut saved.borrow_mut(), &paths, &target),
                            3
                        );
                        assert_eq!(
                            add_favorite_entries(&mut saved.borrow_mut(), &paths, &target),
                            0
                        );
                    });
                    window.start(mode);
                    let position = (0..780)
                        .step_by(2)
                        .map(|y| LogicalPosition::new(110.0, y as f32))
                        .find(|&position| {
                            window.move_to(position);
                            window.ui.get_fav_hover_panel()
                                && window.ui.get_fav_hover_container() == container
                        })
                        .expect("favorites target must be reachable");
                    if cancel {
                        window.ui.window().dispatch_event(WindowEvent::KeyPressed {
                            text: Key::Escape.into(),
                        });
                        window.ui.window().dispatch_event(WindowEvent::KeyReleased {
                            text: Key::Escape.into(),
                        });
                    }
                    window.release(position);
                    assert!(window.batch.borrow().is_none());
                    assert_eq!(panel_to_state(&window.panels.borrow()[0]), source_before);
                    assert_eq!(
                        window.panels.borrow()[0].tabs.selected_ids(),
                        selected_before
                    );
                    if cancel {
                        assert_eq!(*store.borrow(), store_before);
                    } else {
                        assert_eq!(
                            store
                                .borrow()
                                .nodes
                                .iter()
                                .filter(|node| node.path.is_some())
                                .count(),
                            4
                        );
                        assert!(
                            store
                                .borrow()
                                .nodes
                                .iter()
                                .all(|node| node.path.is_none() || node.parent == container)
                        );
                    }
                    window.ui.hide().unwrap();
                }
            }
        }
    });
}

#[test]
fn changed_group_membership_rejects_split_and_bookmark_collection_without_partial_changes() {
    let mut panels = vec![mixed_panel(0, 2, false), panel(2, 0, 0)];
    let batch = capture(&panels[0].tabs);
    let source = &mut panels[0].tabs;
    let group = source.groups.entries()[1].id.clone();
    source
        .groups
        .assign(&mut source.tabs, &mut source.active, &[5], &group)
        .unwrap();
    let before: Vec<_> = panels.iter().map(panel_to_state).collect();
    let mut layout = LayoutNode::row_chain(&[1.0, 1.0]);
    let before_layout = layout.clone();
    assert!(batch.paths(&panels[0].tabs).is_err());
    assert!(
        transfer::split_batch(&mut panels, &mut layout, &batch, 1, SplitDir::Row, true).is_none()
    );
    assert_eq!(
        panels.iter().map(panel_to_state).collect::<Vec<_>>(),
        before
    );
    assert_eq!(layout, before_layout);
}
