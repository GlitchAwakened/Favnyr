use super::batch::{Batch, Destination};
use super::{TabBook, TabId, tab_to_state, tests::sample};
use std::path::PathBuf;

fn ids(book: &TabBook) -> Vec<TabId> {
    book.tabs.iter().map(|t| t.id).collect()
}

fn group(book: &mut TabBook, indices: &[usize], name: &str, slot: u8) -> String {
    book.groups
        .create(&mut book.tabs, &mut book.active, indices, name, slot)
        .unwrap()
}

#[test]
fn capture_freezes_selection_in_book_order_and_keeps_the_transported_active_tab() {
    let mut book = sample(6, 2);
    let original = ids(&book);
    book.select_ids(&[original[4], original[2]]);
    let batch = Batch::capture(&book, 4, None).unwrap();
    assert_eq!(batch.ids, [original[2], original[4]]);
    assert_eq!(batch.active, original[2]);
    book.select_ids(&[original[1], original[4]]);
    let other = Batch::capture(&book, 4, None).unwrap();
    assert_eq!(other.active, original[4]);
    assert_eq!(batch.ids, [original[2], original[4]]);
    assert_eq!(Batch::capture(&book, 0, None).unwrap().ids, [original[0]]);
    let id = group(&mut book, &[1, 2, 3], "Group 01", 2);
    book.groups.set_collapsed(&id, true).unwrap();
    let header = Batch::capture(&book, usize::MAX, Some(&id)).unwrap();
    assert_eq!(header.ids, original[1..4]);
    assert_eq!(header.active, original[2]);
}

#[test]
fn reordering_a_focused_group_keeps_its_outline_without_activating_a_member() {
    let mut book = sample(5, 4);
    let id = group(&mut book, &[0, 1], "Group 01", 2);
    book.highlight_group(&id);
    let active = book.tabs[book.active].id;
    let batch = Batch::capture(&book, usize::MAX, Some(&id)).unwrap();
    assert!(batch.reorder(&mut book, &Destination::outside(5)).unwrap());
    assert!(book.group_highlighted(&id));
    assert_eq!(book.tabs[book.active].id, active);
    book.reset_selection();
    assert!(book.group_highlighted(&id));
    book.close(4).unwrap();
    assert!(book.group_highlighted(&id));
    book.close(3).unwrap();
    assert!(!book.group_highlighted(&id));
    assert_eq!(book.tabs[book.active].id, active);
}

#[test]
fn focused_header_selection_transfers_or_joins_without_losing_members_or_focus() {
    for collapsed in [false, true] {
        for join in [false, true] {
            let mut source = sample(6, 3);
            let id = group(&mut source, &[1, 2], "Group 01", 2);
            source.groups.set_collapsed(&id, collapsed).unwrap();
            source.focus_group(&id);
            source.click(3, true, false);
            let original = ids(&source);
            let batch = Batch::capture(&source, 3, None).unwrap();
            assert_eq!(batch.ids, original[1..4]);
            assert_eq!(batch.paths(&source).unwrap().len(), 3);
            let mut target = sample(3, 2);
            let target_group = group(&mut target, &[0, 1], "Group 02", 4);
            target.groups.set_collapsed(&target_group, true).unwrap();
            let destination = if join {
                Destination {
                    gap: 1,
                    group: Some(target_group.clone()),
                }
            } else {
                Destination::outside(3)
            };
            batch
                .transfer(&mut source, &mut target, &destination)
                .unwrap();
            assert_eq!(ids(&source), [original[0], original[4], original[5]]);
            assert!(source.group_page().is_none());
            assert!(!source.group_highlighted(&id));
            assert!(source.groups.get(&id).is_none());
            assert_eq!(target.selected_ids(), original[1..4]);
            assert_eq!(target.tabs[target.active].id, original[3]);
            assert!(target.group_page().is_none());
            if join {
                assert_eq!(target.groups.entries().len(), 1);
                assert!(!target.groups.get(&target_group).unwrap().collapsed);
                for tab in target.tabs.iter().filter(|tab| batch.ids.contains(&tab.id)) {
                    assert_eq!(tab.tab_group_id.as_deref(), Some(target_group.as_str()));
                }
            } else {
                let imported = target
                    .groups
                    .entries()
                    .iter()
                    .find(|group| group.name == "Group 01")
                    .unwrap();
                assert_eq!(imported.color_slot, 2);
                assert_eq!(imported.collapsed, collapsed);
                assert!(target.tabs[target.active].tab_group_id.is_none());
            }
        }
    }
}

