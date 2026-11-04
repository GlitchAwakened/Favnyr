use super::*;
use crate::workspace::{TabState, WorkspaceState};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    key: usize,
    group: Option<String>,
}

impl GroupedTab for Entry {
    fn tab_group_id(&self) -> Option<&str> {
        self.group.as_deref()
    }
    fn set_tab_group_id(&mut self, id: Option<String>) {
        self.group = id;
    }
}

fn entries(count: usize) -> Vec<Entry> {
    (0..count).map(|key| Entry { key, group: None }).collect()
}

#[test]
fn suggested_names_do_not_create_a_group_or_reuse_existing_names() {
    let mut groups = TabGroups::default();
    let mut tabs = entries(2);
    let mut active = 0;
    groups
        .create(&mut tabs, &mut active, &[0], "Group", 0)
        .unwrap();
    let before = groups.clone();
    assert_eq!(groups.suggested_name(" Group ").unwrap(), "Group (2)");
    assert_eq!(groups.suggested_name(" "), Err(GroupError::InvalidName));
    assert_eq!(groups, before);
}

fn keys(tabs: &[Entry]) -> Vec<usize> {
    tabs.iter().map(|t| t.key).collect()
}

fn assert_invariants(groups: &TabGroups, tabs: &[Entry]) {
    let before = tabs.to_vec();
    let mut normalized = groups.clone();
    let mut after = before.clone();
    normalized.normalize(&mut after);
    assert_eq!(after, before);
    assert_eq!(&normalized, groups);
    let mut all = keys(tabs);
    all.sort_unstable();
    assert_eq!(all, (0..tabs.len()).collect::<Vec<_>>());
}

#[test]
fn tab_groups_create_a_stable_block_and_keep_each_possible_active_tab() {
    for current in 0..6 {
        let mut tabs = entries(6);
        let mut active = current;
        let mut groups = TabGroups::default();
        let id = groups
            .create(&mut tabs, &mut active, &[4, 1, 3], "Group 1", 4)
            .unwrap();
        assert_eq!(keys(&tabs), [0, 1, 3, 4, 2, 5]);
        assert_eq!(tabs[active].key, current);
        assert!(tabs[1..4].iter().all(|t| t.tab_group_id() == Some(&id)));
        assert_eq!(groups.get(&id).unwrap().color_slot, 4);
        assert!(!groups.get(&id).unwrap().collapsed);
        assert_invariants(&groups, &tabs);
    }
}

#[test]
fn tab_groups_creation_inside_a_residual_group_does_not_split_it() {
    let mut tabs = entries(6);
    let mut active = 2;
    let mut groups = TabGroups::default();
    let first = groups
        .create(&mut tabs, &mut active, &[1, 2, 3], "Group 1", 0)
        .unwrap();
    let second = groups
        .create(&mut tabs, &mut active, &[2, 5], "Group 2", 1)
        .unwrap();
    assert_eq!(keys(&tabs), [0, 1, 3, 2, 5, 4]);
    assert_eq!(tabs[active].key, 2);
    assert!(tabs[1..3].iter().all(|t| t.tab_group_id() == Some(&first)));
    assert!(tabs[3..5].iter().all(|t| t.tab_group_id() == Some(&second)));
    assert_invariants(&groups, &tabs);
}

#[test]
fn tab_groups_assignment_keeps_existing_members_and_appends_only_new_ones() {
    let mut tabs = entries(6);
    let mut active = 3;
    let mut groups = TabGroups::default();
    let id = groups
        .create(&mut tabs, &mut active, &[2, 3], "Group 1", 0)
        .unwrap();
    groups.set_collapsed(&id, true).unwrap();
    assert!(!groups.assign(&mut tabs, &mut active, &[3, 2], &id).unwrap());
    assert!(groups.get(&id).unwrap().collapsed);
    assert!(
        groups
            .assign(&mut tabs, &mut active, &[5, 2, 0], &id)
            .unwrap()
    );
    assert_eq!(keys(&tabs), [1, 2, 3, 0, 5, 4]);
    assert_eq!(tabs[active].key, 3);
    assert!(!groups.get(&id).unwrap().collapsed);
    assert_invariants(&groups, &tabs);
}

