//! Panel layout tree and nested split management.
//!
//! Favnyr displays an arbitrary number of panels organized into nested
//! horizontal/vertical splits, VSCode-style. Slint cannot instantiate
//! components **recursively**, so the representation is kept separate from
//! rendering:
//!
//!  1. the layout lives here, in Rust, as a **binary tree** ([`LayoutNode`])
//!     — easy to manipulate (split / merge / resize);
//!  2. at render time, the tree is **flattened** into a list of absolutely
//!     positioned rectangles ([`Layout`]) that the GUI places directly. This
//!     approach avoids any recursion in Slint repeaters.
//!
//! **Leaves** reference a panel by index into
//! [`crate::workspace::WorkspaceState::panels`]: the tree describes *where*
//! each panel goes, while the `Vec<PanelState>` describes *what* each panel
//! contains. This separation lets the GUI consume a flat list of panels.

use serde::{Deserialize, Serialize};

/// Two ratios closer than this are the same ratio: past a pixel or so of a
/// panel, a difference no one can see and no one asked for.
const RATIO_EPSILON: f32 = 1e-4;

/// Orientation of a split.
///
/// - [`SplitDir::Row`]: children **side by side** (**vertical** separator) —
///   this is the UI's "◧ Side by side".
/// - [`SplitDir::Column`]: children **stacked** (**horizontal** separator) —
///   this is the "⊟ One below another".
///
/// We deliberately ban "horizontal / vertical" from the exposed vocabulary
/// (ambiguous); these internal names describe the **axis along which
/// children are arranged**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitDir {
    /// Children arranged horizontally (left → right).
    Row,
    /// Children stacked vertically (top → bottom).
    Column,
}

/// Node of the layout tree (recursive, **in-memory** representation).
///
/// Directly serializable to TOML: the scalar fields (`dir`, `ratio`) are
/// declared **before** the subtrees (`first`, `second`) to comply with the
/// TOML "values before tables" rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LayoutNode {
    /// Leaf: an actual panel, designated by its index in `panels`.
    Leaf { panel: usize },
    /// Internal node: two children separated according to `dir`, `ratio`
    /// being the share (∈ ]0,1[) allotted to the **first** child.
    Split {
        dir: SplitDir,
        ratio: f32,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

/// Step of a leaf's `panel` index during a traversal (see [`Side`]).
///
/// Path to a node from the root: a sequence of [`Side`]. The root has an
/// empty path.
pub type NodePath = Vec<Side>;

/// Side of a split, used to designate a child within a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// `first` child (left or top).
    First,
    /// `second` child (right or bottom).
    Second,
}

/// Rectangle in pixels (absolute coordinates within the panel container).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Geometry of a leaf panel after flattening.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelGeom {
    /// Index of the panel in `panels`.
    pub panel: usize,
    pub rect: Rect,
}

/// Geometry of a separator (resize handle) after flattening. `path`
/// identifies the corresponding `Split` node, to adjust its `ratio` during a
/// resize drag; `area` is the **total** area that this split divides
/// (needed to convert a pointer position into a ratio local to this split).
#[derive(Debug, Clone, PartialEq)]
pub struct SplitterGeom {
    pub path: NodePath,
    pub dir: SplitDir,
    pub rect: Rect,
    pub area: Rect,
}

/// Result of flattening a [`LayoutNode`] over a given area.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub panels: Vec<PanelGeom>,
    pub splitters: Vec<SplitterGeom>,
}

impl LayoutNode {
    /// Builds a default tree from a flat list of panels and their
    /// `stretch`: a left-leaning **chain of `Row` splits**, with all panels
    /// side by side. This shape is used as a fallback when a workspace
    /// doesn't contain an explicit tree.
    pub fn row_chain(stretches: &[f32]) -> LayoutNode {
        fn build(idx: usize, stretches: &[f32]) -> LayoutNode {
            let n = stretches.len();
            if idx + 1 >= n {
                return LayoutNode::Leaf { panel: idx };
            }
            let remaining: f32 = stretches[idx..].iter().filter(|s| s.is_finite()).sum();
            let here = stretches[idx].max(0.0);
            let ratio = if remaining > 0.0 {
                (here / remaining).clamp(0.05, 0.95)
            } else {
                0.5
            };
            LayoutNode::Split {
                dir: SplitDir::Row,
                ratio,
                first: Box::new(LayoutNode::Leaf { panel: idx }),
                second: Box::new(build(idx + 1, stretches)),
            }
        }
        if stretches.is_empty() {
            return LayoutNode::Leaf { panel: 0 };
        }
        build(0, stretches)
    }

