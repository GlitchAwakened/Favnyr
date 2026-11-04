use super::*;
use crate::bridge::{
    LayoutNode, Panel, SidebarSectionsState, WorkspaceState, build_panels, deserialize_tab,
    panel_to_state, serialize_tab, workspaces_differ,
};

pub(super) fn sample(count: usize, active: usize) -> TabBook {
    TabBook {
        identity: Default::default(),
        selection: Default::default(),
        focused_group: None,
        active_group: None,
        groups: Default::default(),
        tabs: (0..count)
            .map(|_| Tab::new(PathBuf::from("folder01"), false))
            .collect(),
        active,
    }
}

fn ids(book: &TabBook) -> Vec<TabId> {
    book.tabs.iter().map(|tab| tab.id).collect()
}

fn selected_indices(book: &TabBook) -> Vec<usize> {
    book.tabs
        .iter()
        .enumerate()
        .filter_map(|(i, tab)| book.selected(tab.id).then_some(i))
        .collect()
}

#[test]
fn tab_selection_control_click_can_exclude_the_active_tab_without_navigation() {
    let mut book = sample(5, 2);
    book.reset_selection();
    assert!(!book.click(4, true, false));
    assert_eq!(selected_indices(&book), [2, 4]);
    assert_eq!(book.active, 2);
    assert!(!book.click(2, true, false));
    assert_eq!(selected_indices(&book), [4]);
    assert!(!book.click(4, true, false));
    assert!(selected_indices(&book).is_empty());
    assert_eq!(book.active, 2);
    assert!(!book.click(2, false, false));
    assert_eq!(selected_indices(&book), [2]);
    assert!(book.click(0, false, false));
    assert_eq!(selected_indices(&book), [0]);
    assert_eq!(book.active, 0);
}

#[test]
fn tab_selection_shift_keeps_its_anchor_and_control_shift_adds_a_range() {
    let mut book = sample(6, 3);
    book.reset_selection();
    assert!(!book.click(1, false, true));
    assert_eq!(selected_indices(&book), [1, 2, 3]);
    assert!(!book.click(5, false, true));
    assert_eq!(selected_indices(&book), [3, 4, 5]);
    book.click(0, true, false);
    book.click(1, true, true);
    assert_eq!(selected_indices(&book), [0, 1, 3, 4, 5]);
    assert_eq!(book.active, 3);
}

#[test]
fn tab_selection_context_keeps_a_batch_and_never_changes_the_active_folder() {
    let mut book = sample(4, 2);
    book.reset_selection();
    book.click(0, true, false);
    assert_eq!(book.select_context(0), 2);
    assert_eq!(selected_indices(&book), [0, 2]);
    assert_eq!(book.select_context(1), 1);
    assert_eq!(selected_indices(&book), [1]);
    assert_eq!(book.active, 2);
    assert_eq!(book.select_context(99), 0);
    assert!(!book.click(99, true, true));
    assert_eq!(selected_indices(&book), [1]);
    assert!(book.reset_selection());
    assert!(!book.reset_selection());
    assert_eq!(selected_indices(&book), [2]);
}

#[test]
fn tab_selection_ranges_skip_hidden_members_and_reconcile_after_collapse() {
    let mut book = sample(6, 1);
    let id = group(&mut book, &[0, 1, 2]);
    book.reset_selection();
    book.click(5, false, true);
    assert_eq!(selected_indices(&book), [1, 2, 3, 4, 5]);
    book.groups.set_collapsed(&id, true).unwrap();
    book.reconcile_selection();
    assert_eq!(book.visible_indices(), [3, 4, 5]);
    assert_eq!(selected_indices(&book), [3, 4, 5]);
    book.click(4, false, true);
    assert_eq!(selected_indices(&book), [4]);
    assert!(!book.click(0, true, false));
    assert_eq!(book.select_context(0), 0);
    assert_eq!(selected_indices(&book), [4]);
    assert!(!book.select(1));
    assert!(!book.groups.get(&id).unwrap().collapsed);
    assert!(book.select(0));
    assert!(!book.groups.get(&id).unwrap().collapsed);
    assert_eq!(selected_indices(&book), [0]);
    assert_eq!(book.visible_indices(), [0, 1, 2, 3, 4, 5]);
}

