use mux_core::{CellRect, SlotStackView};

pub fn workspace_bounds(cols: u16, rows: u16) -> CellRect {
    CellRect {
        x: 0,
        y: rows.min(1),
        cols,
        rows: rows.saturating_sub(2),
    }
}

/// Leave one collapsed edge per hidden member, above/below the active member.
/// On short terminals, share the available rows between both sides while
/// preserving the expanded pane's borders and at least one content row.
pub fn stack_frame(rect: CellRect, stack: &SlotStackView) -> CellRect {
    let before = stack.active.unwrap_or(0).min(stack.sessions.len());
    let after = stack.sessions.len().saturating_sub(before + 1);
    let budget = usize::from(rect.rows.saturating_sub(3));
    let mut top = before.min(budget / 2);
    let bottom = after.min(budget - top);
    top = before.min(budget - bottom);
    CellRect {
        y: rect.y.saturating_add(top as u16),
        rows: rect.rows.saturating_sub((top + bottom) as u16),
        ..rect
    }
}

pub fn content(rect: CellRect) -> CellRect {
    CellRect {
        x: rect.x.saturating_add(1),
        y: rect.y.saturating_add(1),
        cols: rect.cols.saturating_sub(2),
        rows: rect.rows.saturating_sub(2),
    }
}

pub fn contains(rect: CellRect, x: u16, y: u16) -> bool {
    x >= rect.x
        && y >= rect.y
        && x < rect.x.saturating_add(rect.cols)
        && y < rect.y.saturating_add(rect.rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mux_core::SessionId;

    #[test]
    fn stack_edges_preserve_content_and_stay_inside_short_panes() {
        for rows in 0..20 {
            for count in 1..12 {
                for active in 0..count {
                    let rect = CellRect {
                        x: 4,
                        y: 2,
                        cols: 20,
                        rows,
                    };
                    let stack = SlotStackView {
                        sessions: (0..count).map(|id| SessionId(id as u64)).collect(),
                        active: Some(active),
                    };
                    let frame = stack_frame(rect, &stack);
                    let top = frame.y - rect.y;
                    let bottom = rect.y + rect.rows - frame.y - frame.rows;
                    assert!(usize::from(top) <= active);
                    assert!(usize::from(bottom) < count - active);
                    assert_eq!(frame.rows + top + bottom, rows);
                    assert!(frame.rows >= rows.min(3));
                    if usize::from(rows) >= count + 2 {
                        assert_eq!(usize::from(top), active);
                        assert_eq!(usize::from(bottom), count - active - 1);
                    }
                }
            }
        }
    }
}
