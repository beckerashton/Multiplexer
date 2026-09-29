use mux_core::SessionId;

/// A screen-local cell selection for one terminal session.
/// Coordinates are zero-based `(column, row)` and both endpoints are
/// inclusive. Keyboard selection clips its absolute buffer range to this
/// representation for rendering and text extraction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Selection {
    pub session: SessionId,
    pub start: (u16, u16),
    pub end: (u16, u16),
}

/// Extracts plain text for the inclusive selection. Reversed drags are
/// normalized in terminal reading order and endpoints outside the visible
/// screen are clipped. `vt100` emits a newline between selected physical rows
/// only when the prior row was not soft-wrapped.
#[must_use]
pub fn selected_text(screen: &vt100::Screen, selection: &Selection) -> String {
    let (rows, cols) = screen.size();
    if rows == 0 || cols == 0 {
        return String::new();
    }

    let start = clamp_point(selection.start, cols, rows);
    let end = clamp_point(selection.end, cols, rows);
    let (mut start_col, start_row, mut end_col, end_row) =
        if reading_order(start) <= reading_order(end) {
            (start.0, start.1, end.0, end.1)
        } else {
            (end.0, end.1, start.0, start.1)
        };

    // A selection endpoint can land on either half of a double-width glyph. Include
    // the glyph once, rather than returning an empty continuation cell or a
    // partial terminal character.
    if screen
        .cell(start_row, start_col)
        .is_some_and(vt100::Cell::is_wide_continuation)
    {
        start_col = start_col.saturating_sub(1);
    }
    if screen
        .cell(end_row, end_col)
        .is_some_and(vt100::Cell::is_wide)
        && end_col + 1 < cols
    {
        end_col += 1;
    }

    // `contents_between` has an exclusive final column. The selected end is
    // inclusive, so use its following cell (or `cols` at the right edge).
    screen.contents_between(
        start_row,
        start_col,
        end_row,
        end_col.saturating_add(1).min(cols),
    )
}

fn clamp_point((col, row): (u16, u16), cols: u16, rows: u16) -> (u16, u16) {
    (col.min(cols - 1), row.min(rows - 1))
}

fn reading_order((col, row): (u16, u16)) -> (u16, u16) {
    (row, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(bytes: &[u8], rows: u16, cols: u16) -> vt100::Screen {
        let mut parser = vt100::Parser::new(rows, cols, 0);
        parser.process(bytes);
        parser.screen().clone()
    }

    #[test]
    fn reverse_drag_is_normalized_and_inclusive() {
        let screen = screen(b"alpha", 2, 8);
        let selection = Selection {
            session: SessionId(7),
            start: (3, 0),
            end: (1, 0),
        };
        assert_eq!(selected_text(&screen, &selection), "lph");
    }

    #[test]
    fn multiline_selection_uses_newline_for_hard_rows_and_clips_bounds() {
        let screen = screen(b"one\r\ntwo", 3, 6);
        let selection = Selection {
            session: SessionId(3),
            start: (99, 1),
            end: (0, 0),
        };
        assert_eq!(selected_text(&screen, &selection), "one\ntwo");
    }

    #[test]
    fn soft_wrapped_rows_do_not_add_a_copy_newline() {
        let screen = screen(b"abcde", 2, 3);
        let selection = Selection {
            session: SessionId(3),
            start: (0, 0),
            end: (1, 1),
        };
        assert_eq!(selected_text(&screen, &selection), "abcde");
    }

    #[test]
    fn wide_glyph_selection_never_returns_its_continuation_as_text() {
        let screen = screen("a界z".as_bytes(), 1, 8);
        let continuation_only = Selection {
            session: SessionId(1),
            start: (2, 0),
            end: (2, 0),
        };
        let lead_only = Selection {
            session: SessionId(1),
            start: (1, 0),
            end: (1, 0),
        };
        assert_eq!(selected_text(&screen, &continuation_only), "界");
        assert_eq!(selected_text(&screen, &lead_only), "界");
    }
}
