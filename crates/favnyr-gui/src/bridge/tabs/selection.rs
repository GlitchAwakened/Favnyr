//! Transient selection uses identities, never paths or mutable tab indices.

use std::collections::HashSet;

use super::{TabBook, TabId};

#[derive(Debug, Default)]
pub(in crate::bridge) struct Selection {
    ids: HashSet<TabId>,
    groups: HashSet<String>,
    anchor: Option<Target>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Tab(TabId),
    Group(String),
}

impl TabBook {
    pub(in crate::bridge) fn selected(&self, id: TabId) -> bool {
        self.selection.ids.contains(&id)
    }

    pub(in crate::bridge) fn group_selected(&self, id: &str) -> bool {
        self.selection.groups.contains(id)
    }

    pub(in crate::bridge) fn selection_len(&self) -> usize {
        self.selection.ids.len()
    }

    pub(in crate::bridge) fn selected_ids(&self) -> Vec<TabId> {
        self.tabs
            .iter()
            .filter(|tab| self.selected(tab.id))
            .map(|tab| tab.id)
            .collect()
    }

    pub(in crate::bridge) fn select_ids(&mut self, ids: &[TabId]) {
        self.select_batch(ids, &[]);
    }

    pub(super) fn select_batch(&mut self, ids: &[TabId], groups: &[String]) {
        self.selection.ids = ids.iter().copied().collect();
        self.selection.groups = groups.iter().cloned().collect();
        self.selection.anchor = ids.first().map(|&id| {
            self.tabs
                .iter()
                .find(|tab| tab.id == id)
                .and_then(|tab| tab.tab_group_id.as_ref())
                .filter(|group| self.group_selected(group))
                .map_or(Target::Tab(id), |group| Target::Group(group.clone()))
        });
        self.reconcile_selection();
    }

    pub(in crate::bridge) fn select_group(&mut self, id: &str) -> bool {
        if self.groups.set_collapsed(id, false).is_err() {
            return false;
        }
        self.selection.ids = self
            .tabs
            .iter()
            .filter(|t| t.tab_group_id.as_deref() == Some(id))
            .map(|t| t.id)
            .collect();
        self.selection.groups.clear();
        self.selection.anchor = self
            .tabs
            .iter()
            .find(|t| self.selected(t.id))
            .map(|t| Target::Tab(t.id));
        self.highlight_group(id);
        true
    }