#[test]
fn transfer_keeps_complete_groups_separate_and_preserves_tab_settings_and_history() {
    let mut source = sample(6, 2);
    let full = group(&mut source, &[1, 2], "Group 01", 3);
    let partial = group(&mut source, &[3, 4], "Group 02", 4);
    source.tabs[2].history.push(PathBuf::from("folder02"));
    source.tabs[2].ext_filter = "png".into();
    let original = ids(&source);
    source.select_ids(&original[1..4]);
    let batch = Batch::capture(&source, 1, None).unwrap();
    let mut target = sample(3, 0);
    let existing = group(&mut target, &[0, 1], "Group 01", 1);
    let target_ids = ids(&target);
    batch
        .transfer(&mut source, &mut target, &Destination::outside(1))
        .unwrap();
    assert_eq!(ids(&source), [original[0], original[4], original[5]]);
    assert_eq!(source.tabs[source.active].id, original[4]);
    assert_eq!(source.groups.entries().len(), 1);
    assert_eq!(source.groups.entries()[0].id, partial);
    assert_eq!(
        ids(&target),
        [
            target_ids[0],
            target_ids[1],
            original[1],
            original[2],
            original[3],
            target_ids[2]
        ]
    );
    assert_eq!(target.tabs[target.active].id, original[2]);
    let imported = target.tabs[2].tab_group_id.as_ref().unwrap();
    assert_ne!(imported, &existing);
    assert_ne!(imported, &full);
    assert_eq!(target.groups.get(&existing).unwrap().color_slot, 1);
    let metadata = target.groups.get(imported).unwrap();
    assert_eq!(metadata.name, "Group 01 (2)");
    assert_eq!(metadata.color_slot, 3);
    assert!(!metadata.collapsed);
    assert_eq!(target.tabs[3].tab_group_id.as_ref(), Some(imported));
    assert!(target.tabs[4].tab_group_id.is_none());
    assert!(target.tabs[3].history.can_back());
    assert_eq!(target.tabs[3].ext_filter, "png");
    assert_eq!(target.selected_ids(), batch.ids);
}

#[test]
fn explicit_join_expands_the_target_and_uses_its_metadata() {
    let mut source = sample(3, 1);
    let id = group(&mut source, &[0, 1], "Group 01", 5);
    source.groups.set_collapsed(&id, true).unwrap();
    let batch = Batch::capture(&source, 0, Some(&id)).unwrap();
    let mut target = sample(3, 0);
    let target_group = group(&mut target, &[0, 1], "Group 02", 2);
    target.groups.set_collapsed(&target_group, true).unwrap();
    let old = ids(&target);
    batch
        .transfer(
            &mut source,
            &mut target,
            &Destination {
                gap: 1,
                group: Some(target_group.clone()),
            },
        )
        .unwrap();
    assert_eq!(
        ids(&target),
        [old[0], batch.ids[0], batch.ids[1], old[1], old[2]]
    );
    assert_eq!(target.groups.entries().len(), 1);
    assert!(!target.groups.get(&target_group).unwrap().collapsed);
    assert_eq!(target.groups.get(&target_group).unwrap().color_slot, 2);
    assert!(
        target.tabs[..4]
            .iter()
            .all(|t| t.tab_group_id.as_ref() == Some(&target_group))
    );
    assert!(source.groups.is_empty());
}