    /// Panel indices of the leaves, in infix traversal order.
    pub fn leaf_indices(&self) -> Vec<usize> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<usize>) {
        match self {
            LayoutNode::Leaf { panel } => out.push(*panel),
            LayoutNode::Split { first, second, .. } => {
                first.collect_leaves(out);
                second.collect_leaves(out);
            }
        }
    }

    /// Swaps the PLACES of two views: the leaves holding `a` and `b` exchange
    /// their panel index.
    ///
    /// Nothing else moves — not the shape of the tree, not a single ratio, not
    /// the panels themselves. Each view therefore takes the size of its new
    /// place, which is exactly what swapping two tiles means, and everything
    /// keyed by panel index (the active view, its tabs, its history) stays
    /// valid because no index is created or destroyed.
    ///
    /// Refused, with nothing written, when the two are the same or when either
    /// has no leaf: a half-applied swap would leave the tree holding one index
    /// twice, which `is_valid_for` would then reject.
    pub fn swap_panels(&mut self, a: usize, b: usize) -> bool {
        if a == b {
            return false;
        }
        let leaves = self.leaf_indices();
        if !leaves.contains(&a) || !leaves.contains(&b) {
            return false;
        }
        self.each_leaf_mut(&mut |panel| {
            if *panel == a {
                *panel = b;
            } else if *panel == b {
                *panel = a;
            }
        });
        true
    }

    fn each_leaf_mut(&mut self, visit: &mut impl FnMut(&mut usize)) {
        match self {
            LayoutNode::Leaf { panel } => visit(panel),
            LayoutNode::Split { first, second, .. } => {
                first.each_leaf_mut(visit);
                second.each_leaf_mut(visit);
            }
        }
    }

    /// True if the tree covers **exactly** the panels `0..panel_count`, each
    /// exactly once (no missing, duplicated, or out-of-bounds index). Used
    /// as a safeguard before using a tree loaded from disk.
    pub fn is_valid_for(&self, panel_count: usize) -> bool {
        if panel_count == 0 {
            return false;
        }
        let mut seen = vec![false; panel_count];
        for i in self.leaf_indices() {
            if i >= panel_count || seen[i] {
                return false;
            }
            seen[i] = true;
        }
        seen.iter().all(|&b| b)
    }

    /// Mutable reference to the node designated by `path` (root if empty).
    pub fn node_at_mut(&mut self, path: &[Side]) -> Option<&mut LayoutNode> {
        let mut node = self;
        for side in path {
            match node {
                LayoutNode::Split { first, second, .. } => {
                    node = match side {
                        Side::First => first,
                        Side::Second => second,
                    };
                }
                LayoutNode::Leaf { .. } => return None,
            }
        }
        Some(node)
    }

    /// Adjusts the `ratio` of the split located at `path`. No-op if the path
    /// doesn't point to a `Split`. The value is clamped to `[min, 1 - min]`.
    pub fn set_ratio(&mut self, path: &[Side], ratio: f32, min: f32) -> bool {
        if let Some(LayoutNode::Split { ratio: r, .. }) = self.node_at_mut(path) {
            *r = ratio.clamp(min, 1.0 - min);
            true
        } else {
            false
        }
    }

    /// Number of tracks the subtree occupies along `dir`: how many columns it
    /// shows across a [`SplitDir::Row`], how many bands down a
    /// [`SplitDir::Column`].
    ///
    /// A split ALONG `dir` puts its children end to end, so their tracks add
    /// up; a split ACROSS it lays them over the same tracks, so the wider of
    /// the two decides. A leaf is one track.
    fn tracks(&self, dir: SplitDir) -> usize {
        match self {
            LayoutNode::Leaf { .. } => 1,
            LayoutNode::Split {
                dir: d,
                first,
                second,
                ..
            } => {
                let (a, b) = (first.tracks(dir), second.tracks(dir));
                if *d == dir { a + b } else { a.max(b) }
            }
        }
    }

    /// Gives every track the same size, recursively.
    ///
    /// Each split hands a side the room its tracks are worth, separators
    /// included, so the panels come out the same WIDTH across a row and the
    /// same HEIGHT down a column. Two panels side by side next to a third
    /// therefore take a third each, not a half and two quarters — and the
    /// result does not depend on which way the tree happens to lean, which the
    /// user cannot see.
    ///
    /// Where a row and a column cross, the track count of the crossing side is
    /// the widest of its bands, so the sizing is as even as that shape allows.
    fn spread(&mut self, area: Rect, gap: f32, min: f32) {
        let LayoutNode::Split {
            dir,
            ratio,
            first,
            second,
        } = self
        else {
            return;
        };
        let d = *dir;
        let n1 = first.tracks(d) as f32;
        let n = n1 + second.tracks(d) as f32;
        let span = match d {
            SplitDir::Row => area.w,
            SplitDir::Column => area.h,
        };
        // Room the first side needs for every track in `area` to come out the
        // same size. Each of the `n` tracks gets an equal share of what the
        // `n - 1` separators leave, and the first side also keeps the
        // separators that fall inside it — which reduces to this.
        let first_span = n1 / n * (span + gap) - gap;
        let avail = span - gap;
        if avail > f32::EPSILON {
            *ratio = (first_span / avail).clamp(min, 1.0 - min);
        }
        let (a1, _, a2) = split_area(area, d, *ratio, gap);
        first.spread(a1, gap, min);
        second.spread(a2, gap, min);
    }

    /// Ratios of the subtree, a node before its children — a fixed order, so a
    /// copy taken now can be written back later.
    fn collect_ratios(&self, out: &mut Vec<f32>) {
        if let LayoutNode::Split {
            ratio,
            first,
            second,
            ..
        } = self
        {
            out.push(*ratio);
            first.collect_ratios(out);
            second.collect_ratios(out);
        }
    }

    fn write_ratios(&mut self, next: &mut impl Iterator<Item = f32>) {
        if let LayoutNode::Split {
            ratio,
            first,
            second,
            ..
        } = self
        {
            if let Some(r) = next.next() {
                *ratio = r;
            }
            first.write_ratios(next);
            second.write_ratios(next);
        }
    }

    /// Reference to the node designated by `path` (root if empty).
    fn node_at(&self, path: &[Side]) -> Option<&LayoutNode> {
        let mut node = self;
        for side in path {
            match node {
                LayoutNode::Split { first, second, .. } => {
                    node = match side {
                        Side::First => first,
                        Side::Second => second,
                    };
                }
                LayoutNode::Leaf { .. } => return None,
            }
        }
        Some(node)
    }

    /// Ratios of the subtree at `path`, in [`Self::collect_ratios`] order.
    /// Empty for a leaf or an invalid path.
    pub fn ratios(&self, path: &[Side]) -> Vec<f32> {
        let mut out = Vec::new();
        if let Some(node) = self.node_at(path) {
            node.collect_ratios(&mut out);
        }
        out
    }

    /// Evens out the subtree at `path` over `area`: every panel it holds ends
    /// up the same width across a row, the same height down a column.
    ///
    /// The root path evens out the whole tree; any other evens out only what
    /// that separator governs, leaving hand-set proportions elsewhere alone.
    ///
    /// `area` is the rectangle THAT SUBTREE occupies — the `area` field the
    /// flattening already records on every [`SplitterGeom`]. It is needed
    /// because separators take absolute pixels while ratios are relative: only
    /// with both can the tracks come out truly equal.
    ///
    /// Returns the ratios as they stood BEFORE **and** the ones written in
    /// their place, or `None` when nothing moved — there is then nothing to
    /// undo. Both are handed back together on purpose: a caller holding the
    /// tree through a lock would otherwise have to reach for it a second time
    /// just to read what it had itself just written.
    pub fn equalize(
        &mut self,
        path: &[Side],
        area: Rect,
        gap: f32,
        min: f32,
    ) -> Option<(Vec<f32>, Vec<f32>)> {
        let before = self.ratios(path);
        let node = self.node_at_mut(path)?;
        node.spread(area, gap, min);
        let mut after = Vec::new();
        node.collect_ratios(&mut after);
        let moved = before
            .iter()
            .zip(&after)
            .any(|(a, b)| (a - b).abs() > RATIO_EPSILON);
        moved.then_some((before, after))
    }

    /// Writes ratios back into the subtree at `path` — the way back from
    /// [`Self::equalize`].
    ///
    /// Refused, and nothing written, when the subtree no longer holds the same
    /// number of splits: the layout changed under the copy, and a half-applied
    /// restore would be worse than none.
    pub fn restore_ratios(&mut self, path: &[Side], ratios: &[f32]) -> bool {
        let Some(node) = self.node_at_mut(path) else {
            return false;
        };
        let mut here = Vec::new();
        node.collect_ratios(&mut here);
        if here.len() != ratios.len() {
            return false;
        }
        node.write_ratios(&mut ratios.iter().copied());
        true
    }

    /// Splits the leaf of panel `panel`: it becomes a `Split`.
    /// `new_first = false` → the existing panel stays first (left/top) and
    /// `new_panel` comes second (right/bottom); `new_first = true` → the
    /// reverse (for a drop on the west/north edge). Returns `false` if no
    /// leaf `panel` exists. First match only (indices are unique).
    pub fn split_leaf(
        &mut self,
        panel: usize,
        dir: SplitDir,
        new_panel: usize,
        ratio: f32,
        new_first: bool,
    ) -> bool {
        match self {
            LayoutNode::Leaf { panel: p } if *p == panel => {
                let kept = LayoutNode::Leaf { panel: *p };
                let fresh = LayoutNode::Leaf { panel: new_panel };
                let (first, second, r) = if new_first {
                    // The new panel takes the `ratio` share (first child).
                    (fresh, kept, ratio)
                } else {
                    (kept, fresh, ratio)
                };
                *self = LayoutNode::Split {
                    dir,
                    ratio: r.clamp(0.05, 0.95),
                    first: Box::new(first),
                    second: Box::new(second),
                };
                true
            }
            LayoutNode::Leaf { .. } => false,
            LayoutNode::Split { first, second, .. } => {
                first.split_leaf(panel, dir, new_panel, ratio, new_first)
                    || second.split_leaf(panel, dir, new_panel, ratio, new_first)
            }
        }
    }

    /// Removes the leaf of panel `idx`: its parent `Split` is replaced by
    /// the sibling leaf (merge), then all leaf indices `> idx` are
    /// **decremented** to stay consistent with the caller removing
    /// `panels[idx]`. Returns `false` if the tree is reduced to a single
    /// leaf (the last panel is never removed).
    pub fn remove_panel(&mut self, idx: usize) -> bool {
        if matches!(self, LayoutNode::Leaf { .. }) {
            return false;
        }
        if Self::collapse_leaf(self, idx) {
            self.reindex_after_removal(idx);
            true
        } else {
            false
        }
    }

    fn collapse_leaf(node: &mut LayoutNode, idx: usize) -> bool {
        // If one of the direct children is the target leaf, this node
        // becomes the other child.
        let replacement = match node {
            LayoutNode::Split { first, second, .. } => {
                if matches!(**first, LayoutNode::Leaf { panel } if panel == idx) {
                    Some(std::mem::replace(
                        &mut **second,
                        LayoutNode::Leaf { panel: 0 },
                    ))
                } else if matches!(**second, LayoutNode::Leaf { panel } if panel == idx) {
                    Some(std::mem::replace(
                        &mut **first,
                        LayoutNode::Leaf { panel: 0 },
                    ))
                } else {
                    None
                }
            }
            LayoutNode::Leaf { .. } => return false,
        };
        if let Some(repl) = replacement {
            *node = repl;
            return true;
        }
        // Otherwise, descend into the subtrees.
        if let LayoutNode::Split { first, second, .. } = node {
            Self::collapse_leaf(first, idx) || Self::collapse_leaf(second, idx)
        } else {
            false
        }
    }

    fn reindex_after_removal(&mut self, removed: usize) {
        match self {
            LayoutNode::Leaf { panel } => {
                if *panel > removed {
                    *panel -= 1;
                }
            }
            LayoutNode::Split { first, second, .. } => {
                first.reindex_after_removal(removed);
                second.reindex_after_removal(removed);
            }
        }
    }

    /// Flattens the tree over the area `area`, reserving `gap` pixels for
    /// each separator. Returns the absolute position of each panel and each
    /// handle.
    pub fn compute(&self, area: Rect, gap: f32) -> Layout {
        let mut out = Layout::default();
        let mut path = Vec::new();
        self.compute_into(area, gap, &mut path, &mut out);
        out
    }

    fn compute_into(&self, area: Rect, gap: f32, path: &mut NodePath, out: &mut Layout) {
        match self {
            LayoutNode::Leaf { panel } => out.panels.push(PanelGeom {
                panel: *panel,
                rect: area,
            }),
            LayoutNode::Split {
                dir,
                ratio,
                first,
                second,
            } => {
                let (a1, sep, a2) = split_area(area, *dir, *ratio, gap);
                out.splitters.push(SplitterGeom {
                    path: path.clone(),
                    dir: *dir,
                    rect: sep,
                    area,
                });
                path.push(Side::First);
                first.compute_into(a1, gap, path, out);
                path.pop();
                path.push(Side::Second);
                second.compute_into(a2, gap, path, out);
                path.pop();
            }
        }
    }
}

