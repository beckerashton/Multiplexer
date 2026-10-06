use mux_core::SessionId;

use crate::selection::{Selection, selected_text};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Point {
    row: usize,
    col: u16,
}

pub enum Outcome {
    Continue,
    Cancel,
    Yank(String),
}

/// A stable view of terminal history. The live PTY keeps running independently.
pub struct SelectionMode {
    pub session: SessionId,
    screen: vt100::Screen,
    history: usize,
    top: usize,
    cursor: Point,
    anchor: Option<Point>,
    linewise: bool,
    count: usize,
    pending_g: bool,
}

impl SelectionMode {
    pub fn new(session: SessionId, screen: &vt100::Screen) -> Self {
        let mut screen = screen.clone();
        screen.set_scrollback(usize::MAX);
        let history = screen.scrollback();
        let (row, col) = screen.cursor_position();
        let top = history;
        let cursor = Point {
            row: history + usize::from(row),
            col: col.min(screen.size().1 - 1),
        };
        let mut mode = Self {
            session,
            screen,
            history,
            top,
            cursor,
            anchor: None,
            linewise: false,
            count: 0,
            pending_g: false,
        };
        mode.reveal();
        mode
    }

    pub fn screen(&self) -> &vt100::Screen {
        &self.screen
    }

    pub fn cursor(&self) -> (u16, u16) {
        (self.cursor.col, (self.cursor.row - self.top) as u16)
    }

    pub fn hint(&self) -> String {
        format!(
            "SELECT {} {}/{} | hjkl w/b/e 0/$ gg/G | v/V select y yank Esc exit",
            if self.anchor.is_some() {
                if self.linewise { "LINE" } else { "VISUAL" }
            } else {
                "NORMAL"
            },
            self.cursor.row + 1,
            self.total_rows()
        )
    }

    fn total_rows(&self) -> usize {
        self.history + usize::from(self.screen.size().0)
    }

    fn range(&self) -> Option<(Point, Point)> {
        let anchor = self.anchor?;
        let (mut start, mut end) = if anchor <= self.cursor {
            (anchor, self.cursor)
        } else {
            (self.cursor, anchor)
        };
        if self.linewise {
            start.col = 0;
            end.col = self.screen.size().1 - 1;
        }
        Some((start, end))
    }

    pub fn visible_selection(&self) -> Option<Selection> {
        let (start, end) = self.range()?;
        let bottom = self.top + usize::from(self.screen.size().0) - 1;
        if end.row < self.top || start.row > bottom {
            return None;
        }
        Some(Selection {
            session: self.session,
            start: if start.row < self.top {
                (0, 0)
            } else {
                (start.col, (start.row - self.top) as u16)
            },
            end: if end.row > bottom {
                (self.screen.size().1 - 1, self.screen.size().0 - 1)
            } else {
                (end.col, (end.row - self.top) as u16)
            },
        })
    }

    // Place an absolute buffer row in the snapshot viewport and return its
    // local row. Always restore the view with reveal() after inspecting rows.
    fn inspect_row(&mut self, row: usize) -> u16 {
        self.screen.set_scrollback(self.history.saturating_sub(row));
        row.saturating_sub(self.history) as u16
    }

    fn class(&mut self, point: Point) -> u8 {
        let row = self.inspect_row(point.row);
        let col = if self
            .screen
            .cell(row, point.col)
            .is_some_and(vt100::Cell::is_wide_continuation)
        {
            point.col.saturating_sub(1)
        } else {
            point.col
        };
        let ch = self
            .screen
            .cell(row, col)
            .and_then(|cell| cell.contents().chars().next());
        match ch {
            None => 0,
            Some(ch) if ch.is_whitespace() => 0,
            Some(ch) if ch.is_alphanumeric() || ch == '_' => 1,
            Some(_) => 2,
        }
    }

    fn next(&self, point: Point) -> Point {
        if point.col + 1 < self.screen.size().1 {
            Point {
                col: point.col + 1,
                ..point
            }
        } else if point.row + 1 < self.total_rows() {
            Point {
                row: point.row + 1,
                col: 0,
            }
        } else {
            point
        }
    }

    fn previous(&self, point: Point) -> Point {
        if point.col > 0 {
            Point {
                col: point.col - 1,
                ..point
            }
        } else if point.row > 0 {
            Point {
                row: point.row - 1,
                col: self.screen.size().1 - 1,
            }
        } else {
            point
        }
    }

