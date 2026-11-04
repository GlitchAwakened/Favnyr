//! Group commands keep stable identities while menus and editors are open.

use std::{cell::RefCell, rc::Rc};

use favnyr_core::{annotations::MAX_COLOR_SLOT, tab_groups::GroupError};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use super::{BookId, TabBook, TabId};
use crate::bridge::{
    AppState, MainWindow, TabDropGap, TabGroupChoice, i18n, show_notice, update_window_title,
};

#[derive(Clone)]
struct Context {
    owner: BookId,
    tabs: Vec<TabId>,
    group: Option<String>,
}

impl Context {
    fn indices(&self, book: &TabBook) -> Result<Vec<usize>, GroupError> {
        if book.identity != self.owner || self.tabs.is_empty() {
            return Err(GroupError::InvalidSelection);
        }
        if let Some(id) = self.group.as_deref() {
            if book.groups.get(id).is_none() {
                return Err(GroupError::MissingGroup);
            }
            let members: Vec<_> = book
                .tabs
                .iter()
                .filter(|t| t.tab_group_id.as_deref() == Some(id))
                .map(|t| t.id)
                .collect();
            if members != self.tabs {
                return Err(GroupError::InvalidSelection);
            }
        }
        self.tabs
            .iter()
            .map(|id| {
                book.tabs
                    .iter()
                    .position(|t| t.id == *id)
                    .ok_or(GroupError::InvalidSelection)
            })
            .collect()
    }
}

#[derive(Default)]
pub(super) struct State {
    context: RefCell<Option<Context>>,
    editor: RefCell<Option<Context>>,
}

pub(super) fn prepare(
    window: &MainWindow,
    state: &AppState,
    session: &State,
    panel: usize,
    group: Option<&str>,
) {
    session.context.borrow_mut().take();
    window.set_group_choices(ModelRc::default());
    window.set_group_can_remove(false);
    let panels = state.panels.borrow();
    let Some(panel) = panels.get(panel) else {
        return;
    };
    let book = &panel.tabs;
    let ids = match group {
        Some(id) => book
            .tabs
            .iter()
            .filter(|t| t.tab_group_id.as_deref() == Some(id))
            .map(|t| t.id)
            .collect(),
        None => book.selected_ids(),
    };
    let context = Context {
        owner: book.identity,
        tabs: ids,
        group: group.map(str::to_owned),
    };
    let Ok(indices) = context.indices(book) else {
        return;
    };
    window.set_group_can_remove(indices.iter().any(|&i| book.tabs[i].tab_group_id.is_some()));
    window.set_group_choices(ModelRc::new(VecModel::from(
        book.groups
            .entries()
            .iter()
            .map(|g| TabGroupChoice {
                id: g.id.as_str().into(),
                name: g.name.as_str().into(),
                slot: i32::from(g.color_slot),
                checked: indices
                    .iter()
                    .all(|&i| book.tabs[i].tab_group_id.as_deref() == Some(&g.id)),
            })
            .collect::<Vec<_>>(),
    )));
    if let Some(g) = group.and_then(|id| book.groups.get(id)) {
        window.set_group_current_color(i32::from(g.color_slot));
    }
    if let Some(&i) = indices.first() {
        window.set_tab_menu_folder_slot(i32::from(
            state
                .annotations
                .borrow()
                .color_of(&book.tabs[i].current_path),
        ));
    }
    *session.context.borrow_mut() = Some(context);
}

pub(super) fn publish(window: &MainWindow, state: &AppState, panel: usize) {
    let group_page = state
        .panels
        .borrow()
        .get(panel)
        .is_some_and(|p| p.tabs.group_page().is_some());
    if group_page {
        crate::bridge::search::clear(state, panel);
        if panel == *state.active_panel.borrow() {
            state.filter.borrow_mut().clear();
            window.set_active_filter("".into());
            window.set_active_ext_filter_on(false);
        }
    }
    let panels = state.panels.borrow();
    let Some(panel_state) = panels.get(panel) else {
        return;
    };
    panel_state.tab_presentation.update(
        &panel_state.tabs,
        panel_state.tab_bar_mode,
        &state.annotations.borrow(),
    );
    let next = panel_state.tab_presentation.state(&panel_state.tabs);
    let model = window.get_tab_bars();
    if model.row_data(panel).as_ref() != Some(&next) {
        model.set_row_data(panel, next);
    }
}

