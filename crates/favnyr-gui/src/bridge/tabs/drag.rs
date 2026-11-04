//! Tab drag lifecycle, shared by single tabs, selections and group headers.

use super::batch::{Batch, Destination};
use crate::bridge::{
    AppState, MainWindow, NoticeKind, SplitDir, Transfer, actions, add_paths_to_favorite,
    dropped_outside, i18n, notice, panel_to_state, push_equalize_state, push_geometry_inplace,
    refresh_all_panels, show_notice, switch_active_panel, transfer_tab_to, update_window_title,
    window_logical_to_screen,
};
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};

/// Live tab drag. Shared through `AppState` so the queries that answer what a
/// cursor position means can see what is being dragged, instead of each caller
/// keeping its own copy of it.
#[derive(Default)]
pub(in crate::bridge) struct Session {
    batch: RefCell<Option<Batch>>,
    hover: Cell<isize>,
    requires_batch: Cell<bool>,
}
impl Session {
    pub(super) fn destination(
        &self,
        projection: &super::projection::Presentation,
        book: &super::TabBook,
        position: f32,
    ) -> crate::bridge::TabDropGap {
        let batch = self.batch.borrow();
        projection.destination(book, position, batch.as_ref())
    }
    fn end_hover(&self) {
        let previous = self.hover.replace(0);
        if previous != 0 {
            crate::winmsg::hover_end(previous);
        }
    }
    fn cancel(&self) {
        self.end_hover();
        self.batch.borrow_mut().take();
    }
}

fn failed(window: &MainWindow, state: &AppState, key: &str) {
    show_notice(window, i18n::tr(state.config.borrow().language, key));
}

pub(in crate::bridge) fn snapshot(
    panel: &crate::bridge::Panel,
    batch: &Batch,
) -> Option<crate::detached_view::DetachedView> {
    let indices = batch.indices(&panel.tabs).ok()?;
    let mut view = panel_to_state(panel);
    view.tab_groups = batch.complete_groups(&panel.tabs);
    view.tabs = indices
        .iter()
        .map(|&i| {
            let mut tab = super::tab_to_state(&panel.tabs.tabs[i]);
            if tab
                .tab_group_id
                .as_deref()
                .is_some_and(|id| view.tab_groups.get(id).is_none())
            {
                tab.tab_group_id = None;
            }
            tab
        })
        .collect();
    view.active_tab = batch.ids.iter().position(|id| *id == batch.active)?;
    Some(crate::detached_view::DetachedView {
        view,
        dirs: indices
            .iter()
            .map(|&i| panel.tabs.tabs[i].current_path.clone())
            .collect(),
        position: None,
    })
}

fn detach(window: &MainWindow, state: &AppState, batch: &Batch, at: (i32, i32)) {
    let mut panels = state.panels.borrow_mut();
    let mut layout = state.layout.borrow_mut();
    let Some(panel) = panels.iter().find(|p| p.tabs.identity == batch.owner) else {
        return;
    };
    if (panels.len() == 1 && batch.ids.len() == panel.tabs.tabs.len())
        || !layout.is_valid_for(panels.len())
    {
        return;
    }
    let Some(saved) = snapshot(panel, batch) else {
        return;
    };
    if let Err(error) = actions::spawn_detached_view(&saved.view, &saved.dirs, Some(at)) {
        tracing::warn!(%error, "detaching tabs failed");
        drop(layout);
        drop(panels);
        failed(window, state, "tab_drag_detach_failed");
        return;
    }
    // The validated books and layout stay borrowed until removal completes.
    let removed = super::transfer::remove_batch(
        &mut panels,
        &mut layout,
        &mut state.active_panel.borrow_mut(),
        batch,
    );
    debug_assert_eq!(removed, Some(super::transfer::Removal::Remaining));
    drop(layout);
    drop(panels);
    refresh_all_panels(window, state);
    state.persist_workspace();
}

