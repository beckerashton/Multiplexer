use std::{
    cell::RefCell,
    io::{self, Write},
    rc::Rc,
};

use mux_core::{
    BindingConfig, BroadcastScope, InputRouter, PendingConfirmation, Workspace, WorkspaceView,
};
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::{Backend, CrosstermBackend},
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget},
};

use crate::{backend::TerminalBackend, selection::Selection, selection_mode::SelectionMode};

// Dracula's standard palette: https://spec.draculatheme.com/#sec-Standard
mod theme {
    use ratatui::style::Color;

    pub const BACKGROUND: Color = Color::Reset;
    pub const FOREGROUND: Color = Color::Rgb(0xf8, 0xf8, 0xf2);
    pub const SELECTION: Color = Color::Rgb(0x44, 0x47, 0x5a);
    pub const MUTED: Color = Color::Rgb(0x62, 0x72, 0xa4);
    pub const PURPLE: Color = Color::Rgb(0xbd, 0x93, 0xf9);
    pub const CYAN: Color = Color::Rgb(0x8b, 0xe9, 0xfd);
    pub const PINK: Color = Color::Rgb(0xff, 0x79, 0xc6);
    pub const RED: Color = Color::Rgb(0xff, 0x55, 0x55);
}

/// Buffer a complete Ratatui update before exposing it to the host. Sync mode
/// prevents tearing on supporting terminals; hiding the cursor also protects
/// terminals that ignore sync mode. TerminalGuard owns final mode restoration.
pub struct FrameWriter<W> {
    output: W,
    pending: Vec<u8>,
}

impl<W: Write> Write for FrameWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // Crossterm flushes cursor commands independently. Commit only after
        // the entire Ratatui draw (including its final cursor) has completed.
        Ok(())
    }
}

impl<W: Write> FrameWriter<W> {
    fn commit(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let mut frame = Vec::with_capacity(self.pending.len() + 32);
        frame.extend_from_slice(b"\x1b[?2026h\x1b[?25l");
        frame.append(&mut self.pending);
        frame.extend_from_slice(b"\x1b[?2026l");
        self.output.write_all(&frame)?;
        self.output.flush()
    }
}

#[derive(PartialEq)]
struct Scene {
    buffer: Buffer,
    cursor: Option<(u16, u16)>,
}

pub struct SharedWriter<W>(Rc<RefCell<FrameWriter<W>>>);

impl<W: Write> Write for SharedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub struct Renderer<W: Write> {
    terminal: Terminal<CrosstermBackend<SharedWriter<W>>>,
    writer: Rc<RefCell<FrameWriter<W>>>,
    previous: Option<Scene>,
}

impl Renderer<io::Stdout> {
    pub fn new() -> io::Result<Self> {
        let (cols, rows) = crossterm::terminal::size()?;
        Self::with_output(io::stdout(), Rect::new(0, 0, cols, rows))
    }
}

