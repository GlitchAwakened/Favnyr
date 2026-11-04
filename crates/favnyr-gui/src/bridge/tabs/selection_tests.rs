use super::{TabBook, projection::Presentation, tests::sample};
use slint::Model;

fn group(book: &mut TabBook, indices: &[usize], name: &str, collapsed: bool) -> String {
    let id = book
        .groups
        .create(&mut book.tabs, &mut book.active, indices, name, 2)
        .unwrap();
    book.groups.set_collapsed(&id, collapsed).unwrap();
    id
}

fn selected(book: &TabBook) -> Vec<usize> {
    book.tabs
        .iter()
        .enumerate()
        .filter_map(|(i, tab)| book.selected(tab.id).then_some(i))
        .collect()
}

#[test]
fn control_header_clicks_select_whole_groups_without_navigation_or_folding() {
    let mut book = sample(8, 7);
    let first = group(&mut book, &[0, 1, 2], "Group 01", true);
    let second = group(&mut book, &[4, 5], "Group 02", false);
    book.reset_selection();
    let before = (
        book.active,
        book.groups.clone(),
        book.active_group.clone(),
        book.focused_group.clone(),
    );
    book.click_group(&first, true, false);
    book.click_group(&second, true, false);
    book.click(3, true, false);
    assert_eq!(selected(&book), [0, 1, 2, 3, 4, 5, 7]);
    assert!(book.group_selected(&first) && book.group_selected(&second));
    book.reconcile_selection();
    assert_eq!(selected(&book), [0, 1, 2, 3, 4, 5, 7]);
    book.click_group(&first, true, false);
    assert_eq!(selected(&book), [3, 4, 5, 7]);
    assert!(!book.group_selected(&first));
    book.click_group("missing", true, true);
    assert_eq!(selected(&book), [3, 4, 5, 7]);
    assert_eq!(
        (
            book.active,
            book.groups.clone(),
            book.active_group.clone(),
            book.focused_group.clone()
        ),
        before
    );
}

#[test]
fn shift_ranges_include_headers_once_and_reuse_their_original_anchor() {
    let mut book = sample(9, 0);
    let first = group(&mut book, &[1, 2], "Group 01", true);
    let second = group(&mut book, &[4, 5, 6], "Group 02", false);
    book.reset_selection();
    book.click(8, false, true);
    assert_eq!(selected(&book), (0..9).collect::<Vec<_>>());
    assert!(book.group_selected(&first) && book.group_selected(&second));
    book.click_group(&first, false, true);
    assert_eq!(selected(&book), [0, 1, 2]);
    assert!(!book.group_selected(&second));
    book.click_group(&second, true, false);
    book.click(7, true, true);
    assert_eq!(selected(&book), [0, 1, 2, 4, 5, 6, 7]);
    book.click(8, false, true);
    assert_eq!(selected(&book), [4, 5, 6, 7, 8]);
    assert!(!book.group_selected(&first));
    assert_eq!(book.active, 0);
    assert!(book.groups.get(&first).unwrap().collapsed);
}

#[test]
fn excluding_a_visible_child_releases_the_group_as_a_unit_but_keeps_other_members() {
    let mut book = sample(5, 4);
    let id = group(&mut book, &[0, 1, 2], "Group 01", false);
    book.reset_selection();
    book.click_group(&id, true, false);
    book.click(1, true, false);
    assert!(!book.group_selected(&id));
    assert_eq!(selected(&book), [0, 2, 4]);
    book.reconcile_selection();
    assert_eq!(selected(&book), [0, 2, 4]);
    book.click_group(&id, true, false);
    assert_eq!(selected(&book), [0, 1, 2, 4]);
    assert!(book.group_selected(&id));
    assert_eq!(book.select_context(1), 4);
    assert!(book.group_selected(&id));
    assert_eq!(book.select_context(3), 1);
    assert!(!book.group_selected(&id));
    assert_eq!(selected(&book), [3]);
}

#[test]
fn group_page_is_a_range_anchor_and_selection_reset_keeps_its_focus() {
    let mut book = sample(6, 5);
    let id = group(&mut book, &[1, 2], "Group 01", true);
    book.focus_group(&id);
    book.click(4, false, true);
    assert_eq!(selected(&book), [1, 2, 3, 4]);
    assert!(book.group_selected(&id));
    assert!(book.reset_selection());
    assert!(!book.reset_selection());
    assert!(selected(&book).is_empty());
    assert_eq!(book.group_page().unwrap().id, id);
    assert!(book.group_highlighted(&id));
    book.click_group(&id, true, false);
    book.groups.dissolve(&mut book.tabs, &id).unwrap();
    book.reconcile_selection();
    assert!(!book.group_selected(&id));
    assert!(book.group_page().is_none());
    assert_eq!(selected(&book), [1, 2]);
}

#[test]
fn group_selection_marks_the_header_without_rebuilding_any_bar_geometry() {
    for mode in 0..=2 {
        let mut book = sample(6, 5);
        let first = group(&mut book, &[0, 1], "Group 01", true);
        let second = group(&mut book, &[2, 3], "Group 02", false);
        book.reset_selection();
        let projection = Presentation::default();
        projection.update(&book, mode, &Default::default());
        let before = projection.state(&book);
        book.click_group(&first, true, false);
        book.click_group(&second, true, false);
        projection.update(&book, mode, &Default::default());
        assert_eq!(projection.state(&book), before);
        for row in projection.model.iter().filter(|row| row.is_group) {
            assert!(row.selection_marked);
            assert!(!row.group_highlighted);
        }
        book.reset_selection();
        projection.update(&book, mode, &Default::default());
        assert_eq!(projection.state(&book), before);
        assert!(projection.model.iter().all(|row| !row.selection_marked));
    }
}

