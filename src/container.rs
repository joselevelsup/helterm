use std::fmt::Debug;

use gpui::Entity;

use crate::terminal::TerminalView;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Children side by side.
    Row,
    /// Children stacked.
    Col,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDir {
    Left,
    Right,
    Up,
    Down,
}

/// Anything that can live as a leaf of the split tree.
pub trait Leaf: Clone {
    fn leaf_id(&self) -> u64;
}

impl Leaf for Entity<TerminalView> {
    fn leaf_id(&self) -> u64 {
        self.entity_id().as_u64()
    }
}

impl Leaf for u64 {
    fn leaf_id(&self) -> u64 {
        *self
    }
}

/// A pane never gets smaller than this share of its split.
const MIN_RATIO: f32 = 0.1;
const MAX_RATIO: f32 = 1. - MIN_RATIO;

/// Sway-style binary split tree. A leaf is one terminal; splits nest freely.
#[derive(Debug, Clone, PartialEq)]
pub enum Node<L> {
    Leaf(L),
    Split {
        axis: Axis,
        /// Share of the space the first child gets (0.5 = even).
        ratio: f32,
        first: Box<Node<L>>,
        second: Box<Node<L>>,
    },
}

impl<L: Leaf> Node<L> {
    pub fn contains(&self, id: u64) -> bool {
        match self {
            Node::Leaf(l) => l.leaf_id() == id,
            Node::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    pub fn first_leaf(&self) -> Option<&L> {
        match self {
            Node::Leaf(l) => Some(l),
            Node::Split { first, .. } => first.first_leaf(),
        }
    }

    pub fn last_leaf(&self) -> Option<&L> {
        match self {
            Node::Leaf(l) => Some(l),
            Node::Split { second, .. } => second.last_leaf(),
        }
    }

    pub fn find_leaf(&self, id: u64) -> Option<&L> {
        match self {
            Node::Leaf(l) if l.leaf_id() == id => Some(l),
            Node::Leaf(_) => None,
            Node::Split { first, second, .. } => {
                first.find_leaf(id).or_else(|| second.find_leaf(id))
            }
        }
    }

    /// Replace the leaf with `id` by a split holding it and `new_leaf` along `axis`.
    pub fn split_focused(&mut self, id: u64, axis: Axis, new_leaf: L) {
        match self {
            Node::Leaf(l) if l.leaf_id() == id => {
                let old = self.clone();
                *self = Node::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(old),
                    second: Box::new(Node::Leaf(new_leaf)),
                };
            }
            Node::Split { first, second, .. } => {
                if first.contains(id) {
                    first.split_focused(id, axis, new_leaf)
                } else {
                    second.split_focused(id, axis, new_leaf)
                }
            }
            Node::Leaf(_) => {}
        }
    }

    /// Remove the leaf with `id`. Returns false if this whole node was removed.
    /// A split left with a single child collapses into that child.
    pub fn remove(&mut self, id: u64) -> bool {
        match self {
            Node::Leaf(l) => l.leaf_id() != id,
            Node::Split { first, second, .. } => {
                if first.contains(id) {
                    if first.remove(id) {
                        true
                    } else {
                        *self = (**second).clone();
                        true
                    }
                } else if second.contains(id) {
                    if second.remove(id) {
                        true
                    } else {
                        *self = (**first).clone();
                        true
                    }
                } else {
                    false
                }
            }
        }
    }

    /// Sway-style focus movement: walk up to the nearest ancestor split whose
    /// axis matches the movement direction, then jump to a leaf of the sibling
    /// on that side. Returns None when there is nothing to move to.
    pub fn focus_dir(&self, id: u64, dir: FocusDir) -> Option<&L> {
        match self {
            Node::Leaf(_) => None,
            Node::Split { axis, first, second, .. } => {
                let horizontal = matches!(dir, FocusDir::Left | FocusDir::Right);
                if (*axis == Axis::Row) == horizontal {
                    if first.contains(id) {
                        // A deeper split along the same axis wins over this one.
                        first
                            .focus_dir(id, dir)
                            .or(match dir {
                                FocusDir::Right | FocusDir::Down => second.first_leaf(),
                                _ => None,
                            })
                    } else if second.contains(id) {
                        second
                            .focus_dir(id, dir)
                            .or(match dir {
                                FocusDir::Left | FocusDir::Up => first.last_leaf(),
                                _ => None,
                            })
                    } else {
                        None
                    }
                } else if first.contains(id) {
                    first.focus_dir(id, dir)
                } else {
                    second.focus_dir(id, dir)
                }
            }
        }
    }

    /// Grow (`delta` > 0) or shrink leaf `id` along `axis` (Row: width, Col:
    /// height) by moving the nearest split above it that runs that way. False
    /// if there is no such split, e.g. widening a pane that spans the width.
    pub fn resize(&mut self, id: u64, axis: Axis, delta: f32) -> bool {
        let Node::Split { axis: split_axis, ratio, first, second } = self else {
            return false;
        };
        let in_first = first.contains(id);
        if !in_first && !second.contains(id) {
            return false;
        }
        let child = if in_first { first } else { second };
        // The nearest split wins, so look deeper first.
        if child.resize(id, axis, delta) {
            return true;
        }
        if *split_axis != axis {
            return false;
        }
        // The first child's share grows when it's the one growing.
        let delta = if in_first { delta } else { -delta };
        *ratio = (*ratio + delta).clamp(MIN_RATIO, MAX_RATIO);
        true
    }

    /// Swap leaves `a` and `b`. The split sizes stay with the positions. False
    /// unless both exist and differ.
    pub fn swap(&mut self, a: u64, b: u64) -> bool {
        let (Some(leaf_a), Some(leaf_b)) = (self.find_leaf(a).cloned(), self.find_leaf(b).cloned()) else {
            return false;
        };
        if a == b {
            return false;
        }
        // One pass, so the tree never holds two copies of the same leaf.
        fn exchange<L: Leaf>(node: &mut Node<L>, a: u64, b: u64, leaf_a: &L, leaf_b: &L) {
            match node {
                Node::Leaf(l) if l.leaf_id() == a => *l = leaf_b.clone(),
                Node::Leaf(l) if l.leaf_id() == b => *l = leaf_a.clone(),
                Node::Leaf(_) => {}
                Node::Split { first, second, .. } => {
                    exchange(first, a, b, leaf_a, leaf_b);
                    exchange(second, a, b, leaf_a, leaf_b);
                }
            }
        }
        exchange(self, a, b, &leaf_a, &leaf_b);
        true
    }

    pub fn leaves(&self) -> Vec<&L> {
        match self {
            Node::Leaf(l) => vec![l],
            Node::Split { first, second, .. } => {
                let mut out = first.leaves();
                out.extend(second.leaves());
                out
            }
        }
    }

    fn sorted_ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.leaves().iter().map(|l| l.leaf_id()).collect();
        ids.sort_unstable();
        ids
    }