impl<W: Write> Renderer<W> {
    fn with_output(output: W, area: Rect) -> io::Result<Self> {
        let writer = Rc::new(RefCell::new(FrameWriter {
            output,
            pending: Vec::new(),
        }));
        let backend = CrosstermBackend::new(SharedWriter(Rc::clone(&writer)));
        // Use the same sampled dimensions for composition and output, avoiding
        // a second size query halfway through a resize frame.
        let terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )?;
        Ok(Self {
            terminal,
            writer,
            previous: None,
        })
    }

    fn present(&mut self, scene: Scene) -> io::Result<()> {
        // Ratatui also emits cursor instructions on an unchanged draw. Avoid
        // the entire draw when both the composed cells and cursor are equal.
        if self.previous.as_ref() == Some(&scene) {
            return Ok(());
        }
        if self
            .previous
            .as_ref()
            .is_none_or(|old| old.buffer.area != scene.buffer.area)
        {
            self.terminal = Terminal::with_options(
                CrosstermBackend::new(SharedWriter(Rc::clone(&self.writer))),
                TerminalOptions {
                    viewport: Viewport::Fixed(scene.buffer.area),
                },
            )?;
            self.terminal.backend_mut().clear()?;
        }
        self.terminal.draw(|frame| {
            *frame.buffer_mut() = scene.buffer.clone();
        })?;
        // Restore position before showing the cursor, including on hosts that
        // ignore synchronized updates. All commands remain in the same batch.
        if let Some(cursor) = scene.cursor {
            self.terminal.set_cursor_position(cursor)?;
            self.terminal.show_cursor()?;
        }
        self.writer.borrow_mut().commit()?;
        self.previous = Some(scene);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        workspace: &Workspace,
        backend: &mut TerminalBackend,
        router: &InputRouter,
        config: &BindingConfig,
        status: &str,
        quit_pending: bool,
        selection_mode: Option<&SelectionMode>,
    ) -> io::Result<()> {
        let (cols, rows) = crossterm::terminal::size()?;
        let scene = compose(
            Rect::new(0, 0, cols, rows),
            workspace,
            backend,
            router,
            config,
            status,
            quit_pending,
            selection_mode,
        );
        self.present(scene)
    }
}

