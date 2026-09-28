use mux_core::{Axis, CellRect, Direction, LayoutError, LayoutTree, SlotId};

const MIN: CellRect = CellRect {
    x: 0,
    y: 0,
    cols: 8,
    rows: 3,
};

fn bounds() -> CellRect {
    CellRect {
        x: 4,
        y: 2,
        cols: 80,
        rows: 30,
    }
}

#[test]
fn nested_geometry_partitions_every_cell_and_respects_minimums() {
    let mut tree = LayoutTree::new(SlotId(1));
    tree.split(SlotId(1), Axis::Vertical, 500, SlotId(2))
        .unwrap();
    tree.split(SlotId(1), Axis::Horizontal, 500, SlotId(3))
        .unwrap();
    tree.split(SlotId(2), Axis::Horizontal, 500, SlotId(4))
        .unwrap();
    let rects = tree.geometry(bounds(), MIN);
    assert_eq!(rects.len(), 4);
    assert!(
        rects
            .values()
            .all(|rect| rect.cols >= MIN.cols && rect.rows >= MIN.rows)
    );
    for y in bounds().y..bounds().y + bounds().rows {
        for x in bounds().x..bounds().x + bounds().cols {
            assert_eq!(
                rects
                    .values()
                    .filter(|rect| x >= rect.x
                        && x < rect.x + rect.cols
                        && y >= rect.y
                        && y < rect.y + rect.rows)
                    .count(),
                1,
                "cell {x},{y}"
            );
        }
    }
}

#[test]
fn spatial_neighbor_prefers_longest_shared_edge_then_center() {
    let mut tree = LayoutTree::new(SlotId(1));
    tree.split(SlotId(1), Axis::Vertical, 500, SlotId(2))
        .unwrap();
    tree.split(SlotId(2), Axis::Horizontal, 500, SlotId(3))
        .unwrap();
    let rects = tree.geometry(bounds(), MIN);
    // Slot 1 touches both 2 and 3 to the right. They tie on shared edge, so
    // deterministic traversal chooses slot 2 (the first child of the split).
    assert_eq!(
        tree.neighbor(SlotId(1), Direction::Right, bounds(), MIN),
        Some(SlotId(2))
    );
    assert_eq!(
        tree.neighbor(SlotId(2), Direction::Left, bounds(), MIN),
        Some(SlotId(1))
    );
    assert_eq!(
        rects[&SlotId(2)].x,
        rects[&SlotId(1)].x + rects[&SlotId(1)].cols
    );
}

#[test]
fn resize_crosses_t_junction_and_is_bounded_by_descendant_minimums() {
    let mut tree = LayoutTree::new(SlotId(1));
    tree.split(SlotId(1), Axis::Vertical, 500, SlotId(2))
        .unwrap();
    tree.split(SlotId(2), Axis::Horizontal, 500, SlotId(3))
        .unwrap();
    let before = tree.geometry(bounds(), MIN);
    tree.resize(SlotId(1), Direction::Right, 6, bounds(), MIN)
        .unwrap();
    let after = tree.geometry(bounds(), MIN);
    assert!(after[&SlotId(1)].cols > before[&SlotId(1)].cols);
    assert!(
        after
            .values()
            .all(|rect| rect.cols >= MIN.cols && rect.rows >= MIN.rows)
    );
    assert_eq!(
        tree.resize(SlotId(1), Direction::Right, 200, bounds(), MIN),
        Err(LayoutError::MinimumSize)
    );
}

#[test]
fn swap_moves_slot_identity_without_changing_partition() {
    let mut tree = LayoutTree::new(SlotId(1));
    tree.split(SlotId(1), Axis::Vertical, 500, SlotId(2))
        .unwrap();
    let before = tree.geometry(bounds(), MIN);
    tree.swap_slots(SlotId(1), SlotId(2)).unwrap();
    let after = tree.geometry(bounds(), MIN);
    assert_eq!(before[&SlotId(1)], after[&SlotId(2)]);
    assert_eq!(before[&SlotId(2)], after[&SlotId(1)]);
}

#[test]
fn vertical_wrap_stays_in_column_and_preserves_full_height_panes() {
    let mut tree = LayoutTree::new(SlotId(1));
    tree.split(SlotId(1), Axis::Vertical, 500, SlotId(2))
        .unwrap();
    tree.split(SlotId(1), Axis::Horizontal, 500, SlotId(3))
        .unwrap();
    assert_eq!(
        tree.vertical_wrap_neighbor(SlotId(1), Direction::Up, bounds(), MIN),
        Some(SlotId(3))
    );
    assert_eq!(
        tree.vertical_wrap_neighbor(SlotId(3), Direction::Down, bounds(), MIN),
        Some(SlotId(1))
    );
    assert_eq!(
        tree.vertical_wrap_neighbor(SlotId(2), Direction::Up, bounds(), MIN),
        Some(SlotId(2))
    );
    assert_eq!(
        tree.vertical_wrap_neighbor(SlotId(2), Direction::Down, bounds(), MIN),
        Some(SlotId(2))
    );
}