fn error_key(error: GroupError) -> &'static str {
    match error {
        GroupError::InvalidName => "tab_group_invalid_name",
        GroupError::NameTaken => "tab_group_name_taken",
        _ => "tab_group_target_changed",
    }
}

fn mutate(
    state: &AppState,
    context: &Context,
    action: impl FnOnce(&mut TabBook, &[usize]) -> Result<(), GroupError>,
) -> Result<usize, GroupError> {
    let mut panels = state.panels.borrow_mut();
    let index = panels
        .iter()
        .position(|p| p.tabs.identity == context.owner)
        .ok_or(GroupError::InvalidSelection)?;
    let book = &mut panels[index].tabs;
    let indices = context.indices(book)?;
    action(book, &indices)?;
    book.reconcile_selection();
    Ok(index)
}

fn commit(window: &MainWindow, state: &AppState, result: Result<usize, GroupError>) {
    match result {
        Ok(panel) => {
            publish(window, state, panel);
            state.persist_workspace();
            update_window_title(window, state);
        }
        Err(error) => show_notice(
            window,
            i18n::tr(state.config.borrow().language, error_key(error)),
        ),
    }
}

/// Validate membership before removing anything. A replacement keeps the
/// final view usable when every tab belongs to the deleted group.
fn take_group(
    book: &mut TabBook,
    context: &Context,
    replacement: Option<super::Tab>,
) -> Result<Vec<super::Tab>, GroupError> {
    context.group.as_ref().ok_or(GroupError::MissingGroup)?;
    let mut indices = context.indices(book)?;
    if indices.len() == book.tabs.len() {
        book.tabs
            .push(replacement.ok_or(GroupError::InvalidSelection)?);
    }
    indices.sort_unstable();
    let mut closed = Vec::with_capacity(indices.len());
    for index in indices.into_iter().rev() {
        closed.push(
            book.take_tab(index)
                .expect("group membership was validated"),
        );
    }
    closed.reverse();
    book.reset_selection();
    Ok(closed)
}

fn delete_group(
    window: &MainWindow,
    state: &AppState,
    context: &Context,
) -> Result<(), GroupError> {
    use crate::bridge::{
        TabDisplayDefaults, home_dir, load_directory, remember_closed_panel, remember_closed_tabs,
        switch_active_panel, take_view,
    };
    let (panel, whole_view, active) = {
        let panels = state.panels.borrow();
        let panel = panels
            .iter()
            .position(|p| p.tabs.identity == context.owner)
            .ok_or(GroupError::InvalidSelection)?;
        let book = &panels[panel].tabs;
        context.group.as_ref().ok_or(GroupError::MissingGroup)?;
        let indices = context.indices(book)?;
        (
            panel,
            indices.len() == book.tabs.len(),
            book.tabs[book.active].id,
        )
    };
    if whole_view && state.panels.borrow().len() > 1 {
        let closed = take_view(state, panel).ok_or(GroupError::InvalidSelection)?;
        remember_closed_panel(state, closed);
    } else {
        let replacement = whole_view.then(|| {
            super::Tab::new(
                home_dir(),
                TabDisplayDefaults::from(&*state.config.borrow()),
            )
        });
        let closed = take_group(
            &mut state.panels.borrow_mut()[panel].tabs,
            context,
            replacement,
        )?;
        remember_closed_tabs(state, closed, Some(active));
    }
    window.set_closed_tabs_available(state.has_closed_tabs());
    let next_path = state.with_tabs(|book| {
        (book.group_page().is_none() && book.tabs[book.active].id != active)
            .then(|| book.tabs[book.active].current_path.clone())
    });
    if let Some(path) = next_path {
        load_directory(window, state, &path, false);
    } else {
        switch_active_panel(window, state);
    }
    state.persist_workspace();
    update_window_title(window, state);
    Ok(())
}