#[allow(clippy::too_many_arguments)]
fn compose(
    area: Rect,
    workspace: &Workspace,
    backend: &mut TerminalBackend,
    router: &InputRouter,
    config: &BindingConfig,
    status: &str,
    quit_pending: bool,
    selection_mode: Option<&SelectionMode>,
) -> Scene {
    let mut buffer = Buffer::empty(area);
    buffer.set_style(
        area,
        Style::default().fg(theme::FOREGROUND).bg(theme::BACKGROUND),
    );
    let view = workspace.view();
    let tab = view
        .tabs
        .iter()
        .find(|tab| tab.id == view.active_tab)
        .expect("active tab");
    let (cols, rows) = (area.width, area.height);
    let bounds = crate::pane_geometry::workspace_bounds(cols, rows);
    let rects = tab.layout.geometry(bounds, view.minimum_pane_size);

    let mut cursor = None;
    for (slot_id, rect) in rects {
        let Some(slot) = tab.slots.get(&slot_id) else {
            continue;
        };
        let Some(id) = slot
            .stack
            .active
            .and_then(|index| slot.stack.sessions.get(index))
            .copied()
        else {
            continue;
        };
        let frame = crate::pane_geometry::stack_frame(rect, &slot.stack);
        let inner = crate::pane_geometry::content(frame);
        let width = inner.cols;
        let height = inner.rows;
        if width == 0 || height == 0 {
            continue;
        }
        if backend.resize(id, (width, height)).is_err() {
            continue;
        }
        let focused = slot_id == tab.focused_slot;
        let header = format!(
            " {} · {}/{}{} ",
            tab.layout
                .slots()
                .iter()
                .position(|id| *id == slot_id)
                .unwrap_or(0)
                + 1,
            slot.stack.active.unwrap_or(0) + 1,
            slot.stack.sessions.len(),
            if tab.manual_broadcast_targets.contains(&slot_id) {
                " *"
            } else {
                ""
            }
        );
        paint_stack_edges(&mut buffer, rect, frame, &slot.stack);
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(if focused { theme::PURPLE } else { theme::MUTED }))
            .title(header)
            .render(
                Rect::new(frame.x, frame.y, frame.cols, frame.rows),
                &mut buffer,
            );
        let mode = selection_mode.filter(|mode| mode.session == id);
        let mode_selection = mode.and_then(SelectionMode::visible_selection);
        if let Some(screen) = mode
            .map(SelectionMode::screen)
            .or_else(|| backend.screen(id))
        {
            paint_screen(
                &mut buffer,
                screen,
                Rect::new(inner.x, inner.y, width, height),
                mode_selection.as_ref(),
            );
            if focused && (mode.is_some() || (!screen.hide_cursor() && screen.scrollback() == 0)) {
                let (row, col) = mode
                    .map(|mode| {
                        let (col, row) = mode.cursor();
                        (row, col)
                    })
                    .unwrap_or_else(|| screen.cursor_position());
                if row < height && col < width {
                    cursor = Some((inner.x.saturating_add(col), inner.y.saturating_add(row)));
                }
            }
        }
    }
    let scope = match view.broadcast_scope {
        BroadcastScope::Focused => "focused".to_string(),
        BroadcastScope::VisibleTab => format!("BROADCAST ALL ({})", tab.slots.len()),
        BroadcastScope::Manual => {
            format!("BROADCAST MANUAL ({})", tab.manual_broadcast_targets.len())
        }
    };
    let prompt = if quit_pending {
        confirmation_text(&view, true)
    } else if view.pending_confirmation.is_some() {
        confirmation_text(&view, false)
    } else if let Some(mode) = selection_mode {
        mode.hint()
    } else if router.resize_mode() {
        "RESIZE: hjkl / HJKL · Alt-hjkl focus · Esc exits".into()
    } else if router.broadcast_pending() {
        "BROADCAST MENU".into()
    } else if router.leader_pending() {
        "LEADER".into()
    } else if router.carry_pending() {
        "CARRY: choose 1-9".into()
    } else {
        String::new()
    };
    if rows > 0 && cols > 0 {
        buffer.set_stringn(
            0,
            rows - 1,
            clip(
                &if selection_mode.is_some() && !status.is_empty() {
                    format!("{} · {}", status, prompt)
                } else {
                    format!("tab {}  {}  {}  {}", tab.number, scope, prompt, status)
                },
                cols,
            ),
            cols as usize,
            Style::default().fg(if quit_pending || view.pending_confirmation.is_some() {
                theme::RED
            } else if view.broadcast_scope != BroadcastScope::Focused {
                theme::PINK
            } else if router.leader_pending() || router.carry_pending() || router.modal() {
                theme::CYAN
            } else {
                theme::FOREGROUND
            }),
        );
    }
    if rows > 0 && cols > 0 {
        let mut x = 0;
        for (index, (label, active)) in tab_labels(&view).into_iter().enumerate() {
            if index > 0 {
                let (next, _) = buffer.set_stringn(
                    x,
                    0,
                    " │ ",
                    cols.saturating_sub(x) as usize,
                    Style::default().fg(theme::MUTED),
                );
                x = next;
            }
            let style = Style::default()
                .fg(if active {
                    theme::PURPLE
                } else {
                    theme::FOREGROUND
                })
                .bg(if active {
                    theme::SELECTION
                } else {
                    theme::BACKGROUND
                })
                .add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                });
            let (next, _) = buffer.set_stringn(x, 0, label, cols.saturating_sub(x) as usize, style);
            x = next;
            if x >= cols {
                break;
            }
        }
    }
    if tab.slots.is_empty() && bounds.rows > 0 && cols > 0 {
        Paragraph::new("Empty tab — leader | or a to start a shell; Alt-1…9 to switch tabs")
            .style(Style::default().fg(theme::MUTED))
            .render(Rect::new(0, bounds.y, cols, bounds.rows), &mut buffer);
    }
    if (router.leader_pending() || router.modal())
        && !quit_pending
        && view.pending_confirmation.is_none()
    {
        let runtime = crate::runtime_config::RuntimeConfig {
            bindings: config.clone(),
        };
        if router.resize_mode() {
            bindings_popup(
                &mut buffer,
                &runtime.resize_help(),
                " Resize mode ",
                "hjkl / HJKL resize · Esc exits",
            );
        } else if router.broadcast_pending() {
            bindings_popup(
                &mut buffer,
                &runtime.broadcast_help(),
                " Broadcast ",
                "Press a command · Esc cancels",
            );
        } else {
            leader_popup(&mut buffer, config);
        }
        cursor = None;
    }
    Scene { buffer, cursor }
}