#[test]
fn tab_selection_survives_reordering_and_replaces_a_removed_anchor_with_the_active_tab() {
    let mut book = sample(6, 2);
    let original = ids(&book);
    book.reset_selection();
    book.click(0, true, false);
    assert!(book.move_tab(0, 6));
    assert_eq!(selected_indices(&book), [1, 5]);
    assert_eq!(book.tabs[book.active].id, original[2]);
    book.take_tab(5).unwrap();
    book.click(3, false, true);
    assert_eq!(selected_indices(&book), [1, 2, 3]);
    assert_eq!(book.tabs[book.active].id, original[2]);
    book.open(PathBuf::from("folder02"), false);
    assert_eq!(selected_indices(&book), [5]);
    book.duplicate(0);
    assert_eq!(selected_indices(&book), [1]);
}

#[test]
fn tab_selection_is_not_persisted_and_does_not_mark_the_workspace_dirty() {
    let mut panel = view(sample(4, 0), 0);
    let before = workspace(&panel);
    panel.tabs.click(3, false, true);
    let after = workspace(&panel);
    assert_eq!(before, after);
    assert!(!workspaces_differ(&before, &after));
    let (panels, _, _) = build_panels(after, "Group");
    assert_eq!(selected_indices(&panels[0].tabs), [0]);
}

#[test]
fn mixed_group_selection_does_not_change_the_saved_workspace() {
    let mut book = sample(5, 4);
    let id = group(&mut book, &[0, 1]);
    book.groups.set_collapsed(&id, true).unwrap();
    let mut panel = view(book, 0);
    let before = workspace(&panel);
    panel.tabs.click_group(&id, true, false);
    panel.tabs.click(3, true, true);
    let after = workspace(&panel);
    assert_eq!(before, after);
    assert!(!workspaces_differ(&before, &after));
    let (panels, _, _) = build_panels(after, "Group");
    assert!(!panels[0].tabs.group_selected(&id));
    assert_eq!(selected_indices(&panels[0].tabs), [4]);
}

fn group(book: &mut TabBook, indices: &[usize]) -> String {
    book.groups
        .create(&mut book.tabs, &mut book.active, indices, "Group 1", 3)
        .unwrap()
}

fn view(book: TabBook, mode: u8) -> Panel {
    Panel::from_book(book, favnyr_core::columns::default_columns(), mode, 152.0)
}

fn workspace(panel: &Panel) -> WorkspaceState {
    WorkspaceState {
        active_panel: 0,
        panels: vec![panel_to_state(panel)],
        layout: None,
        workspace_name: None,
        closed_tabs: Vec::new(),
        sidebar_sections: SidebarSectionsState::default(),
    }
}

#[test]
fn tab_groups_contextual_openings_inherit_membership_but_generic_insertions_do_not() {
    let mut book = sample(4, 1);
    let id = group(&mut book, &[0, 1, 2]);
    book.groups.set_collapsed(&id, true).unwrap();
    assert!(book.duplicate(1));
    assert_eq!(book.active, 2);
    assert_eq!(book.tabs[2].tab_group_id.as_deref(), Some(id.as_str()));
    assert!(!book.groups.get(&id).unwrap().collapsed);
    book.groups.set_collapsed(&id, true).unwrap();
    assert_eq!(book.open_after_active(PathBuf::from("folder02"), false), 3);
    assert_eq!(book.tabs[3].tab_group_id.as_deref(), Some(id.as_str()));
    assert!(!book.groups.get(&id).unwrap().collapsed);

    let mut incoming = Tab::new(PathBuf::from("folder03"), false);
    incoming.tab_group_id = Some(id.clone());
    let at = book.insert_tab_at(1, incoming);
    assert_eq!(at, 5);
    assert!(book.tabs[at].tab_group_id.is_none());
    assert!(
        book.tabs[..at]
            .iter()
            .all(|t| t.tab_group_id.as_ref() == Some(&id))
    );
    let at = book.open(PathBuf::from("folder04"), false);
    assert!(book.tabs[at].tab_group_id.is_none());
}

#[test]
fn tab_groups_closure_and_reopening_do_not_reuse_a_group_from_another_view() {
    let mut source = sample(2, 1);
    let source_group = group(&mut source, &[0]);
    let closed = source.close(0).unwrap();
    assert!(source.groups.is_empty());
    let saved = tab_to_state(&closed);
    assert!(saved.tab_group_id.is_none());

    let mut target = sample(2, 0);
    let target_group = group(&mut target, &[0, 1]);
    assert_eq!(source_group, target_group);
    let at = target.insert_tab_at(1, tab_from_state(saved));
    assert_eq!(at, 2);
    assert!(target.tabs[at].tab_group_id.is_none());
    assert_eq!(target.groups.entries().len(), 1);
    assert_eq!(target.groups.entries()[0].id, target_group);
}