#[test]
fn tab_groups_removal_is_local_to_each_block_and_dissolve_keeps_order() {
    let mut tabs = entries(7);
    let mut active = 4;
    let mut groups = TabGroups::default();
    let a = groups
        .create(&mut tabs, &mut active, &[0, 1, 2], "Group 1", 0)
        .unwrap();
    let b = groups
        .create(&mut tabs, &mut active, &[3, 4, 5], "Group 2", 0)
        .unwrap();
    assert!(
        groups
            .remove_members(&mut tabs, &mut active, &[1, 3, 6])
            .unwrap()
    );
    assert_eq!(keys(&tabs), [0, 2, 1, 4, 5, 3, 6]);
    assert_eq!(tabs[active].key, 4);
    assert_invariants(&groups, &tabs);
    let before = keys(&tabs);
    groups.dissolve(&mut tabs, &a).unwrap();
    groups.dissolve(&mut tabs, &b).unwrap();
    assert_eq!(keys(&tabs), before);
    assert!(tabs.iter().all(|t| t.group.is_none()));
    assert!(groups.is_empty());
}

#[test]
fn tab_groups_last_member_removal_cleans_metadata_and_never_reuses_ids() {
    let mut tabs = entries(2);
    let mut active = 0;
    let mut groups = TabGroups::default();
    let first = groups
        .create(&mut tabs, &mut active, &[0], "Group 1", 0)
        .unwrap();
    assert!(groups.remove_members(&mut tabs, &mut active, &[0]).unwrap());
    assert!(groups.is_empty());
    let next = groups
        .create(&mut tabs, &mut active, &[0], "Group 1", 0)
        .unwrap();
    assert_ne!(next, first);
    assert!(groups.get(&first).is_none());
}

#[test]
fn tab_groups_rejected_commands_do_not_change_content_or_focus() {
    let mut tabs = entries(4);
    let mut active = 1;
    let mut groups = TabGroups::default();
    let id = groups
        .create(&mut tabs, &mut active, &[0, 1], "Group 1", 0)
        .unwrap();
    let before = (groups.clone(), tabs.clone(), active);
    for indices in [vec![], vec![0, 0], vec![0, 9]] {
        assert!(
            groups
                .create(&mut tabs, &mut active, &indices, "Group 2", 0)
                .is_err()
        );
        assert!(
            groups
                .assign(&mut tabs, &mut active, &indices, &id)
                .is_err()
        );
        assert!(
            groups
                .remove_members(&mut tabs, &mut active, &indices)
                .is_err()
        );
        assert!(
            groups
                .move_members(&mut tabs, &mut active, &indices, 3)
                .is_err()
        );
    }
    for name in ["", "  ", "my\nname", "  GROUP 1  "] {
        assert!(
            groups
                .create(&mut tabs, &mut active, &[2], name, 0)
                .is_err()
        );
    }
    assert!(
        groups
            .create(&mut tabs, &mut active, &[2], "Group 2", MAX_COLOR_SLOT + 1)
            .is_err()
    );
    assert!(
        groups
            .assign(&mut tabs, &mut active, &[2], "missing")
            .is_err()
    );
    assert!(groups.dissolve(&mut tabs, "missing").is_err());
    assert_eq!((groups, tabs, active), before);
}

#[test]
fn tab_groups_metadata_is_optional_unicode_and_separate_from_identity() {
    let mut tabs = entries(3);
    let mut active = 0;
    let mut groups = TabGroups::default();
    let a = groups
        .create(&mut tabs, &mut active, &[0], " Group 1 ", 0)
        .unwrap();
    let b = groups
        .create(&mut tabs, &mut active, &[1], "Group 2", 0)
        .unwrap();
    assert_eq!(groups.rename(&a, "group 2"), Err(GroupError::NameTaken));
    assert!(groups.rename(&a, "文件 / : ?").unwrap());
    assert_eq!(groups.get(&a).unwrap().name, "文件 / : ?");
    assert!(groups.set_color(&a, MAX_COLOR_SLOT).unwrap());
    assert_eq!(
        groups.set_color(&a, MAX_COLOR_SLOT + 1),
        Err(GroupError::InvalidColor)
    );
    assert!(groups.set_collapsed(&a, true).unwrap());
    assert!(!groups.set_collapsed(&a, true).unwrap());
    assert_eq!(tabs[0].tab_group_id(), Some(a.as_str()));
    assert_eq!(tabs[1].tab_group_id(), Some(b.as_str()));
    assert!(groups.rename(&a, &"界".repeat(MAX_NAME_CHARS)).unwrap());
    assert_eq!(
        groups.rename(&a, &"界".repeat(MAX_NAME_CHARS + 1)),
        Err(GroupError::InvalidName)
    );
}