pub(in crate::bridge) fn install(window: &MainWindow, state: &AppState) {
    let session = state.tab_drag.clone();
    let s = session.clone();
    let st = state.clone();
    let weak = window.as_weak();
    window.on_tab_drag_started(move |panel, tab, group| {
        s.cancel();
        let Some(w) = weak.upgrade() else {
            return false;
        };
        let batch = st.panels.borrow().get(panel as usize).and_then(|p| {
            Batch::capture(
                &p.tabs,
                tab as usize,
                (!group.is_empty()).then_some(group.as_str()),
            )
        });
        let Some(batch) = batch else { return false };
        s.requires_batch
            .set(batch.requires_batch_protocol(&st.panels.borrow()[panel as usize].tabs));
        w.set_tab_drag_count(batch.ids.len() as i32);
        w.set_external_tab_legacy(false);
        w.set_tab_drag_can_detach(
            st.panels.borrow().len() > 1
                || batch.ids.len() < st.panels.borrow()[panel as usize].tabs.tabs.len(),
        );
        w.set_tab_drag_external_blocked(false);
        *s.batch.borrow_mut() = Some(batch);
        true
    });
    let s = session.clone();
    window.on_tab_drag_cancelled(move || s.cancel());
    let s = session.clone();
    let weak = window.as_weak();
    window.on_tab_drag_progress(move |_, x, y| {
        let Some(w) = weak.upgrade() else { return };
        let (sx, sy) = window_logical_to_screen(&w, x, y);
        let target = crate::winmsg::target_at(sx, sy).unwrap_or(0);
        let modern = crate::winmsg::supports_batch(target);
        let blocked = s.requires_batch.get() && !modern;
        w.set_tab_drag_external_blocked(target != 0 && blocked);
        let allowed = if blocked { 0 } else { target };
        if s.hover.get() != allowed {
            s.end_hover();
            if allowed != 0 && modern {
                crate::winmsg::begin_batch_hover(allowed, w.get_tab_drag_count() as usize);
            }
        }
        if allowed != 0 {
            if modern {
                crate::winmsg::batch_hover(allowed, sx, sy);
            } else {
                crate::winmsg::hover(allowed, sx, sy);
            }
        }
        s.hover.set(allowed);
    });
    let s = session.clone();
    let st = state.clone();
    let weak = window.as_weak();
    window.on_tab_to_favorite(move |container| {
        s.end_hover();
        let Some(w) = weak.upgrade() else { return };
        let Some(batch) = s.batch.borrow_mut().take() else {
            return;
        };
        let paths = {
            let panels = st.panels.borrow();
            let Some(p) = panels.iter().find(|p| p.tabs.identity == batch.owner) else {
                return;
            };
            let Ok(paths) = batch.paths(&p.tabs) else {
                return;
            };
            paths
        };
        add_paths_to_favorite(&w, &st, &paths, &container);
    });
    let s = session;
    let st = state.clone();
    let weak = window.as_weak();
    window.on_tab_drag_completed(move |target, zone, gap, group, x, y| {
        s.end_hover();
        let Some(w) = weak.upgrade() else { return };
        let Some(batch) = s.batch.borrow_mut().take() else {
            return;
        };
        let source = {
            let panels = st.panels.borrow();
            let Some(source) = panels.iter().position(|p| p.tabs.identity == batch.owner) else {
                return;
            };
            if batch.indices(&panels[source].tabs).is_err() {
                failed(&w, &st, "tab_drag_changed");
                return;
            }
            source
        };
        let at = window_logical_to_screen(&w, x, y);
        let outside = dropped_outside(&w, x, y);
        if let Some(other) = crate::winmsg::target_at(at.0, at.1) {
            if crate::winmsg::supports_batch(other) {
                super::ipc::send(&w, &st, batch, other, at);
                return;
            }
            let (blocked, from) = {
                let panels = st.panels.borrow();
                (
                    batch.requires_batch_protocol(&panels[source].tabs),
                    batch.indices(&panels[source].tabs).unwrap()[0],
                )
            };
            if blocked {
                failed(&w, &st, "tab_drag_external_unavailable");
                return;
            }
            match transfer_tab_to(&w, &st, source, from, other, at) {
                Transfer::Moved => {
                    refresh_all_panels(&w, &st);
                    return;
                }
                Transfer::Emptied | Transfer::Blocked => return,
                Transfer::Failed if !outside => return,
                Transfer::Failed => {}
            }
        }
        if outside {
            detach(&w, &st, &batch, at);
            return;
        }
        if target < 0 {
            return;
        }
        let destination = Destination {
            gap: if gap < 0 { usize::MAX } else { gap as usize },
            group: (!group.is_empty()).then(|| group.to_string()),
        };
        let same_view = source == target as usize && !(2..=5).contains(&zone);
        let whole_view = gap < 0
            && (2..=5).contains(&zone)
            && batch.ids.len() == st.panels.borrow()[source].tabs.tabs.len();
        let renamed = if source != target as usize && group.is_empty() && (gap >= 0 || zone == 1) {
            let panels = st.panels.borrow();
            panels.get(target as usize).is_some_and(|p| {
                super::transfer::group_names_overlap(
                    &batch.complete_groups(&panels[source].tabs),
                    &p.tabs.groups,
                )
            })
        } else {
            false
        };
        let result = {
            let mut panels = st.panels.borrow_mut();
            let mut layout = st.layout.borrow_mut();
            if gap >= 0 || zone == 1 {
                if same_view && gap < 0 {
                    Some(source)
                } else {
                    super::transfer::move_batch(
                        &mut panels,
                        &mut layout,
                        &batch,
                        target as usize,
                        &destination,
                    )
                }
            } else if (2..=5).contains(&zone) {
                super::transfer::split_batch(
                    &mut panels,
                    &mut layout,
                    &batch,
                    target as usize,
                    if zone >= 4 {
                        SplitDir::Column
                    } else {
                        SplitDir::Row
                    },
                    zone == 2 || zone == 4,
                )
            } else {
                None
            }
        };
        if let Some(active) = result {
            let was_active = *st.active_panel.borrow() == active;
            *st.active_panel.borrow_mut() = active;
            if whole_view {
                *st.equalize_undo.borrow_mut() = None;
                if !was_active {
                    switch_active_panel(&w, &st);
                }
                push_geometry_inplace(&w, &st);
                push_equalize_state(&w, &st);
            } else if same_view && was_active {
                super::groups::publish(&w, &st, active);
                update_window_title(&w, &st);
            } else {
                refresh_all_panels(&w, &st);
            }
            st.persist_workspace();
            if renamed {
                notice(
                    &w,
                    i18n::tr(st.config.borrow().language, "tab_drag_groups_renamed"),
                    NoticeKind::Success,
                );
            }
        }
    });
}
