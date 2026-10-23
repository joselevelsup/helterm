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