/// Cuts `area` in two along `dir`, reserving `gap` for the separator between
/// the halves. Returns the first child's rectangle, the separator's, and the
/// second child's.
///
/// Shared by the flattening and by the equalizer: both need to know where a
/// child actually lands, and two copies of this arithmetic would eventually
/// disagree.
fn split_area(area: Rect, dir: SplitDir, ratio: f32, gap: f32) -> (Rect, Rect, Rect) {
    let r = ratio.clamp(0.0, 1.0);
    match dir {
        SplitDir::Row => {
            let avail = (area.w - gap).max(0.0);
            let w1 = avail * r;
            (
                Rect { w: w1, ..area },
                Rect {
                    x: area.x + w1,
                    y: area.y,
                    w: gap,
                    h: area.h,
                },
                Rect {
                    x: area.x + w1 + gap,
                    w: avail - w1,
                    ..area
                },
            )
        }
        SplitDir::Column => {
            let avail = (area.h - gap).max(0.0);
            let h1 = avail * r;
            (
                Rect { h: h1, ..area },
                Rect {
                    x: area.x,
                    y: area.y + h1,
                    w: area.w,
                    h: gap,
                },
                Rect {
                    y: area.y + h1 + gap,
                    h: avail - h1,
                    ..area
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 0.5, "expected ~{b}, got {a}");
    }

    fn area() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 600.0,
        }
    }

    fn leaf(panel: usize) -> LayoutNode {
        LayoutNode::Leaf { panel }
    }

    fn split(dir: SplitDir, ratio: f32, first: LayoutNode, second: LayoutNode) -> LayoutNode {
        LayoutNode::Split {
            dir,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// Panel sizes over the reference area, indexed by panel number.
    fn sizes(tree: &LayoutNode, gap: f32) -> Vec<(f32, f32)> {
        let l = tree.compute(area(), gap);
        let mut out = vec![(0.0, 0.0); l.panels.len()];
        for p in &l.panels {
            out[p.panel] = (p.rect.w, p.rect.h);
        }
        out
    }

    /// The area a given separator governs, as the GUI reads it.
    fn area_of(tree: &LayoutNode, path: &[Side], gap: f32) -> Rect {
        tree.compute(area(), gap)
            .splitters
            .iter()
            .find(|s| s.path == path)
            .expect("separator at that path")
            .area
    }

    #[test]
    fn single_leaf_fills_area() {
        let tree = LayoutNode::Leaf { panel: 0 };
        let l = tree.compute(area(), 8.0);
        assert_eq!(l.panels.len(), 1);
        assert!(l.splitters.is_empty());
        assert_eq!(l.panels[0].rect, area());
    }

    #[test]
    fn row_split_halves_minus_gap() {
        let tree = LayoutNode::Split {
            dir: SplitDir::Row,
            ratio: 0.5,
            first: Box::new(LayoutNode::Leaf { panel: 0 }),
            second: Box::new(LayoutNode::Leaf { panel: 1 }),
        };
        let l = tree.compute(area(), 8.0);
        assert_eq!(l.panels.len(), 2);
        assert_eq!(l.splitters.len(), 1);
        // avail = 992, half = 496
        approx(l.panels[0].rect.w, 496.0);
        approx(l.panels[1].rect.w, 496.0);
        approx(l.panels[0].rect.h, 600.0);
        // no overlap, separator between the two
        approx(l.splitters[0].rect.x, 496.0);
        approx(l.splitters[0].rect.w, 8.0);
        approx(l.panels[1].rect.x, 504.0);
        assert_eq!(l.splitters[0].dir, SplitDir::Row);
    }

    #[test]
    fn column_split_stacks() {
        let tree = LayoutNode::Split {
            dir: SplitDir::Column,
            ratio: 0.25,
            first: Box::new(LayoutNode::Leaf { panel: 0 }),
            second: Box::new(LayoutNode::Leaf { panel: 1 }),
        };
        let l = tree.compute(area(), 8.0);
        // avail = 592, first = 148
        approx(l.panels[0].rect.h, 148.0);
        approx(l.panels[1].rect.h, 444.0);
        approx(l.panels[0].rect.w, 1000.0);
        approx(l.splitters[0].rect.y, 148.0);
        approx(l.splitters[0].rect.h, 8.0);
        assert_eq!(l.splitters[0].dir, SplitDir::Column);
    }

    #[test]
    fn nested_split_geometry() {
        // Root Row 0.5; right child re-split into Column 0.5.
        let tree = LayoutNode::Split {
            dir: SplitDir::Row,
            ratio: 0.5,
            first: Box::new(LayoutNode::Leaf { panel: 0 }),
            second: Box::new(LayoutNode::Split {
                dir: SplitDir::Column,
                ratio: 0.5,
                first: Box::new(LayoutNode::Leaf { panel: 1 }),
                second: Box::new(LayoutNode::Leaf { panel: 2 }),
            }),
        };
        let l = tree.compute(area(), 8.0);
        assert_eq!(l.panels.len(), 3);
        assert_eq!(l.splitters.len(), 2);
        // panel 0 = left half, full height
        let p0 = l.panels.iter().find(|p| p.panel == 0).unwrap();
        approx(p0.rect.w, 496.0);
        approx(p0.rect.h, 600.0);
        // panels 1 and 2 = right half, stacked (≈ (600-8)/2)
        let p1 = l.panels.iter().find(|p| p.panel == 1).unwrap();
        let p2 = l.panels.iter().find(|p| p.panel == 2).unwrap();
        approx(p1.rect.w, 496.0);
        approx(p1.rect.h, 296.0);
        approx(p2.rect.h, 296.0);
        approx(p1.rect.x, 504.0);
    }

    #[test]
    fn row_chain_matches_stretches() {
        // 3 panels with stretch 2,1,1 → the first one takes half.
        let tree = LayoutNode::row_chain(&[2.0, 1.0, 1.0]);
        assert_eq!(tree.leaf_indices(), vec![0, 1, 2]);
        let l = tree.compute(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 408.0,
                h: 100.0,
            },
            8.0,
        );
        // total stretch = 4; gaps = 2*8 = 16; remainder 392.
        // p0 = 2/4 ≈ 196 (at the first level ratio=0.5 over 400 available)
        let p0 = l.panels.iter().find(|p| p.panel == 0).unwrap();
        approx(p0.rect.w, 200.0);
    }

    #[test]
    fn validity_checks() {
        let good = LayoutNode::row_chain(&[1.0, 1.0, 1.0]);
        assert!(good.is_valid_for(3));
        assert!(!good.is_valid_for(2)); // index 2 out of bounds
        assert!(!good.is_valid_for(4)); // panel 3 missing

        let dup = LayoutNode::Split {
            dir: SplitDir::Row,
            ratio: 0.5,
            first: Box::new(LayoutNode::Leaf { panel: 0 }),
            second: Box::new(LayoutNode::Leaf { panel: 0 }),
        };
        assert!(!dup.is_valid_for(1));
    }

    #[test]
    fn set_ratio_via_path() {
        let mut tree = LayoutNode::Split {
            dir: SplitDir::Row,
            ratio: 0.5,
            first: Box::new(LayoutNode::Leaf { panel: 0 }),
            second: Box::new(LayoutNode::Split {
                dir: SplitDir::Column,
                ratio: 0.5,
                first: Box::new(LayoutNode::Leaf { panel: 1 }),
                second: Box::new(LayoutNode::Leaf { panel: 2 }),
            }),
        };
        // root
        assert!(tree.set_ratio(&[], 0.7, 0.05));
        // Column node = Second child of the root
        assert!(tree.set_ratio(&[Side::Second], 0.3, 0.05));
        // path to a leaf → failure
        assert!(!tree.set_ratio(&[Side::First], 0.5, 0.05));

        if let LayoutNode::Split { ratio, second, .. } = &tree {
            approx(*ratio, 0.7);
            if let LayoutNode::Split { ratio: r2, .. } = second.as_ref() {
                approx(*r2, 0.3);
            } else {
                panic!("expected Split");
            }
        } else {
            panic!("expected Split");
        }
    }

    #[test]
    fn ratio_clamped() {
        let mut tree = LayoutNode::Split {
            dir: SplitDir::Row,
            ratio: 0.5,
            first: Box::new(LayoutNode::Leaf { panel: 0 }),
            second: Box::new(LayoutNode::Leaf { panel: 1 }),
        };
        tree.set_ratio(&[], 0.001, 0.05);
        if let LayoutNode::Split { ratio, .. } = &tree {
            approx(*ratio, 0.05);
        }
        tree.set_ratio(&[], 0.999, 0.05);
        if let LayoutNode::Split { ratio, .. } = &tree {
            approx(*ratio, 0.95);
        }
    }

    #[test]
    fn split_leaf_creates_split() {
        let mut tree = LayoutNode::Leaf { panel: 0 };
        assert!(tree.split_leaf(0, SplitDir::Row, 1, 0.5, false));
        assert!(tree.is_valid_for(2));
        assert_eq!(tree.leaf_indices(), vec![0, 1]);
        assert!(matches!(
            tree,
            LayoutNode::Split {
                dir: SplitDir::Row,
                ..
            }
        ));

        // Re-split panel 1 (nested leaf) into Column.
        assert!(tree.split_leaf(1, SplitDir::Column, 2, 0.5, false));
        assert!(tree.is_valid_for(3));
        assert_eq!(tree.leaf_indices(), vec![0, 1, 2]);

        // Non-existent panel → false.
        assert!(!tree.split_leaf(9, SplitDir::Row, 3, 0.5, false));
    }

    #[test]
    fn split_leaf_new_first_puts_new_panel_first() {
        let mut tree = LayoutNode::Leaf { panel: 0 };
        assert!(tree.split_leaf(0, SplitDir::Row, 1, 0.5, true));
        // new_first → the new panel (1) is the FIRST child.
        assert_eq!(tree.leaf_indices(), vec![1, 0]);
    }

    #[test]
    fn remove_panel_collapses_and_reindexes() {
        // Chain of 3 panels: leaves [0,1,2].
        let mut tree = LayoutNode::row_chain(&[1.0, 1.0, 1.0]);
        // Remove panel 1 → [0, 2] remain, reindexed as [0, 1].
        assert!(tree.remove_panel(1));
        assert!(tree.is_valid_for(2));
        assert_eq!(tree.leaf_indices(), vec![0, 1]);

        // Remove again → 1 leaf.
        assert!(tree.remove_panel(0));
        assert!(tree.is_valid_for(1));
        assert_eq!(tree.leaf_indices(), vec![0]);

        // Last panel: refused.
        assert!(!tree.remove_panel(0));
    }

    #[test]
    fn remove_panel_keeps_sibling_subtree() {
        // Row { 0, Column { 1, 2 } }; removing 0 → only Column{1,2} remains,
        // reindexed as Column{0,1}.
        let mut tree = LayoutNode::Split {
            dir: SplitDir::Row,
            ratio: 0.5,
            first: Box::new(LayoutNode::Leaf { panel: 0 }),
            second: Box::new(LayoutNode::Split {
                dir: SplitDir::Column,
                ratio: 0.5,
                first: Box::new(LayoutNode::Leaf { panel: 1 }),
                second: Box::new(LayoutNode::Leaf { panel: 2 }),
            }),
        };
        assert!(tree.remove_panel(0));
        assert!(tree.is_valid_for(2));
        assert_eq!(tree.leaf_indices(), vec![0, 1]);
        assert!(matches!(
            tree,
            LayoutNode::Split {
                dir: SplitDir::Column,
                ..
            }
        ));
    }

    #[test]
    fn splitter_carries_area() {
        let tree = LayoutNode::Split {
            dir: SplitDir::Row,
            ratio: 0.5,
            first: Box::new(LayoutNode::Leaf { panel: 0 }),
            second: Box::new(LayoutNode::Leaf { panel: 1 }),
        };
        let l = tree.compute(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            0.0,
        );
        // Full fractional area, separator halfway.
        approx(l.splitters[0].area.w, 1.0);
        approx(l.splitters[0].rect.x, 0.5);
    }

    #[test]
    fn toml_round_trip() {
        let tree = LayoutNode::Split {
            dir: SplitDir::Row,
            ratio: 0.5,
            first: Box::new(LayoutNode::Leaf { panel: 0 }),
            second: Box::new(LayoutNode::Split {
                dir: SplitDir::Column,
                ratio: 0.6,
                first: Box::new(LayoutNode::Leaf { panel: 1 }),
                second: Box::new(LayoutNode::Leaf { panel: 2 }),
            }),
        };
        let s = toml::to_string_pretty(&tree).expect("serialize");
        let back: LayoutNode = toml::from_str(&s).expect("parse");
        assert_eq!(back, tree);
    }

    /// One view split in two, then the right half split again: the three come
    /// out the same width, not a half and two quarters.
    #[test]
    fn equalize_gives_every_panel_the_same_width() {
        let mut tree = split(
            SplitDir::Row,
            0.5,
            leaf(0),
            split(SplitDir::Row, 0.5, leaf(1), leaf(2)),
        );
        assert!(tree.equalize(&[], area(), 8.0, 0.08).is_some());
        let s = sizes(&tree, 8.0);
        approx(s[0].0, s[1].0);
        approx(s[1].0, s[2].0);
        // Nothing lost on the way: three panels and two separators fill the area.
        approx(s[0].0 + s[1].0 + s[2].0 + 16.0, area().w);
    }

    /// Three panels in a row can be held by a tree leaning either way, and the
    /// user cannot tell which. Both must even out to the same thing.
    #[test]
    fn equalize_ignores_which_way_the_tree_leans() {
        let mut right = split(
            SplitDir::Row,
            0.5,
            leaf(0),
            split(SplitDir::Row, 0.5, leaf(1), leaf(2)),
        );
        let mut left = split(
            SplitDir::Row,
            0.5,
            split(SplitDir::Row, 0.5, leaf(0), leaf(1)),
            leaf(2),
        );
        right.equalize(&[], area(), 8.0, 0.08);
        left.equalize(&[], area(), 8.0, 0.08);
        for (a, b) in sizes(&right, 8.0).iter().zip(sizes(&left, 8.0)) {
            approx(a.0, b.0);
        }
    }

    /// A panel beside two stacked ones: the reading kept is "same width", so
    /// the three are equally wide and the stacked pair shares the height.
    #[test]
    fn equalize_keeps_widths_equal_across_a_stack() {
        let mut tree = split(
            SplitDir::Row,
            0.8,
            leaf(0),
            split(SplitDir::Column, 0.3, leaf(1), leaf(2)),
        );
        tree.equalize(&[], area(), 8.0, 0.08);
        let s = sizes(&tree, 8.0);
        approx(s[0].0, s[1].0);
        approx(s[1].0, s[2].0);
        approx(s[1].1, s[2].1);
        approx(s[0].1, area().h);
    }

    /// A separator evens out only what it governs: proportions set by hand
    /// elsewhere in the tree stay put.
    #[test]
    fn equalize_below_the_root_leaves_the_rest_alone() {
        let mut tree = split(
            SplitDir::Row,
            0.8,
            leaf(0),
            split(SplitDir::Row, 0.2, leaf(1), leaf(2)),
        );
        let nested = area_of(&tree, &[Side::Second], 8.0);
        assert!(tree.equalize(&[Side::Second], nested, 8.0, 0.08).is_some());
        let s = sizes(&tree, 8.0);
        approx(s[1].0, s[2].0);
        approx(s[0].0, (area().w - 8.0) * 0.8);
    }

    /// Nothing moved means nothing to put back: the caller is told so and
    /// leaves no stale way back armed.
    #[test]
    fn equalize_reports_nothing_when_already_even() {
        let mut tree = split(SplitDir::Row, 0.5, leaf(0), leaf(1));
        assert!(tree.equalize(&[], area(), 8.0, 0.08).is_none());
    }

    #[test]
    fn restore_ratios_puts_back_what_equalize_took() {
        let mut tree = split(
            SplitDir::Row,
            0.8,
            leaf(0),
            split(SplitDir::Row, 0.2, leaf(1), leaf(2)),
        );
        let (before, _) = tree.equalize(&[], area(), 8.0, 0.08).expect("layout moved");
        assert!(tree.restore_ratios(&[], &before));
        approx(sizes(&tree, 8.0)[0].0, (area().w - 8.0) * 0.8);
    }

    /// A view closed under the copy leaves one split fewer: the saved ratios no
    /// longer describe this tree, and a half-applied restore is refused whole.
    #[test]
    fn restore_ratios_refuses_a_shape_that_changed() {
        let mut tree = split(
            SplitDir::Row,
            0.8,
            leaf(0),
            split(SplitDir::Row, 0.2, leaf(1), leaf(2)),
        );
        let (before, _) = tree.equalize(&[], area(), 8.0, 0.08).expect("layout moved");
        assert!(tree.remove_panel(2));
        assert!(!tree.restore_ratios(&[], &before));
    }

    /// The canonical case: four views in a 2x2, the two on top change places.
    /// Their sizes are equal, so what moves is the content, not the geometry.
    #[test]
    fn swap_panels_exchanges_two_places() {
        let mut tree = split(
            SplitDir::Column,
            0.5,
            split(SplitDir::Row, 0.5, leaf(0), leaf(1)),
            split(SplitDir::Row, 0.5, leaf(2), leaf(3)),
        );
        let before = tree.compute(area(), 8.0);
        assert!(tree.swap_panels(0, 1));
        let after = tree.compute(area(), 8.0);
        let place = |l: &Layout, panel: usize| {
            l.panels
                .iter()
                .find(|p| p.panel == panel)
                .expect("panel")
                .rect
        };
        assert_eq!(place(&after, 0), place(&before, 1));
        assert_eq!(place(&after, 1), place(&before, 0));
        // The two left untouched really are untouched.
        assert_eq!(place(&after, 2), place(&before, 2));
        assert_eq!(place(&after, 3), place(&before, 3));
    }

    /// Views of different sizes: each takes the size of its new place. That is
    /// the whole point of swapping places rather than contents.
    #[test]
    fn swap_panels_hands_over_the_size_of_the_new_place() {
        let mut tree = split(SplitDir::Row, 0.8, leaf(0), leaf(1));
        let wide = sizes(&tree, 8.0)[0].0;
        let narrow = sizes(&tree, 8.0)[1].0;
        assert!(tree.swap_panels(0, 1));
        approx(sizes(&tree, 8.0)[0].0, narrow);
        approx(sizes(&tree, 8.0)[1].0, wide);
    }

    /// The shape and the proportions are untouched: only two indices move.
    #[test]
    fn swap_panels_leaves_the_tree_and_its_ratios_alone() {
        let mut tree = split(
            SplitDir::Row,
            0.7,
            leaf(0),
            split(SplitDir::Column, 0.3, leaf(1), leaf(2)),
        );
        let ratios = tree.ratios(&[]);
        assert!(tree.swap_panels(0, 2));
        assert_eq!(tree.ratios(&[]), ratios);
        assert!(tree.is_valid_for(3));
    }

    /// Doing it twice puts everything back — the operation is its own undo.
    #[test]
    fn swap_panels_twice_restores() {
        let mut tree = split(
            SplitDir::Row,
            0.7,
            leaf(0),
            split(SplitDir::Column, 0.3, leaf(1), leaf(2)),
        );
        let before = tree.clone();
        assert!(tree.swap_panels(0, 2));
        assert!(tree.swap_panels(0, 2));
        assert_eq!(tree, before);
    }

    /// A view dropped on itself, and an index with no leaf: refused, and above
    /// all nothing written — a half-applied swap would hold one index twice.
    #[test]
    fn swap_panels_refuses_what_it_cannot_do_whole() {
        let mut tree = split(SplitDir::Row, 0.5, leaf(0), leaf(1));
        let before = tree.clone();
        assert!(!tree.swap_panels(1, 1));
        assert_eq!(tree, before);
        assert!(!tree.swap_panels(0, 9));
        assert_eq!(tree, before);
        assert!(tree.is_valid_for(2));
    }
}