fn paint_stack_edges(
    buffer: &mut Buffer,
    rect: mux_core::CellRect,
    frame: mux_core::CellRect,
    stack: &mux_core::SlotStackView,
) {
    if rect.cols < 2 {
        return;
    }
    let style = Style::default().fg(theme::MUTED);
    let active = stack.active.unwrap_or(0);
    for y in rect.y..rect.y + rect.rows {
        let (member, left, right) = if y < frame.y {
            (active - usize::from(frame.y - y), "╭", "╮")
        } else if y >= frame.y + frame.rows {
            (active + 1 + usize::from(y - frame.y - frame.rows), "╰", "╯")
        } else {
            continue;
        };
        buffer.set_string(rect.x, y, left, style);
        for x in rect.x + 1..rect.x + rect.cols - 1 {
            buffer.set_string(x, y, "─", style);
        }
        buffer.set_string(rect.x + rect.cols - 1, y, right, style);
        buffer.set_stringn(
            rect.x + 1,
            y,
            format!(" {} ", member + 1),
            usize::from(rect.cols.saturating_sub(2)),
            style,
        );
    }
}

fn tab_labels(view: &WorkspaceView) -> Vec<(String, bool)> {
    view.tabs
        .iter()
        .map(|tab| {
            let active = tab.id == view.active_tab;
            let panes: String = tab
                .layout
                .slots()
                .iter()
                .filter(|id| tab.slots.contains_key(id))
                .map(|id| {
                    if active && *id == tab.focused_slot {
                        '▪'
                    } else {
                        '▫'
                    }
                })
                .collect();
            let panes = if panes.is_empty() { "∅" } else { &panes };
            (format!(" {} {} ", tab.number, panes), active)
        })
        .collect()
}

/// Balance whole command groups across two columns, wrapping within each column.
fn popup_columns(help: &[(String, bool)], width: usize) -> [Vec<(String, bool)>; 2] {
    let width = width.max(1);
    let mut groups: Vec<Vec<(String, bool)>> = Vec::new();
    for (text, heading) in help {
        if *heading || groups.is_empty() {
            groups.push(Vec::new());
        }
        let group = groups.last_mut().unwrap();
        let mut line = String::new();
        for word in text.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                group.push((std::mem::take(&mut line), *heading));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            for ch in word.chars() {
                if line.chars().count() == width {
                    group.push((std::mem::take(&mut line), *heading));
                }
                line.push(ch);
            }
        }
        if !line.is_empty() {
            group.push((line, *heading));
        }
    }
    let height = |groups: &[Vec<(String, bool)>]| {
        groups.iter().map(Vec::len).sum::<usize>() + groups.len().saturating_sub(1)
    };
    let split = (1..groups.len())
        .min_by_key(|&split| height(&groups[..split]).max(height(&groups[split..])))
        .unwrap_or(groups.len());
    let mut columns = [Vec::new(), Vec::new()];
    for (index, group) in groups.into_iter().enumerate() {
        let column = &mut columns[usize::from(index >= split)];
        if !column.is_empty() {
            column.push((String::new(), false));
        }
        column.extend(group);
    }
    columns
}

fn leader_popup(buffer: &mut Buffer, config: &BindingConfig) {
    let help = crate::runtime_config::RuntimeConfig {
        bindings: config.clone(),
    }
    .popup_help();
    bindings_popup(
        buffer,
        &help,
        &format!(
            " Key bindings · {} ",
            crate::runtime_config::format_key(config.leader())
        ),
        "Press a binding · Esc dismisses",
    );
}

