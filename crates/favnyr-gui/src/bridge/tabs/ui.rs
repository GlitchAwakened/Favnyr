//! Tab interactions publish through the view's stable tab projection.

use slint::{ComponentHandle, Model};
use std::path::PathBuf;

use crate::bridge::{AppState, MainWindow, load_directory};

fn publish_selection(window: &MainWindow, state: &AppState, target: Option<PathBuf>) {
    if let Some(path) = target {
        load_directory(window, state, &path, false);
    } else {
        super::groups::publish(window, state, *state.active_panel.borrow());
    }
}

fn reset_panel_selection(window: &MainWindow, state: &AppState, panel: usize) -> bool {
    let changed = {
        let mut panels = state.panels.borrow_mut();
        let Some(source) = panels.get_mut(panel) else {
            return false;
        };
        source.tabs.reset_selection()
    };
    // Clearing a batch keeps the focused group. Only repair marks that differ
    // from the book, without republishing a valid group outline on each click.
    let stale_mark = {
        let panels = state.panels.borrow();
        let book = &panels[panel].tabs;
        window.get_panels().row_data(panel).is_some_and(|view| {
            view.tabs.iter().any(|row| {
                row.selection_marked
                    || (row.is_group
                        && row.group_highlighted != book.group_highlighted(&row.group_id))
            })
        })
    };
    if changed || stale_mark {
        super::groups::publish(window, state, panel);
    }
    changed
}

pub(in crate::bridge) fn install(window: &MainWindow, state: &AppState) {
    let groups = super::groups::install(window, state);
    let st = state.clone();
    let weak = window.as_weak();
    window.on_tab_clicked(move |index, modifiers| {
        let Some(window) = weak.upgrade() else { return };
        let target = st.with_tabs_mut(|book| {
            book.click(index as usize, modifiers & 1 != 0, modifiers & 2 != 0)
                .then(|| book.tabs[book.active].current_path.clone())
        });
        publish_selection(&window, &st, target);
    });

    let st = state.clone();
    let weak = window.as_weak();
    window.on_tab_wheel_selected(move |index| {
        let Some(window) = weak.upgrade() else { return };
        let target = st.with_tabs_mut(|book| {
            book.select_from_wheel(index as usize)
                .then(|| book.tabs[book.active].current_path.clone())
                .filter(|_| book.group_page().is_none())
        });
        publish_selection(&window, &st, target);
    });

    let st = state.clone();
    let weak = window.as_weak();
    window.on_tab_context_selection(move |panel, index| {
        let Some(window) = weak.upgrade() else {
            return 0;
        };
        let count = {
            let mut panels = st.panels.borrow_mut();
            let Some(source) = panels.get_mut(panel as usize) else {
                return 0;
            };
            source.tabs.select_context(index as usize)
        };
        super::groups::publish(&window, &st, panel as usize);
        super::groups::prepare(&window, &st, &groups, panel as usize, None);
        count as i32
    });

    let st = state.clone();
    let weak = window.as_weak();
    window.on_reset_tab_selection(move || {
        let Some(window) = weak.upgrade() else {
            return false;
        };
        reset_panel_selection(&window, &st, *st.active_panel.borrow())
    });

    let st = state.clone();
    let weak = window.as_weak();
    window.on_reset_panel_tab_selection(move |panel| {
        let Some(window) = weak.upgrade() else {
            return false;
        };
        let Ok(panel) = usize::try_from(panel) else {
            return false;
        };
        reset_panel_selection(&window, &st, panel)
    });
}