pub(super) fn install(window: &MainWindow, state: &AppState) -> Rc<State> {
    let session = Rc::new(State::default());
    let st = state.clone();
    window.on_tab_drop_gap(move |panel, position, groups| {
        let panels = st.panels.borrow();
        panels.get(panel as usize).map_or(
            TabDropGap {
                index: -1,
                position: 0.0,
                ..Default::default()
            },
            |p| {
                if groups {
                    st.tab_drag
                        .destination(&p.tab_presentation, &p.tabs, position)
                } else {
                    p.tab_presentation.gap(&p.tabs, position, None)
                }
            },
        )
    });

    let st = state.clone();
    let weak = window.as_weak();
    window.on_group_clicked(move |panel, id, modifiers| {
        let Some(w) = weak.upgrade() else { return };
        if let Some(source) = st.panels.borrow_mut().get_mut(panel as usize) {
            source
                .tabs
                .click_group(&id, modifiers & 1 != 0, modifiers & 2 != 0);
        }
        publish(&w, &st, panel as usize);
    });

    let st = state.clone();
    let weak = window.as_weak();
    window.on_group_toggle(move |panel, id| {
        let Some(w) = weak.upgrade() else { return };
        let changed = {
            let mut panels = st.panels.borrow_mut();
            let Some(p) = panels.get_mut(panel as usize) else {
                return;
            };
            let Some(g) = p.tabs.groups.get(&id) else {
                return;
            };
            let collapsed = !g.collapsed;
            let changed = p.tabs.groups.set_collapsed(&id, collapsed).unwrap_or(false);
            p.tabs.reconcile_selection();
            p.tabs.focus_group(&id);
            changed
        };
        if changed {
            commit(&w, &st, Ok(panel as usize));
        }
    });

    let st = state.clone();
    let weak = window.as_weak();
    let target = session.clone();
    window.on_group_context(move |panel, id| {
        if let Some(w) = weak.upgrade() {
            if let Some(source) = st.panels.borrow_mut().get_mut(panel as usize)
                && !source.tabs.group_selected(&id)
            {
                source.tabs.focus_group(&id);
            }
            publish(&w, &st, panel as usize);
            prepare(&w, &st, &target, panel as usize, Some(&id));
        }
    });

    let st = state.clone();
    let weak = window.as_weak();
    let target = session.clone();
    window.on_group_command(move |action, id| {
        let Some(w) = weak.upgrade() else { return };
        let Some(mut context) = target.context.borrow().clone() else {
            commit(&w, &st, Err(GroupError::InvalidSelection));
            return;
        };
        // New and edit open a draft; all other commands commit immediately.
        if action == 7 {
            if let Err(error) = delete_group(&w, &st, &context) {
                commit(&w, &st, Err(error));
            }
            return;
        }
        if action == 0 || action == 3 {
            let panels = st.panels.borrow();
            let Some(p) = panels.iter().find(|p| p.tabs.identity == context.owner) else {
                commit(&w, &st, Err(GroupError::InvalidSelection));
                return;
            };
            if let Err(error) = context.indices(&p.tabs) {
                commit(&w, &st, Err(error));
                return;
            }
            let (name, color) = if action == 3 {
                let Some(g) = context
                    .group
                    .as_deref()
                    .and_then(|id| p.tabs.groups.get(id))
                else {
                    return;
                };
                (g.name.clone(), g.color_slot)
            } else {
                context.group = None;
                let fallback = i18n::tr(st.config.borrow().language, "tab_group_default_name");
                let Ok(name) = p.tabs.groups.suggested_name(&fallback) else {
                    return;
                };
                (name, 0)
            };
            w.set_group_editor_creating(action == 0);
            w.set_group_editor_name(name.into());
            w.set_group_editor_color(i32::from(color));
            w.set_group_editor_error("".into());
            *target.editor.borrow_mut() = Some(context);
            w.set_group_editor_open(true);
            return;
        }
        let result = mutate(&st, &context, |book, indices| {
            let group = context.group.as_deref().unwrap_or("");
            match action {
                1 => {
                    book.groups
                        .assign(&mut book.tabs, &mut book.active, indices, &id)?;
                    book.highlight_group(&id);
                }
                2 => {
                    book.groups
                        .remove_members(&mut book.tabs, &mut book.active, indices)?;
                    if book.active_group.is_none() {
                        book.highlight_active_group();
                    }
                }
                5 => {
                    if !book.select_group(group) {
                        return Err(GroupError::MissingGroup);
                    }
                }
                6 => {
                    book.groups.dissolve(&mut book.tabs, group)?;
                }
                _ => return Err(GroupError::InvalidSelection),
            }
            Ok(())
        });
        commit(&w, &st, result);
    });

    let st = state.clone();
    let weak = window.as_weak();
    let target = session.clone();
    window.on_group_color(move |color| {
        let Some(w) = weak.upgrade() else { return };
        let Some(context) = target.context.borrow().clone() else {
            return;
        };
        let result = mutate(&st, &context, |book, _| {
            let color = u8::try_from(color).map_err(|_| GroupError::InvalidColor)?;
            book.groups.set_color(
                context.group.as_deref().ok_or(GroupError::MissingGroup)?,
                color,
            )?;
            Ok(())
        });
        if result.is_ok() {
            w.set_group_current_color(color);
        }
        commit(&w, &st, result);
    });

    let st = state.clone();
    let weak = window.as_weak();
    let target = session.clone();
    window.on_group_editor_submit(move |name, color| {
        let Some(w) = weak.upgrade() else {
            return false;
        };
        let Some(context) = target.editor.borrow().clone() else {
            return false;
        };
        let result = mutate(&st, &context, |book, indices| {
            let color = u8::try_from(color)
                .ok()
                .filter(|c| *c <= MAX_COLOR_SLOT)
                .ok_or(GroupError::InvalidColor)?;
            if let Some(id) = context.group.as_deref() {
                book.groups.rename(id, &name)?;
                book.groups.set_color(id, color)?;
            } else {
                let id =
                    book.groups
                        .create(&mut book.tabs, &mut book.active, indices, &name, color)?;
                book.highlight_group(&id);
            }
            Ok(())
        });
        match result {
            Ok(panel) => {
                commit(&w, &st, Ok(panel));
                target.editor.borrow_mut().take();
                true
            }
            Err(error) => {
                w.set_group_editor_error(
                    i18n::tr(st.config.borrow().language, error_key(error)).into(),
                );
                false
            }
        }
    });
    session
}