#[test]
fn focused_header_control_selection_keeps_the_group_and_either_neighbor() {
    use super::batch::{Batch, Destination};
    for mode in 0..=2 {
        for collapsed in [false, true] {
            for neighbor in [0, 3] {
                for grab_header in [false, true] {
                    let mut book = sample(6, neighbor);
                    let id = group(&mut book, &[1, 2], "Group 01", collapsed);
                    let original: Vec<_> = book.tabs.iter().map(|tab| tab.id).collect();
                    book.focus_group(&id);
                    assert!(book.selected_ids().is_empty());
                    let projection = Presentation::default();
                    projection.update(&book, mode, &Default::default());
                    let before = projection.state(&book);
                    book.click(neighbor, true, false);
                    let expected = if neighbor == 0 {
                        vec![0, 1, 2]
                    } else {
                        vec![1, 2, 3]
                    };
                    assert_eq!(
                        selected(&book),
                        expected,
                        "mode {mode}, collapsed {collapsed}, neighbor {neighbor}"
                    );
                    assert!(book.group_selected(&id));
                    assert_eq!(book.active, neighbor);
                    assert_eq!(book.group_page().unwrap().id, id);
                    assert_eq!(book.groups.get(&id).unwrap().collapsed, collapsed);
                    projection.update(&book, mode, &Default::default());
                    assert_eq!(projection.state(&book), before);
                    assert!(
                        projection
                            .model
                            .iter()
                            .find(|row| row.index == neighbor as i32)
                            .unwrap()
                            .selection_marked
                    );
                    assert!(
                        projection
                            .model
                            .iter()
                            .find(|row| row.is_group)
                            .unwrap()
                            .selection_marked
                    );
                    let batch = Batch::capture(&book, neighbor, grab_header.then_some(id.as_str()))
                        .unwrap();
                    let members: Vec<_> = expected.iter().map(|&index| original[index]).collect();
                    assert_eq!(batch.ids, members);
                    assert!(batch.contains_group(&book, &id));
                    assert!(batch.reorder(&mut book, &Destination::outside(6)).unwrap());
                    let mut reordered: Vec<_> = original
                        .iter()
                        .copied()
                        .filter(|tab| !members.contains(tab))
                        .collect();
                    reordered.extend(members);
                    assert_eq!(
                        book.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(),
                        reordered
                    );
                    assert!(book.group_selected(&id));
                    assert_eq!(book.group_page().unwrap().id, id);
                    assert_eq!(book.groups.get(&id).unwrap().collapsed, collapsed);
                }
            }
        }
    }
}

#[test]
fn focused_header_keeps_a_sole_selected_retained_tab_visible() {
    for mode in 0..=2 {
        let mut book = sample(5, 3);
        let id = group(&mut book, &[1, 2], "Group 01", false);
        book.focus_group(&id);
        // An explicit selection can contain only the retained former active tab.
        book.select_ids(&[book.tabs[3].id]);
        let projection = Presentation::default();
        projection.update(&book, mode, &Default::default());
        assert_eq!(projection.state(&book).active, -1);
        assert!(
            projection
                .model
                .iter()
                .find(|row| row.index == 3)
                .unwrap()
                .selection_marked
        );
    }
}

#[test]
fn focused_header_control_selection_respects_explicit_exclusions_and_reset() {
    let mut book = sample(6, 3);
    let first = group(&mut book, &[1, 2], "Group 01", false);
    let second = group(&mut book, &[4, 5], "Group 02", true);
    book.focus_group(&first);
    book.click_group(&second, true, false);
    assert_eq!(selected(&book), [1, 2, 4, 5]);
    assert!(book.group_selected(&first) && book.group_selected(&second));
    book.click_group(&second, true, false);
    book.click_group(&first, true, false);
    assert!(selected(&book).is_empty());
    book.click(3, true, false);
    assert_eq!(selected(&book), [3]);
    assert!(!book.group_selected(&first));
    book.click(3, true, false);
    assert!(selected(&book).is_empty());
    book.reset_selection();
    book.click(3, true, false);
    assert_eq!(selected(&book), [1, 2, 3]);
    book.click(1, true, false);
    assert_eq!(selected(&book), [2, 3]);
    assert!(!book.group_selected(&first));
    book.reconcile_selection();
    assert_eq!(selected(&book), [2, 3]);
    assert_eq!(book.group_page().unwrap().id, first);
    assert!(book.groups.get(&second).unwrap().collapsed);
}

#[test]
fn focused_member_control_selection_does_not_implicitly_select_its_whole_group() {
    let mut book = sample(5, 3);
    let id = group(&mut book, &[1, 2], "Group 01", false);
    book.click(1, false, false);
    book.click(3, true, false);
    assert_eq!(selected(&book), [1, 3]);
    assert!(!book.group_selected(&id));
    assert!(book.group_page().is_none());
    assert_eq!(book.active, 1);
    book.focus_group(&id);
    book.click(1, true, false);
    assert_eq!(selected(&book), [1]);
    assert!(!book.group_selected(&id));
    book.click(2, true, false);
    assert_eq!(selected(&book), [1, 2]);
    assert!(!book.group_selected(&id));
}