#[test]
fn tab_groups_cross_view_transfer_preserves_target_block_and_cleans_the_source() {
    let mut source = sample(2, 1);
    let source_active = source.tabs[1].id;
    group(&mut source, &[0]);
    let mut target = sample(3, 0);
    let target_id = group(&mut target, &[0, 1]);
    let mut panels = vec![view(source, 0), view(target, 2)];
    let mut layout = LayoutNode::row_chain(&[1.0, 1.0]);
    assert_eq!(
        transfer::move_between(&mut panels, &mut layout, 0, 0, 1, 1),
        Some(1)
    );
    assert!(panels[0].tabs.groups.is_empty());
    assert_eq!(panels[0].tabs.tabs[panels[0].tabs.active].id, source_active);
    let target = &panels[1].tabs;
    assert_eq!(target.active, 2);
    let imported = target.tabs[2].tab_group_id.as_deref().unwrap();
    assert_ne!(imported, target_id);
    assert_eq!(target.groups.get(imported).unwrap().name, "Group 1 (2)");
    assert!(
        target.tabs[..2]
            .iter()
            .all(|t| t.tab_group_id.as_ref() == Some(&target_id))
    );
    assert!(layout.is_valid_for(panels.len()));
}

#[test]
fn tab_groups_workspace_round_trip_keeps_live_settings_in_all_bar_positions() {
    for mode in 0..=2 {
        let mut book = sample(4, 3);
        let id = group(&mut book, &[0, 2]);
        book.groups.set_collapsed(&id, true).unwrap();
        for (i, tab) in book.tabs.iter_mut().enumerate() {
            tab.current_path = PathBuf::from(format!("folder{:02}", i + 1));
            tab.item_arrangement = ItemArrangement::Grid;
            tab.grid_zoom = Some(3);
            tab.zoom = 5;
            tab.preview = i % 2 == 0;
            tab.show_hidden = true;
            tab.group_mode = GroupMode::Mixed;
        }
        let panel = view(book, mode);
        let original_ids = ids(&panel.tabs);
        let before = workspace(&panel);
        let encoded = toml::to_string_pretty(&before).unwrap();
        let stored = toml::from_str(&encoded).unwrap();
        let (panels, layout, active) = build_panels(stored, "Group");
        let after = workspace(&panels[0]);
        assert_eq!(after, before);
        assert!(!workspaces_differ(&before, &after));
        assert!(
            ids(&panels[0].tabs)
                .iter()
                .all(|id| !original_ids.contains(id))
        );
        assert_eq!(active, 0);
        assert!(layout.is_valid_for(1));
    }
}

#[test]
fn tab_groups_content_changes_mark_workspace_dirty_but_collapse_and_focus_do_not() {
    let mut book = sample(4, 0);
    let id = group(&mut book, &[0, 1]);
    let mut panel = view(book, 1);
    let original = workspace(&panel);
    panel.tabs.groups.set_collapsed(&id, true).unwrap();
    panel.tabs.active = 3;
    assert!(!workspaces_differ(&original, &workspace(&panel)));

    panel.tabs.groups.rename(&id, "Group 2").unwrap();
    assert!(workspaces_differ(&original, &workspace(&panel)));
    panel.tabs.groups.rename(&id, "Group 1").unwrap();
    panel.tabs.groups.set_color(&id, 5).unwrap();
    assert!(workspaces_differ(&original, &workspace(&panel)));
    panel.tabs.groups.set_color(&id, 3).unwrap();
    assert!(!workspaces_differ(&original, &workspace(&panel)));
    panel
        .tabs
        .groups
        .assign(&mut panel.tabs.tabs, &mut panel.tabs.active, &[3], &id)
        .unwrap();
    assert!(workspaces_differ(&original, &workspace(&panel)));
    panel
        .tabs
        .groups
        .dissolve(&mut panel.tabs.tabs, &id)
        .unwrap();
    assert!(workspaces_differ(&original, &workspace(&panel)));
}

#[test]
fn tab_groups_detached_view_preserves_metadata_and_authoritative_native_paths() {
    for mode in 0..=2 {
        let mut book = sample(3, 1);
        let id = group(&mut book, &[0, 1]);
        book.groups.set_collapsed(&id, true).unwrap();
        let panel = view(book, mode);
        let mut saved = panel_to_state(&panel);
        let dirs = vec![
            PathBuf::from("目录01"),
            PathBuf::from("folder02"),
            PathBuf::from("folder03"),
        ];
        #[cfg(unix)]
        let dirs = {
            use std::os::unix::ffi::OsStringExt;
            let mut dirs = dirs;
            dirs[2] = PathBuf::from(std::ffi::OsString::from_vec(b"folder\xff".to_vec()));
            dirs
        };
        let args = crate::detached_view::arguments(&saved, &dirs, None).unwrap();
        let packet = crate::detached_view::parse(args.into_iter().skip(1)).unwrap();
        let restored = Panel::from_detached(&packet.view, &packet.dirs, "Group").unwrap();
        for ((tab, path), stored) in restored.tabs.tabs.iter().zip(&dirs).zip(&mut saved.tabs) {
            assert_eq!(&tab.current_path, path);
            stored.path = path.display().to_string();
        }
        assert_eq!(panel_to_state(&restored), saved);
        assert_eq!(restored.displayed_path, dirs[1]);
    }
}