#[cfg(test)]
mod tests {
    use super::super::tests::sample;
    use super::*;

    fn group_context(book: &TabBook, id: &str) -> Context {
        Context {
            owner: book.identity,
            tabs: book
                .tabs
                .iter()
                .filter(|tab| tab.tab_group_id.as_deref() == Some(id))
                .map(|tab| tab.id)
                .collect(),
            group: Some(id.to_owned()),
        }
    }

    #[test]
    fn deleting_a_group_keeps_other_tabs_and_returns_restorable_members() {
        for collapsed in [false, true] {
            let mut book = sample(4, 0);
            let outside = book.tabs[0].id;
            let id = book
                .groups
                .create(&mut book.tabs, &mut book.active, &[1, 2], "Group01", 2)
                .unwrap();
            book.groups.set_collapsed(&id, collapsed).unwrap();
            let context = group_context(&book, &id);
            book.tabs[1].current_path = "folder02".into();
            book.tabs[1].show_hidden = true;
            book.focus_group(&id);
            let closed = take_group(&mut book, &context, None).unwrap();
            assert_eq!(
                closed.iter().map(|tab| tab.id).collect::<Vec<_>>(),
                context.tabs
            );
            assert_eq!(book.tabs.len(), 2);
            assert!(book.tabs.iter().any(|tab| tab.id == outside));
            assert!(book.groups.get(&id).is_none());
            assert!(book.group_page().is_none());
            assert!(book.active < book.tabs.len());
            let saved = crate::bridge::tab_to_state(&closed[0]);
            assert!(saved.tab_group_id.is_none());
            let restored = crate::bridge::tab_from_state(saved);
            assert_eq!(restored.current_path, std::path::Path::new("folder02"));
            assert!(restored.show_hidden);
        }
    }

