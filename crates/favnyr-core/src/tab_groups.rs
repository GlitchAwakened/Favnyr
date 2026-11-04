//! Flat, contiguous tab groups and their workspace representation.
//! Operations only move entries and metadata, never paths or files.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::annotations::MAX_COLOR_SLOT;
use crate::reorder::MovePlan;

pub const MAX_NAME_CHARS: usize = 80;

/// Minimal interface shared by stored tabs and live tabs with navigation history.
pub trait GroupedTab {
    fn tab_group_id(&self) -> Option<&str>;
    fn set_tab_group_id(&mut self, id: Option<String>);
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TabGroupState {
    pub id: String,
    pub name: String,
    pub color_slot: u8,
    pub collapsed: bool,
}

/// The counter is transient and does not participate in workspace equality.
#[derive(Debug, Clone)]
pub struct TabGroups {
    entries: Vec<TabGroupState>,
    next_id: u64,
}

impl Default for TabGroups {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            next_id: 1,
        }
    }
}

impl PartialEq for TabGroups {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}
impl Eq for TabGroups {}

impl Serialize for TabGroups {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.entries.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TabGroups {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = toml::Value::deserialize(deserializer)?;
        let mut groups = Self::default();
        if let Some(values) = value.as_array() {
            let mut seen = HashSet::new();
            for value in values {
                let Some(table) = value.as_table() else {
                    continue;
                };
                let Some(id) = table
                    .get("id")
                    .and_then(toml::Value::as_str)
                    .filter(|id| valid_id(id))
                else {
                    continue;
                };
                if !seen.insert(id) {
                    continue;
                }
                let name = table
                    .get("name")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("");
                let slot = table
                    .get("color_slot")
                    .and_then(toml::Value::as_integer)
                    .filter(|&v| (0..=i64::from(MAX_COLOR_SLOT)).contains(&v))
                    .unwrap_or(0);
                groups.entries.push(TabGroupState {
                    id: id.into(),
                    name: checked_name(name).unwrap_or_default(),
                    color_slot: slot as u8,
                    collapsed: table
                        .get("collapsed")
                        .and_then(toml::Value::as_bool)
                        .unwrap_or(false),
                });
                if let Some(number) = id.strip_prefix('g').and_then(|n| n.parse::<u64>().ok()) {
                    groups.next_id = groups.next_id.max(number.saturating_add(1));
                }
            }
        }
        Ok(groups)
    }
}

/// Invalid metadata is discarded locally instead of rejecting every tab.
pub fn deserialize_membership<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let value = toml::Value::deserialize(deserializer)?;
    Ok(value.as_str().filter(|id| valid_id(id)).map(str::to_owned))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

