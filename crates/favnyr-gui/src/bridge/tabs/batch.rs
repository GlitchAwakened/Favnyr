//! Frozen drag identities and atomic tab-book edits, independent of the UI.

use super::{BookId, TabBook, TabId};
use favnyr_core::tab_groups::{GroupError, TabGroups};
use std::collections::HashSet;

#[derive(Clone, Debug)]
pub(in crate::bridge) struct Batch {
    pub owner: BookId,
    pub ids: Vec<TabId>,
    pub active: TabId,
    /// Complete groups and their frozen membership, including hidden children.
    pub(super) groups: Vec<(String, Vec<TabId>)>,
}

#[derive(Clone, Debug)]
pub(in crate::bridge) struct Destination {
    pub gap: usize,
    pub group: Option<String>,
}

impl Destination {
    pub fn outside(gap: usize) -> Self {
        Self { gap, group: None }
    }
    pub fn resolve(&self, book: &TabBook) -> Result<usize, GroupError> {
        match self.group.as_deref() {
            None => Ok(book.groups.outside_gap(&book.tabs, self.gap)),
            Some(id) => {
                if book.groups.get(id).is_none() {
                    return Err(GroupError::MissingGroup);
                }
                let first = book
                    .tabs
                    .iter()
                    .position(|t| t.tab_group_id.as_deref() == Some(id))
                    .ok_or(GroupError::MissingGroup)?;
                let end = book
                    .tabs
                    .iter()
                    .rposition(|t| t.tab_group_id.as_deref() == Some(id))
                    .unwrap()
                    + 1;
                Ok(self.gap.clamp(first, end))
            }
        }
    }
}

/// New directory tabs share the same group destination rules as moved tabs.
pub(in crate::bridge) fn insert(
    book: &mut TabBook,
    tab: super::Tab,
    dest: &Destination,
) -> Result<(), GroupError> {
    let mut source = TabBook {
        identity: Default::default(),
        selection: Default::default(),
        focused_group: None,
        active_group: None,
        groups: Default::default(),
        tabs: vec![tab],
        active: 0,
    };
    Batch::single(&source, 0)
        .expect("one tab")
        .transfer(&mut source, book, dest)
}

impl Batch {
    pub fn capture(book: &TabBook, index: usize, group: Option<&str>) -> Option<Self> {
        let ids: Vec<_> = if let Some(id) = group {
            book.groups.get(id)?;
            if book.group_selected(id) {
                book.selected_ids()
            } else {
                book.tabs
                    .iter()
                    .filter(|t| t.tab_group_id.as_deref() == Some(id))
                    .map(|t| t.id)
                    .collect()
            }
        } else {
            let tab = book.tabs.get(index)?;
            if book.selected(tab.id) {
                book.selected_ids()
            } else {
                vec![tab.id]
            }
        };
        let grabbed = if let Some(id) = group {
            book.tabs
                .iter()
                .find(|t| t.tab_group_id.as_deref() == Some(id))?
                .id
        } else {
            book.tabs.get(index)?.id
        };
        let active = book.tabs.get(book.active)?.id;
        Some(Self::from_ids(book, ids, active, grabbed))
    }