    /// Where leaf `id` sits: its parent's axis, its side, and its sibling subtree.
    pub fn placement(&self, id: u64) -> Option<Placement> {
        let Node::Split { axis, ratio, first, second } = self else {
            return None;
        };
        let is_leaf = |n: &Node<L>| matches!(n, Node::Leaf(l) if l.leaf_id() == id);
        let (axis, ratio) = (*axis, *ratio);
        if is_leaf(first) {
            return Some(Placement { axis, ratio, first: true, sibling: second.sorted_ids() });
        }
        if is_leaf(second) {
            return Some(Placement { axis, ratio, first: false, sibling: first.sorted_ids() });
        }
        first.placement(id).or_else(|| second.placement(id))
    }

    /// Put `leaf` back where `placement` says it was. False if its sibling
    /// subtree no longer exists in this exact shape.
    pub fn restore(&mut self, placement: &Placement, leaf: L) -> bool {
        if self.sorted_ids() == placement.sibling {
            let old = Box::new(self.clone());
            let new = Box::new(Node::Leaf(leaf));
            let (first, second) = if placement.first { (new, old) } else { (old, new) };
            *self = Node::Split { axis: placement.axis, ratio: placement.ratio, first, second };
            return true;
        }
        match self {
            Node::Leaf(_) => false,
            Node::Split { first, second, .. } => {
                first.restore(placement, leaf.clone()) || second.restore(placement, leaf)
            }
        }
    }
}

