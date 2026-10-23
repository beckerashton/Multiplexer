use mux_core::{CellRect, PaneVisibility, SlotStackView};

pub fn workspace_bounds(cols: u16, rows: u16, top_bar_hidden: bool) -> CellRect {
    if top_bar_hidden {
        return CellRect { x: 0, y: 0, cols, rows };
    }
    CellRect {
        x: 0,
        y: rows.min(1),
        cols,
        rows: rows.saturating_sub(1),
    }
}

/// Leave one collapsed edge per hidden member, above/below the active member.
/// On short terminals, share the available rows between both sides while
/// preserving at least one content row and any expanded pane borders.
pub fn stack_frame(rect: CellRect, stack: &SlotStackView, visibility: PaneVisibility) -> CellRect {
    if !visibility.stack_lines {
        return rect;
    }
    let before = stack.active.unwrap_or(0).min(stack.sessions.len());
    let after = stack.sessions.len().saturating_sub(before + 1);
    let budget = usize::from(
        rect.rows
            .saturating_sub(1 + u16::from(visibility.top) + u16::from(visibility.bottom)),
    );
    let mut top = before.min(budget / 2);
    let bottom = after.min(budget - top);
    top = before.min(budget - bottom);
    CellRect {
        y: rect.y.saturating_add(top as u16),
        rows: rect.rows.saturating_sub((top + bottom) as u16),
        ..rect
    }
}

pub fn pane_content(rect: CellRect, stack: &SlotStackView, visibility: PaneVisibility) -> CellRect {
    inset(stack_frame(rect, stack, visibility), visibility)
}

pub fn inset(rect: CellRect, visibility: PaneVisibility) -> CellRect {
    let left = u16::from(visibility.left).min(rect.cols);
    let top = u16::from(visibility.top).min(rect.rows);
    CellRect {
        x: rect.x.saturating_add(left),
        y: rect.y.saturating_add(top),
        cols: rect.cols.saturating_sub(left + u16::from(visibility.right)),
        rows: rect.rows.saturating_sub(top + u16::from(visibility.bottom)),
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
    fn every_hidden_decoration_releases_its_cells() {
        let rect = CellRect {
            x: 0,
            y: 1,
            cols: 80,
            rows: 23,
        };
        let stack = SlotStackView {
            sessions: vec![SessionId(1), SessionId(2), SessionId(3)],
            active: Some(1),
        };
        for flags in 0..32 {
            let v = PaneVisibility {
                top: flags & 1 != 0,
                left: flags & 2 != 0,
                bottom: flags & 4 != 0,
                right: flags & 8 != 0,
                stack_lines: flags & 16 != 0,
            };
            let inner = pane_content(rect, &stack, v);
            assert_eq!(inner.x, u16::from(v.left));
            assert_eq!(inner.y, 1 + u16::from(v.top) + u16::from(v.stack_lines));
            assert_eq!(inner.cols, 80 - u16::from(v.left) - u16::from(v.right));
            assert_eq!(
                inner.rows,
                23 - u16::from(v.top) - u16::from(v.bottom) - 2 * u16::from(v.stack_lines)
            );
        }
        assert_eq!(
            workspace_bounds(80, 24, true),
            CellRect {
                x: 0,
                y: 0,
                cols: 80,
                rows: 24
            }
        );
    }

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
                    let frame = stack_frame(rect, &stack, PaneVisibility::default());
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
