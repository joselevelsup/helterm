use std::fmt::Debug;

use gpui::Entity;

use crate::terminal::TerminalView;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDir {
    Left,
    Right,
    Up,
    Down,
}

/// Anything that can live in the layout.
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

const MIN_RATIO: f32 = 0.1;
const MAX_RATIO: f32 = 1. - MIN_RATIO;
const MIN_WEIGHT: f32 = 0.2;
const MAX_WEIGHT: f32 = 5.;

/// One row of the layout: panes side by side, evenly wide.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Slot<L> {
    pub panes: Vec<L>,
    /// Height share within the stack (flex grow); unused for main.
    pub weight: f32,
}

impl<L> Slot<L> {
    fn new(leaf: L) -> Self {
        Slot { panes: vec![leaf], weight: 1. }
    }
}

/// Main-stack layout: the main slot on the left, the other slots stacked on the
/// right. Never empty: an empty tab has no layout at all.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "SavedLayout<L>", bound(deserialize = "L: serde::Deserialize<'de>"))]
pub struct Layout<L> {
    /// Share of the width main gets while there is a stack.
    pub main_ratio: f32,
    /// `slots[0]` is main, the rest the stack from top to bottom.
    pub slots: Vec<Slot<L>>,
}

/// What state files hold: this layout, or the split tree of older versions.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum SavedLayout<L> {
    Layout { main_ratio: f32, slots: Vec<Slot<L>> },
    Tree(Tree<L>),
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum Tree<L> {
    Leaf(L),
    Split { first: Box<Tree<L>>, second: Box<Tree<L>> },
}

impl<L> Tree<L> {
    fn into_leaves(self, out: &mut Vec<L>) {
        match self {
            Tree::Leaf(l) => out.push(l),
            Tree::Split { first, second } => {
                first.into_leaves(out);
                second.into_leaves(out);
            }
        }
    }
}

impl<L> From<SavedLayout<L>> for Layout<L> {
    fn from(saved: SavedLayout<L>) -> Self {
        match saved {
            SavedLayout::Layout { main_ratio, slots } => Layout { main_ratio, slots },
            // Old split trees: the first pane becomes main, the rest the stack.
            SavedLayout::Tree(tree) => {
                let mut leaves = vec![];
                tree.into_leaves(&mut leaves);
                Layout { main_ratio: 0.5, slots: leaves.into_iter().map(Slot::new).collect() }
            }
        }
    }
}

impl<L> Layout<L> {
    pub fn new(leaf: L) -> Self {
        Layout { main_ratio: 0.5, slots: vec![Slot::new(leaf)] }
    }

    /// The same layout (shape and sizes) with every leaf turned into `f(leaf)`.
    pub fn map<M>(&self, f: &mut impl FnMut(&L) -> M) -> Layout<M> {
        Layout {
            main_ratio: self.main_ratio,
            slots: self
                .slots
                .iter()
                .map(|s| Slot { panes: s.panes.iter().map(&mut *f).collect(), weight: s.weight })
                .collect(),
        }
    }

    /// All panes: main first, then the stack, each slot left to right.
    pub fn leaves(&self) -> Vec<&L> {
        self.slots.iter().flat_map(|s| &s.panes).collect()
    }

    pub fn first_leaf(&self) -> Option<&L> {
        self.slots.first().and_then(|s| s.panes.first())
    }

    /// Add a slot at the bottom of the stack.
    pub fn push(&mut self, leaf: L) {
        self.slots.push(Slot::new(leaf));
    }
}

impl<L: Leaf> Layout<L> {
    /// Slot and position of leaf `id`.
    fn locate(&self, id: u64) -> Option<(usize, usize)> {
        self.slots.iter().enumerate().find_map(|(s, slot)| {
            slot.panes.iter().position(|l| l.leaf_id() == id).map(|p| (s, p))
        })
    }

    pub fn contains(&self, id: u64) -> bool {
        self.locate(id).is_some()
    }

    pub fn find_leaf(&self, id: u64) -> Option<&L> {
        self.locate(id).map(|(s, p)| &self.slots[s].panes[p])
    }

    /// Put `leaf` right of leaf `id`, in the same slot.
    pub fn split_side(&mut self, id: u64, leaf: L) {
        if let Some((s, p)) = self.locate(id) {
            self.slots[s].panes.insert(p + 1, leaf);
        }
    }

