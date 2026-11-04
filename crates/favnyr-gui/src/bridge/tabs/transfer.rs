//! Validated tab movements between views, without filesystem or window access.

use super::super::{LayoutNode, MAX_PANELS, Panel, SplitDir};
use super::batch::{Batch, Destination};

pub(super) fn group_names_overlap(
    incoming: &favnyr_core::tab_groups::TabGroups,
    target: &favnyr_core::tab_groups::TabGroups,
) -> bool {
    incoming.entries().iter().any(|a| {
        target
            .entries()
            .iter()
            .any(|b| a.name.to_lowercase() == b.name.to_lowercase())
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(in crate::bridge) enum Removal {
    Remaining,
    LastView,
}

/// The last book stays valid until the caller closes its window.
pub(in crate::bridge) fn remove_batch(
    panels: &mut Vec<Panel>,
    layout: &mut LayoutNode,
    active: &mut usize,
    batch: &Batch,
) -> Option<Removal> {
    let source = panels.iter().position(|p| p.tabs.identity == batch.owner)?;
    batch.indices(&panels[source].tabs).ok()?;
    if !layout.is_valid_for(panels.len()) {
        return None;
    }
    let empty = batch.ids.len() == panels[source].tabs.tabs.len();
    if empty && panels.len() == 1 {
        return Some(Removal::LastView);
    }
    let mut next = layout.clone();
    if empty && !next.remove_view(source) {
        return None;
    }
    batch.take_validated(&mut panels[source].tabs);
    if empty {
        panels.remove(source);
    }
    if empty && source < *active {
        *active -= 1;
    }
    *active = (*active).min(panels.len() - 1);
    *layout = next;
    Some(Removal::Remaining)
}

/// Validate the destination before exposing any incoming tab to the live views.
pub(in crate::bridge) fn receive(
    panels: &mut Vec<Panel>,
    layout: &mut LayoutNode,
    mut incoming: Panel,
    target: usize,
    zone: i32,
    destination: Option<&Destination>,
) -> Option<usize> {
    if target >= panels.len() || !layout.is_valid_for(panels.len()) {
        return None;
    }
    if destination.is_some() || zone == 1 {
        let book = &mut incoming.tabs;
        // Capture all members, including those hidden by collapsed group headers.
        let batch = Batch::all(book)?;
        batch
            .transfer(
                book,
                &mut panels[target].tabs,
                destination.unwrap_or(&Destination::outside(usize::MAX)),
            )
            .ok()?;
        Some(target)
    } else if (2..=5).contains(&zone) && panels.len() < MAX_PANELS {
        let index = panels.len();
        let mut next = layout.clone();
        if !next.split_leaf(
            target,
            if zone >= 4 {
                SplitDir::Column
            } else {
                SplitDir::Row
            },
            index,
            0.5,
            zone == 2 || zone == 4,
        ) {
            return None;
        }
        panels.push(incoming);
        *layout = next;
        Some(index)
    } else {
        None
    }
}

pub(in crate::bridge) fn move_batch(
    panels: &mut Vec<Panel>,
    layout: &mut LayoutNode,
    batch: &Batch,
    target: usize,
    destination: &Destination,
) -> Option<usize> {
    let source = panels.iter().position(|p| p.tabs.identity == batch.owner)?;
    if target >= panels.len() || !layout.is_valid_for(panels.len()) {
        return None;
    }
    batch.indices(&panels[source].tabs).ok()?;
    if source == target {
        batch.reorder(&mut panels[source].tabs, destination).ok()?;
        return Some(source);
    }
    let empty = batch.ids.len() == panels[source].tabs.tabs.len();
    let mut next_layout = layout.clone();
    if empty && !next_layout.remove_view(source) {
        return None;
    }
    if source < target {
        let (a, b) = panels.split_at_mut(target);
        batch
            .transfer(&mut a[source].tabs, &mut b[0].tabs, destination)
            .ok()?;
    } else {
        let (a, b) = panels.split_at_mut(source);
        batch
            .transfer(&mut b[0].tabs, &mut a[target].tabs, destination)
            .ok()?;
    }
    if empty {
        panels.remove(source);
    }
    *layout = next_layout;
    Some(target - usize::from(empty && source < target))
}

pub(in crate::bridge) fn split_batch(
    panels: &mut Vec<Panel>,
    layout: &mut LayoutNode,
    batch: &Batch,
    target: usize,
    dir: SplitDir,
    new_first: bool,
) -> Option<usize> {
    let source = panels.iter().position(|p| p.tabs.identity == batch.owner)?;
    if target >= panels.len() || !layout.is_valid_for(panels.len()) {
        return None;
    }
    batch.indices(&panels[source].tabs).ok()?;
    if batch.ids.len() == panels[source].tabs.tabs.len() {
        if source == target {
            return None;
        }
        // Moving an entire view retains its models, tasks and local group identities.
        layout.move_view_beside(source, target, dir, new_first);
        return Some(source);
    }
    if panels.len() >= MAX_PANELS {
        return None;
    }
    let mut next_layout = layout.clone();
    let index = panels.len();
    if !next_layout.split_leaf(target, dir, index, 0.5, new_first) {
        return None;
    }
    let mut book = super::TabBook {
        identity: Default::default(),
        selection: Default::default(),
        focused_group: None,
        active_group: None,
        groups: Default::default(),
        tabs: Vec::new(),
        active: 0,
    };
    batch
        .transfer(
            &mut panels[source].tabs,
            &mut book,
            &Destination::outside(0),
        )
        .ok()?;
    // New-view initialization resets selection; a moved batch keeps its marks.
    let selection = std::mem::take(&mut book.selection);
    let source = &panels[source];
    let mut panel = Panel::from_book(
        book,
        source.columns.clone(),
        source.tab_bar_mode,
        source.vbar_user_w,
    );
    panel.tabs.selection = selection;
    panel.tabs.reconcile_selection();
    panel.tab_bar_restore_mode = source.tab_bar_restore_mode;
    panels.push(panel);
    *layout = next_layout;
    Some(index)
}

#[cfg(test)]
pub(super) fn move_between(
    panels: &mut Vec<Panel>,
    layout: &mut LayoutNode,
    src: usize,
    from: usize,
    target: usize,
    gap: usize,
) -> Option<usize> {
    if src == target {
        return None;
    }
    let batch = Batch::single(&panels.get(src)?.tabs, from)?;
    move_batch(panels, layout, &batch, target, &Destination::outside(gap))
}

#[cfg(test)]
pub(super) fn split_tab(
    panels: &mut Vec<Panel>,
    layout: &mut LayoutNode,
    src: usize,
    from: usize,
    target: usize,
    dir: SplitDir,
    first: bool,
) -> Option<usize> {
    let batch = Batch::single(&panels.get(src)?.tabs, from)?;
    split_batch(panels, layout, &batch, target, dir, first)
}

#[cfg(test)]
mod tests {
    use super::super::{Tab, TabBook, TabId};
    use super::*;
    use crate::bridge::panel_to_state;
    use std::path::PathBuf;

    fn panel(count: usize, active: usize, mode: u8) -> Panel {
        Panel::from_book(
            TabBook {
                identity: Default::default(),
                selection: Default::default(),
                focused_group: None,
                active_group: None,
                groups: Default::default(),
                tabs: (0..count)
                    .map(|i| Tab::new(PathBuf::from(format!("folder{:02}", i + 1)), false))
                    .collect(),
                active,
            },
            favnyr_core::columns::default_columns(),
            mode,
            137.0,
        )
    }

    fn ids(panels: &[Panel]) -> Vec<Vec<TabId>> {
        panels
            .iter()
            .map(|panel| panel.tabs.tabs.iter().map(|tab| tab.id).collect())
            .collect()
    }

    #[test]
    fn acknowledged_removal_prunes_views_but_keeps_the_last_book_valid() {
        for source in 0..3 {
            let mut panels = vec![panel(1, 0, 0), panel(1, 0, 0), panel(1, 0, 0)];
            let mut layout = LayoutNode::row_chain(&[1.0; 3]);
            let mut active = 2;
            let batch = Batch::single(&panels[source].tabs, 0).unwrap();
            assert_eq!(
                remove_batch(&mut panels, &mut layout, &mut active, &batch),
                Some(Removal::Remaining)
            );
            assert_eq!(panels.len(), 2);
            assert_eq!(active, 1);
            assert!(layout.is_valid_for(2));
            assert!(remove_batch(&mut panels, &mut layout, &mut active, &batch).is_none());
        }
        let mut panels = vec![panel(1, 0, 0)];
        let mut layout = LayoutNode::Leaf { panel: 0 };
        let batch = Batch::single(&panels[0].tabs, 0).unwrap();
        let before = ids(&panels);
        assert_eq!(
            remove_batch(&mut panels, &mut layout, &mut 0, &batch),
            Some(Removal::LastView)
        );
        assert_eq!(ids(&panels), before);
    }

    #[test]
    fn received_batches_keep_all_members_and_create_exactly_one_split() {
        for zone in 1..=5 {
            let mut panels = vec![panel(1, 0, 0)];
            let mut layout = LayoutNode::Leaf { panel: 0 };
            let mut incoming = panel(2, 1, 2);
            let book = &mut incoming.tabs;
            let group = book
                .groups
                .create(&mut book.tabs, &mut book.active, &[0, 1], "Group 01", 2)
                .unwrap();
            book.groups.set_collapsed(&group, true).unwrap();
            let expected = book.tabs.iter().map(|t| t.id).collect::<Vec<_>>();
            let target = receive(&mut panels, &mut layout, incoming, 0, zone, None).unwrap();
            assert_eq!(panels.len(), if zone == 1 { 1 } else { 2 });
            assert_eq!(&ids(&panels)[target][usize::from(zone == 1)..], &expected);
            assert_eq!(
                panels[target].tabs.tabs[panels[target].tabs.active].id,
                expected[1]
            );
            assert!(panels[target].tabs.groups.entries()[0].collapsed);
            assert_eq!(panels[target].tab_bar_mode, if zone == 1 { 0 } else { 2 });
            assert!(layout.is_valid_for(panels.len()));
        }
    }

    #[test]
    fn refused_receives_are_atomic_and_explicit_groups_are_reused() {
        let mut panels = vec![panel(2, 0, 0)];
        let mut layout = LayoutNode::Leaf { panel: 0 };
        let before = ids(&panels);
        let destination = Destination {
            gap: 0,
            group: Some("missing".into()),
        };
        assert!(
            receive(
                &mut panels,
                &mut layout,
                panel(2, 1, 2),
                0,
                1,
                Some(&destination)
            )
            .is_none()
        );
        assert!(receive(&mut panels, &mut layout, panel(2, 1, 2), 0, 0, None).is_none());
        assert!(receive(&mut panels, &mut layout, panel(2, 1, 2), 5, 1, None).is_none());
        assert_eq!(ids(&panels), before);
        assert_eq!(layout, LayoutNode::Leaf { panel: 0 });
        let book = &mut panels[0].tabs;
        let group = book
            .groups
            .create(&mut book.tabs, &mut book.active, &[0, 1], "Group 01", 2)
            .unwrap();
        let destination = Destination {
            gap: 2,
            group: Some(group.clone()),
        };
        assert_eq!(
            receive(
                &mut panels,
                &mut layout,
                panel(2, 1, 2),
                0,
                1,
                Some(&destination)
            ),
            Some(0)
        );
        assert_eq!(panels[0].tabs.groups.entries().len(), 1);
        assert!(
            panels[0]
                .tabs
                .tabs
                .iter()
                .all(|t| t.tab_group_id.as_deref() == Some(&group))
        );
        let mut panels: Vec<_> = (0..MAX_PANELS).map(|_| panel(1, 0, 0)).collect();
        let mut layout = LayoutNode::row_chain(&[1.0; MAX_PANELS]);
        assert!(receive(&mut panels, &mut layout, panel(2, 1, 2), 0, 2, None).is_none());
        assert_eq!(panels.len(), MAX_PANELS);
        assert!(layout.is_valid_for(MAX_PANELS));
    }

    #[test]
    fn moving_an_earlier_tab_does_not_change_the_source_active_folder() {
        let mut panels = vec![panel(3, 1, 0), panel(2, 0, 2)];
        let mut layout = LayoutNode::row_chain(&[1.0, 1.0]);
        let before = ids(&panels);
        assert_eq!(move_between(&mut panels, &mut layout, 0, 0, 1, 1), Some(1));
        assert_eq!(ids(&panels)[0], before[0][1..]);
        assert_eq!(panels[0].tabs.active, 0);
        assert_eq!(panels[0].tabs.tabs[0].id, before[0][1]);
        assert_eq!(panels[1].tabs.active, 1);
        assert_eq!(panels[1].tabs.tabs[1].id, before[0][0]);
        assert!(layout.is_valid_for(panels.len()));
    }

    #[test]
    fn an_emptied_source_is_pruned_on_either_side_of_the_target() {
        for source in [0, 1] {
            let target = 1 - source;
            let mut panels = vec![panel(2, 0, 0), panel(2, 0, 2)];
            panels[source] = panel(1, 0, 1);
            let transferred = panels[source].tabs.tabs[0].id;
            let mut layout = LayoutNode::row_chain(&[1.0, 1.0]);
            assert_eq!(
                move_between(&mut panels, &mut layout, source, 0, target, usize::MAX),
                Some(0)
            );
            assert_eq!(panels.len(), 1);
            assert_eq!(panels[0].tabs.tabs.len(), 3);
            assert_eq!(panels[0].tabs.tabs[panels[0].tabs.active].id, transferred);
            assert!(layout.is_valid_for(1));
        }
    }

    #[test]
    fn split_preserves_source_focus_and_inherits_presentation_on_each_axis() {
        for mode in [0, 1, 2, 3] {
            for (direction, first) in [
                (SplitDir::Row, true),
                (SplitDir::Row, false),
                (SplitDir::Column, true),
                (SplitDir::Column, false),
            ] {
                for target in [0, 1] {
                    let mut panels = vec![panel(3, 1, mode), panel(1, 0, 0)];
                    panels[0].tab_bar_restore_mode = 2;
                    let before = ids(&panels);
                    let saved = panel_to_state(&panels[0]);
                    let mut layout = LayoutNode::row_chain(&[1.0, 1.0]);
                    let moved = split_tab(&mut panels, &mut layout, 0, 0, target, direction, first)
                        .unwrap();
                    assert_eq!(moved, 2);
                    assert_eq!(panels[0].tabs.tabs[panels[0].tabs.active].id, before[0][1]);
                    assert_eq!(panels[moved].tabs.tabs[0].id, before[0][0]);
                    assert_eq!(panel_to_state(&panels[moved]).tabs, saved.tabs[..1]);
                    assert_eq!(panels[moved].columns, saved.columns);
                    assert_eq!(panels[moved].tab_bar_mode, mode);
                    assert_eq!(panels[moved].tab_bar_restore_mode, 2);
                    assert_eq!(panels[moved].vbar_user_w, 137.0);
                    assert!(layout.is_valid_for(3));
                }
            }
        }
    }

    #[test]
    fn rejected_splits_and_moves_leave_content_focus_and_layout_untouched() {
        for malformed in [false, true] {
            let mut panels = vec![panel(3, 2, 1), panel(1, 0, 2)];
            let mut layout = if malformed {
                LayoutNode::Leaf { panel: 0 }
            } else {
                LayoutNode::row_chain(&[1.0, 1.0])
            };
            let before = ids(&panels);
            let saved: Vec<_> = panels.iter().map(panel_to_state).collect();
            let saved_layout = layout.clone();
            for (source, from, target) in [(9, 0, 1), (0, 9, 1), (0, 0, 9)] {
                assert!(
                    split_tab(
                        &mut panels,
                        &mut layout,
                        source,
                        from,
                        target,
                        SplitDir::Row,
                        true
                    )
                    .is_none()
                );
                assert!(move_between(&mut panels, &mut layout, source, from, target, 0).is_none());
            }
            if malformed {
                assert!(
                    split_tab(&mut panels, &mut layout, 0, 0, 1, SplitDir::Row, true).is_none()
                );
                assert!(move_between(&mut panels, &mut layout, 0, 0, 1, 0).is_none());
            }
            assert_eq!(ids(&panels), before);
            assert_eq!(panels.iter().map(panel_to_state).collect::<Vec<_>>(), saved);
            assert_eq!(layout, saved_layout);
        }
        let mut sole = vec![panel(1, 0, 0)];
        let mut layout = LayoutNode::Leaf { panel: 0 };
        let before = ids(&sole);
        assert!(split_tab(&mut sole, &mut layout, 0, 0, 0, SplitDir::Row, true).is_none());
        assert_eq!(ids(&sole), before);
    }

    #[test]
    fn splitting_at_the_panel_limit_requires_an_emptied_source() {
        let mut panels: Vec<_> = (0..MAX_PANELS).map(|_| panel(1, 0, 0)).collect();
        panels[0] = panel(2, 1, 2);
        let mut layout = LayoutNode::row_chain(&[1.0; MAX_PANELS]);
        let before = ids(&panels);
        assert!(split_tab(&mut panels, &mut layout, 0, 0, 1, SplitDir::Row, true).is_none());
        assert_eq!(ids(&panels), before);
        let transferred = panels[1].tabs.tabs[0].id;
        let target = split_tab(&mut panels, &mut layout, 1, 0, 0, SplitDir::Column, false).unwrap();
        assert_eq!(panels.len(), MAX_PANELS);
        assert_eq!(panels[target].tabs.tabs[0].id, transferred);
        assert!(layout.is_valid_for(MAX_PANELS));
    }

    #[test]
    fn a_selected_batch_creates_one_split_with_its_active_member_and_groups() {
        for (dir, first) in [
            (SplitDir::Row, true),
            (SplitDir::Row, false),
            (SplitDir::Column, true),
            (SplitDir::Column, false),
        ] {
            let mut panels = vec![panel(4, 2, 2), panel(2, 0, 0)];
            let source = &mut panels[0].tabs;
            let group = source
                .groups
                .create(&mut source.tabs, &mut source.active, &[1, 2], "Group 01", 2)
                .unwrap();
            source.groups.set_collapsed(&group, true).unwrap();
            let batch = Batch::capture(source, 0, Some(&group)).unwrap();
            let mut layout = LayoutNode::row_chain(&[1.0, 1.0]);
            let index = split_batch(&mut panels, &mut layout, &batch, 1, dir, first).unwrap();
            assert_eq!(panels.len(), 3);
            assert_eq!(ids(&panels)[index], batch.ids);
            assert_eq!(
                panels[index].tabs.tabs[panels[index].tabs.active].id,
                batch.active
            );
            assert_eq!(panels[index].tab_bar_mode, 2);
            assert!(panels[index].tabs.groups.entries()[0].collapsed);
            assert!(layout.is_valid_for(3));
            assert_eq!(panels[0].tabs.tabs.len(), 2);
            assert!(panels[0].tabs.groups.is_empty());
        }
    }

    #[test]
    fn moving_a_whole_book_at_capacity_keeps_its_identity_models_and_group_ids() {
        let mut panels: Vec<_> = (0..MAX_PANELS).map(|_| panel(3, 1, 0)).collect();
        let source = &mut panels[0].tabs;
        let group = source
            .groups
            .create(
                &mut source.tabs,
                &mut source.active,
                &[0, 1, 2],
                "Group 01",
                2,
            )
            .unwrap();
        let batch = Batch::capture(source, 0, Some(&group)).unwrap();
        let model = panels[0].tab_presentation.model.clone();
        let saved: Vec<_> = panels.iter().map(panel_to_state).collect();
        let mut layout = LayoutNode::row_chain(&[1.0; MAX_PANELS]);
        assert_eq!(
            split_batch(&mut panels, &mut layout, &batch, 1, SplitDir::Column, false),
            Some(0)
        );
        assert_eq!(panels.len(), MAX_PANELS);
        assert_eq!(panels[0].tabs.identity, batch.owner);
        assert!(std::rc::Rc::ptr_eq(
            &model,
            &panels[0].tab_presentation.model
        ));
        assert_eq!(panels.iter().map(panel_to_state).collect::<Vec<_>>(), saved);
        assert!(layout.is_valid_for(MAX_PANELS));
    }
}