#[test]
fn internal_join_reorders_members_and_exterior_drop_removes_partial_membership() {
    let mut book = sample(6, 2);
    let id = group(&mut book, &[1, 2, 3], "Group 01", 0);
    let original = ids(&book);
    let batch = Batch::single(&book, 3).unwrap();
    batch
        .reorder(
            &mut book,
            &Destination {
                gap: 1,
                group: Some(id.clone()),
            },
        )
        .unwrap();
    assert_eq!(
        ids(&book),
        [
            original[0],
            original[3],
            original[1],
            original[2],
            original[4],
            original[5]
        ]
    );
    assert_eq!(book.tabs[book.active].id, original[2]);
    assert!(
        book.tabs[1..4]
            .iter()
            .all(|t| t.tab_group_id.as_ref() == Some(&id))
    );
    batch.reorder(&mut book, &Destination::outside(0)).unwrap();
    assert_eq!(book.tabs[0].id, original[3]);
    assert!(book.tabs[0].tab_group_id.is_none());
    assert!(book.groups.get(&id).is_some());
}

#[test]
fn stale_batches_and_missing_destinations_reject_before_either_book_changes() {
    let mut source = sample(4, 1);
    let id = group(&mut source, &[1, 2], "Group 01", 1);
    let batch = Batch::capture(&source, 0, Some(&id)).unwrap();
    let mut target = sample(2, 0);
    let snapshot = |book: &TabBook| {
        (
            ids(book),
            book.active,
            book.groups.clone(),
            book.tabs.iter().map(tab_to_state).collect::<Vec<_>>(),
        )
    };
    let before = (snapshot(&source), snapshot(&target));
    assert!(
        batch
            .transfer(
                &mut source,
                &mut target,
                &Destination {
                    gap: 0,
                    group: Some("missing".into())
                }
            )
            .is_err()
    );
    assert_eq!((snapshot(&source), snapshot(&target)), before);
    source
        .groups
        .assign(&mut source.tabs, &mut source.active, &[3], &id)
        .unwrap();
    let before = (snapshot(&source), snapshot(&target));
    assert!(
        batch
            .transfer(&mut source, &mut target, &Destination::outside(0))
            .is_err()
    );
    assert_eq!((snapshot(&source), snapshot(&target)), before);
    assert!(batch.indices(&target).is_err());
}

#[test]
fn detachment_snapshot_keeps_native_paths_complete_groups_and_the_active_member() {
    let mut book = sample(5, 2);
    let id = group(&mut book, &[1, 2], "Group 01", 3);
    book.groups.set_collapsed(&id, true).unwrap();
    let batch = Batch::capture(&book, 0, Some(&id)).unwrap();
    let panel =
        crate::bridge::Panel::from_book(book, favnyr_core::columns::default_columns(), 2, 145.0);
    let saved = super::drag::snapshot(&panel, &batch).unwrap();
    assert_eq!(saved.view.tabs.len(), 2);
    assert_eq!(saved.view.active_tab, 1);
    assert_eq!(saved.view.tab_bar_mode, 2);
    assert_eq!(saved.view.tab_groups.get(&id).unwrap().color_slot, 3);
    assert!(saved.view.tab_groups.get(&id).unwrap().collapsed);
    assert_eq!(
        saved.dirs,
        panel.tabs.tabs[1..3]
            .iter()
            .map(|t| t.current_path.clone())
            .collect::<Vec<_>>()
    );
    let fragment = Batch::single(&panel.tabs, 1).unwrap();
    let saved = super::drag::snapshot(&panel, &fragment).unwrap();
    assert!(saved.view.tab_groups.is_empty());
    assert!(saved.view.tabs[0].tab_group_id.is_none());
    assert_eq!(panel.tabs.tabs.len(), 5);
}