/// A leaf's spot in the tree, recorded so it can be put back after floating.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    axis: Axis,
    /// The split's size, so putting the leaf back restores it too.
    ratio: f32,
    /// Whether the leaf was the first (left/top) child.
    first: bool,
    /// Sorted leaf ids of the sibling subtree it was split from.
    sibling: Vec<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_and_restore_round_trips_exactly() {
        // Row{ Col{1, 3}, 2 }: 3 is the bottom of the left column.
        let mut tree = Node::Leaf(1u64);
        tree.split_focused(1, Axis::Row, 2);
        tree.split_focused(1, Axis::Col, 3);
        let original = tree.clone();

        for id in [1, 2, 3] {
            let placement = tree.placement(id).unwrap();
            tree.remove(id);
            assert!(!tree.contains(id));
            assert!(tree.restore(&placement, id));
            assert_eq!(tree, original, "leaf {id}");
        }
    }

    #[test]
    fn restore_fails_when_the_sibling_changed() {
        let mut tree = Node::Leaf(1u64);
        tree.split_focused(1, Axis::Row, 2);
        let placement = tree.placement(2).unwrap();
        tree.remove(2);
        tree.split_focused(1, Axis::Col, 4);
        tree.remove(1); // 2's sibling is gone entirely
        assert!(!tree.restore(&placement, 2));
        assert!(Node::Leaf(1u64).placement(1).is_none());
    }

    fn ratio_of(node: &Node<u64>) -> f32 {
        match node {
            Node::Split { ratio, .. } => *ratio,
            Node::Leaf(_) => panic!("not a split"),
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn resize_grows_the_selected_pane_and_clamps() {
        // [1 | 2]
        let mut tree = Node::Leaf(1u64);
        tree.split_focused(1, Axis::Row, 2);
        assert!(tree.resize(1, Axis::Row, 0.05));
        assert!(close(ratio_of(&tree), 0.55)); // 1 (left) grew
        assert!(tree.resize(2, Axis::Row, 0.05));
        assert!(close(ratio_of(&tree), 0.5)); // 2 (right) grew back
        for _ in 0..20 {
            tree.resize(1, Axis::Row, 0.05);
        }
        assert!(close(ratio_of(&tree), MAX_RATIO));
        // No stacked split: can't change the height.
        assert!(!tree.resize(1, Axis::Col, 0.05));
        assert!(!tree.resize(9, Axis::Row, 0.05)); // unknown leaf
    }

    #[test]
    fn resize_moves_the_nearest_split_on_that_axis() {
        // Row{ Col{1, 3}, 2 }: 3 is below 1 in the left column.
        let mut tree = Node::Leaf(1u64);
        tree.split_focused(1, Axis::Row, 2);
        tree.split_focused(1, Axis::Col, 3);
        let Node::Split { first: left, .. } = &tree else { unreachable!() };
        assert!(close(ratio_of(left), 0.5));

        // Width of 3: only the outer Row split can do that.
        assert!(tree.resize(3, Axis::Row, 0.05));
        assert!(close(ratio_of(&tree), 0.55));
        // Height of 3 (bottom half of the column): the inner split's top share shrinks.
        assert!(tree.resize(3, Axis::Col, 0.05));
        let Node::Split { first: left, .. } = &tree else { unreachable!() };
        assert!(close(ratio_of(left), 0.45));
        assert!(close(ratio_of(&tree), 0.55)); // outer untouched

        // [1 | [2 | 3]]: widening 3 moves the inner split, not the outer one.
        let mut tree = Node::Leaf(1u64);
        tree.split_focused(1, Axis::Row, 2);
        tree.split_focused(2, Axis::Row, 3);
        assert!(tree.resize(3, Axis::Row, 0.05));
        assert!(close(ratio_of(&tree), 0.5));
        let Node::Split { second: right, .. } = &tree else { unreachable!() };
        assert!(close(ratio_of(right), 0.45));
    }

    #[test]
    fn swap_exchanges_panes_and_keeps_sizes_in_place() {
        // Row{ Col{1, 3}, 2 } with a wider left column.
        let mut tree = Node::Leaf(1u64);
        tree.split_focused(1, Axis::Row, 2);
        tree.split_focused(1, Axis::Col, 3);
        tree.resize(3, Axis::Row, 0.2);

        assert!(tree.swap(3, 2));
        let ids: Vec<u64> = tree.leaves().into_iter().copied().collect();
        assert_eq!(ids, vec![1, 2, 3]); // 2 now bottom-left, 3 on the right
        assert!(close(ratio_of(&tree), 0.7)); // the left column stays wide
        assert_eq!(tree.focus_dir(2, FocusDir::Right).copied(), Some(3));

        assert!(tree.swap(2, 3)); // and back
        let ids: Vec<u64> = tree.leaves().into_iter().copied().collect();
        assert_eq!(ids, vec![1, 3, 2]);

        assert!(!tree.swap(1, 1));
        assert!(!tree.swap(1, 9));
        assert!(!Node::Leaf(1u64).swap(1, 2));
    }

    #[test]
    fn floating_and_restoring_keeps_the_size() {
        let mut tree = Node::Leaf(1u64);
        tree.split_focused(1, Axis::Row, 2);
        tree.resize(1, Axis::Row, 0.2);
        let sized = tree.clone();
        let placement = tree.placement(2).unwrap();
        tree.remove(2);
        assert!(tree.restore(&placement, 2));
        assert_eq!(tree, sized);
    }

    #[test]
    fn split_focused_creates_sibling() {
        let mut tree = Node::Leaf(1u64);
        tree.split_focused(1, Axis::Row, 2);
        assert_eq!(
            tree,
            Node::Split {
                axis: Axis::Row,
                ratio: 0.5,
                first: Box::new(Node::Leaf(1)),
                second: Box::new(Node::Leaf(2)),
            }
        );
        assert!(tree.contains(2));
    }

    #[test]
    fn remove_collapses_split() {
        let mut tree = Node::Leaf(1u64);
        tree.split_focused(1, Axis::Col, 2);
        assert!(tree.remove(1));
        assert_eq!(tree, Node::Leaf(2));
        // removing a leaf that is not present leaves the tree untouched
        assert!(tree.remove(1));
        assert_eq!(tree, Node::Leaf(2));
        assert!(!tree.remove(2));
    }

    #[test]
    fn nested_split_keeps_sibling_subtree() {
        // [1 | 2] with 1 split into [1 over 3]: Row{Col{1,3}, 2}
        let mut tree = Node::Leaf(1u64);
        tree.split_focused(1, Axis::Row, 2);
        tree.split_focused(1, Axis::Col, 3);
        assert!(tree.remove(1));
        assert_eq!(
            tree,
            Node::Split {
                axis: Axis::Row,
                ratio: 0.5,
                first: Box::new(Node::Leaf(3)),
                second: Box::new(Node::Leaf(2)),
            }
        );
    }

    #[test]
    fn focus_dir_navigates_siblings() {
        // [3 | 4] over [1], with 2 to the right of both
        let mut tree = Node::Leaf(1u64);
        tree.split_focused(1, Axis::Row, 2);
        tree.split_focused(1, Axis::Col, 3);
        tree.split_focused(3, Axis::Row, 4);

        // Layout: 1 top-left; 3 bottom-left, 4 bottom-right of left half; 2 right half
        assert_eq!(tree.focus_dir(1, FocusDir::Right).copied(), Some(2));
        assert_eq!(tree.focus_dir(2, FocusDir::Left).copied(), Some(4));
        assert_eq!(tree.focus_dir(1, FocusDir::Down).copied(), Some(3));
        assert_eq!(tree.focus_dir(1, FocusDir::Up).copied(), None);
        assert_eq!(tree.focus_dir(3, FocusDir::Right).copied(), Some(4));
        assert_eq!(tree.focus_dir(4, FocusDir::Left).copied(), Some(3));
        assert_eq!(tree.focus_dir(4, FocusDir::Up).copied(), Some(1));
        assert_eq!(tree.focus_dir(2, FocusDir::Right).copied(), None);
    }
}