#[test]
fn tab_groups_moves_preserve_whole_blocks_and_detach_fragments() {
    let mut tabs = entries(6);
    let mut active = 1;
    let mut groups = TabGroups::default();
    let a = groups
        .create(&mut tabs, &mut active, &[0, 1, 2], "Group 1", 3)
        .unwrap();
    let b = groups
        .create(&mut tabs, &mut active, &[3, 4], "Group 2", 0)
        .unwrap();
    assert!(
        groups
            .move_members(&mut tabs, &mut active, &[0], 2)
            .unwrap()
    );
    assert_eq!(keys(&tabs), [1, 0, 2, 3, 4, 5]);
    assert_eq!(tabs[1].tab_group_id(), Some(a.as_str()));
    assert!(
        groups
            .move_members(&mut tabs, &mut active, &[1], 4)
            .unwrap()
    );
    assert_eq!(keys(&tabs), [1, 2, 3, 4, 0, 5]);
    assert!(tabs[4].group.is_none());
    assert!(
        groups
            .move_members(&mut tabs, &mut active, &[2, 3], 0)
            .unwrap()
    );
    assert_eq!(keys(&tabs), [3, 4, 1, 2, 0, 5]);
    assert_eq!(groups.entries()[0].id, b);
    assert_eq!(tabs[active].key, 1);
    assert_invariants(&groups, &tabs);
}

#[test]
fn tab_groups_workspace_round_trip_preserves_legacy_fields_and_native_order() {
    let mut ws: WorkspaceState = toml::from_str("[[panels]]\nactive_tab = 1\n[[panels.tabs]]\npath = 'folder01'\n[[panels.tabs]]\npath = 'folder02'\npreview = true\nzoom = 5\n").unwrap();
    let original = ws.clone();
    let legacy = toml::to_string(&ws).unwrap();
    assert!(!legacy.contains("tab_group"));
    let view = &mut ws.panels[0];
    let id = view
        .tab_groups
        .create(&mut view.tabs, &mut view.active_tab, &[0, 1], "Group 1", 4)
        .unwrap();
    view.tab_groups.set_collapsed(&id, true).unwrap();
    let text = toml::to_string_pretty(&ws).unwrap();
    let restored: WorkspaceState = toml::from_str(&text).unwrap();
    let restored = restored.sanitized();
    assert_eq!(restored.panels, ws.panels);
    let mut ungrouped = restored.panels[0].clone();
    ungrouped
        .tab_groups
        .dissolve(&mut ungrouped.tabs, &id)
        .unwrap();
    assert_eq!(ungrouped, original.panels[0]);
}