fn bindings_popup(buffer: &mut Buffer, help: &[(String, bool)], title: &str, hint: &str) {
    let area = buffer.area;
    if area.width < 4 || area.height < 4 {
        return;
    }
    let desired_width = help
        .iter()
        .map(|(line, _)| line.chars().count())
        .max()
        .unwrap_or(1)
        .saturating_mul(2)
        .saturating_add(5)
        .min(u16::MAX as usize) as u16;
    let width = area.width.saturating_sub(4).max(4).min(desired_width);
    let column_width = width.saturating_sub(3) / 2;
    let columns = popup_columns(help, column_width as usize);
    let content_rows = columns
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0)
        .min(u16::MAX as usize - 3) as u16;
    let height = (content_rows + 3).min(area.height.saturating_sub(2));
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    Clear.render(popup, buffer);
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title)
        .style(Style::default().fg(theme::FOREGROUND).bg(theme::BACKGROUND))
        .border_style(Style::default().fg(theme::PURPLE))
        .render(popup, buffer);
    let available_rows = height.saturating_sub(3);
    for (col, lines) in columns.iter().enumerate() {
        for (row, (line, heading)) in lines.iter().take(available_rows as usize).enumerate() {
            buffer.set_stringn(
                popup.x + 1 + col as u16 * (column_width + 1),
                popup.y + 1 + row as u16,
                line,
                column_width as usize,
                Style::default()
                    .fg(if *heading {
                        theme::PURPLE
                    } else {
                        theme::FOREGROUND
                    })
                    .bg(theme::BACKGROUND),
            );
        }
    }
    let hint = if content_rows > available_rows {
        "More bindings: multiplexer --help · Esc dismisses"
    } else {
        hint
    };
    buffer.set_stringn(
        popup.x + 1,
        popup.y + height - 2,
        hint,
        width.saturating_sub(2) as usize,
        Style::default().fg(theme::CYAN).bg(theme::BACKGROUND),
    );
}

fn paint_screen(
    buffer: &mut Buffer,
    screen: &vt100::Screen,
    area: Rect,
    selection: Option<&Selection>,
) {
    for row in 0..area.height {
        for col in 0..area.width {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            // Do not let a wide glyph overwrite the adjacent pane or gutter.
            if cell.is_wide() && col + 1 >= area.width {
                continue;
            }
            let mut style = Style::default()
                .fg(convert_color(cell.fgcolor(), theme::FOREGROUND))
                .bg(convert_color(cell.bgcolor(), theme::BACKGROUND));
            for (enabled, modifier) in [
                (cell.bold(), Modifier::BOLD),
                (cell.underline(), Modifier::UNDERLINED),
                (cell.italic(), Modifier::ITALIC),
                (cell.inverse(), Modifier::REVERSED),
            ] {
                if enabled {
                    style = style.add_modifier(modifier);
                }
            }
            if selection.is_some_and(|selection| {
                selection_contains(selection, col, row)
                    || (cell.is_wide() && selection_contains(selection, col + 1, row))
            }) {
                style = style
                    .fg(theme::FOREGROUND)
                    .bg(theme::SELECTION)
                    .remove_modifier(Modifier::REVERSED);
            }
            let text = if cell.contents().is_empty() {
                " "
            } else {
                cell.contents()
            };
            buffer[(area.x + col, area.y + row)]
                .set_symbol(text)
                .set_style(style);
        }
    }
}

fn selection_contains(selection: &Selection, col: u16, row: u16) -> bool {
    let (start, end) =
        if (selection.start.1, selection.start.0) <= (selection.end.1, selection.end.0) {
            (selection.start, selection.end)
        } else {
            (selection.end, selection.start)
        };
    (start.1..=end.1).contains(&row)
        && (row != start.1 || col >= start.0)
        && (row != end.1 || col <= end.0)
}

fn confirmation_text(view: &WorkspaceView, quit_pending: bool) -> String {
    if quit_pending {
        let count = view
            .sessions
            .values()
            .filter(|state| !matches!(state, mux_core::SessionState::Exited))
            .count();
        return format!("QUIT: terminate {count} session(s)? y/n");
    }
    let Some(pending) = view.pending_confirmation else {
        return String::new();
    };
    let tab = |id| view.tabs.iter().find(|tab| tab.id == id);
    match pending {
        PendingConfirmation::KillSession { .. } => "KILL 1 session? y/n".into(),
        PendingConfirmation::KillStack { tab: tab_id, slot } => {
            let count = tab(tab_id)
                .and_then(|tab| tab.slots.get(&slot))
                .map(|slot| slot.stack.sessions.len())
                .unwrap_or(0);
            format!("KILL {count} session(s)? y/n")
        }
        PendingConfirmation::CloseTab(tab_id) => {
            let count = tab(tab_id)
                .map(|tab| {
                    tab.slots
                        .values()
                        .map(|slot| slot.stack.sessions.len())
                        .sum::<usize>()
                })
                .unwrap_or(0);
            format!("CLOSE TAB: terminate {count} session(s)? y/n")
        }
    }
}