#[test]
fn tab_groups_legacy_single_tab_transfer_is_explicitly_ungrouped() {
    let mut book = sample(2, 0);
    group(&mut book, &[0, 1]);
    let original = tab_to_state(&book.tabs[0]);
    let (received, _) = deserialize_tab(&serialize_tab(&book.tabs[0], (0, 0))).unwrap();
    let mut expected = original.clone();
    expected.tab_group_id = None;
    assert_eq!(tab_to_state(&received), expected);
    assert_eq!(tab_to_state(&book.tabs[0]), original);
}

#[test]
fn tab_groups_invalid_names_are_repaired_on_workspace_and_detached_view_restore() {
    let input = "[[panels]]\ntab_groups = [{id='g1', name=5}, {id='g2', name='Group'}]\n[[panels.tabs]]\npath='folder01'\ntab_group_id='g1'\n[[panels.tabs]]\npath='folder02'\ntab_group_id='g2'\n";
    let stored: WorkspaceState = toml::from_str(input).unwrap();
    let dirs = vec![PathBuf::from("folder01"), PathBuf::from("folder02")];
    let detached = Panel::from_detached(&stored.panels[0], &dirs, "Group").unwrap();
    let (panels, _, _) = build_panels(stored, "Group");
    for panel in [&panels[0], &detached] {
        assert_eq!(panel.tabs.groups.get("g1").unwrap().name, "Group (2)");
        assert_eq!(panel.tabs.groups.get("g2").unwrap().name, "Group");
        assert_eq!(panel.tabs.tabs.len(), 2);
    }
}

#[test]
fn tab_block_moves_keep_the_active_identity_and_all_tab_state() {
    for active in 0..5 {
        let mut book = sample(5, active);
        let original = ids(&book);
        book.tabs[1].history.push(PathBuf::from("folder02"));
        book.tabs[1].ext_filter = "png".into();
        book.tabs[1].ext_filter_on = true;
        book.tabs[1].cursor = 7;
        let saved: Vec<_> = book.tabs.iter().map(tab_to_state).collect();
        assert!(book.move_tabs(&[original[3], original[1]], 5));
        let expected = [0, 2, 4, 1, 3];
        assert_eq!(ids(&book), expected.map(|i| original[i]));
        assert_eq!(book.tabs[book.active].id, original[active]);
        for (tab, source) in book.tabs.iter().zip(expected) {
            assert_eq!(tab_to_state(tab), saved[source]);
        }
        let moved = &book.tabs[3];
        assert!(moved.history.can_back());
        assert_eq!(moved.ext_filter, "png");
        assert!(moved.ext_filter_on);
        assert_eq!(moved.cursor, 7);
    }
}

#[test]
fn tab_moves_reject_stale_or_duplicate_identities_without_partial_mutation() {
    let mut book = sample(4, 2);
    let original = ids(&book);
    let foreign = Tab::new(PathBuf::from("folder01"), false).id;
    for requested in [vec![], vec![original[1], foreign], vec![original[1]; 2]] {
        assert!(!book.move_tabs(&requested, 0));
        assert_eq!(ids(&book), original);
        assert_eq!(book.active, 2);
    }
    assert!(!book.move_tab(usize::MAX, 0));
    assert!(!book.move_tabs(&original, 2));
    assert_eq!(ids(&book), original);
    assert_eq!(book.active, 2);
}

#[test]
fn tab_extraction_preserves_surviving_focus_and_chooses_a_close_neighbor() {
    for active in 0..4 {
        for removed in 0..4 {
            let mut book = sample(4, active);
            let original = ids(&book);
            assert_eq!(book.take_tab(removed).unwrap().id, original[removed]);
            let expected_active = if active != removed {
                original[active]
            } else {
                original[if removed == 3 { 2 } else { removed + 1 }]
            };
            assert_eq!(book.tabs[book.active].id, expected_active);
            let expected: Vec<_> = original
                .into_iter()
                .enumerate()
                .filter_map(|(i, id)| (i != removed).then_some(id))
                .collect();
            assert_eq!(ids(&book), expected);
        }
    }
    let mut sole = sample(1, 0);
    let id = sole.tabs[0].id;
    assert!(sole.close(0).is_none());
    assert!(sole.take_tab(4).is_none());
    assert_eq!(sole.take_tab(0).unwrap().id, id);
    assert!(sole.tabs.is_empty());
    assert_eq!(sole.active, 0);
    assert!(sole.take_tab(0).is_none());
}

