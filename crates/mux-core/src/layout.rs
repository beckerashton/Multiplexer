use std::{cmp::Reverse, collections::BTreeMap};

use crate::{Axis, CellRect, Direction, SlotId};

/// A binary partition of terminal cells. `Horizontal` means a horizontal
/// divider (top/bottom); `Vertical` means a vertical divider (left/right).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayoutTree {
    root: Node,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Node {
    Leaf(SlotId),
    Split {
        axis: Axis,
        /// Thousandths assigned to the first child.  500 is an even split.
        ratio: u16,
        first: Box<Node>,
        second: Box<Node>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutRemoval {
    Removed { neighbor: SlotId },
    LastSlot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutError {
    UnknownSlot(SlotId),
    InvalidRatio(u16),
    LastSlot,
    NoNeighbor { slot: SlotId, direction: Direction },
    MinimumSize,
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSlot(slot) => write!(f, "unknown slot {}", slot.0),
            Self::InvalidRatio(ratio) => write!(f, "invalid split ratio {ratio}"),
            Self::LastSlot => f.write_str("the final layout slot cannot be removed"),
            Self::NoNeighbor { slot, direction } => {
                write!(f, "slot {} has no {direction:?} neighbor", slot.0)
            }
            Self::MinimumSize => f.write_str("operation would violate the minimum pane size"),
        }
    }
}

impl std::error::Error for LayoutError {}

impl LayoutTree {
    pub const RATIO_SCALE: u16 = 1000;

    pub fn new(root: SlotId) -> Self {
        Self {
            root: Node::Leaf(root),
        }
    }

    pub fn slots(&self) -> Vec<SlotId> {
        let mut slots = Vec::new();
        self.root.collect_slots(&mut slots);
        slots
    }

    pub fn contains(&self, slot: SlotId) -> bool {
        self.root.contains(slot)
    }

    /// Splits `slot`, retaining it as the first child and placing `new_slot`
    /// second. Callers allocate the new stable slot id.
    pub fn split(
        &mut self,
        slot: SlotId,
        axis: Axis,
        ratio: u16,
        new_slot: SlotId,
    ) -> Result<SlotId, LayoutError> {
        if ratio == 0 || ratio >= Self::RATIO_SCALE {
            return Err(LayoutError::InvalidRatio(ratio));
        }
        if self.contains(new_slot) {
            return Err(LayoutError::UnknownSlot(new_slot));
        }
        let leaf = self
            .root
            .find_leaf_mut(slot)
            .ok_or(LayoutError::UnknownSlot(slot))?;
        *leaf = Node::Split {
            axis,
            ratio,
            first: Box::new(Node::Leaf(slot)),
            second: Box::new(Node::Leaf(new_slot)),
        };
        Ok(new_slot)
    }

    /// Inserts an already-existing slot next to `anchor`. This is used for
    /// carrying whole stacks between tabs, so it never changes session ids.
    pub fn insert_beside(
        &mut self,
        anchor: SlotId,
        axis: Axis,
        ratio: u16,
        slot: SlotId,
    ) -> Result<(), LayoutError> {
        self.split(anchor, axis, ratio, slot).map(|_| ())
    }

    pub fn remove(&mut self, slot: SlotId) -> Result<LayoutRemoval, LayoutError> {
        if !self.contains(slot) {
            return Err(LayoutError::UnknownSlot(slot));
        }
        if matches!(&self.root, Node::Leaf(_)) {
            return Ok(LayoutRemoval::LastSlot);
        }
        let sibling = self
            .root
            .sibling_leaf(slot)
            .expect("non-root leaf has a sibling subtree");
        let neighbor = sibling.first_slot().expect("layout subtree has a leaf");
        let root = std::mem::replace(&mut self.root, Node::Leaf(SlotId(0)));
        self.root = root.remove_leaf(slot).expect("non-root leaf is removable");
        Ok(LayoutRemoval::Removed { neighbor })
    }

    /// Returns cell rectangles that exactly partition `bounds`. Each leaf is
    /// at least `min.cols` by `min.rows` whenever the supplied bounds can fit
    /// the tree's calculated minimum requirements.
    pub fn geometry(&self, bounds: CellRect, min: CellRect) -> BTreeMap<SlotId, CellRect> {
        let mut result = BTreeMap::new();
        self.root
            .geometry(bounds, min.cols.max(1), min.rows.max(1), &mut result);
        result
    }

    pub fn neighbor(
        &self,
        slot: SlotId,
        direction: Direction,
        bounds: CellRect,
        min: CellRect,
    ) -> Option<SlotId> {
        let rects = self.geometry(bounds, min);
        let source = *rects.get(&slot)?;
        self.slots()
            .into_iter()
            .enumerate()
            .filter_map(|(order, candidate)| {
                let rect = rects[&candidate];
                (candidate != slot && touches_in_direction(source, rect, direction)).then_some((
                    (
                        Reverse(shared_boundary(source, rect, direction)),
                        center_offset(source, rect, direction),
                        order,
                    ),
                    candidate,
                ))
            })
            .min_by_key(|(rank, _)| *rank)
            .map(|(_, candidate)| candidate)
    }

    /// Find the opposite vertical edge in the same column, using the normal
    /// overlap/center/traversal tie breakers. A full-height pane may select itself.
    pub fn vertical_wrap_neighbor(
        &self,
        slot: SlotId,
        direction: Direction,
        bounds: CellRect,
        min: CellRect,
    ) -> Option<SlotId> {
        let rects = self.geometry(bounds, min);
        let source = *rects.get(&slot)?;
        self.slots()
            .into_iter()
            .enumerate()
            .filter_map(|(order, candidate)| {
                let rect = rects[&candidate];
                let edge = match direction {
                    Direction::Up => {
                        rect.y.saturating_add(rect.rows) == bounds.y.saturating_add(bounds.rows)
                    }
                    Direction::Down => rect.y == bounds.y,
                    _ => false,
                };
                let shared = shared_boundary(source, rect, direction);
                (edge && shared > 0).then_some((
                    (
                        Reverse(shared),
                        center_offset(source, rect, direction),
                        order,
                    ),
                    candidate,
                ))
            })
            .min_by_key(|(rank, _)| *rank)
            .map(|(_, slot)| slot)
    }

    pub fn resize(
        &mut self,
        slot: SlotId,
        direction: Direction,
        delta: i16,
        bounds: CellRect,
        min: CellRect,
    ) -> Result<(), LayoutError> {
        if !self.contains(slot) {
            return Err(LayoutError::UnknownSlot(slot));
        }
        if delta == 0 {
            return Ok(());
        }
        let neighbor = self
            .neighbor(slot, direction, bounds, min)
            .ok_or(LayoutError::NoNeighbor { slot, direction })?;
        let rects = self.geometry(bounds, min);
        let source = rects[&slot];
        let axis = match direction {
            Direction::Left | Direction::Right => Axis::Vertical,
            Direction::Up | Direction::Down => Axis::Horizontal,
        };
        let target_boundary = match direction {
            Direction::Right => source.x as i32 + source.cols as i32 + delta as i32,
            Direction::Left => source.x as i32 - delta as i32,
            Direction::Down => source.y as i32 + source.rows as i32 + delta as i32,
            Direction::Up => source.y as i32 - delta as i32,
        };
        self.root
            .resize_boundary(slot, neighbor, axis, target_boundary, bounds, min)
    }

    pub fn swap_slots(&mut self, a: SlotId, b: SlotId) -> Result<(), LayoutError> {
        if !self.contains(a) {
            return Err(LayoutError::UnknownSlot(a));
        }
        if !self.contains(b) {
            return Err(LayoutError::UnknownSlot(b));
        }
        if a == b {
            return Ok(());
        }
        self.root.swap_slots(a, b);
        Ok(())
    }
}

impl Node {
    fn contains(&self, wanted: SlotId) -> bool {
        match self {
            Self::Leaf(slot) => *slot == wanted,
            Self::Split { first, second, .. } => first.contains(wanted) || second.contains(wanted),
        }
    }

    fn collect_slots(&self, result: &mut Vec<SlotId>) {
        match self {
            Self::Leaf(slot) => result.push(*slot),
            Self::Split { first, second, .. } => {
                first.collect_slots(result);
                second.collect_slots(result);
            }
        }
    }

    fn find_leaf_mut(&mut self, wanted: SlotId) -> Option<&mut Node> {
        match self {
            Self::Leaf(slot) if *slot == wanted => Some(self),
            Self::Leaf(_) => None,
            Self::Split { first, second, .. } => first
                .find_leaf_mut(wanted)
                .or_else(|| second.find_leaf_mut(wanted)),
        }
    }

    fn first_slot(&self) -> Option<SlotId> {
        match self {
            Self::Leaf(slot) => Some(*slot),
            Self::Split { first, .. } => first.first_slot(),
        }
    }

    fn sibling_leaf(&self, wanted: SlotId) -> Option<&Node> {
        match self {
            Self::Leaf(_) => None,
            Self::Split { first, second, .. } => {
                if first.contains(wanted) {
                    Some(second)
                } else if second.contains(wanted) {
                    Some(first)
                } else {
                    first
                        .sibling_leaf(wanted)
                        .or_else(|| second.sibling_leaf(wanted))
                }
            }
        }
    }

    fn remove_leaf(self, wanted: SlotId) -> Option<Node> {
        match self {
            Self::Leaf(slot) => {
                if slot == wanted {
                    None
                } else {
                    Some(Self::Leaf(slot))
                }
            }
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let first = first.remove_leaf(wanted);
                let second = second.remove_leaf(wanted);
                match (first, second) {
                    (Some(first), Some(second)) => Some(Self::Split {
                        axis,
                        ratio,
                        first: Box::new(first),
                        second: Box::new(second),
                    }),
                    (Some(only), None) | (None, Some(only)) => Some(only),
                    (None, None) => None,
                }
            }
        }
    }

    fn min_span(&self, min_cols: u16, min_rows: u16) -> (u32, u32) {
        match self {
            Self::Leaf(_) => (min_cols as u32, min_rows as u32),
            Self::Split {
                axis,
                first,
                second,
                ..
            } => {
                let (first_cols, first_rows) = first.min_span(min_cols, min_rows);
                let (second_cols, second_rows) = second.min_span(min_cols, min_rows);
                match axis {
                    Axis::Vertical => (first_cols + second_cols, first_rows.max(second_rows)),
                    Axis::Horizontal => (first_cols.max(second_cols), first_rows + second_rows),
                }
            }
        }
    }

    fn geometry(
        &self,
        bounds: CellRect,
        min_cols: u16,
        min_rows: u16,
        result: &mut BTreeMap<SlotId, CellRect>,
    ) {
        match self {
            Self::Leaf(slot) => {
                result.insert(*slot, bounds);
            }
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (first_need_cols, first_need_rows) = first.min_span(min_cols, min_rows);
                let (second_need_cols, second_need_rows) = second.min_span(min_cols, min_rows);
                let (available, first_need, second_need) = match axis {
                    Axis::Vertical => (bounds.cols as u32, first_need_cols, second_need_cols),
                    Axis::Horizontal => (bounds.rows as u32, first_need_rows, second_need_rows),
                };
                let proportional =
                    available.saturating_mul(*ratio as u32) / LayoutTree::RATIO_SCALE as u32;
                let first_span = if available >= first_need + second_need {
                    proportional.clamp(first_need, available - second_need)
                } else {
                    // A very small terminal cannot satisfy every leaf minimum;
                    // retain an exact, deterministic partition until it grows.
                    proportional.min(available)
                } as u16;
                let (first_bounds, second_bounds) = bounds_at(*axis, bounds, first_span);
                first.geometry(first_bounds, min_cols, min_rows, result);
                second.geometry(second_bounds, min_cols, min_rows, result);
            }
        }
    }

    fn swap_slots(&mut self, a: SlotId, b: SlotId) {
        match self {
            Self::Leaf(slot) => {
                if *slot == a {
                    *slot = b;
                } else if *slot == b {
                    *slot = a;
                }
            }
            Self::Split { first, second, .. } => {
                first.swap_slots(a, b);
                second.swap_slots(a, b);
            }
        }
    }

    fn resize_boundary(
        &mut self,
        source: SlotId,
        neighbor: SlotId,
        axis: Axis,
        target_boundary: i32,
        bounds: CellRect,
        min: CellRect,
    ) -> Result<(), LayoutError> {
        match self {
            Self::Leaf(_) => Err(LayoutError::NoNeighbor {
                slot: source,
                direction: Direction::Right,
            }),
            Self::Split {
                axis: split_axis,
                ratio,
                first,
                second,
            } => {
                let source_first = first.contains(source);
                let neighbor_second = second.contains(neighbor);
                let source_second = second.contains(source);
                let neighbor_first = first.contains(neighbor);
                if *split_axis == axis
                    && ((source_first && neighbor_second) || (source_second && neighbor_first))
                {
                    let total = match axis {
                        Axis::Vertical => bounds.cols,
                        Axis::Horizontal => bounds.rows,
                    } as i32;
                    let origin = match axis {
                        Axis::Vertical => bounds.x,
                        Axis::Horizontal => bounds.y,
                    } as i32;
                    let desired_first = target_boundary - origin;
                    let (first_min_cols, first_min_rows) =
                        first.min_span(min.cols.max(1), min.rows.max(1));
                    let (second_min_cols, second_min_rows) =
                        second.min_span(min.cols.max(1), min.rows.max(1));
                    let (first_min, second_min) = match axis {
                        Axis::Vertical => (first_min_cols, second_min_cols),
                        Axis::Horizontal => (first_min_rows, second_min_rows),
                    };
                    if desired_first < first_min as i32 || total - desired_first < second_min as i32
                    {
                        return Err(LayoutError::MinimumSize);
                    }
                    // Geometry floors the ratio back to cells. Round up here
                    // so a requested outward step cannot disappear (or move
                    // two cells on the left/top edge) on that conversion.
                    *ratio = ((desired_first * LayoutTree::RATIO_SCALE as i32 + total - 1) / total)
                        .clamp(1, (LayoutTree::RATIO_SCALE - 1) as i32)
                        as u16;
                    return Ok(());
                }
                let (first_bounds, second_bounds) =
                    child_bounds(*split_axis, *ratio, bounds, min, first, second);
                if first.contains(source) && first.contains(neighbor) {
                    first.resize_boundary(
                        source,
                        neighbor,
                        axis,
                        target_boundary,
                        first_bounds,
                        min,
                    )
                } else if second.contains(source) && second.contains(neighbor) {
                    second.resize_boundary(
                        source,
                        neighbor,
                        axis,
                        target_boundary,
                        second_bounds,
                        min,
                    )
                } else {
                    Err(LayoutError::NoNeighbor {
                        slot: source,
                        direction: Direction::Right,
                    })
                }
            }
        }
    }
}