    fn hard_boundary(&mut self, a: Point, b: Point) -> bool {
        if a.row == b.row {
            return false;
        }
        let row = self.inspect_row(a.row.min(b.row));
        !self.screen.row_wrapped(row)
    }

    fn word(&mut self, key: u8) {
        let mut point = self.cursor;
        match key {
            b'w' => {
                let class = self.class(point);
                while self.next(point) != point && self.class(point) == class {
                    let next = self.next(point);
                    let boundary = self.hard_boundary(point, next);
                    point = next;
                    if boundary && class != 0 {
                        break;
                    }
                }
                while self.next(point) != point && self.class(point) == 0 {
                    point = self.next(point);
                }
            }
            b'b' => {
                point = self.previous(point);
                while self.previous(point) != point && self.class(point) == 0 {
                    point = self.previous(point);
                }
                let class = self.class(point);
                while self.previous(point) != point
                    && !self.hard_boundary(point, self.previous(point))
                    && self.class(self.previous(point)) == class
                {
                    point = self.previous(point);
                }
            }
            b'e' => {
                point = self.next(point);
                while self.next(point) != point && self.class(point) == 0 {
                    point = self.next(point);
                }
                let class = self.class(point);
                while self.next(point) != point
                    && !self.hard_boundary(point, self.next(point))
                    && self.class(self.next(point)) == class
                {
                    point = self.next(point);
                }
            }
            _ => {}
        }
        self.cursor = point;
    }

    fn reveal(&mut self) {
        self.cursor.row = self.cursor.row.min(self.total_rows() - 1);
        self.cursor.col = self.cursor.col.min(self.screen.size().1 - 1);
        let row = self.inspect_row(self.cursor.row);
        if self
            .screen
            .cell(row, self.cursor.col)
            .is_some_and(vt100::Cell::is_wide_continuation)
        {
            self.cursor.col = self.cursor.col.saturating_sub(1);
        }
        if self.cursor.row < self.top {
            self.top = self.cursor.row;
        }
        if self.cursor.row >= self.top + usize::from(self.screen.size().0) {
            self.top = self.cursor.row + 1 - usize::from(self.screen.size().0);
        }
        self.top = self.top.min(self.history);
        self.screen.set_scrollback(self.history - self.top);
    }

    pub fn selected_text(&mut self) -> String {
        self.selected_text_with_line_ending(if cfg!(windows) { "\r\n" } else { "\n" })
    }

    fn selected_text_with_line_ending(&mut self, line_ending: &str) -> String {
        let Some((start, end)) = self.range() else {
            return String::new();
        };
        let mut text = String::new();
        for row in start.row..=end.row {
            let local = self.inspect_row(row);
            let row_text = selected_text(
                &self.screen,
                &Selection {
                    session: self.session,
                    start: (if row == start.row { start.col } else { 0 }, local),
                    end: (
                        if row == end.row {
                            end.col
                        } else {
                            self.screen.size().1 - 1
                        },
                        local,
                    ),
                },
            );
            if (row < end.row && (self.linewise || !self.screen.row_wrapped(local)))
                || (row == end.row && self.linewise)
            {
                text.push_str(row_text.trim_end());
                text.push_str(line_ending);
            } else {
                text.push_str(&row_text);
            }
        }
        self.reveal();
        text
    }