    /// Remove leaf `id`. Returns false if the layout is now empty. An empty slot
    /// goes away; an empty main is replaced by the top of the stack.
    pub fn remove(&mut self, id: u64) -> bool {
        if let Some((s, p)) = self.locate(id) {
            self.slots[s].panes.remove(p);
            if self.slots[s].panes.is_empty() {
                self.slots.remove(s);
            }
        }
        !self.slots.is_empty()
    }

    /// The pane next to leaf `id` in direction `dir`. Right of main is the top
    /// of the stack, left of the stack is main.
    pub fn focus_dir(&self, id: u64, dir: FocusDir) -> Option<&L> {
        let (s, p) = self.locate(id)?;
        let panes = &self.slots[s].panes;
        // Same column if the neighbour slot is wide enough, else its last pane.
        let at = |s: usize| self.slots.get(s).and_then(|slot| slot.panes.get(p.min(slot.panes.len() - 1)));
        match dir {
            FocusDir::Left if p > 0 => panes.get(p - 1),
            FocusDir::Left if s > 0 => self.slots[0].panes.last(),
            FocusDir::Right if p + 1 < panes.len() => panes.get(p + 1),
            FocusDir::Right if s == 0 => self.slots.get(1).and_then(|slot| slot.panes.first()),
            FocusDir::Up if s > 1 => at(s - 1),
            FocusDir::Down if s > 0 => at(s + 1),
            _ => None,
        }
    }

    /// Left / right move the edge between main and the stack; down / up grow /
    /// shrink the height of leaf `id`'s stack slot. False if nothing changed.
    pub fn resize(&mut self, id: u64, dir: FocusDir, step: f32) -> bool {
        let Some((s, _)) = self.locate(id) else {
            return false;
        };
        let stack = self.slots.len() - 1;
        match dir {
            FocusDir::Left | FocusDir::Right if stack > 0 => {
                let step = if dir == FocusDir::Right { step } else { -step };
                self.main_ratio = (self.main_ratio + step).clamp(MIN_RATIO, MAX_RATIO);
                true
            }
            FocusDir::Up | FocusDir::Down if s > 0 && stack > 1 => {
                // Weights are relative: scale so a step moves about `step` of the height.
                let step = if dir == FocusDir::Down { step } else { -step } * stack as f32;
                let weight = &mut self.slots[s].weight;
                *weight = (*weight + step).clamp(MIN_WEIGHT, MAX_WEIGHT);
                true
            }
            _ => false,
        }
    }

    /// Swap leaves `a` and `b`; sizes stay with the positions. False unless
    /// both exist and differ.
    pub fn swap(&mut self, a: u64, b: u64) -> bool {
        let (Some((sa, pa)), Some((sb, pb))) = (self.locate(a), self.locate(b)) else {
            return false;
        };
        if a == b {
            return false;
        }
        let leaf_a = self.slots[sa].panes[pa].clone();
        self.slots[sa].panes[pa] = std::mem::replace(&mut self.slots[sb].panes[pb], leaf_a);
        true
    }

    /// Swap leaf `id`'s slot with main. False if it already is main.
    pub fn promote(&mut self, id: u64) -> bool {
        match self.locate(id) {
            Some((s, _)) if s > 0 => {
                let (main, stack) = self.slots.split_at_mut(1);
                std::mem::swap(&mut main[0].panes, &mut stack[s - 1].panes);
                true
            }
            _ => false,
        }
    }

    /// Where leaf `id` sits, so it can be put back after floating.
    pub fn placement(&self, id: u64) -> Option<Placement> {
        let (slot, pos) = self.locate(id)?;
        let alone = self.slots[slot].panes.len() == 1;
        Some(Placement { slot, pos, own_slot: alone.then_some(self.slots[slot].weight) })
    }

    /// Put `leaf` back where `placement` says it was, or at the bottom of the
    /// stack if that slot is gone.
    pub fn restore(&mut self, placement: &Placement, leaf: L) {
        match placement.own_slot {
            Some(weight) => {
                let at = placement.slot.min(self.slots.len());
                self.slots.insert(at, Slot { panes: vec![leaf], weight });
            }
            None => match self.slots.get_mut(placement.slot) {
                Some(slot) => slot.panes.insert(placement.pos.min(slot.panes.len()), leaf),
                None => self.push(leaf),
            },
        }
    }
}