    /// Individual hidden tabs cannot be targeted independently of their header.
    pub(in crate::bridge) fn visible_indices(&self) -> Vec<usize> {
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| {
                let collapsed = tab
                    .tab_group_id
                    .as_deref()
                    .and_then(|id| self.groups.get(id))
                    .is_some_and(|group| group.collapsed);
                (!collapsed).then_some(index)
            })
            .collect()
    }

    pub(in crate::bridge) fn reset_selection(&mut self) -> bool {
        let active = self
            .tabs
            .get(self.active)
            .filter(|_| self.group_page().is_none())
            .filter(|tab| {
                tab.tab_group_id
                    .as_deref()
                    .and_then(|id| self.groups.get(id))
                    .is_none_or(|g| !g.collapsed)
            })
            .map(|tab| tab.id);
        let changed = !self.selection.groups.is_empty()
            || self.selection.ids.len() != usize::from(active.is_some())
            || active.is_some_and(|id| !self.selected(id));
        self.selection.ids.clear();
        self.selection.ids.extend(active);
        self.selection.groups.clear();
        self.selection.anchor = active.map(Target::Tab);
        changed
    }

    pub(in crate::bridge) fn reconcile_selection(&mut self) {
        if self
            .active_group
            .as_deref()
            .is_some_and(|id| self.groups.get(id).is_none())
        {
            self.active_group = None;
            self.highlight_active_group();
        }
        if self
            .focused_group
            .as_deref()
            .is_some_and(|id| self.groups.get(id).is_none())
        {
            self.focused_group = None;
        }
        self.selection
            .groups
            .retain(|id| self.groups.get(id).is_some());
        let visible: HashSet<_> = self
            .visible_indices()
            .into_iter()
            .map(|i| self.tabs[i].id)
            .collect();
        let members: HashSet<_> = if self.selection.groups.is_empty() {
            HashSet::new()
        } else {
            self.tabs
                .iter()
                .filter(|tab| {
                    tab.tab_group_id
                        .as_deref()
                        .is_some_and(|id| self.group_selected(id))
                })
                .map(|tab| tab.id)
                .collect()
        };
        self.selection
            .ids
            .retain(|id| visible.contains(id) || members.contains(id));
        self.selection.ids.extend(members);
        let anchor_exists = match &self.selection.anchor {
            Some(Target::Tab(id)) => visible.contains(id),
            Some(Target::Group(id)) => self.groups.get(id).is_some(),
            None => true,
        };
        if !anchor_exists {
            self.selection.anchor = None;
        }
    }

    /// Returns only whether the active folder changed. Modified clicks never
    /// navigate, including when they remove the active tab from the selection.
    pub(in crate::bridge) fn click(&mut self, index: usize, ctrl: bool, shift: bool) -> bool {
        let Some(tab) = self.tabs.get(index) else {
            return false;
        };
        let target = tab.id;
        if !ctrl && !shift {
            return self.select(index);
        }
        if !self.visible_indices().contains(&index) {
            return false;
        }
        self.modify_selection(Target::Tab(target), ctrl, shift);
        false
    }

    /// Modified header clicks select a whole group without folding or navigating.
    pub(in crate::bridge) fn click_group(&mut self, id: &str, ctrl: bool, shift: bool) {
        if (ctrl || shift) && self.groups.get(id).is_some() {
            self.modify_selection(Target::Group(id.to_owned()), ctrl, shift);
        }
    }

    /// Range order follows the rendered bar, including collapsed group headers.
    fn selection_targets(&self) -> Vec<Target> {
        let mut targets = Vec::new();
        let mut previous = None;
        for tab in &self.tabs {
            let group = tab
                .tab_group_id
                .as_deref()
                .and_then(|id| self.groups.get(id));
            if let Some(group) = group
                && previous != Some(group.id.as_str())
            {
                targets.push(Target::Group(group.id.clone()));
            }
            if group.is_none_or(|group| !group.collapsed) {
                targets.push(Target::Tab(tab.id));
            }
            previous = group.map(|group| group.id.as_str());
        }
        targets
    }

    fn add_target(&mut self, target: &Target) {
        match target {
            Target::Tab(id) => {
                self.selection.ids.insert(*id);
            }
            Target::Group(id) => {
                self.selection.groups.insert(id.clone());
                self.selection.ids.extend(
                    self.tabs
                        .iter()
                        .filter(|tab| tab.tab_group_id.as_deref() == Some(id))
                        .map(|tab| tab.id),
                );
            }
        }
    }

    fn modify_selection(&mut self, target: Target, ctrl: bool, shift: bool) {
        if shift {
            let targets = self.selection_targets();
            let Some(to) = targets.iter().position(|entry| entry == &target) else {
                return;
            };
            let fallback = self.group_page().map_or_else(
                || self.tabs.get(self.active).map(|tab| Target::Tab(tab.id)),
                |group| Some(Target::Group(group.id.clone())),
            );
            let anchor = self
                .selection
                .anchor
                .as_ref()
                .or(fallback.as_ref())
                .and_then(|anchor| targets.iter().position(|entry| entry == anchor))
                .unwrap_or(to);
            // Keep the starting point when subsequent Shift clicks resize the range.
            self.selection.anchor = Some(targets[anchor].clone());
            if !ctrl {
                self.selection.ids.clear();
                self.selection.groups.clear();
            }
            for entry in &targets[anchor.min(to)..=anchor.max(to)] {
                self.add_target(entry);
            }
        } else {
            // A focused header has no active folder or explicit selection.
            // Start an additive selection from that group, just as a normal
            // tab starts from its active tab. Selecting a visible member instead
            // starts an individual selection. An explicit empty selection keeps
            // its anchor and must not implicitly reselect the group.
            if self.selection.anchor.is_none()
                && self.selection.ids.is_empty()
                && self.selection.groups.is_empty()
                && let Some(group) = self.group_page()
            {
                let origin = Target::Group(group.id.clone());
                let own_member = match &target {
                    Target::Tab(id) => self.tabs.iter().any(|tab| {
                        tab.id == *id && tab.tab_group_id.as_deref() == Some(group.id.as_str())
                    }),
                    Target::Group(_) => false,
                };
                if origin != target && !own_member {
                    self.add_target(&origin);
                }
            }
            match &target {
                Target::Tab(id) => {
                    // Excluding one visible child turns its whole-group selection
                    // into individual members, leaving the other children selected.
                    if let Some(group) = self
                        .tabs
                        .iter()
                        .find(|tab| tab.id == *id)
                        .and_then(|tab| tab.tab_group_id.as_deref())
                    {
                        self.selection.groups.remove(group);
                    }
                    if !self.selection.ids.remove(id) {
                        self.selection.ids.insert(*id);
                    }
                }
                Target::Group(id) if self.selection.groups.remove(id) => {
                    for tab in &self.tabs {
                        if tab.tab_group_id.as_deref() == Some(id) {
                            self.selection.ids.remove(&tab.id);
                        }
                    }
                }
                Target::Group(_) => self.add_target(&target),
            }
            self.selection.anchor = Some(target);
        }
    }

    /// A context menu keeps a selection it belongs to, otherwise it targets
    /// just the clicked tab. Neither case changes the active folder.
    pub(in crate::bridge) fn select_context(&mut self, index: usize) -> usize {
        if !self.visible_indices().contains(&index) {
            return 0;
        }
        let Some(tab) = self.tabs.get(index) else {
            return 0;
        };
        let id = tab.id;
        if !self.selected(id) {
            self.selection.ids.clear();
            self.selection.groups.clear();
            self.selection.ids.insert(id);
            self.selection.anchor = Some(Target::Tab(id));
        }
        self.selection_len()
    }
}