#[test]
fn legacy_protocol_is_allowed_only_when_no_group_metadata_would_be_lost() {
    let mut book = sample(3, 0);
    assert!(
        !Batch::single(&book, 0)
            .unwrap()
            .requires_batch_protocol(&book)
    );
    let id = group(&mut book, &[0], "Group 01", 0);
    assert!(
        Batch::single(&book, 0)
            .unwrap()
            .requires_batch_protocol(&book)
    );
    book.groups
        .assign(&mut book.tabs, &mut book.active, &[1], &id)
        .unwrap();
    assert!(
        !Batch::single(&book, 0)
            .unwrap()
            .requires_batch_protocol(&book)
    );
    assert!(
        Batch::capture(&book, 0, Some(&id))
            .unwrap()
            .requires_batch_protocol(&book)
    );
}

#[test]
fn all_nonempty_subsets_transfer_once_in_order_with_surviving_focus() {
    for mask in 1u32..(1 << 5) {
        for active in 0..5 {
            let mut source = sample(5, active);
            let original = ids(&source);
            let selected: Vec<_> = original
                .iter()
                .enumerate()
                .filter_map(|(i, &id)| (mask & (1 << i) != 0).then_some(id))
                .collect();
            source.select_ids(&selected);
            let grabbed = original.iter().position(|id| *id == selected[0]).unwrap();
            let batch = Batch::capture(&source, grabbed, None).unwrap();
            let mut target = sample(2, 1);
            let previous = ids(&target);
            batch
                .transfer(&mut source, &mut target, &Destination::outside(1))
                .unwrap();
            let kept: Vec<_> = original
                .iter()
                .copied()
                .filter(|id| !selected.contains(id))
                .collect();
            assert_eq!(ids(&source), kept);
            let mut expected = vec![previous[0]];
            expected.extend(&selected);
            expected.push(previous[1]);
            assert_eq!(ids(&target), expected);
            assert_eq!(
                target.tabs[target.active].id,
                if selected.contains(&original[active]) {
                    original[active]
                } else {
                    selected[0]
                }
            );
            if !selected.contains(&original[active]) {
                assert_eq!(source.tabs[source.active].id, original[active]);
            } else if !kept.is_empty() {
                let neighbor = original[active + 1..]
                    .iter()
                    .chain(original[..active].iter().rev())
                    .find(|id| !selected.contains(id))
                    .unwrap();
                assert_eq!(&source.tabs[source.active].id, neighbor);
            }
        }
    }
}

#[test]
fn directory_insertion_uses_explicit_group_membership_and_rejects_a_missing_group() {
    let mut book = sample(3, 0);
    let id = group(&mut book, &[0, 1], "Group 01", 2);
    book.groups.set_collapsed(&id, true).unwrap();
    let tab = super::Tab::new(PathBuf::from("folder02"), false);
    let incoming = tab.id;
    super::batch::insert(
        &mut book,
        tab,
        &Destination {
            gap: usize::MAX,
            group: Some(id.clone()),
        },
    )
    .unwrap();
    assert_eq!(book.tabs[2].id, incoming);
    assert_eq!(book.tabs[2].tab_group_id.as_ref(), Some(&id));
    assert_eq!(book.active, 2);
    assert!(!book.groups.get(&id).unwrap().collapsed);
    let before = ids(&book);
    let tab = super::Tab::new(PathBuf::from("folder03"), false);
    assert!(
        super::batch::insert(
            &mut book,
            tab,
            &Destination {
                gap: 0,
                group: Some("missing".into())
            }
        )
        .is_err()
    );
    assert_eq!(ids(&book), before);
    assert_eq!(book.active, 2);
}