fn child_bounds(
    axis: Axis,
    ratio: u16,
    bounds: CellRect,
    min: CellRect,
    first: &Node,
    second: &Node,
) -> (CellRect, CellRect) {
    let min_cols = min.cols.max(1);
    let min_rows = min.rows.max(1);
    let (first_need_cols, first_need_rows) = first.min_span(min_cols, min_rows);
    let (second_need_cols, second_need_rows) = second.min_span(min_cols, min_rows);
    let (available, first_need, second_need) = match axis {
        Axis::Vertical => (bounds.cols as u32, first_need_cols, second_need_cols),
        Axis::Horizontal => (bounds.rows as u32, first_need_rows, second_need_rows),
    };
    let proportional = available.saturating_mul(ratio as u32) / LayoutTree::RATIO_SCALE as u32;
    let span = if available >= first_need + second_need {
        proportional.clamp(first_need, available - second_need)
    } else {
        proportional.min(available)
    } as u16;
    bounds_at(axis, bounds, span)
}

fn bounds_at(axis: Axis, bounds: CellRect, span: u16) -> (CellRect, CellRect) {
    match axis {
        Axis::Vertical => (
            CellRect {
                cols: span,
                ..bounds
            },
            CellRect {
                x: bounds.x.saturating_add(span),
                cols: bounds.cols.saturating_sub(span),
                ..bounds
            },
        ),
        Axis::Horizontal => (
            CellRect {
                rows: span,
                ..bounds
            },
            CellRect {
                y: bounds.y.saturating_add(span),
                rows: bounds.rows.saturating_sub(span),
                ..bounds
            },
        ),
    }
}