    #[test]
    fn deleting_the_final_group_requires_a_replacement_and_rejects_stale_context() {
        let mut book = sample(2, 0);
        let id = book
            .groups
            .create(&mut book.tabs, &mut book.active, &[0, 1], "Group01", 0)
            .unwrap();
        let context = group_context(&book, &id);
        assert!(take_group(&mut book, &context, None).is_err());
        assert_eq!(book.tabs.len(), 2);
        assert!(book.groups.get(&id).is_some());
        let replacement = super::super::Tab::new("folder02".into(), false);
        let replacement_id = replacement.id;
        assert_eq!(
            take_group(&mut book, &context, Some(replacement))
                .unwrap()
                .len(),
            2
        );
        assert_eq!(book.tabs.len(), 1);
        assert_eq!(book.tabs[0].id, replacement_id);
        assert_eq!(book.active, 0);
        assert!(book.groups.entries().is_empty());
        assert!(take_group(&mut book, &context, None).is_err());
        assert_eq!(book.tabs[0].id, replacement_id);
    }

    #[test]
    fn group_deletion_requires_a_second_activation_and_resets_after_menu_closure() {
        use slint::ComponentHandle;
        crate::test_support::run_with_windows(|windows| {
            let ui = MainWindow::new().unwrap();
            ui.window().set_size(slint::LogicalSize::new(600., 400.));
            let count = Rc::new(std::cell::Cell::new(0));
            let invoked = count.clone();
            ui.on_group_command(move |action, _| {
                assert_eq!(action, 7);
                invoked.set(invoked.get() + 1);
            });
            ui.show().unwrap();
            let adapter = windows.borrow()[0].clone();
            let mut pixels = vec![slint::Rgb8Pixel::default(); 600 * 400];
            let mut render = || {
                for _ in 0..2 {
                    ui.window().request_redraw();
                    adapter.draw_if_needed(|renderer| {
                        renderer.render(&mut pixels, 600);
                    });
                    slint::platform::update_timers_and_animations();
                }
            };
            let nav = ui.global::<crate::CtxNav>();
            for cancel in [true, false] {
                ui.set_group_menu_open(true);
                render();
                assert!(!ui.get_group_menu_confirm_delete());
                nav.invoke_disarm();
                nav.set_level(1);
                nav.invoke_step(-1);
                render();
                nav.set_activate(nav.get_activate() + 1);
                render();
                assert!(ui.get_group_menu_confirm_delete());
                assert!(ui.get_group_menu_open());
                assert_eq!(count.get(), 0);
                if cancel {
                    ui.set_group_menu_open(false);
                    render();
                    assert!(!ui.get_group_menu_confirm_delete());
                } else {
                    nav.set_activate(nav.get_activate() + 1);
                    render();
                    assert!(!ui.get_group_menu_open());
                    assert_eq!(count.get(), 1);
                }
            }
            ui.hide().unwrap();
        });
    }

    #[test]
    fn frozen_menu_targets_follow_identity_and_reject_replaced_tabs_or_books() {
        let mut book = sample(4, 0);
        let context = Context {
            owner: book.identity,
            tabs: vec![book.tabs[1].id, book.tabs[3].id],
            group: None,
        };
        assert_eq!(context.indices(&book).unwrap(), [1, 3]);
        book.tabs.swap(1, 2);
        assert_eq!(context.indices(&book).unwrap(), [2, 3]);
        let original = std::mem::take(&mut book.identity);
        assert_eq!(context.indices(&book), Err(GroupError::InvalidSelection));
        book.identity = original;
        book.tabs[3] = super::super::Tab::new("folder01".into(), false);
        assert_eq!(context.indices(&book), Err(GroupError::InvalidSelection));
    }

    #[test]
    fn group_menu_rejects_changed_membership_before_mutating() {
        let mut book = sample(4, 0);
        let id = book
            .groups
            .create(&mut book.tabs, &mut book.active, &[1, 2], "Group 01", 0)
            .unwrap();
        let context = Context {
            owner: book.identity,
            tabs: vec![book.tabs[1].id, book.tabs[2].id],
            group: Some(id.clone()),
        };
        assert!(context.indices(&book).is_ok());
        book.groups
            .assign(&mut book.tabs, &mut book.active, &[3], &id)
            .unwrap();
        assert_eq!(context.indices(&book), Err(GroupError::InvalidSelection));
        book.groups.dissolve(&mut book.tabs, &id).unwrap();
        assert_eq!(context.indices(&book), Err(GroupError::MissingGroup));
    }
}
