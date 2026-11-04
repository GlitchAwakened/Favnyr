//! Stable moves of selected entries to a gap expressed before their removal.

/// A validated permutation shared by file ordering and tab ordering.
pub struct MovePlan {
    sources: Vec<usize>,
}

impl MovePlan {
    /// Keeps both the selected and remaining entries in their original order.
    /// Invalid or duplicate source indices reject the whole move. A no-op
    /// returns no plan, so callers can avoid publishing unchanged models.
    pub fn new(len: usize, selected: &[usize], gap: usize) -> Option<Self> {
        if selected.is_empty() {
            return None;
        }
        let mut included = vec![false; len];
        for &index in selected {
            if index >= len || included[index] {
                return None;
            }
            included[index] = true;
        }
        let gap = gap.min(len);
        let removed_before = included[..gap].iter().filter(|&&value| value).count();
        let (moved, mut remaining): (Vec<_>, Vec<_>) = (0..len).partition(|&i| included[i]);
        let at = gap - removed_before;
        remaining.splice(at..at, moved);
        remaining
            .iter()
            .enumerate()
            .any(|(i, &source)| i != source)
            .then_some(Self { sources: remaining })
    }

    pub fn sources(&self) -> &[usize] {
        &self.sources
    }

    /// Applies the permutation without cloning entries or replacing their
    /// allocation. The two index arrays keep each swap linear overall.
    pub fn apply<T>(&self, items: &mut [T]) -> bool {
        if items.len() != self.sources.len() {
            return false;
        }
        let mut source_at: Vec<_> = (0..items.len()).collect();
        let mut position_of = source_at.clone();
        for (target, &source) in self.sources.iter().enumerate() {
            let old_position = position_of[source];
            let displaced = source_at[target];
            items.swap(target, old_position);
            source_at.swap(target, old_position);
            position_of[source] = target;
            position_of[displaced] = old_position;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noncontiguous_entries_form_a_stable_block_at_the_original_gap() {
        for (selected, gap, expected) in [
            (vec![3, 1], 0, vec![1, 3, 0, 2, 4]),
            (vec![1, 3], 3, vec![0, 2, 1, 3, 4]),
            (vec![1, 3], usize::MAX, vec![0, 2, 4, 1, 3]),
        ] {
            let plan = MovePlan::new(5, &selected, gap).unwrap();
            assert_eq!(plan.sources(), expected);
            let mut entries: Vec<_> = (0..5).map(Box::new).collect();
            let allocations: Vec<_> = entries.iter().map(|e| &**e as *const usize).collect();
            assert!(plan.apply(&mut entries));
            for (entry, &source) in entries.iter().zip(&expected) {
                assert_eq!(**entry, source);
                assert_eq!(&**entry as *const usize, allocations[source]);
            }
        }
    }

    #[test]
    fn unchanged_and_invalid_moves_have_no_plan() {
        for (len, selected, gap) in [
            (0, vec![], 0),
            (0, vec![0], 0),
            (4, vec![], 2),
            (4, vec![1, 2], 1),
            (4, vec![1, 2], 2),
            (4, vec![1, 2], 3),
            (4, vec![3, 2, 1, 0], 2),
            (4, vec![1, 4], 0),
            (4, vec![1, 1], 0),
        ] {
            assert!(MovePlan::new(len, &selected, gap).is_none());
        }
        let plan = MovePlan::new(4, &[2], 0).unwrap();
        let mut changed = [0, 1, 2];
        assert!(!plan.apply(&mut changed));
        assert_eq!(changed, [0, 1, 2]);
    }

    #[test]
    fn every_small_selection_preserves_membership_and_relative_order() {
        for len in 1..=7 {
            for mask in 1..(1usize << len) {
                let selected: Vec<_> = (0..len).filter(|i| mask & (1 << i) != 0).collect();
                let remaining: Vec<_> = (0..len).filter(|i| mask & (1 << i) == 0).collect();
                for gap in 0..=len + 1 {
                    let Some(plan) = MovePlan::new(len, &selected, gap) else {
                        continue;
                    };
                    let mut entries: Vec<_> = (0..len).collect();
                    assert!(plan.apply(&mut entries));
                    assert_eq!(entries, plan.sources());
                    let mut sorted = entries.clone();
                    sorted.sort_unstable();
                    assert_eq!(sorted, (0..len).collect::<Vec<_>>());
                    let at = entries.iter().position(|i| *i == selected[0]).unwrap();
                    assert_eq!(&entries[at..at + selected.len()], selected);
                    entries.drain(at..at + selected.len());
                    assert_eq!(entries, remaining);
                }
            }
        }
    }
}