fn overlap(a_start: u16, a_len: u16, b_start: u16, b_len: u16) -> bool {
    a_start < b_start.saturating_add(b_len) && b_start < a_start.saturating_add(a_len)
}

fn touches_in_direction(source: CellRect, candidate: CellRect, direction: Direction) -> bool {
    match direction {
        Direction::Left => {
            candidate.x.saturating_add(candidate.cols) == source.x
                && overlap(source.y, source.rows, candidate.y, candidate.rows)
        }
        Direction::Right => {
            source.x.saturating_add(source.cols) == candidate.x
                && overlap(source.y, source.rows, candidate.y, candidate.rows)
        }
        Direction::Up => {
            candidate.y.saturating_add(candidate.rows) == source.y
                && overlap(source.x, source.cols, candidate.x, candidate.cols)
        }
        Direction::Down => {
            source.y.saturating_add(source.rows) == candidate.y
                && overlap(source.x, source.cols, candidate.x, candidate.cols)
        }
    }
}

fn shared_boundary(source: CellRect, candidate: CellRect, direction: Direction) -> u16 {
    match direction {
        Direction::Left | Direction::Right => source
            .y
            .saturating_add(source.rows)
            .min(candidate.y.saturating_add(candidate.rows))
            .saturating_sub(source.y.max(candidate.y)),
        Direction::Up | Direction::Down => source
            .x
            .saturating_add(source.cols)
            .min(candidate.x.saturating_add(candidate.cols))
            .saturating_sub(source.x.max(candidate.x)),
    }
}

fn center_offset(source: CellRect, candidate: CellRect, direction: Direction) -> u16 {
    let (source_center, candidate_center) = match direction {
        Direction::Left | Direction::Right => (
            source.y as u32 * 2 + source.rows as u32,
            candidate.y as u32 * 2 + candidate.rows as u32,
        ),
        Direction::Up | Direction::Down => (
            source.x as u32 * 2 + source.cols as u32,
            candidate.x as u32 * 2 + candidate.cols as u32,
        ),
    };
    source_center
        .abs_diff(candidate_center)
        .min(u16::MAX as u32) as u16
}