#[test]
fn tab_groups_corrupt_metadata_recovers_locally_without_dropping_tabs() {
    let input = r#"
[[panels]]
active_tab = 3
tab_groups = [
 {id = "g4", name = 10, color_slot = 400, collapsed = "bad"},
 {id = "g4", name = "ignored"},
 {id = "g5", name = "Group 1"},
 {id = "g6", name = "group 1"},
 {id = "empty", name = "Group 2"},
 {id = "bad id", name = "Group 3"},
 22
]
[[panels.tabs]]
path = "folder01"
tab_group_id = "g4"
[[panels.tabs]]
path = "folder02"
tab_group_id = "missing"
[[panels.tabs]]
path = "folder03"
tab_group_id = "g4"
[[panels.tabs]]
path = "folder04"
tab_group_id = 5
[[panels.tabs]]
path = "folder05"
tab_group_id = "g5"
[[panels.tabs]]
path = "folder06"
tab_group_id = "g6"
"#;
    let ws: WorkspaceState = toml::from_str(input).unwrap();
    let paths: Vec<_> = ws.panels[0].tabs.iter().map(|t| t.path.clone()).collect();
    let mut ws = ws.sanitized();
    let view = &mut ws.panels[0];
    assert_eq!(view.active_tab, 3);
    assert_eq!(
        view.tabs.iter().map(|t| t.path.clone()).collect::<Vec<_>>(),
        paths
    );
    assert_eq!(view.tab_groups.entries().len(), 3);
    assert_eq!(view.tab_groups.get("g4").unwrap().color_slot, 0);
    assert!(!view.tab_groups.get("g4").unwrap().collapsed);
    assert!(view.tabs[1..4].iter().all(|t| t.tab_group_id.is_none()));
    assert!(view.tab_groups.repair_names("Group 1").unwrap());
    assert_eq!(view.tab_groups.get("g4").unwrap().name, "Group 1 (2)");
    assert_eq!(view.tab_groups.get("g6").unwrap().name, "Group 1 (3)");
    let next = view
        .tab_groups
        .create(&mut view.tabs, &mut view.active_tab, &[3], "Group 4", 0)
        .unwrap();
    assert_eq!(next, "g7");
}

#[test]
fn tab_groups_invalid_container_and_closed_tab_membership_are_discarded() {
    for invalid in ["0", "false", "'bad'", "{}"] {
        let text = format!(
            "[[panels]]\ntab_groups = {invalid}\n[[panels.tabs]]\npath = 'folder01'\ntab_group_id = 'g1'\n[[closed_tabs]]\npath = 'folder02'\ntab_group_id = 'g1'\n"
        );
        let ws: WorkspaceState = toml::from_str(&text).unwrap();
        let ws = ws.sanitized();
        assert!(ws.panels[0].tab_groups.is_empty());
        assert!(ws.panels[0].tabs[0].tab_group_id.is_none());
        assert!(ws.closed_tabs[0].tab_group_id.is_none());
        assert_eq!(ws.closed_tabs[0].path, "folder02");
    }
    let tab: TabState = toml::from_str("path = 'folder01'\ntab_group_id = ['bad']").unwrap();
    assert!(tab.tab_group_id.is_none());
}

#[test]
fn tab_groups_every_small_selection_preserves_tabs_focus_and_group_contiguity() {
    let mut original_tabs = entries(6);
    let mut original_groups = TabGroups::default();
    let mut active = 0;
    let first = original_groups
        .create(&mut original_tabs, &mut active, &[0, 1, 2], "Group 1", 0)
        .unwrap();
    original_groups
        .create(&mut original_tabs, &mut active, &[3, 4], "Group 2", 1)
        .unwrap();
    for mask in 1..(1 << original_tabs.len()) {
        let selected: Vec<_> = (0..original_tabs.len())
            .filter(|i| mask & (1 << i) != 0)
            .collect();
        for current in 0..original_tabs.len() {
            for action in 0..10 {
                let mut tabs = original_tabs.clone();
                let mut groups = original_groups.clone();
                let mut active = current;
                let mut created = None;
                match action {
                    0 => {
                        created = Some(
                            groups
                                .create(&mut tabs, &mut active, &selected, "Group 3", 0)
                                .unwrap(),
                        );
                    }
                    1 => {
                        groups
                            .assign(&mut tabs, &mut active, &selected, &first)
                            .unwrap();
                    }
                    2 => {
                        groups
                            .remove_members(&mut tabs, &mut active, &selected)
                            .unwrap();
                    }
                    _ => {
                        groups
                            .move_members(&mut tabs, &mut active, &selected, action - 3)
                            .unwrap();
                    }
                }
                assert_eq!(tabs[active].key, current);
                for tab in &tabs {
                    if !selected.contains(&tab.key) {
                        assert_eq!(tab.group, original_tabs[tab.key].group);
                    } else {
                        match action {
                            0 => assert_eq!(tab.group, created),
                            1 => assert_eq!(tab.group.as_ref(), Some(&first)),
                            2 => assert!(tab.group.is_none()),
                            _ => assert!(
                                tab.group.is_none() || tab.group == original_tabs[tab.key].group
                            ),
                        }
                    }
                }
                assert_invariants(&groups, &tabs);
            }
        }
    }
}
