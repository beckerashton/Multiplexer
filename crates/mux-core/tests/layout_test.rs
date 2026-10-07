use mux_core::{Axis, CellRect, Direction, LayoutError, LayoutTree, SlotId};

const MIN: CellRect = CellRect {
    x: 0,
    y: 0,
    cols: 8,
    rows: 3,
};

#[test]
fn equalize_balances_nested_panes_on_only_the_requested_axis() {
    for (axis, perpendicular) in [
        (Axis::Vertical, Axis::Horizontal),
        (Axis::Horizontal, Axis::Vertical),
    ] {
        let mut tree = LayoutTree::new(SlotId(1));
        assert!(!tree.equalize(axis));
        tree.split(SlotId(1), axis, 800, SlotId(2)).unwrap();
        tree.split(SlotId(2), axis, 200, SlotId(3)).unwrap();
        tree.split(SlotId(1), perpendicular, 300, SlotId(4))
            .unwrap();
        let area = CellRect {
            cols: 120,
            rows: 120,
            ..bounds()
        };
        let before = tree.geometry(area, MIN);
        let slots = tree.slots();
        assert!(tree.equalize(axis));
        assert!(tree.is_valid());
        assert_eq!(tree.slots(), slots);
        let after = tree.geometry(area, MIN);
        for (slot, rect) in &after {
            match axis {
                Axis::Vertical => {
                    assert_eq!(rect.cols, 40);
                    assert_eq!((rect.y, rect.rows), (before[slot].y, before[slot].rows));
                }
                Axis::Horizontal => {
                    assert_eq!(rect.rows, 40);
                    assert_eq!((rect.x, rect.cols), (before[slot].x, before[slot].cols));
                }
            }
        }
        assert!(!tree.equalize(axis));
        let small = tree.geometry(
            CellRect {
                cols: 25,
                rows: 10,
                ..area
            },
            MIN,
        );
        assert!(
            small
                .values()
                .all(|r| r.cols >= MIN.cols && r.rows >= MIN.rows)
        );
        assert_eq!(
            small
                .values()
                .map(|r| r.cols as u32 * r.rows as u32)
                .sum::<u32>(),
            250
        );
    }
}

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

#[test]
fn every_resize_pushes_the_requested_edge_out_by_one_cell() {
    for (axis, forward, backward) in [
        (Axis::Vertical, Direction::Right, Direction::Left),
        (Axis::Horizontal, Direction::Down, Direction::Up),
    ] {
        for span in 24..=400 {
            let area = CellRect {
                x: 4,
                y: 2,
                cols: span,
                rows: span,
            };
            for (slot, direction) in [(SlotId(1), forward), (SlotId(2), backward)] {
                let mut tree = LayoutTree::new(SlotId(1));
                tree.split(SlotId(1), axis, 500, SlotId(2)).unwrap();
                for _ in 0..3 {
                    let before = tree.geometry(area, MIN)[&slot];
                    tree.resize(slot, direction, 1, area, MIN).unwrap();
                    let after = tree.geometry(area, MIN)[&slot];
                    match direction {
                        Direction::Right => {
                            assert_eq!(after.x, before.x);
                            assert_eq!(after.cols, before.cols + 1, "span {span}");
                        }
                        Direction::Left => {
                            assert_eq!(after.x + 1, before.x, "span {span}");
                            assert_eq!(after.x + after.cols, before.x + before.cols);
                        }
                        Direction::Down => {
                            assert_eq!(after.y, before.y);
                            assert_eq!(after.rows, before.rows + 1, "span {span}");
                        }
                        Direction::Up => {
                            assert_eq!(after.y + 1, before.y, "span {span}");
                            assert_eq!(after.y + after.rows, before.y + before.rows);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn nested_resize_uses_selected_side_and_outer_edges_do_not_move() {
    let mut tree = LayoutTree::new(SlotId(1));
    tree.split(SlotId(1), Axis::Vertical, 500, SlotId(2))
        .unwrap();
    tree.split(SlotId(1), Axis::Horizontal, 500, SlotId(3))
        .unwrap();
    tree.split(SlotId(3), Axis::Vertical, 500, SlotId(4))
        .unwrap();
    for direction in [Direction::Left, Direction::Right, Direction::Up] {
        let before = tree.geometry(bounds(), MIN)[&SlotId(4)];
        tree.resize(SlotId(4), direction, 1, bounds(), MIN).unwrap();
        let after = tree.geometry(bounds(), MIN)[&SlotId(4)];
        match direction {
            Direction::Left => assert_eq!(after.x, before.x - 1),
            Direction::Right => assert_eq!(after.x + after.cols, before.x + before.cols + 1),
            Direction::Up => assert_eq!(after.y, before.y - 1),
            _ => unreachable!(),
        }
    }
    let before = tree.clone();
    assert_eq!(
        tree.resize(SlotId(4), Direction::Down, 1, bounds(), MIN),
        Err(LayoutError::NoNeighbor {
            slot: SlotId(4),
            direction: Direction::Down
        })
    );
    assert_eq!(tree, before);
}
