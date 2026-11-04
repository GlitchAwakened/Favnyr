//! Shared geometry for tab rendering, scrolling and drop targeting.

use super::{TabBook, TabId};
use crate::bridge::{TabBarState, TabDropGap, TabInfo, tab_title};
use favnyr_core::annotations::AnnotationStore;
use slint::{Model, VecModel};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

#[derive(Debug, Clone, PartialEq)]
struct Shape {
    mode: u8,
    tabs: Vec<(TabId, PathBuf, Option<String>)>,
    groups: Vec<(String, String, bool)>,
}

impl Shape {
    fn capture(book: &TabBook, mode: u8) -> Self {
        Self {
            mode,
            tabs: book
                .tabs
                .iter()
                .map(|t| (t.id, t.current_path.clone(), t.tab_group_id.clone()))
                .collect(),
            groups: book
                .groups
                .entries()
                .iter()
                .map(|g| (g.id.clone(), g.name.clone(), g.collapsed))
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    index: usize,
    tab_id: TabId,
    group: Option<String>,
    header: bool,
    /// Last visible entry of its group, which closes the attachment rail.
    tail: bool,
    first: usize,
    end: usize,
    title: String,
    /// Main-axis extent, boundary gaps included.
    extent: f32,
    offset: f32,
    pad_lead: f32,
    pad_trail: f32,
}

impl Entry {
    fn same_identity(&self, other: &Self) -> bool {
        self.header == other.header
            && if self.header {
                self.group == other.group
            } else {
                self.tab_id == other.tab_id
            }
    }
}

/// Stable model owned by a view. Geometry changes only with the visible shape;
/// selection, annotations and group colors update rows without rebuilding it.
#[derive(Default)]
pub(in crate::bridge) struct Presentation {
    pub(in crate::bridge) model: Rc<VecModel<TabInfo>>,
    shape: RefCell<Option<Shape>>,
    entries: RefCell<Vec<Entry>>,
    revision: Cell<i32>,
}

impl Presentation {
    pub(in crate::bridge) fn update(
        &self,
        book: &TabBook,
        mode: u8,
        annotations: &AnnotationStore,
    ) {
        let next = Shape::capture(book, mode);
        let mut retained = None;
        if self.shape.borrow().as_ref() != Some(&next) {
            let mut entries = Vec::new();
            let mut index = 0;
            while index < book.tabs.len() {
                let group = book.tabs[index]
                    .tab_group_id
                    .as_deref()
                    .and_then(|id| book.groups.get(id));
                let end = if let Some(group) = group {
                    index
                        + book.tabs[index..]
                            .iter()
                            .take_while(|t| t.tab_group_id.as_deref() == Some(&group.id))
                            .count()
                } else {
                    index + 1
                };
                let first = index;
                if let Some(group) = group {
                    let extent = if mode == 0 {
                        estimate_group_width(&group.name, end - first)
                    } else {
                        VTAB_EXTENT
                    };
                    entries.push(Entry {
                        index,
                        tab_id: book.tabs[index].id,
                        group: Some(group.id.clone()),
                        header: true,
                        tail: false,
                        first,
                        end,
                        title: group.name.clone(),
                        extent,
                        offset: 0.0,
                        pad_lead: 0.0,
                        pad_trail: 0.0,
                    });
                }
                while index < end {
                    if group.is_none_or(|g| !g.collapsed) {
                        let title = tab_title(&book.tabs[index].current_path);
                        let extent = if mode == 0 {
                            estimate_tab_width(&title)
                        } else {
                            VTAB_EXTENT
                        };
                        entries.push(Entry {
                            index,
                            tab_id: book.tabs[index].id,
                            group: group.map(|g| g.id.clone()),
                            header: false,
                            tail: false,
                            first,
                            end,
                            title,
                            extent,
                            offset: 0.0,
                            pad_lead: 0.0,
                            pad_trail: 0.0,
                        });
                    }
                    index += 1;
                }
            }
            // A group's two ends are set apart from its neighbours, which sit
            // one pitch away, so the run reads as a single block instead of
            // adjacent entries that happen to share a colour. The gap is
            // reserved inside the entry's own extent, which keeps the bridge
            // the sole source of truth for offsets.
            for i in 0..entries.len() {
                if entries[i].group.is_none() {
                    continue;
                }
                let tail = i + 1 == entries.len() || entries[i + 1].group != entries[i].group;
                entries[i].tail = tail;
                if entries[i].header {
                    entries[i].pad_lead = GROUP_BOUNDARY;
                }
                if tail {
                    entries[i].pad_trail = GROUP_BOUNDARY;
                }
            }
            let mut offset = 0.0;
            for entry in &mut entries {
                entry.extent += entry.pad_lead + entry.pad_trail;
                entry.offset = offset;
                offset += entry.extent + 2.0;
            }
            let previous = self.entries.borrow();
            let prefix = previous
                .iter()
                .zip(&entries)
                .take_while(|(old, new)| old.same_identity(new))
                .count();
            let suffix = previous[prefix..]
                .iter()
                .rev()
                .zip(entries[prefix..].iter().rev())
                .take_while(|(old, new)| old.same_identity(new))
                .count();
            retained = Some((prefix, suffix));
            drop(previous);
            *self.entries.borrow_mut() = entries;
            *self.shape.borrow_mut() = Some(next);
            self.revision.set(self.revision.get().wrapping_add(1));
        }
        let group_page = book.group_page().is_some();
        let rows: Vec<_> = self
            .entries
            .borrow()
            .iter()
            .map(|entry| {
                let group = entry.group.as_deref().and_then(|id| book.groups.get(id));
                let tab = &book.tabs[entry.index];
                let selected = if entry.header {
                    entry
                        .group
                        .as_deref()
                        .is_some_and(|id| book.group_selected(id))
                } else {
                    book.selected(tab.id)
                };
                TabInfo {
                    index: if entry.header { -1 } else { entry.index as i32 },
                    group_id: entry.group.as_deref().unwrap_or("").into(),
                    is_group: entry.header,
                    grouped: group.is_some(),
                    rail_tail: entry.tail,
                    group_slot: group.map_or(0, |g| i32::from(g.color_slot)),
                    count: (entry.end - entry.first) as i32,
                    collapsed: group.is_some_and(|g| g.collapsed),
                    group_highlighted: entry.header
                        && entry
                            .group
                            .as_deref()
                            .is_some_and(|id| book.group_highlighted(id)),
                    selection_marked: selected
                        && (entry.header
                            || book.selection_len() > 1
                            || entry.index != book.active
                            || group_page),
                    title: entry.title.as_str().into(),
                    folder_slot: if entry.header {
                        0
                    } else {
                        i32::from(annotations.color_of(&tab.current_path))
                    },
                    path: if entry.header {
                        "".into()
                    } else {
                        tab.current_path.display().to_string().into()
                    },
                    width: entry.extent,
                    offset: entry.offset,
                    pad_lead: entry.pad_lead,
                    pad_trail: entry.pad_trail,
                }
            })
            .collect();
        if self.model.row_count() != rows.len() {
            let (prefix, suffix) = retained.unwrap_or_default();
            if prefix == 0 && suffix == 0 {
                self.model.set_vec(rows);
                return;
            }
            // Folding changes the member count, but the header keeps its
            // identity and keyboard focus. Retain unchanged entries at both
            // ends rather than resetting the whole repeater.
            let removed = self.model.row_count() - prefix - suffix;
            for _ in 0..removed {
                self.model.remove(prefix);
            }
            for (index, row) in rows[prefix..rows.len() - suffix].iter().enumerate() {
                self.model.insert(prefix + index, row.clone());
            }
        }
        for (i, row) in rows.into_iter().enumerate() {
            if self.model.row_data(i).as_ref() != Some(&row) {
                self.model.set_row_data(i, row);
            }
        }
    }

    pub(in crate::bridge) fn state(&self, book: &TabBook) -> TabBarState {
        let page = book.group_page();
        TabBarState {
            active: if page.is_some() {
                -1
            } else {
                book.active as i32
            },
            count: book.tabs.len() as i32,
            revision: self.revision.get(),
            group_page: page.is_some(),
            group_index: page
                .and_then(|g| {
                    book.tabs
                        .iter()
                        .position(|t| t.tab_group_id.as_deref() == Some(&g.id))
                })
                .map_or(-1, |i| i as i32),
            group_name: page.map_or("", |g| g.name.as_str()).into(),
            group_collapsed: page.is_some_and(|g| g.collapsed),
        }
    }

    /// A collapsed group contributes its header, addressed by its first logical
    /// index. Activating that destination never activates the hidden member.
    pub(in crate::bridge) fn step(&self, book: &TabBook, forward: bool) -> i32 {
        let entries = self.entries.borrow();
        let page = book.group_page();
        let mut active = page
            .and_then(|g| {
                entries
                    .iter()
                    .find(|e| e.header && e.group.as_deref() == Some(&g.id))
            })
            .map_or(book.active, |e| e.first);
        let shown: Vec<usize> = entries
            .iter()
            .filter_map(|entry| {
                let collapsed = entry
                    .group
                    .as_deref()
                    .and_then(|id| book.groups.get(id))
                    .is_some_and(|group| group.collapsed);
                if collapsed {
                    if page.is_none() && (entry.first..entry.end).contains(&book.active) {
                        active = entry.first;
                    }
                    entry.header.then_some(entry.first)
                } else {
                    (!entry.header).then_some(entry.index)
                }
            })
            .collect();
        let next = if forward {
            let before_members = page.is_some_and(|g| !g.collapsed);
            shown
                .iter()
                .find(|&&i| i > active || before_members && i == active)
                .or(shown.first())
        } else {
            shown.iter().rev().find(|&&i| i < active).or(shown.last())
        };
        next.map_or(book.active, |&index| index) as i32
    }

    pub(in crate::bridge) fn scroll(
        &self,
        index: i32,
        forward: bool,
        viewport: f32,
        extent: f32,
    ) -> f32 {
        let entries = self.entries.borrow();
        let visible = if index < 0 {
            -1
        } else {
            entries
                .iter()
                .position(|e| !e.header && e.index == index as usize)
                .or_else(|| {
                    entries
                        .iter()
                        .position(|e| e.header && (e.first..e.end).contains(&(index as usize)))
                })
                .map_or(-2, |i| i as i32)
        };
        if visible == -2 {
            return viewport;
        }
        let geometry: Vec<_> = entries.iter().map(|e| (e.extent, e.offset)).collect();
        let target = scroll_target(&geometry, visible, forward, viewport, extent);
        if visible >= 0 {
            let entry = &entries[visible as usize];
            if let Some(header) = entries[..visible as usize]
                .iter()
                .rposition(|e| e.header && e.group == entry.group)
            {
                let span = entry.offset + entry.extent - entries[header].offset;
                if entry.index == entry.first && span <= extent {
                    // Keep the header alongside the first member when both fit.
                    return scroll_target(&geometry, header as i32, forward, target, extent);
                }
            }
        }
        target
    }

    /// Legacy receivers use only exterior gaps and cannot join a group.
    pub(in crate::bridge) fn gap(
        &self,
        book: &TabBook,
        position: f32,
        source: Option<usize>,
    ) -> TabDropGap {
        self.gap_impl(book, position, source, None, false)
    }

    /// A complete group in the local drag cannot absorb its own selection.
    pub(in crate::bridge) fn destination(
        &self,
        book: &TabBook,
        position: f32,
        batch: Option<&super::batch::Batch>,
    ) -> TabDropGap {
        self.gap_impl(book, position, None, batch, true)
    }

    fn gap_impl(
        &self,
        book: &TabBook,
        position: f32,
        source: Option<usize>,
        batch: Option<&super::batch::Batch>,
        joins: bool,
    ) -> TabDropGap {
        let entries = self.entries.borrow();
        let Some(entry) = entries
            .iter()
            .find(|e| position < e.offset + e.extent + 2.0)
            .or(entries.last())
        else {
            return TabDropGap {
                index: 0,
                position: 0.0,
                ..Default::default()
            };
        };
        let before = position < entry.offset + entry.extent / 2.0;
        let collapsed = entry
            .group
            .as_deref()
            .and_then(|id| book.groups.get(id))
            .is_some_and(|g| g.collapsed);
        let mut gap = if entry.header || collapsed {
            if before { entry.first } else { entry.end }
        } else {
            entry.index + usize::from(!before)
        };
        let join = joins
            && entry.group.is_some()
            // A group cannot be dropped into itself, so hovering its own
            // header or any of its members offers no membership to take.
            && !entry.group.as_deref().is_some_and(|id| {
                batch.is_some_and(|batch| batch.contains_group(book, id))
            })
            && if entry.header {
                position >= entry.offset + 8.0 && position < entry.offset + entry.extent - 8.0
            } else {
                !collapsed && gap >= entry.first && gap < entry.end
            };
        if join && entry.header {
            gap = entry.end;
        }
        let within_source_group = source
            .and_then(|i| book.tabs.get(i))
            .and_then(|t| t.tab_group_id.as_deref())
            .is_some_and(|id| {
                gap > 0
                    && gap < book.tabs.len()
                    && book.tabs[gap - 1].tab_group_id.as_deref() == Some(id)
                    && book.tabs[gap].tab_group_id.as_deref() == Some(id)
            });
        if !within_source_group && !join {
            gap = book.groups.outside_gap(&book.tabs, gap);
        }
        let coordinate = entries
            .iter()
            .find(|e| {
                e.index >= gap && !(join && e.header && e.group == entry.group && e.index == gap)
            })
            .map(|e| e.offset)
            .unwrap_or_else(|| entries.last().map_or(0.0, |e| e.offset + e.extent + 2.0));
        TabDropGap {
            index: gap as i32,
            position: coordinate,
            group_id: if join {
                entry.group.as_deref().unwrap_or("").into()
            } else {
                "".into()
            },
            group_name: if join {
                entry
                    .group
                    .as_deref()
                    .and_then(|id| book.groups.get(id))
                    .map_or("", |g| g.name.as_str())
                    .into()
            } else {
                "".into()
            },
        }
    }
}

/// Rendered extent of a label, in logical pixels, for an 11 px sans-serif
/// interface font. Shared by every entry of the bar so one estimate cannot
/// drift from another.
fn estimate_text_width(text: &str) -> f32 {
    text.chars()
        .map(|c| match c {
            'i' | 'l' | 'j' | 't' | 'f' | 'r' | '.' | ',' | '\'' | '!' | ':' | ';' | '|' | ' ' => {
                3.4
            }
            'm' | 'w' | 'M' | 'W' | '@' => 10.0,
            c if c.is_ascii_uppercase() || c.is_ascii_digit() => 7.2,
            // CJK / full-width.
            c if (c as u32) >= 0x2E80 => 11.5,
            _ => 6.0,
        })
        .sum()
}

/// Imposed width of a tab, in logical pixels, estimated from its title.
/// The bridge is the bar's geometric source of truth: widths and
/// offsets are provided as plain values to the Slint model (`TabInfo`). The
/// drop-point computation therefore stays independent of the reactive layout and shares
/// the rendering's geometry. A wider title is simply elided by the `TabItem`.
pub(in crate::bridge) fn estimate_tab_width(title: &str) -> f32 {
    // TabItem's fixed chrome: padding-left 10 + spacing 6 + "×" slot 18 +
    // padding-right 4 = 38 px.
    const CHROME: f32 = 38.0;
    // Design bounds (e.g. TabItem's min/max-width).
    (CHROME + estimate_text_width(title)).clamp(90.0, 180.0)
}

/// Imposed width of a group header, estimated from its name and the number of
/// tabs it holds. A header carries no close button, so its floor is lower than
/// a tab's: a short name yields a short header instead of a half-empty one. The
/// ceiling is the tab ceiling, so a header can never outgrow the widest tab.
pub(in crate::bridge) fn estimate_group_width(name: &str, count: usize) -> f32 {
    // Header chrome: leading pad 4 + chevron chip 18 + gap 4 + gap before the
    // count 5 + trailing pad 8 = 39 px.
    const CHROME: f32 = 39.0;
    (CHROME + estimate_text_width(name) + estimate_text_width(&count.to_string()))
        .clamp(76.0, 180.0)
}

/// Gap reserved at each end of a group, against the 2 px pitch between
/// neighbouring entries.
const GROUP_BOUNDARY: f32 = 5.0;

/// Fixed vertical extent and spacing shared by the three bar projections.
const VTAB_EXTENT: f32 = 26.0;

/// Width, in logical pixels, of the vertical tab bar.
/// Uses the user-chosen width if positive, otherwise 40% of the
/// panel with a 180 px cap. The result stays between 110 px
/// and 60% of the panel. This computation must match `vbar-w` on the Slint side.
pub(in crate::bridge) fn vbar_width(panel_w: f32, user_w: f32) -> f32 {
    let base = if user_w > 0.0 {
        user_w
    } else {
        (panel_w * 0.40).min(180.0)
    };
    base.clamp(110.0, (panel_w * 0.60).max(110.0))
}

/// Reveals an entry or steps towards the next hidden entry along the bar axis.
/// Extents and offsets are the same values used to render the bar.
pub(in crate::bridge) fn scroll_target(
    geo: &[(f32, f32)],
    idx: i32,
    forward: bool,
    viewport_x: f32,
    view_w: f32,
) -> f32 {
    let Some(&(lw, lo)) = geo.last() else {
        return viewport_x;
    };
    let content_w = lo + lw;
    if content_w <= view_w + 0.5 {
        return 0.0; // no overflow
    }
    let max_scroll = view_w - content_w; // < 0 (right end)
    let vleft = -viewport_x; // visible left edge (content coords)
    let target = if idx >= 0 {
        // Reveal tab `idx`: overflowing right edge → align right; hidden
        // left edge → align left; otherwise already visible (unchanged).
        match geo.get(idx as usize) {
            Some(&(w, off)) if off + w > vleft + view_w => view_w - (off + w),
            Some(&(_w, off)) if off < vleft => -off,
            _ => viewport_x,
        }
    } else if forward {
        // 1st tab not entirely visible on the right → align its right edge.
        geo.iter()
            .find(|(w, off)| off + w > vleft + view_w + 0.5)
            .map(|(w, off)| view_w - (off + w))
            .unwrap_or(max_scroll)
    } else {
        // Last tab hidden on the left (just before the visible zone) → left edge.
        let mut t = 0.0_f32;
        for &(_w, off) in geo {
            if off < vleft - 0.5 {
                t = -off;
            } else {
                break;
            }
        }
        t
    };
    target.clamp(max_scroll, 0.0)
}

#[cfg(test)]
mod tests {
    use super::super::tests::sample;
    use super::*;

    #[test]
    #[ignore = "Manual timing diagnostic; no machine-dependent CI threshold"]
    fn grouped_bar_timing_diagnostic() {
        use std::{hint::black_box, time::Instant};
        for count in [40, 400] {
            for mode in 0..=2 {
                let mut book = sample(count, 0);
                for first in (0..count).step_by(10) {
                    book.groups
                        .create(
                            &mut book.tabs,
                            &mut book.active,
                            &(first..first + 5).collect::<Vec<_>>(),
                            &format!("Group {first:03}"),
                            2,
                        )
                        .unwrap();
                }
                let annotations = AnnotationStore::default();
                let projection = Presentation::default();
                projection.update(&book, mode, &annotations);
                let revision = projection.revision.get();
                let extent = projection
                    .model
                    .iter()
                    .last()
                    .map(|r| r.offset + r.width)
                    .unwrap();
                let started = Instant::now();
                for n in 0..10_000 {
                    black_box(projection.destination(
                        &book,
                        extent * (n % 1000) as f32 / 1000.0,
                        None,
                    ));
                }
                let hover = started.elapsed();
                assert_eq!(projection.revision.get(), revision);
                let started = Instant::now();
                for n in 0..100 {
                    book.click(1 + n % (count - 1), true, false);
                    projection.update(&book, mode, &annotations);
                }
                let selection = started.elapsed();
                assert_eq!(projection.revision.get(), revision);
                let group = book.groups.entries()[0].id.clone();
                let started = Instant::now();
                for n in 0..100 {
                    book.groups.set_collapsed(&group, n % 2 == 0).unwrap();
                    book.reconcile_selection();
                    projection.update(&book, mode, &annotations);
                }
                let collapse = started.elapsed();
                println!(
                    "tabs={count} mode={mode}: hover={:.2} us/query, selection={:.2} us/update, collapse={:.2} us/update",
                    hover.as_secs_f64() * 1e6 / 10_000.0,
                    selection.as_secs_f64() * 1e6 / 100.0,
                    collapse.as_secs_f64() * 1e6 / 100.0,
                );
            }
        }
    }

    #[test]
    fn collapsed_projection_hides_every_member_and_preserves_logical_indices() {
        for mode in 0..=2 {
            let mut book = sample(5, 2);
            let id = book
                .groups
                .create(&mut book.tabs, &mut book.active, &[1, 2, 3], "Group 01", 2)
                .unwrap();
            book.groups.set_collapsed(&id, true).unwrap();
            book.focus_group(&id);
            let projection = Presentation::default();
            projection.update(&book, mode, &AnnotationStore::default());
            let rows: Vec<_> = projection.model.iter().collect();
            assert_eq!(rows.iter().map(|r| r.index).collect::<Vec<_>>(), [0, -1, 4]);
            assert!(rows[1].is_group && rows[1].collapsed);
            assert_eq!(rows[1].count, 3);
            assert_eq!(projection.state(&book).count, 5);
            assert_eq!(projection.state(&book).active, -1);
            assert_eq!(book.active, 2);
            assert_eq!(
                projection
                    .gap(&book, rows[1].offset + rows[1].width - 1.0, None)
                    .index,
                4
            );
            assert_eq!(projection.gap(&book, rows[1].offset + 1.0, None).index, 1);
            assert_eq!(
                projection.scroll(3, true, -10.0, 80.0),
                projection.scroll(1, true, -10.0, 80.0)
            );
            let tail = &rows[2];
            assert_eq!(
                projection.scroll(4, true, 0.0, 80.0),
                80.0 - tail.offset - tail.width
            );
            assert!(book.select(3));
            projection.update(&book, mode, &AnnotationStore::default());
            assert_eq!(projection.model.row_count(), 6);
            assert!(!book.groups.get(&id).unwrap().collapsed);
        }
    }

    #[test]
    fn selection_and_group_color_leave_projection_geometry_and_folder_colors_unchanged() {
        let mut book = sample(4, 0);
        let id = book
            .groups
            .create(&mut book.tabs, &mut book.active, &[1, 2], "Group 01", 1)
            .unwrap();
        let projection = Presentation::default();
        let annotations = AnnotationStore::default();
        projection.update(&book, 0, &annotations);
        let revision = projection.state(&book).revision;
        let geometry: Vec<_> = projection
            .model
            .iter()
            .map(|r| (r.width, r.offset, r.folder_slot))
            .collect();
        book.click(1, true, false);
        book.groups.set_color(&id, 4).unwrap();
        projection.update(&book, 0, &annotations);
        assert_eq!(revision, projection.state(&book).revision);
        assert_eq!(
            geometry,
            projection
                .model
                .iter()
                .map(|r| (r.width, r.offset, r.folder_slot))
                .collect::<Vec<_>>()
        );
        assert!(
            projection
                .model
                .iter()
                .filter(|r| r.grouped)
                .all(|r| r.group_slot == 4)
        );
        book.groups.rename(&id, "Group 02").unwrap();
        projection.update(&book, 0, &annotations);
        assert_ne!(revision, projection.state(&book).revision);
    }

    #[test]
    fn resetting_a_selection_with_a_missing_folder_preserves_group_focus() {
        for mode in 0..=2 {
            let mut book = sample(3, 0);
            let group = book
                .groups
                .create(&mut book.tabs, &mut book.active, &[0, 1], "Group 01", 1)
                .unwrap();
            book.tabs[1].current_path = PathBuf::from("folder01/missing");
            book.select(0);
            book.click(1, true, false);
            let projection = Presentation::default();
            let annotations = AnnotationStore::default();
            projection.update(&book, mode, &annotations);
            assert_eq!(
                projection
                    .model
                    .iter()
                    .filter(|row| row.selection_marked)
                    .count(),
                2
            );
            book.reset_selection();
            projection.update(&book, mode, &annotations);
            assert!(projection.model.iter().all(|row| !row.selection_marked));
            assert!(
                projection
                    .model
                    .iter()
                    .any(|row| row.is_group && row.group_id == group && row.group_highlighted)
            );
            assert!(!book.groups.get(&group).unwrap().collapsed);
        }
    }

    #[test]
    fn external_drops_do_not_cut_groups_but_members_can_reorder_inside() {
        let mut book = sample(5, 0);
        book.groups
            .create(&mut book.tabs, &mut book.active, &[1, 2, 3], "Group 01", 0)
            .unwrap();
        let projection = Presentation::default();
        projection.update(&book, 0, &AnnotationStore::default());
        let member = projection.model.iter().find(|r| r.index == 2).unwrap();
        assert_eq!(projection.gap(&book, member.offset + 1.0, None).index, 4);
        assert_eq!(projection.gap(&book, member.offset + 1.0, Some(1)).index, 2);
        assert_eq!(projection.gap(&book, -50.0, None).index, 0);
        assert_eq!(projection.gap(&book, 9999.0, None).index, 5);
    }

    #[test]
    fn group_headers_and_internal_gaps_have_explicit_destinations_in_all_orientations() {
        for mode in 0..=2 {
            for collapsed in [false, true] {
                let mut book = super::super::tests::sample(5, 0);
                let id = book
                    .groups
                    .create(&mut book.tabs, &mut book.active, &[1, 2, 3], "Group 01", 2)
                    .unwrap();
                book.groups.set_collapsed(&id, collapsed).unwrap();
                let projection = Presentation::default();
                projection.update(&book, mode, &AnnotationStore::default());
                let header = projection.model.row_data(1).unwrap();
                let middle =
                    projection.destination(&book, header.offset + header.width / 2.0, None);
                assert_eq!(middle.index, 4);
                assert_eq!(middle.group_id, id);
                assert_eq!(middle.group_name, "Group 01");
                let before = projection.destination(&book, header.offset + 1.0, None);
                assert_eq!(before.index, 1);
                assert!(before.group_id.is_empty());
                let after = projection.destination(&book, header.offset + header.width - 1.0, None);
                assert_eq!(after.index, 4);
                assert!(after.group_id.is_empty());
                if !collapsed {
                    let first_member = projection.model.row_data(2).unwrap();
                    let gap = projection.destination(&book, first_member.offset + 1.0, None);
                    assert_eq!(gap.index, 1);
                    assert_eq!(gap.group_id, id);
                    assert_eq!(gap.position, first_member.offset);
                    let member = projection.model.row_data(3).unwrap();
                    let gap = projection.destination(&book, member.offset + 1.0, None);
                    assert_eq!(gap.index, 2);
                    assert_eq!(gap.group_id, id);
                }
                let end = projection.destination(&book, 9999.0, None);
                assert_eq!(end.index, 5);
                assert!(end.group_id.is_empty());
                // A group dragged as a whole finds no membership to take over
                // its own span, header and members alike, since it cannot be
                // dropped into itself. Refused membership sends the gap back
                // outside the group, where such a drop is a plain reorder.
                // Any other group still joins at the very same spots.
                let mut spots = vec![header.offset + header.width / 2.0];
                if !collapsed {
                    spots.push(projection.model.row_data(3).unwrap().offset + 1.0);
                }
                let own_batch = super::super::batch::Batch::capture(&book, 0, Some(&id)).unwrap();
                let mut other_book = sample(4, 0);
                let other_id = other_book
                    .groups
                    .create(
                        &mut other_book.tabs,
                        &mut other_book.active,
                        &[0, 1],
                        "Group 02",
                        1,
                    )
                    .unwrap();
                // Group identifiers are local to each book and may coincide.
                assert_eq!(other_id, id);
                let other_batch =
                    super::super::batch::Batch::capture(&other_book, 0, Some(&other_id)).unwrap();
                for position in spots {
                    let free = projection.destination(&book, position, None);
                    assert_eq!(free.group_id, id);
                    let own = projection.destination(&book, position, Some(&own_batch));
                    assert!(own.group_id.is_empty() && own.group_name.is_empty());
                    assert!(own.index <= 1 || own.index >= 4, "{}", own.index);
                    let other = projection.destination(&book, position, Some(&other_batch));
                    assert_eq!(other.group_id, id);
                    assert_eq!(other.index, free.index);
                }
            }
        }
    }

    #[test]
    fn tab_width_is_clamped_and_monotonic() {
        // Short title → min bound (90); very long → max bound (180).
        assert_eq!(estimate_tab_width("~"), 90.0);
        assert_eq!(
            estimate_tab_width("a-really-endless-folder-name-that-keeps-going"),
            180.0
        );
        // A longer title is never NARROWER (monotonicity).
        let w1 = estimate_tab_width("folder_items");
        let w2 = estimate_tab_width("folder_items_01");
        assert!(w2 >= w1, "{w2} >= {w1}");
        // Intermediate zone: well within the bounds.
        assert!(w1 > 90.0 && w1 < 180.0, "{w1}");
    }

    #[test]
    fn group_headers_hug_short_names_and_share_the_tab_ceiling() {
        // A header carries no close button, so a short name yields a short
        // header instead of the half-empty one a tab's floor would impose.
        assert_eq!(estimate_group_width("ab", 2), 76.0);
        assert!(estimate_group_width("ab", 2) < estimate_tab_width("ab"));
        // The count takes part in the estimate: it is drawn, so it is measured.
        assert!(
            estimate_group_width("folder_group", 100) > estimate_group_width("folder_group", 1)
        );
        // Same ceiling as a tab: a header never outgrows the widest tab.
        assert_eq!(
            estimate_group_width("a-really-endless-group-name-that-keeps-going", 9),
            estimate_tab_width("a-really-endless-folder-name-that-keeps-going")
        );
    }

    #[test]
    fn stepping_enters_a_collapsed_group_once_without_opening_it() {
        for mode in 0..=2 {
            let mut book = sample(5, 0);
            let id = book
                .groups
                .create(&mut book.tabs, &mut book.active, &[1, 2, 3], "Group 01", 2)
                .unwrap();
            let projection = Presentation::default();
            projection.update(&book, mode, &AnnotationStore::default());
            // Unfolded, every tab is a destination and both ends wrap around.
            assert_eq!(projection.step(&book, true), 1);
            assert_eq!(projection.step(&book, false), 4);
            // A collapsed group is addressed through its first logical index.
            book.groups.set_collapsed(&id, true).unwrap();
            book.reconcile_selection();
            projection.update(&book, mode, &AnnotationStore::default());
            assert_eq!(projection.step(&book, true), 1);
            assert_eq!(projection.step(&book, false), 4);
            assert!(book.groups.get(&id).unwrap().collapsed);
            // A retained member inside a collapsed group occupies the group's
            // position in the navigation order, without appearing in the bar.
            let mut book = sample(5, 2);
            let id = book
                .groups
                .create(&mut book.tabs, &mut book.active, &[1, 2, 3], "Group 01", 2)
                .unwrap();
            book.groups.set_collapsed(&id, true).unwrap();
            book.reconcile_selection();
            let projection = Presentation::default();
            projection.update(&book, mode, &AnnotationStore::default());
            assert_eq!(projection.step(&book, true), 4);
            assert_eq!(projection.step(&book, false), 0);
            assert!(book.groups.get(&id).unwrap().collapsed);
        }
    }

    #[test]
    fn group_focus_survives_selection_changes_and_follows_tab_activation() {
        for mode in 0..=2 {
            let mut book = sample(6, 4);
            let first = book
                .groups
                .create(&mut book.tabs, &mut book.active, &[0, 1], "Group 01", 1)
                .unwrap();
            let second = book
                .groups
                .create(&mut book.tabs, &mut book.active, &[2, 3], "Group 02", 2)
                .unwrap();
            let projection = Presentation::default();
            let check = |book: &TabBook, expected: Option<&str>| {
                projection.update(book, mode, &AnnotationStore::default());
                let highlighted: Vec<_> = projection
                    .model
                    .iter()
                    .filter(|row| row.group_highlighted)
                    .map(|row| row.group_id)
                    .collect();
                assert!(highlighted.len() <= 1);
                assert_eq!(highlighted.first().map(|id| id.as_str()), expected);
            };
            check(&book, None);
            book.select(0);
            check(&book, Some(&first));
            book.select_from_wheel(2);
            check(&book, Some(&second));
            // Targeting a header does not navigate to one of its folders.
            book.highlight_group(&first);
            check(&book, Some(&first));
            assert_eq!(book.active, 2);
            assert!(!book.reset_selection());
            assert!(!book.reset_selection());
            check(&book, Some(&first));
            assert_eq!(book.active, 2);
            // Re-clicking the active member restores its group's outline.
            assert!(!book.click(2, false, false));
            check(&book, Some(&second));
            book.click(4, false, false);
            check(&book, None);
            book.highlight_group(&first);
            book.click(0, true, false);
            check(&book, Some(&first));
            assert_eq!(book.active, 4);
            book.click(3, false, true);
            check(&book, Some(&first));
            assert!(book.select_group(&first));
            check(&book, Some(&first));
            book.groups.rename(&first, "Group 03").unwrap();
            check(&book, Some(&first));
            let members = [book.tabs[0].id, book.tabs[1].id];
            assert!(book.move_tabs(&members, 6));
            check(&book, Some(&first));
            book.groups.dissolve(&mut book.tabs, &first).unwrap();
            book.reconcile_selection();
            check(&book, None);
            book.select(0);
            check(&book, Some(&second));
            book.close(0).unwrap();
            check(&book, Some(&second));
            book.close(0).unwrap();
            check(&book, None);
        }
    }

    #[test]
    fn wheel_navigation_cycles_group_destinations_and_preserves_normal_selection() {
        for mode in 0..=2 {
            let mut book = sample(8, 7);
            let first = book
                .groups
                .create(&mut book.tabs, &mut book.active, &[0, 1, 2], "Group 01", 1)
                .unwrap();
            let second = book
                .groups
                .create(&mut book.tabs, &mut book.active, &[3, 4], "Group 02", 2)
                .unwrap();
            book.groups
                .create(&mut book.tabs, &mut book.active, &[5, 6], "Group 03", 3)
                .unwrap();
            book.groups.set_collapsed(&first, true).unwrap();
            book.groups.set_collapsed(&second, true).unwrap();
            let projection = Presentation::default();
            projection.update(&book, mode, &AnnotationStore::default());
            for (forward, destinations) in
                [(true, vec![0, 3, 5, 6, 7, 0]), (false, vec![7, 6, 5, 3, 0])]
            {
                for expected in destinations {
                    let previous = book.active;
                    let next = projection.step(&book, forward) as usize;
                    assert_eq!(next, expected, "mode {mode}, forward {forward}");
                    assert!(book.select_from_wheel(next));
                    projection.update(&book, mode, &AnnotationStore::default());
                    assert!(book.groups.get(&first).unwrap().collapsed);
                    assert!(book.groups.get(&second).unwrap().collapsed);
                    assert_eq!(
                        book.selection_len(),
                        usize::from(book.group_page().is_none())
                    );
                    if book.group_page().is_some() {
                        assert_eq!(book.active, previous);
                        assert_eq!(projection.state(&book).active, -1);
                    }
                    for header in projection.model.iter().filter(|row| row.is_group) {
                        let active_group = book.focused_group.as_deref();
                        assert_eq!(
                            header.group_highlighted,
                            active_group == Some(header.group_id.as_str())
                        );
                    }
                }
            }
            // Ordinary tab selection still unfolds the group it enters.
            assert!(book.select(2));
            assert!(!book.groups.get(&first).unwrap().collapsed);
        }
        let mut book = sample(3, 2);
        let id = book
            .groups
            .create(&mut book.tabs, &mut book.active, &[0, 1, 2], "Group 01", 1)
            .unwrap();
        book.groups.set_collapsed(&id, true).unwrap();
        let projection = Presentation::default();
        projection.update(&book, 0, &AnnotationStore::default());
        assert_eq!(projection.step(&book, false), 0);
        assert!(book.select_from_wheel(0));
        projection.update(&book, 0, &AnnotationStore::default());
        assert_eq!(projection.step(&book, true), 0);
        assert!(!book.select_from_wheel(0));
        assert!(book.groups.get(&id).unwrap().collapsed);
    }

    #[test]
    fn revealing_a_collapsed_group_targets_its_header_without_a_child() {
        for mode in 0..=2 {
            let mut book = sample(5, 4);
            let id = book
                .groups
                .create(&mut book.tabs, &mut book.active, &[0, 1], "Group 01", 1)
                .unwrap();
            book.groups.set_collapsed(&id, true).unwrap();
            book.select_from_wheel(0);
            let projection = Presentation::default();
            projection.update(&book, mode, &AnnotationStore::default());
            let rows: Vec<_> = projection.model.iter().collect();
            let header = &rows[0];
            let span = header.width;
            let last = rows.last().unwrap();
            let end = last.offset + last.width;
            assert!(header.is_group && header.group_highlighted);
            assert!(rows.iter().all(|row| row.is_group || row.index > 1));
            assert_eq!(projection.state(&book).group_index, 0);
            assert_eq!(projection.scroll(0, true, span - end, span), 0.0);
            assert_eq!(
                projection.scroll(0, false, header.width - end, header.width),
                0.0
            );
            book.groups.set_collapsed(&id, false).unwrap();
            projection.update(&book, mode, &AnnotationStore::default());
            let child = projection.model.iter().find(|r| r.index == 0).unwrap();
            assert_eq!(
                projection.scroll(0, true, -end, child.offset + child.width),
                0.0
            );
        }
    }

    #[test]
    fn group_boundaries_are_padded_and_the_rail_closes_on_the_last_visible_entry() {
        for mode in 0..=2 {
            let mut book = sample(5, 0);
            let id = book
                .groups
                .create(&mut book.tabs, &mut book.active, &[1, 2], "Group 01", 2)
                .unwrap();
            let projection = Presentation::default();
            projection.update(&book, mode, &AnnotationStore::default());
            let rows: Vec<_> = projection.model.iter().collect();
            // One header plus the five tabs.
            assert_eq!(rows.len(), 6);
            // Only the group's two ends are padded, and exactly one entry
            // closes the rail.
            assert_eq!(
                rows.iter()
                    .map(|r| (r.pad_lead, r.pad_trail, r.rail_tail))
                    .collect::<Vec<_>>(),
                [
                    (0.0, 0.0, false),
                    (5.0, 0.0, false),
                    (0.0, 0.0, false),
                    (0.0, 5.0, true),
                    (0.0, 0.0, false),
                    (0.0, 0.0, false),
                ]
            );
            // The gap is reserved inside the entry's own extent, so offsets
            // still accumulate at the single pitch the bar's layout applies.
            for pair in rows.windows(2) {
                assert_eq!(pair[1].offset, pair[0].offset + pair[0].width + 2.0);
            }
            // Collapsed with its active tab outside it, the group is reduced to
            // its header, which then carries both ends of the rail.
            book.groups.set_collapsed(&id, true).unwrap();
            book.reconcile_selection();
            projection.update(&book, mode, &AnnotationStore::default());
            let header = projection.model.row_data(1).unwrap();
            assert!(header.is_group && header.grouped && header.rail_tail);
            assert_eq!((header.pad_lead, header.pad_trail), (5.0, 5.0));
        }
    }

    #[test]
    fn tab_layout_accumulates_offsets_with_spacing() {
        let book = super::super::tests::sample(3, 0);
        let projection = Presentation::default();
        projection.update(&book, 0, &AnnotationStore::default());
        let geo: Vec<_> = projection
            .model
            .iter()
            .map(|t| (t.width, t.offset))
            .collect();
        assert_eq!(geo.len(), 3);
        // First tab at offset 0.
        assert_eq!(geo[0].1, 0.0);
        // Each offset = previous offset + previous width + 2 spacing.
        assert_eq!(geo[1].1, geo[0].1 + geo[0].0 + 2.0);
        assert_eq!(geo[2].1, geo[1].1 + geo[1].0 + 2.0);
        // All widths within the design bounds.
        assert!(geo.iter().all(|(w, _)| (90.0..=180.0).contains(w)));
    }

    #[test]
    fn both_vertical_bars_keep_the_same_fixed_pitch() {
        let book = super::super::tests::sample(3, 0);
        let projection = Presentation::default();
        for mode in [1, 2] {
            projection.update(&book, mode, &AnnotationStore::default());
            assert_eq!(
                projection
                    .model
                    .iter()
                    .map(|t| (t.width, t.offset))
                    .collect::<Vec<_>>(),
                [(26.0, 0.0), (26.0, 28.0), (26.0, 56.0)]
            );
        }
        projection.update(&book, 0, &AnnotationStore::default());
        assert!(projection.model.iter().all(|t| t.width >= 90.0));
    }

    #[test]
    fn tab_scrolling_reveals_entries_and_steps_at_the_same_boundaries() {
        let geo = [(90.0, 0.0), (110.0, 92.0), (90.0, 204.0)];
        assert_eq!(scroll_target(&geo, 0, true, 0.0, 160.0), 0.0);
        assert_eq!(scroll_target(&geo, 1, true, 0.0, 160.0), -42.0);
        assert_eq!(scroll_target(&geo, 2, true, -42.0, 160.0), -134.0);
        assert_eq!(scroll_target(&geo, 1, false, -134.0, 160.0), -92.0);
        assert_eq!(scroll_target(&geo, 0, false, -134.0, 160.0), 0.0);
        assert_eq!(scroll_target(&geo, -1, true, 0.0, 160.0), -42.0);
        assert_eq!(scroll_target(&geo, -1, false, -134.0, 160.0), -92.0);
        assert_eq!(scroll_target(&geo, 9, true, -42.0, 160.0), -42.0);
        assert_eq!(scroll_target(&geo, 2, true, -42.0, 400.0), 0.0);
        assert_eq!(scroll_target(&[], 0, true, -42.0, 160.0), -42.0);
    }
}