#[test]
fn duplication_and_restore_allocate_new_ids_while_transfer_keeps_the_original() {
    let mut source = sample(2, 0);
    let original = source.tabs[0].id;
    let saved = tab_to_state(&source.tabs[0]);
    assert!(source.duplicate(0));
    assert_ne!(source.tabs[1].id, original);
    assert_eq!(tab_to_state(&source.tabs[1]), saved);
    let restored = tab_from_state(saved.clone());
    assert_ne!(restored.id, original);
    assert_ne!(restored.id, source.tabs[1].id);
    assert_eq!(tab_to_state(&restored), saved);
    let mut target = sample(1, 0);
    let moved = source.take_tab(0).unwrap();
    let at = target.insert_tab_at(0, moved);
    assert_eq!(target.tabs[at].id, original);
    assert_eq!(tab_to_state(&target.tabs[at]), saved);
    let text = toml::to_string(&saved).unwrap();
    assert!(!text.contains("id ="));
}

#[test]
fn group_pages_keep_the_last_folder_until_a_member_is_explicitly_activated() {
    for mode in 0..=2 {
        let mut book = sample(5, 4);
        let id = group(&mut book, &[0, 1]);
        let original = book.tabs[4].id;
        let paths: Vec<_> = book.tabs.iter().map(|t| t.current_path.clone()).collect();
        book.groups.set_collapsed(&id, true).unwrap();
        assert!(book.select_from_wheel(0));
        assert_eq!(book.tabs[book.active].id, original);
        assert_eq!(book.group_page().unwrap().id, id);
        assert_eq!(book.selection_len(), 0);
        let projection = super::projection::Presentation::default();
        projection.update(&book, mode, &Default::default());
        assert_eq!(projection.state(&book).active, -1);
        assert_eq!(projection.step(&book, true), 2);
        assert_eq!(projection.step(&book, false), 4);
        book.groups.set_collapsed(&id, false).unwrap();
        book.reconcile_selection();
        projection.update(&book, mode, &Default::default());
        assert!(book.group_page().is_some());
        assert_eq!(projection.step(&book, true), 0);
        assert!(book.select(1));
        assert!(book.group_page().is_none());
        assert_eq!(book.active, 1);
        book.groups.set_collapsed(&id, true).unwrap();
        book.focus_group(&id);
        assert!(book.select(1));
        assert!(!book.groups.get(&id).unwrap().collapsed);
        assert!(book.group_page().is_none());
        assert_eq!(
            book.tabs
                .iter()
                .map(|t| t.current_path.clone())
                .collect::<Vec<_>>(),
            paths
        );
    }
}

#[test]
fn group_pages_release_member_selections_and_follow_group_removal() {
    let mut book = sample(4, 3);
    let id = group(&mut book, &[0, 1]);
    book.tabs[1].current_path = PathBuf::from("folder01/missing");
    book.focus_group(&id);
    book.click(0, true, false);
    book.click(1, true, false);
    assert_eq!(book.selection_len(), 2);
    assert!(book.reset_selection());
    assert!(!book.reset_selection());
    assert_eq!(book.selection_len(), 0);
    assert_eq!(book.group_page().unwrap().id, id);
    book.groups.rename(&id, "Group 02").unwrap();
    assert_eq!(book.group_page().unwrap().name, "Group 02");
    book.groups.dissolve(&mut book.tabs, &id).unwrap();
    book.reconcile_selection();
    assert!(book.group_page().is_none());
    assert!(!book.group_highlighted(&id));
    assert_eq!(book.active, 3);
}

#[test]
fn closing_an_unrelated_tab_keeps_the_group_page_and_closing_its_last_member_clears_it() {
    let mut book = sample(5, 4);
    let id = group(&mut book, &[0, 1]);
    book.groups.set_collapsed(&id, true).unwrap();
    book.focus_group(&id);
    book.close(4).unwrap();
    assert_eq!(book.group_page().unwrap().id, id);
    let retained = book.tabs[book.active].id;
    book.close(0).unwrap();
    assert_eq!(book.group_page().unwrap().id, id);
    book.close(0).unwrap();
    assert!(book.group_page().is_none());
    assert_eq!(book.tabs[book.active].id, retained);
}