    fn from_ids(book: &TabBook, ids: Vec<TabId>, active: TabId, grabbed: TabId) -> Self {
        let chosen: HashSet<_> = ids.iter().copied().collect();
        let indices: Vec<_> = book
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(i, tab)| chosen.contains(&tab.id).then_some(i))
            .collect();
        let groups = book
            .groups
            .complete_subset(&book.tabs, &indices)
            .entries()
            .iter()
            .map(|group| {
                let members = book
                    .tabs
                    .iter()
                    .filter(|tab| tab.tab_group_id.as_deref() == Some(&group.id))
                    .map(|tab| tab.id)
                    .collect();
                (group.id.clone(), members)
            })
            .collect();
        Self {
            owner: book.identity,
            active: if chosen.contains(&active) {
                active
            } else {
                grabbed
            },
            ids,
            groups,
        }
    }

    pub fn all(book: &TabBook) -> Option<Self> {
        let active = book.tabs.get(book.active)?.id;
        Some(Self::from_ids(
            book,
            book.tabs.iter().map(|t| t.id).collect(),
            active,
            active,
        ))
    }

    pub fn contains_group(&self, book: &TabBook, id: &str) -> bool {
        self.owner == book.identity && self.groups.iter().any(|(group, _)| group == id)
    }

    fn select_result(&self, book: &mut TabBook, groups: &[String]) {
        if groups.is_empty() {
            book.select_ids(&self.ids);
        } else {
            book.select_batch(&self.ids, groups);
        }
    }

    pub fn single(book: &TabBook, index: usize) -> Option<Self> {
        let id = book.tabs.get(index)?.id;
        Some(Self::from_ids(book, vec![id], id, id))
    }

    pub fn indices(&self, book: &TabBook) -> Result<Vec<usize>, GroupError> {
        if self.owner != book.identity || self.ids.is_empty() || book.active >= book.tabs.len() {
            return Err(GroupError::InvalidSelection);
        }
        let indices: Vec<_> = self
            .ids
            .iter()
            .map(|id| {
                book.tabs
                    .iter()
                    .position(|t| t.id == *id)
                    .ok_or(GroupError::InvalidSelection)
            })
            .collect::<Result<_, _>>()?;
        if indices.windows(2).any(|p| p[0] >= p[1]) || !self.ids.contains(&self.active) {
            return Err(GroupError::InvalidSelection);
        }
        for (id, expected) in &self.groups {
            let members: Vec<_> = book
                .tabs
                .iter()
                .filter(|t| t.tab_group_id.as_deref() == Some(id))
                .map(|t| t.id)
                .collect();
            if members != *expected || book.groups.get(id).is_none() {
                return Err(GroupError::MissingGroup);
            }
        }
        Ok(indices)
    }

    pub fn complete_groups(&self, book: &TabBook) -> TabGroups {
        let chosen: HashSet<_> = self.ids.iter().copied().collect();
        let indices: Vec<_> = book
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(i, t)| chosen.contains(&t.id).then_some(i))
            .collect();
        book.groups.complete_subset(&book.tabs, &indices)
    }

    /// Resolve every frozen tab before exposing its path to a bookmark action.
    pub fn paths(&self, book: &TabBook) -> Result<Vec<std::path::PathBuf>, GroupError> {
        Ok(self
            .indices(book)?
            .into_iter()
            .map(|index| book.tabs[index].current_path.clone())
            .collect())
    }

    pub fn requires_batch_protocol(&self, book: &TabBook) -> bool {
        self.ids.len() != 1 || !self.complete_groups(book).is_empty()
    }

    pub fn reorder(&self, book: &mut TabBook, target: &Destination) -> Result<bool, GroupError> {
        let indices = self.indices(book)?;
        let gap = target.resolve(book)?;
        if target
            .group
            .as_deref()
            .is_some_and(|id| self.contains_group(book, id))
        {
            return Ok(false);
        }
        let active_group = book.tabs[book.active].tab_group_id.clone();
        let changed = if let Some(id) = target.group.as_deref() {
            book.groups
                .join_at(&mut book.tabs, &mut book.active, &indices, id, gap)?
        } else {
            book.move_tabs(&self.ids, gap)
        };
        let retained: Vec<_> = self
            .groups
            .iter()
            .filter(|(id, _)| book.groups.get(id).is_some())
            .map(|(id, _)| id.clone())
            .collect();
        self.select_result(book, &retained);
        let sole_group = self
            .groups
            .first()
            .filter(|(_, members)| members.len() == self.ids.len())
            .map(|(id, _)| id.as_str());
        if let Some(id) = target.group.as_deref().or(sole_group) {
            book.highlight_group(id);
        } else if book.tabs[book.active].tab_group_id != active_group {
            book.highlight_active_group();
        }
        Ok(changed)
    }

    /// All metadata is prepared on a copy before any tab leaves the source.
    pub fn transfer(
        &self,
        source: &mut TabBook,
        target: &mut TabBook,
        dest: &Destination,
    ) -> Result<(), GroupError> {
        self.indices(source)?;
        let gap = dest.resolve(target)?;
        let complete = self.complete_groups(source);
        let mut groups = target.groups.clone();
        let mut mapping = std::collections::HashMap::new();
        if let Some(id) = dest.group.as_deref() {
            groups.set_collapsed(id, false)?;
        } else {
            for group in complete.entries() {
                mapping.insert(group.id.clone(), groups.import(group)?);
            }
        }
        let mut incoming = self.take_validated(source);
        for tab in &mut incoming {
            tab.tab_group_id = match &dest.group {
                Some(id) => Some(id.clone()),
                None => tab
                    .tab_group_id
                    .as_ref()
                    .and_then(|id| mapping.get(id))
                    .cloned(),
            };
        }
        target.tabs.splice(gap..gap, incoming);
        target.groups = groups;
        target.groups.normalize(&mut target.tabs);
        target.active = target
            .tabs
            .iter()
            .position(|t| t.id == self.active)
            .expect("validated active tab");
        self.select_result(target, &mapping.into_values().collect::<Vec<_>>());
        target.highlight_active_group();
        Ok(())
    }

    /// Call only after validating identities and every fallible destination step.
    pub fn take_validated(&self, book: &mut TabBook) -> Vec<super::Tab> {
        let chosen: HashSet<_> = self.ids.iter().copied().collect();
        let old_active = book.tabs.get(book.active).map(|t| t.id);
        let next_active = book
            .tabs
            .iter()
            .skip(book.active + 1)
            .chain(book.tabs[..book.active].iter().rev())
            .find(|t| !chosen.contains(&t.id))
            .map(|t| t.id);
        let mut moved = Vec::with_capacity(chosen.len());
        let mut kept = Vec::with_capacity(book.tabs.len() - chosen.len());
        for tab in book.tabs.drain(..) {
            if chosen.contains(&tab.id) {
                moved.push(tab);
            } else {
                kept.push(tab);
            }
        }
        book.tabs = kept;
        let active = old_active.filter(|id| !chosen.contains(id)).or(next_active);
        book.active = book
            .tabs
            .iter()
            .position(|t| Some(t.id) == active)
            .unwrap_or(0);
        book.groups.normalize(&mut book.tabs);
        book.reset_selection();
        book.reconcile_selection();
        if book.group_page().is_none() && book.tabs.get(book.active).map(|tab| tab.id) != old_active
        {
            book.highlight_active_group();
        }
        moved
    }
}