fn convert_color(color: vt100::Color, default: Color) -> Color {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

fn clip(value: &str, width: u16) -> String {
    use unicode_width::UnicodeWidthChar;

    let mut used = 0_u16;
    let mut clipped = String::new();
    for character in value.chars() {
        let cell_width = character.width().unwrap_or(0) as u16;
        if used.saturating_add(cell_width) > width {
            break;
        }
        clipped.push(character);
        used = used.saturating_add(cell_width);
    }
    clipped
}

#[cfg(test)]
mod tests {
    #[test]
    fn popup_renders_complete_groups_in_two_columns() {
        let config = mux_core::BindingConfig::default();
        let help = crate::runtime_config::RuntimeConfig {
            bindings: config.clone(),
        }
        .popup_help();
        let area = ratatui::layout::Rect::new(0, 0, 120, 40);
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        super::leader_popup(&mut buffer, &config);
        let rows: Vec<String> = (0..area.height)
            .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        let first = rows
            .iter()
            .find(|row| row.contains("Manage Panes"))
            .unwrap();
        assert!(first.find("Manage Tabs").unwrap() > first.find("Manage Panes").unwrap());
        for (line, _) in help {
            assert!(
                rows.iter().any(|row| row.contains(&line)),
                "missing: {line}"
            );
        }
        assert!(
            rows.iter()
                .any(|row| row.contains("Press a binding · Esc dismisses"))
        );
    }

    #[test]
    fn popup_wraps_without_losing_bindings_and_handles_small_areas() {
        let config = mux_core::BindingConfig::default();
        let help = crate::runtime_config::RuntimeConfig {
            bindings: config.clone(),
        }
        .popup_help();
        let columns = super::popup_columns(&help, 36);
        for column in &columns {
            assert!(column.first().unwrap().1);
            assert!(column.iter().all(|(line, _)| line.chars().count() <= 36));
        }
        let original = help
            .iter()
            .map(|(line, _)| line.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let wrapped = columns
            .iter()
            .flatten()
            .map(|(line, _)| line.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            original.split_whitespace().collect::<Vec<_>>(),
            wrapped.split_whitespace().collect::<Vec<_>>()
        );
        for (width, height) in [(4, 4), (8, 6), (40, 12), (80, 24)] {
            let mut buffer =
                ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(3, 2, width, height));
            super::leader_popup(&mut buffer, &config);
        }
    }

    use super::{clip, confirmation_text};
    use mux_core::{Axis, SessionSpec, Workspace, WorkspaceCommand};
    use std::path::PathBuf;

    #[test]
    fn clip_uses_terminal_cell_width() {
        assert_eq!(clip("a界b", 3), "a界");
        assert_eq!(clip("界", 1), "");
        assert_eq!(clip("e\u{301}x", 1), "e\u{301}");
    }

    fn scene(parser: &vt100::Parser, area: ratatui::layout::Rect) -> super::Scene {
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        super::paint_screen(&mut buffer, parser.screen(), area, None);
        let (row, col) = parser.screen().cursor_position();
        super::Scene {
            buffer,
            cursor: (!parser.screen().hide_cursor()).then_some((col, row)),
        }
    }

    #[test]
    fn fullscreen_frames_are_incremental_and_cursor_updates_are_atomic() {
        use ratatui::layout::Rect;
        for (cols, rows) in [(240, 67), (320, 90)] {
            let area = Rect::new(0, 0, cols, rows);
            let mut renderer = super::Renderer::with_output(Vec::new(), area).unwrap();
            let mut guest = vt100::Parser::new(rows, cols, 0);
            guest.process(b"prompt> ");
            renderer.present(scene(&guest, area)).unwrap();
            let mut host = vt100::Parser::new(rows, cols, 0);
            host.process(&std::mem::take(&mut renderer.writer.borrow_mut().output));
            renderer.present(scene(&guest, area)).unwrap();
            assert!(renderer.writer.borrow().output.is_empty());

            guest.process(b"a");
            renderer.present(scene(&guest, area)).unwrap();
            let bytes = std::mem::take(&mut renderer.writer.borrow_mut().output);
            assert!(
                bytes.len() < 256,
                "single-cell update emitted {} bytes",
                bytes.len()
            );
            assert!(!bytes.windows(3).any(|bytes| bytes == b"[2J"));
            assert!(bytes.starts_with(b"\x1b[?2026h\x1b[?25l"));
            assert!(bytes.ends_with(b"\x1b[?2026l"));
            assert_eq!(
                bytes
                    .windows(8)
                    .filter(|bytes| *bytes == b"\x1b[?2026h")
                    .count(),
                1
            );
            host.process(&bytes);
            assert_eq!(
                host.screen().contents().trim_end(),
                guest.screen().contents().trim_end()
            );
            assert_eq!(
                host.screen().cursor_position(),
                guest.screen().cursor_position()
            );
            assert!(!host.screen().hide_cursor());

            guest.process(b"\x1b[1;3H");
            renderer.present(scene(&guest, area)).unwrap();
            let bytes = std::mem::take(&mut renderer.writer.borrow_mut().output);
            assert!(bytes.len() < 256);
            host.process(&bytes);
            assert_eq!(host.screen().cursor_position(), (0, 2));
            guest.process(b"\x1b[?25l");
            renderer.present(scene(&guest, area)).unwrap();
            host.process(&std::mem::take(&mut renderer.writer.borrow_mut().output));
            assert!(host.screen().hide_cursor());
        }
    }

    #[test]
    fn pane_cells_preserve_unicode_styles_and_selection() {
        use ratatui::{
            buffer::Buffer,
            layout::Rect,
            style::{Color, Modifier},
        };
        let mut parser = vt100::Parser::new(2, 12, 0);
        parser.process("\x1b[1;3;4;7;38;2;12;34;56;48;5;123m界e\u{301}".as_bytes());
        let mut buffer = Buffer::empty(Rect::new(0, 0, 16, 4));
        let pane = Rect::new(2, 1, 12, 2);
        super::paint_screen(&mut buffer, parser.screen(), pane, None);
        let cell = &buffer[(2, 1)];
        assert_eq!(cell.symbol(), "界");
        assert_eq!(cell.fg, Color::Rgb(12, 34, 56));
        assert_eq!(cell.bg, Color::Indexed(123));
        assert!(cell.modifier.contains(
            Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED | Modifier::REVERSED
        ));
        assert_eq!(buffer[(4, 1)].symbol(), "e\u{301}");
        let selection = crate::selection::Selection {
            session: mux_core::SessionId(1),
            start: (1, 0),
            end: (1, 0),
        };
        super::paint_screen(&mut buffer, parser.screen(), pane, Some(&selection));
        assert_eq!(buffer[(2, 1)].bg, super::theme::SELECTION);
        assert_eq!(buffer[(1, 1)].symbol(), " ");
    }

    #[test]
    fn wide_glyph_replacement_and_resize_erase_stale_cells() {
        use ratatui::layout::Rect;
        let area = Rect::new(0, 0, 240, 67);
        let mut renderer = super::Renderer::with_output(Vec::new(), area).unwrap();
        let mut guest = vt100::Parser::new(67, 240, 0);
        let mut host = vt100::Parser::new(67, 240, 0);
        for text in ["界e\u{301}", "\rabc", "\r界 ", "\r\x1b[K"] {
            guest.process(text.as_bytes());
            renderer.present(scene(&guest, area)).unwrap();
            host.process(&std::mem::take(&mut renderer.writer.borrow_mut().output));
            assert_eq!(
                host.screen()
                    .contents()
                    .trim_end()
                    .lines()
                    .map(str::trim_end)
                    .collect::<Vec<_>>(),
                guest
                    .screen()
                    .contents()
                    .trim_end()
                    .lines()
                    .map(str::trim_end)
                    .collect::<Vec<_>>()
            );
        }
        for (cols, rows) in [(320, 90), (200, 55)] {
            guest.screen_mut().set_size(rows, cols);
            host.screen_mut().set_size(rows, cols);
            guest.process(format!("\x1b[{rows};1Hbottom").as_bytes());
            renderer
                .present(scene(&guest, Rect::new(0, 0, cols, rows)))
                .unwrap();
            host.process(&std::mem::take(&mut renderer.writer.borrow_mut().output));
            assert_eq!(
                host.screen()
                    .contents()
                    .trim_end()
                    .lines()
                    .map(str::trim_end)
                    .collect::<Vec<_>>(),
                guest
                    .screen()
                    .contents()
                    .trim_end()
                    .lines()
                    .map(str::trim_end)
                    .collect::<Vec<_>>()
            );
        }
    }

    fn shell() -> SessionSpec {
        SessionSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec![],
            cwd: None,
        }
    }

    #[test]
    fn collapsed_members_render_on_both_sides_of_expanded_pane() {
        let rect = mux_core::CellRect {
            x: 2,
            y: 1,
            cols: 12,
            rows: 9,
        };
        let stack = mux_core::SlotStackView {
            sessions: (0..3).map(mux_core::SessionId).collect(),
            active: Some(1),
        };
        let frame = crate::pane_geometry::stack_frame(rect, &stack);
        let mut buffer = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 16, 12));
        super::paint_stack_edges(&mut buffer, rect, frame, &stack);
        assert_eq!(buffer[(2, 1)].symbol(), "╭");
        assert_eq!(buffer[(4, 1)].symbol(), "1");
        assert_eq!(buffer[(13, 1)].symbol(), "╮");
        assert_eq!(buffer[(2, 9)].symbol(), "╰");
        assert_eq!(buffer[(4, 9)].symbol(), "3");
        assert_eq!(buffer[(13, 9)].symbol(), "╯");
        assert_eq!(buffer[(2, 2)].symbol(), " ");
        assert_eq!(crate::pane_geometry::content(frame).rows, 5);
    }

    #[test]
    fn confirmation_prompts_include_counts_and_replacement_notice() {
        let (mut workspace, _) = Workspace::new(shell());
        workspace
            .execute(WorkspaceCommand::RequestKillFocusedSession)
            .unwrap();
        assert_eq!(
            confirmation_text(&workspace.view(), false),
            "KILL 1 session? y/n"
        );
        workspace.clear_pending_confirmation();
        workspace
            .execute(WorkspaceCommand::AddToFocusedStack { session: shell() })
            .unwrap();
        workspace
            .execute(WorkspaceCommand::RequestKillFocusedStack)
            .unwrap();
        assert_eq!(
            confirmation_text(&workspace.view(), false),
            "KILL 2 session(s)? y/n"
        );
        workspace.clear_pending_confirmation();
        workspace
            .execute(WorkspaceCommand::CreateTab { session: shell() })
            .unwrap();
        let tab = workspace.view().active_tab;
        workspace
            .execute(WorkspaceCommand::RequestCloseTab(tab))
            .unwrap();
        assert_eq!(
            confirmation_text(&workspace.view(), false),
            "CLOSE TAB: terminate 1 session(s)? y/n"
        );
        assert!(confirmation_text(&workspace.view(), true).contains("terminate 3 session(s)"));
        let _ = Axis::Vertical;
    }
}