#[test]
fn mixed_group_and_tab_selection_reorders_as_blocks_then_joins_one_target() {
    let mut book = sample(10, 9);
    let first = group(&mut book, &[1, 2], "Group 01", 1);
    let second = group(&mut book, &[5, 6], "Group 02", 2);
    let target = group(&mut book, &[7, 8], "Group 03", 3);
    book.groups.set_collapsed(&first, true).unwrap();
    book.groups.set_collapsed(&target, true).unwrap();
    book.reset_selection();
    book.click_group(&first, true, false);
    book.click_group(&second, true, false);
    book.click(0, true, false);
    let original = ids(&book);
    let batch = Batch::capture(&book, usize::MAX, Some(&first)).unwrap();
    assert_eq!(
        batch.ids,
        [
            original[0],
            original[1],
            original[2],
            original[5],
            original[6],
            original[9]
        ]
    );
    assert_eq!(Batch::capture(&book, 0, None).unwrap().ids, batch.ids);
    assert_eq!(batch.active, original[9]);
    assert!(batch.reorder(&mut book, &Destination::outside(4)).unwrap());
    assert_eq!(
        ids(&book),
        [
            original[3],
            original[0],
            original[1],
            original[2],
            original[5],
            original[6],
            original[9],
            original[4],
            original[7],
            original[8]
        ]
    );
    assert!(book.group_selected(&first) && book.group_selected(&second));
    assert!(book.groups.get(&first).unwrap().collapsed);
    assert_eq!(book.selected_ids(), batch.ids);
    assert_eq!(book.tabs[book.active].id, original[9]);

    let batch = Batch::capture(&book, usize::MAX, Some(&second)).unwrap();
    assert!(
        !batch
            .reorder(
                &mut book,
                &Destination {
                    gap: 0,
                    group: Some(first.clone())
                }
            )
            .unwrap()
    );
    assert!(
        batch
            .reorder(
                &mut book,
                &Destination {
                    gap: usize::MAX,
                    group: Some(target.clone())
                }
            )
            .unwrap()
    );
    assert_eq!(book.groups.entries().len(), 1);
    assert!(book.groups.get(&first).is_none() && book.groups.get(&second).is_none());
    assert_eq!(book.groups.get(&target).unwrap().color_slot, 3);
    assert!(!book.groups.get(&target).unwrap().collapsed);
    assert!(book.group_highlighted(&target));
    assert_eq!(book.selected_ids(), batch.ids);
    assert!(
        book.tabs
            .iter()
            .filter(|tab| batch.ids.contains(&tab.id))
            .all(|tab| tab.tab_group_id.as_deref() == Some(&target))
    );
    assert_eq!(
        ids(&book)
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        original.len()
    );
}

#[test]
fn mixed_collapsed_groups_transfer_with_metadata_and_remain_draggable() {
    for join in [false, true] {
        let mut source = sample(8, 7);
        let first = group(&mut source, &[0, 1], "Group 01", 1);
        let second = group(&mut source, &[3, 4], "Group 02", 2);
        source.groups.set_collapsed(&first, true).unwrap();
        source.groups.set_collapsed(&second, true).unwrap();
        source.reset_selection();
        source.click_group(&first, true, false);
        source.click_group(&second, true, false);
        let batch = Batch::capture(&source, usize::MAX, Some(&second)).unwrap();
        let original = ids(&source);
        let mut target = sample(3, 2);
        let existing = group(&mut target, &[0, 1], "Group 01", 5);
        target.groups.set_collapsed(&existing, true).unwrap();
        batch
            .transfer(
                &mut source,
                &mut target,
                &Destination {
                    gap: usize::MAX,
                    group: join.then(|| existing.clone()),
                },
            )
            .unwrap();
        assert_eq!(ids(&source), [original[2], original[5], original[6]]);
        assert!(source.groups.is_empty());
        assert_eq!(target.tabs[target.active].id, original[7]);
        assert_eq!(target.selected_ids(), batch.ids);
        if join {
            assert_eq!(target.groups.entries().len(), 1);
            assert!(!target.groups.get(&existing).unwrap().collapsed);
            assert!(
                target
                    .tabs
                    .iter()
                    .filter(|tab| batch.ids.contains(&tab.id))
                    .all(|tab| tab.tab_group_id.as_deref() == Some(&existing))
            );
        } else {
            assert_eq!(target.groups.entries().len(), 3);
            for (old, slot) in [(&first, 1), (&second, 2)] {
                let imported = target
                    .tabs
                    .iter()
                    .find(|tab| {
                        tab.id == batch.groups.iter().find(|(id, _)| id == old).unwrap().1[0]
                    })
                    .unwrap()
                    .tab_group_id
                    .clone()
                    .unwrap();
                assert_ne!(&imported, old);
                assert!(target.group_selected(&imported));
                let metadata = target.groups.get(&imported).unwrap();
                assert!(metadata.collapsed);
                assert_eq!(metadata.color_slot, slot);
                assert_eq!(
                    Batch::capture(&target, usize::MAX, Some(&imported))
                        .unwrap()
                        .ids,
                    batch.ids
                );
            }
        }
    }
}