    pub fn input(&mut self, bytes: &[u8]) -> Outcome {
        // Whole keyboard frames arrive here: Alt, function keys, and unknown
        // escape sequences are consumed atomically, never as hjkl characters.
        let [key] = bytes else {
            return Outcome::Continue;
        };
        if *key == 27 {
            return Outcome::Cancel;
        }
        if key.is_ascii_digit() && (*key != b'0' || self.count > 0) {
            self.count = (self.count * 10 + usize::from(key - b'0')).min(100_000);
            return Outcome::Continue;
        }
        let had_count = self.count > 0;
        let count = std::mem::take(&mut self.count).max(1);
        let previous_g = std::mem::take(&mut self.pending_g);
        match *key {
            b'h' => {
                self.cursor.col = self
                    .cursor
                    .col
                    .saturating_sub(count.min(u16::MAX as usize) as u16)
            }
            b'l' => {
                for _ in 0..count.min(usize::from(self.screen.size().1)) {
                    let row = self.inspect_row(self.cursor.row);
                    let width = if self
                        .screen
                        .cell(row, self.cursor.col)
                        .is_some_and(vt100::Cell::is_wide)
                    {
                        2
                    } else {
                        1
                    };
                    self.cursor.col = self
                        .cursor
                        .col
                        .saturating_add(width)
                        .min(self.screen.size().1 - 1);
                }
            }
            b'j' => self.cursor.row = self.cursor.row.saturating_add(count),
            b'k' => self.cursor.row = self.cursor.row.saturating_sub(count),
            b'0' => self.cursor.col = 0,
            b'$' => {
                let row = self.inspect_row(self.cursor.row);
                self.cursor.col = (0..self.screen.size().1)
                    .rev()
                    .find(|col| {
                        self.screen
                            .cell(row, *col)
                            .is_some_and(|c| !c.contents().is_empty())
                    })
                    .unwrap_or(0);
            }
            b'^' => {
                let row = self.inspect_row(self.cursor.row);
                self.cursor.col = (0..self.screen.size().1)
                    .find(|col| {
                        self.screen
                            .cell(row, *col)
                            .is_some_and(|c| !c.contents().trim().is_empty())
                    })
                    .unwrap_or(0);
            }
            b'g' if previous_g => {
                self.cursor.row = count - 1;
                self.cursor.col = 0;
            }
            b'g' => {
                self.pending_g = true;
                self.count = count;
            }
            b'G' => {
                self.cursor.row = if had_count {
                    count - 1
                } else {
                    self.total_rows() - 1
                };
                self.cursor.col = 0;
            }
            b'w' | b'b' | b'e' => {
                for _ in 0..count.min(self.total_rows() * usize::from(self.screen.size().1)) {
                    self.word(*key);
                }
            }
            4 | 6 => {
                self.cursor.row = self.cursor.row.saturating_add(
                    count * usize::from(self.screen.size().0) / if *key == 4 { 2 } else { 1 },
                )
            }
            21 | 2 => {
                self.cursor.row = self.cursor.row.saturating_sub(
                    count * usize::from(self.screen.size().0) / if *key == 21 { 2 } else { 1 },
                )
            }
            b'v' | b'V' => {
                let linewise = *key == b'V';
                if self.anchor.is_some() && self.linewise == linewise {
                    self.anchor = None;
                } else {
                    self.anchor.get_or_insert(self.cursor);
                    self.linewise = linewise;
                }
            }
            b'y' => {
                if self.anchor.is_none() {
                    self.anchor = Some(self.cursor);
                    self.linewise = true;
                }
                return Outcome::Yank(self.selected_text());
            }
            _ => {}
        }
        self.reveal();
        Outcome::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(text: &[u8], rows: u16, cols: u16) -> SelectionMode {
        let mut parser = vt100::Parser::new(rows, cols, 100);
        parser.process(text);
        SelectionMode::new(SessionId(1), parser.screen())
    }

    fn keys(mode: &mut SelectionMode, keys: &[u8]) -> Outcome {
        let mut result = Outcome::Continue;
        for key in keys {
            result = mode.input(&[*key]);
        }
        result
    }

    fn yank(mode: &mut SelectionMode, input: &[u8]) -> String {
        match keys(mode, input) {
            Outcome::Yank(text) => text,
            _ => panic!("expected yank"),
        }
    }

    #[test]
    fn visual_words_counts_line_edges_and_reverse_selection() {
        let mut m = mode(b"alpha beta, gamma\x1b[1;1H", 3, 30);
        assert_eq!(m.cursor(), (0, 0));
        assert_eq!(yank(&mut m, b"wvey"), "beta");
        // Yank does not destroy state, so a failed clipboard write can retry.
        assert_eq!(yank(&mut m, b"y"), "beta");
        keys(&mut m, b"v0$3h");
        assert_eq!(m.cursor(), (13, 0));
        assert_eq!(yank(&mut m, b"v3hy"), ", ga");
        keys(&mut m, b"v0e");
        assert_eq!(m.cursor(), (4, 0));
        keys(&mut m, b"wb");
        assert_eq!(m.cursor(), (0, 0));
    }

    #[test]
    fn multiline_yanks_trim_trailing_whitespace_and_preserve_line_endings() {
        for line_ending in ["\n", "\r\n"] {
            let mut m = mode(b"  first  \r\n   \r\n  last  \x1b[1;1H", 4, 16);
            keys(&mut m, b"V2j");
            assert_eq!(
                m.selected_text_with_line_ending(line_ending),
                format!("  first{line_ending}{line_ending}  last{line_ending}")
            );
            // Character selections retain the spaces inside a soft-wrapped line.
            let mut m = mode(b"abc def\x1b[1;1H", 3, 4);
            keys(&mut m, b"vj$");
            assert_eq!(m.selected_text_with_line_ending(line_ending), "abc def");
            let mut m = mode(b"one   \r\n  two\x1b[1;1H", 3, 16);
            keys(&mut m, b"vj$");
            assert_eq!(
                m.selected_text_with_line_ending(line_ending),
                format!("one{line_ending}  two")
            );
        }
        let mut m = mode(b"one   \x1b[1;1H", 2, 16);
        assert_eq!(yank(&mut m, b"v5ly"), "one   ");
        let mut m = mode(b"  one   \x1b[1;1H", 2, 16);
        assert_eq!(yank(&mut m, b"y"), "  one\n");
    }

    #[test]
    fn line_selection_crosses_scrollback_and_snapshot_is_frozen() {
        let mut parser = vt100::Parser::new(3, 12, 100);
        parser.process(b"zero\r\none\r\ntwo\r\nthree\r\nfour");
        let mut m = SelectionMode::new(SessionId(1), parser.screen());
        assert_eq!(m.cursor(), (4, 2));
        parser.process(b"\r\nlive output");
        keys(&mut m, b"ggV3j");
        assert_eq!(m.screen().contents(), "one\ntwo\nthree");
        assert_eq!(yank(&mut m, b"y"), "zero\none\ntwo\nthree\n");
        let selected = m.visible_selection().unwrap();
        assert_eq!(selected.start, (0, 0));
        assert_eq!(selected.end, (11, 2));
        keys(&mut m, b"vG");
        assert_eq!(m.cursor(), (0, 2));
        assert!(m.screen().contents().contains("four"));
        assert!(!m.screen().contents().contains("live output"));
    }

    #[test]
    fn wrapping_unicode_and_wide_character_motions() {
        let mut m = mode("a界e\u{301}z\x1b[1;1H".as_bytes(), 2, 4);
        keys(&mut m, b"l");
        assert_eq!(m.cursor(), (1, 0));
        keys(&mut m, b"l");
        assert_eq!(m.cursor(), (3, 0));
        keys(&mut m, b"h");
        assert_eq!(m.cursor(), (1, 0));
        assert_eq!(yank(&mut m, b"0vj$y"), "a界e\u{301}z");
        let mut m = mode("界 word\x1b[1;1H".as_bytes(), 2, 12);
        keys(&mut m, b"w");
        assert_eq!(m.cursor(), (3, 0));
    }

    #[test]
    fn word_motions_distinguish_hard_newlines_from_soft_wraps() {
        let mut m = mode(b"abcd\r\nefgh\x1b[1;1H", 3, 4);
        keys(&mut m, b"w");
        assert_eq!(m.cursor, Point { row: 1, col: 0 });
        keys(&mut m, b"e");
        assert_eq!(m.cursor, Point { row: 1, col: 3 });
        keys(&mut m, b"b");
        assert_eq!(m.cursor, Point { row: 1, col: 0 });
        let mut m = mode(b"abcdefgh\x1b[1;1H", 3, 4);
        keys(&mut m, b"e");
        assert_eq!(m.cursor, Point { row: 1, col: 3 });
        keys(&mut m, b"b");
        assert_eq!(m.cursor, Point { row: 0, col: 0 });
    }

    #[test]
    fn counts_pages_escape_and_unknown_frames() {
        let mut m = mode(b"one\r\ntwo\r\nthree\r\nfour\r\nfive", 3, 12);
        keys(&mut m, b"1G");
        assert_eq!(m.cursor.row, 0);
        keys(&mut m, b"3gg");
        assert_eq!(m.cursor.row, 2);
        keys(&mut m, &[6]);
        assert_eq!(m.cursor.row, 4);
        keys(&mut m, &[2]);
        assert_eq!(m.cursor.row, 1);
        keys(&mut m, b"99999k99999h");
        assert_eq!(m.cursor, Point { row: 0, col: 0 });
        for bytes in [
            b"\x1bh".as_slice(),
            b"\x1b[104;3u",
            b"\x1b[A",
            b"q",
            b"\x02r",
        ] {
            assert!(matches!(m.input(bytes), Outcome::Continue));
            assert_eq!(m.cursor, Point { row: 0, col: 0 });
        }
        assert!(matches!(m.input(&[27]), Outcome::Cancel));
    }
}