fn checked_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(char::is_control))
    .then(|| name.to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupError {
    InvalidSelection,
    InvalidActive,
    InvalidName,
    NameTaken,
    InvalidColor,
    MissingGroup,
    IdentityExhausted,
}

fn selection(len: usize, active: usize, indices: &[usize]) -> Result<Vec<usize>, GroupError> {
    if active >= len {
        return Err(GroupError::InvalidActive);
    }
    let mut indices = indices.to_vec();
    indices.sort_unstable();
    if indices.is_empty()
        || indices.last().is_some_and(|&i| i >= len)
        || indices.windows(2).any(|w| w[0] == w[1])
    {
        return Err(GroupError::InvalidSelection);
    }
    Ok(indices)
}

fn move_entries<T>(tabs: &mut [T], active: &mut usize, indices: &[usize], gap: usize) -> bool {
    let Some(plan) = MovePlan::new(tabs.len(), indices, gap) else {
        return false;
    };
    *active = plan
        .sources()
        .iter()
        .position(|&source| source == *active)
        .expect("active tab was validated");
    plan.apply(tabs)
}

impl TabGroups {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn entries(&self) -> &[TabGroupState] {
        &self.entries
    }

    /// Metadata follows only groups whose entire membership is transported.
    pub fn complete_subset<T: GroupedTab>(&self, tabs: &[T], indices: &[usize]) -> Self {
        let selected: HashSet<_> = indices.iter().copied().collect();
        let mut result = self.clone();
        result.entries.retain(|g| {
            tabs.iter()
                .enumerate()
                .all(|(i, t)| t.tab_group_id() != Some(&g.id) || selected.contains(&i))
        });
        result
    }
    pub fn get(&self, id: &str) -> Option<&TabGroupState> {
        self.entries.iter().find(|g| g.id == id)
    }

    /// Retains only the first contiguous run of each known group, without
    /// removing or moving tabs. Empty names are repaired by the UI with a
    /// translated fallback, never with a hardcoded language here.
    pub fn normalize<T: GroupedTab>(&mut self, tabs: &mut [T]) {
        let known: HashSet<_> = self.entries.iter().map(|g| g.id.as_str()).collect();
        let mut order = HashMap::new();
        let mut previous: Option<String> = None;
        for tab in tabs {
            let id = tab.tab_group_id().map(str::to_owned);
            if let Some(id) = id {
                if !known.contains(id.as_str())
                    || (order.contains_key(&id) && previous.as_deref() != Some(&id))
                {
                    tab.set_tab_group_id(None);
                    previous = None;
                } else {
                    let next = order.len();
                    order.entry(id.clone()).or_insert(next);
                    previous = Some(id);
                }
            } else {
                previous = None;
            }
        }
        self.entries.retain(|g| order.contains_key(&g.id));
        self.entries.sort_by_key(|g| order[&g.id]);
        let mut names = HashSet::new();
        for group in &mut self.entries {
            if !group.name.is_empty() && !names.insert(group.name.to_lowercase()) {
                group.name.clear();
            }
        }
    }

    /// Supplies translated fallback names after metadata validation.
    pub fn repair_names(&mut self, fallback: &str) -> Result<bool, GroupError> {
        let fallback = checked_name(fallback).ok_or(GroupError::InvalidName)?;
        let mut changed = false;
        for i in 0..self.entries.len() {
            if self.entries[i].name.is_empty() {
                self.entries[i].name = self.available_name(&fallback);
                changed = true;
            }
        }
        Ok(changed)
    }

    pub fn suggested_name(&self, base: &str) -> Result<String, GroupError> {
        let base = checked_name(base).ok_or(GroupError::InvalidName)?;
        Ok(self.available_name(&base))
    }

    /// Imports independent metadata without merging groups that share a name.
    pub fn import(&mut self, group: &TabGroupState) -> Result<String, GroupError> {
        let name = self.suggested_name(&group.name)?;
        if group.color_slot > MAX_COLOR_SLOT {
            return Err(GroupError::InvalidColor);
        }
        let id = self.fresh_id()?;
        self.entries.push(TabGroupState {
            id: id.clone(),
            name,
            ..group.clone()
        });
        Ok(id)
    }

    /// A group destination is explicit, including its two inner boundary gaps.
    pub fn join_at<T: GroupedTab>(
        &mut self,
        tabs: &mut [T],
        active: &mut usize,
        indices: &[usize],
        id: &str,
        gap: usize,
    ) -> Result<bool, GroupError> {
        let indices = selection(tabs.len(), *active, indices)?;
        if self.get(id).is_none() {
            return Err(GroupError::MissingGroup);
        }
        let first = tabs
            .iter()
            .position(|t| t.tab_group_id() == Some(id))
            .ok_or(GroupError::MissingGroup)?;
        let end = tabs
            .iter()
            .rposition(|t| t.tab_group_id() == Some(id))
            .unwrap()
            + 1;
        let mut changed = false;
        for &i in &indices {
            if tabs[i].tab_group_id() != Some(id) {
                tabs[i].set_tab_group_id(Some(id.into()));
                changed = true;
            }
        }
        changed |= move_entries(tabs, active, &indices, gap.clamp(first, end));
        changed |= self.set_collapsed(id, false)?;
        self.normalize(tabs);
        Ok(changed)
    }

    fn available_name(&self, base: &str) -> String {
        let mut candidate = base.to_owned();
        let mut number = 2;
        while self
            .entries
            .iter()
            .any(|g| g.name.to_lowercase() == candidate.to_lowercase())
        {
            let suffix = format!(" ({number})");
            candidate = base
                .chars()
                .take(MAX_NAME_CHARS - suffix.chars().count())
                .collect::<String>()
                + &suffix;
            number += 1;
        }
        candidate
    }

    fn validate_name(&self, name: &str, except: Option<&str>) -> Result<String, GroupError> {
        let name = checked_name(name).ok_or(GroupError::InvalidName)?;
        if self
            .entries
            .iter()
            .any(|g| Some(g.id.as_str()) != except && g.name.to_lowercase() == name.to_lowercase())
        {
            return Err(GroupError::NameTaken);
        }
        Ok(name)
    }

    fn fresh_id(&mut self) -> Result<String, GroupError> {
        loop {
            let number = self.next_id;
            self.next_id = number.checked_add(1).ok_or(GroupError::IdentityExhausted)?;
            let id = format!("g{number}");
            if self.get(&id).is_none() {
                return Ok(id);
            }
        }
    }

    /// Moves an outside insertion past a group rather than splitting its run.
    pub fn outside_gap<T: GroupedTab>(&self, tabs: &[T], gap: usize) -> usize {
        let gap = gap.min(tabs.len());
        if gap == 0 || gap == tabs.len() {
            return gap;
        }
        let Some(id) = tabs[gap - 1].tab_group_id() else {
            return gap;
        };
        if tabs[gap].tab_group_id() != Some(id) {
            return gap;
        }
        gap + tabs[gap..]
            .iter()
            .take_while(|t| t.tab_group_id() == Some(id))
            .count()
    }

    pub fn create<T: GroupedTab>(
        &mut self,
        tabs: &mut [T],
        active: &mut usize,
        indices: &[usize],
        name: &str,
        color_slot: u8,
    ) -> Result<String, GroupError> {
        let indices = selection(tabs.len(), *active, indices)?;
        let name = self.validate_name(name, None)?;
        if color_slot > MAX_COLOR_SLOT {
            return Err(GroupError::InvalidColor);
        }
        let mut gap = indices[0];
        if let Some(id) = tabs[gap].tab_group_id() {
            let remaining: Vec<_> = tabs
                .iter()
                .enumerate()
                .filter_map(|(i, t)| {
                    (t.tab_group_id() == Some(id) && indices.binary_search(&i).is_err())
                        .then_some(i)
                })
                .collect();
            if remaining.first().is_some_and(|&i| i < gap)
                && remaining.last().is_some_and(|&i| i > gap)
            {
                gap = remaining.last().unwrap() + 1;
            }
        }
        let id = self.fresh_id()?;
        for &i in &indices {
            tabs[i].set_tab_group_id(Some(id.clone()));
        }
        move_entries(tabs, active, &indices, gap);
        self.entries.push(TabGroupState {
            id: id.clone(),
            name,
            color_slot,
            collapsed: false,
        });
        self.normalize(tabs);
        Ok(id)
    }

    pub fn assign<T: GroupedTab>(
        &mut self,
        tabs: &mut [T],
        active: &mut usize,
        indices: &[usize],
        id: &str,
    ) -> Result<bool, GroupError> {
        let indices = selection(tabs.len(), *active, indices)?;
        if self.get(id).is_none() {
            return Err(GroupError::MissingGroup);
        }
        let gap = tabs
            .iter()
            .rposition(|t| t.tab_group_id() == Some(id))
            .ok_or(GroupError::MissingGroup)?
            + 1;
        let incoming: Vec<_> = indices
            .into_iter()
            .filter(|&i| tabs[i].tab_group_id() != Some(id))
            .collect();
        if incoming.is_empty() {
            return Ok(false);
        }
        for &i in &incoming {
            tabs[i].set_tab_group_id(Some(id.into()));
        }
        move_entries(tabs, active, &incoming, gap);
        self.set_collapsed(id, false)?;
        self.normalize(tabs);
        Ok(true)
    }

    pub fn remove_members<T: GroupedTab>(
        &mut self,
        tabs: &mut [T],
        active: &mut usize,
        indices: &[usize],
    ) -> Result<bool, GroupError> {
        let indices = selection(tabs.len(), *active, indices)?;
        let runs: Vec<_> = self
            .entries
            .iter()
            .filter_map(|group| {
                let moved: Vec<_> = indices
                    .iter()
                    .copied()
                    .filter(|&i| tabs[i].tab_group_id() == Some(&group.id))
                    .collect();
                if moved.is_empty() {
                    return None;
                }
                let end = tabs
                    .iter()
                    .rposition(|t| t.tab_group_id() == Some(&group.id))?
                    + 1;
                Some((moved, end))
            })
            .collect();
        if runs.is_empty() {
            return Ok(false);
        }
        for (moved, end) in runs {
            for &i in &moved {
                tabs[i].set_tab_group_id(None);
            }
            move_entries(tabs, active, &moved, end);
        }
        self.normalize(tabs);
        Ok(true)
    }

    pub fn dissolve<T: GroupedTab>(&mut self, tabs: &mut [T], id: &str) -> Result<(), GroupError> {
        if self.get(id).is_none() {
            return Err(GroupError::MissingGroup);
        }
        for tab in tabs {
            if tab.tab_group_id() == Some(id) {
                tab.set_tab_group_id(None);
            }
        }
        self.entries.retain(|g| g.id != id);
        Ok(())
    }

    pub fn rename(&mut self, id: &str, name: &str) -> Result<bool, GroupError> {
        if self.get(id).is_none() {
            return Err(GroupError::MissingGroup);
        }
        let name = self.validate_name(name, Some(id))?;
        let group = self.entries.iter_mut().find(|g| g.id == id).unwrap();
        let changed = group.name != name;
        group.name = name;
        Ok(changed)
    }

    pub fn set_color(&mut self, id: &str, color: u8) -> Result<bool, GroupError> {
        if color > MAX_COLOR_SLOT {
            return Err(GroupError::InvalidColor);
        }
        let group = self
            .entries
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or(GroupError::MissingGroup)?;
        let changed = group.color_slot != color;
        group.color_slot = color;
        Ok(changed)
    }

    pub fn set_collapsed(&mut self, id: &str, collapsed: bool) -> Result<bool, GroupError> {
        let group = self
            .entries
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or(GroupError::MissingGroup)?;
        let changed = group.collapsed != collapsed;
        group.collapsed = collapsed;
        Ok(changed)
    }

    /// Whole groups retain their metadata; fragments leave their group unless
    /// the entire selection is being reordered inside that same group.
    pub fn move_members<T: GroupedTab>(
        &mut self,
        tabs: &mut [T],
        active: &mut usize,
        indices: &[usize],
        gap: usize,
    ) -> Result<bool, GroupError> {
        let indices = selection(tabs.len(), *active, indices)?;
        let gap = gap.min(tabs.len());
        let inside = if gap > 0
            && gap < tabs.len()
            && tabs[gap - 1].tab_group_id() == tabs[gap].tab_group_id()
        {
            tabs[gap].tab_group_id()
        } else {
            None
        };
        let same_group =
            inside.is_some() && indices.iter().all(|&i| tabs[i].tab_group_id() == inside);
        let gap = if same_group {
            gap
        } else {
            self.outside_gap(tabs, gap)
        };
        let mut changed = false;
        if !same_group {
            let complete: HashSet<_> = self
                .entries
                .iter()
                .filter(|g| {
                    tabs.iter().enumerate().all(|(i, t)| {
                        t.tab_group_id() != Some(&g.id) || indices.binary_search(&i).is_ok()
                    })
                })
                .map(|g| g.id.as_str())
                .collect();
            for &i in &indices {
                if tabs[i]
                    .tab_group_id()
                    .is_some_and(|id| !complete.contains(id))
                {
                    tabs[i].set_tab_group_id(None);
                    changed = true;
                }
            }
        }
        changed |= move_entries(tabs, active, &indices, gap);
        self.normalize(tabs);
        Ok(changed)
    }
}

#[cfg(test)]
mod tests;