/// A leaf's spot in the layout, recorded so it can be put back after floating.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    slot: usize,
    pos: usize,
    /// It had the slot to itself (with this weight), so it gets one back.
    own_slot: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(layout: &Layout<u64>) -> Vec<Vec<u64>> {
        layout.slots.iter().map(|s| s.panes.clone()).collect()
    }

    /// Main [1 | 4], stack [2], [3].
    fn sample() -> Layout<u64> {
        let mut layout = Layout::new(1);
        layout.push(2);
        layout.push(3);
        layout.split_side(1, 4);
        layout
    }

    #[test]
    fn push_stacks_and_split_side_goes_next_to_it() {
        let layout = sample();
        assert_eq!(ids(&layout), vec![vec![1, 4], vec![2], vec![3]]);
        let leaves: Vec<u64> = layout.leaves().into_iter().copied().collect();
        assert_eq!(leaves, vec![1, 4, 2, 3]);
    }

    #[test]
    fn removing_main_promotes_the_top_of_the_stack() {
        let mut layout = sample();
        assert!(layout.remove(4));
        assert!(layout.remove(1));
        assert_eq!(ids(&layout), vec![vec![2], vec![3]]);
        assert!(layout.remove(9)); // not there: untouched
        assert!(layout.remove(2));
        assert!(!layout.remove(3));
    }

    #[test]
    fn focus_moves_between_main_and_stack() {
        let layout = sample();
        let go = |id, dir| layout.focus_dir(id, dir).copied();
        assert_eq!(go(1, FocusDir::Right), Some(4));
        assert_eq!(go(4, FocusDir::Right), Some(2));
        assert_eq!(go(2, FocusDir::Left), Some(4));
        assert_eq!(go(2, FocusDir::Down), Some(3));
        assert_eq!(go(3, FocusDir::Up), Some(2));
        assert_eq!(go(2, FocusDir::Up), None);
        assert_eq!(go(1, FocusDir::Down), None);
        assert_eq!(go(1, FocusDir::Left), None);
        assert_eq!(go(3, FocusDir::Right), None);
    }

    #[test]
    fn resize_moves_the_main_edge_and_stack_heights() {
        let mut layout = sample();
        assert!(layout.resize(2, FocusDir::Right, 0.05));
        assert!((layout.main_ratio - 0.55).abs() < 1e-4);
        for _ in 0..20 {
            layout.resize(1, FocusDir::Left, 0.05);
        }
        assert!((layout.main_ratio - MIN_RATIO).abs() < 1e-4);
        assert!(layout.resize(3, FocusDir::Down, 0.05));
        assert!(layout.slots[2].weight > 1.);
        assert!(!layout.resize(1, FocusDir::Down, 0.05)); // main has no height to change
        assert!(!Layout::new(1u64).resize(1, FocusDir::Right, 0.05)); // no stack
    }

    #[test]
    fn swap_and_promote() {
        let mut layout = sample();
        assert!(layout.swap(4, 3));
        assert_eq!(ids(&layout), vec![vec![1, 3], vec![2], vec![4]]);
        assert!(!layout.swap(1, 1));
        assert!(!layout.swap(1, 9));
        assert!(layout.promote(2));
        assert_eq!(ids(&layout), vec![vec![2], vec![1, 3], vec![4]]);
        assert!(!layout.promote(2));
    }

    #[test]
    fn float_and_restore_round_trips_exactly() {
        let mut layout = sample();
        layout.resize(3, FocusDir::Down, 0.1);
        let original = layout.clone();
        for id in [1, 2, 3, 4] {
            let placement = layout.placement(id).unwrap();
            layout.remove(id);
            assert!(!layout.contains(id));
            layout.restore(&placement, id);
            assert_eq!(layout, original, "leaf {id}");
        }
    }

    #[test]
    fn old_split_trees_load_as_main_and_stack() {
        let old = "[split]\naxis = \"row\"\nratio = 0.5\n[split.first.leaf]\n[split.second.split]\naxis = \"col\"\nratio = 0.5\n[split.second.split.first.leaf]\n[split.second.split.second.leaf]\n";
        #[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
        struct P {}
        let layout: Layout<P> = toml::from_str(old).unwrap();
        assert_eq!(layout.slots.len(), 3);
        let new = toml::to_string(&layout).unwrap();
        assert_eq!(toml::from_str::<Layout<P>>(&new).unwrap(), layout);
    }
}