#[test]
fn changing_any_selected_group_rejects_the_whole_frozen_batch_before_mutation() {
    let mut source = sample(8, 7);
    let first = group(&mut source, &[0, 1], "Group 01", 1);
    let second = group(&mut source, &[3, 4], "Group 02", 2);
    source.reset_selection();
    source.click_group(&first, true, false);
    source.click_group(&second, true, false);
    let batch = Batch::capture(&source, usize::MAX, Some(&first)).unwrap();
    source
        .groups
        .assign(&mut source.tabs, &mut source.active, &[5], &second)
        .unwrap();
    let mut target = sample(2, 0);
    let before = (
        ids(&source),
        source.groups.clone(),
        ids(&target),
        target.groups.clone(),
    );
    assert!(
        batch
            .transfer(&mut source, &mut target, &Destination::outside(0))
            .is_err()
    );
    assert_eq!(
        (
            ids(&source),
            source.groups.clone(),
            ids(&target),
            target.groups.clone()
        ),
        before
    );
}

#[test]
fn every_small_mixed_selection_preserves_identity_order_and_group_boundaries() {
    for collapsed in 0..4 {
        for mask in 1..16 {
            for active in 0..7 {
                for gap in 0..=7 {
                    let mut book = sample(7, active);
                    let first = group(&mut book, &[0, 1], "Group 01", 1);
                    let second = group(&mut book, &[3, 4], "Group 02", 2);
                    book.groups
                        .set_collapsed(&first, collapsed & 1 != 0)
                        .unwrap();
                    book.groups
                        .set_collapsed(&second, collapsed & 2 != 0)
                        .unwrap();
                    book.select_ids(&[]);
                    for (bit, id) in [(1, &first), (2, &second)] {
                        if mask & bit != 0 {
                            book.click_group(id, true, false);
                        }
                    }
                    for (bit, index) in [(4, 2), (8, 5)] {
                        if mask & bit != 0 {
                            book.click(index, true, false);
                        }
                    }
                    let original = ids(&book);
                    let metadata = book.groups.clone();
                    let selected = book.selected_ids();
                    let first_index = book
                        .tabs
                        .iter()
                        .position(|tab| tab.id == selected[0])
                        .unwrap();
                    let header = book.tabs[first_index].tab_group_id.clone();
                    let batch = Batch::capture(&book, first_index, header.as_deref()).unwrap();
                    assert_eq!(batch.ids, selected);
                    batch
                        .reorder(&mut book, &Destination::outside(gap))
                        .unwrap();
                    assert_eq!(book.tabs[book.active].id, original[active]);
                    assert_eq!(book.selected_ids(), selected);
                    let identities: std::collections::HashSet<_> = ids(&book).into_iter().collect();
                    assert_eq!(identities.len(), original.len());
                    assert!(original.iter().all(|id| identities.contains(id)));
                    let mut normalized = book.groups.clone();
                    normalized.normalize(&mut book.tabs);
                    assert_eq!(normalized, book.groups);
                    for previous in metadata.entries() {
                        assert_eq!(book.groups.get(&previous.id), Some(previous));
                    }
                    for (id, range) in [(&first, 0..2), (&second, 3..5)] {
                        let members: Vec<_> = book
                            .tabs
                            .iter()
                            .filter(|tab| tab.tab_group_id.as_deref() == Some(id))
                            .map(|tab| tab.id)
                            .collect();
                        assert_eq!(members, original[range]);
                    }
                }
            }
        }
    }
}
