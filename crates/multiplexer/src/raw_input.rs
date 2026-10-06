use mux_core::InputEvent;

/// Unix terminals supply VT bytes; Windows consoles supply input records.
/// Both feed the same paste, mouse, keyboard, and command decoders.
pub fn start_reader(sender: std::sync::mpsc::SyncSender<Vec<u8>>) {
    std::thread::spawn(move || {
        #[cfg(unix)]
        {
            use std::io::Read;
            let mut stdin = std::io::stdin().lock();
            let mut bytes = [0_u8; 4096];
            while let Ok(count) = stdin.read(&mut bytes) {
                if count == 0 || sender.send(bytes[..count].to_vec()).is_err() {
                    break;
                }
            }
        }
        #[cfg(windows)]
        while let Ok(event) = crossterm::event::read() {
            if sender.send(console_event_bytes(event)).is_err() {
                break;
            }
        }
    });
}

#[cfg(any(windows, test))]
fn console_event_bytes(event: crossterm::event::Event) -> Vec<u8> {
    use crossterm::event::{
        Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    };

    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            let modifiers = u8::from(key.modifiers.contains(KeyModifiers::SHIFT))
                | (u8::from(key.modifiers.contains(KeyModifiers::ALT)) << 1)
                | (u8::from(key.modifiers.contains(KeyModifiers::CONTROL)) << 2);
            let modifier = modifiers + 1;
            let character = match key.code {
                KeyCode::Char(ch) => Some(ch),
                KeyCode::Enter => Some('\r'),
                KeyCode::Tab => Some('\t'),
                KeyCode::Backspace => Some('\x7f'),
                KeyCode::Esc => Some('\x1b'),
                KeyCode::Null => Some('\0'),
                _ => None,
            };
            if let Some(ch) = character {
                // Windows reports AltGr as Ctrl+Alt. Preserve generated
                // symbols and Unicode text; ASCII letters/digits retain
                // their workspace shortcut meaning.
                if matches!(key.code, KeyCode::Char(_))
                    && modifiers & 6 == 6
                    && (!ch.is_ascii()
                        || (!ch.is_ascii_alphanumeric()
                            && !ch.is_ascii_whitespace()
                            && !ch.is_ascii_control()))
                {
                    return ch.to_string().into_bytes();
                }
                if modifiers & 6 == 0 && !(ch == '\t' && modifiers & 1 != 0) {
                    return ch.to_string().into_bytes();
                }
                // Reuse KeyboardDecoder's normalization, including Ctrl-Alt
                // digits. The console already resolved the shifted character.
                return format!("\x1b[{}:{};{}u", ch as u32, ch as u32, modifier).into_bytes();
            }
            let cursor = match key.code {
                KeyCode::Up => Some('A'),
                KeyCode::Down => Some('B'),
                KeyCode::Right => Some('C'),
                KeyCode::Left => Some('D'),
                KeyCode::Home => Some('H'),
                KeyCode::End => Some('F'),
                _ => None,
            };
            if let Some(cursor) = cursor {
                return if modifiers == 0 {
                    format!("\x1b[{cursor}").into_bytes()
                } else {
                    format!("\x1b[1;{modifier}{cursor}").into_bytes()
                };
            }
            if let KeyCode::F(number @ 1..=4) = key.code {
                let final_byte = char::from(b'P' + number - 1);
                return if modifiers == 0 {
                    format!("\x1bO{final_byte}").into_bytes()
                } else {
                    format!("\x1b[1;{modifier}{final_byte}").into_bytes()
                };
            }
            let number = match key.code {
                KeyCode::Insert => 2,
                KeyCode::Delete => 3,
                KeyCode::PageUp => 5,
                KeyCode::PageDown => 6,
                KeyCode::F(number @ 5..=20) => [
                    15, 17, 18, 19, 20, 21, 23, 24, 25, 26, 28, 29, 31, 32, 33, 34,
                ][usize::from(number - 5)],
                KeyCode::BackTab => return b"\x1b[Z".to_vec(),
                _ => return Vec::new(),
            };
            if modifiers == 0 {
                format!("\x1b[{number}~").into_bytes()
            } else {
                format!("\x1b[{number};{modifier}~").into_bytes()
            }
        }
        Event::Mouse(mouse) => {
            let button = |button| match button {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
            };
            let (mut code, suffix) = match mouse.kind {
                MouseEventKind::Down(b) => (button(b), 'M'),
                MouseEventKind::Up(b) => (button(b), 'm'),
                MouseEventKind::Drag(b) => (button(b) | 32, 'M'),
                MouseEventKind::Moved => (35, 'M'),
                MouseEventKind::ScrollUp => (64, 'M'),
                MouseEventKind::ScrollDown => (65, 'M'),
                MouseEventKind::ScrollLeft => (66, 'M'),
                MouseEventKind::ScrollRight => (67, 'M'),
            };
            code |= u8::from(mouse.modifiers.contains(KeyModifiers::SHIFT)) << 2;
            code |= u8::from(mouse.modifiers.contains(KeyModifiers::ALT)) << 3;
            code |= u8::from(mouse.modifiers.contains(KeyModifiers::CONTROL)) << 4;
            format!(
                "\x1b[<{code};{};{}{suffix}",
                u32::from(mouse.column) + 1,
                u32::from(mouse.row) + 1
            )
            .into_bytes()
        }
        Event::Paste(text) => [RawInput::START, text.as_bytes(), RawInput::END].concat(),
        // Resize wakes the loop, which queries terminal::size itself. Focus
        // and key release events have no bytes to forward to hosted programs.
        _ => Vec::new(),
    }
}

/// Frames bracketed paste before command routing. The payload remains opaque,
/// including leader and confirmation bytes.
#[derive(Default)]
pub struct RawInput {
    pending: Vec<u8>,
    paste: Option<Vec<u8>>,
}

impl RawInput {
    const START: &'static [u8] = b"\x1b[200~";
    const END: &'static [u8] = b"\x1b[201~";

    pub fn push(&mut self, bytes: &[u8]) -> Vec<InputEvent> {
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        loop {
            if let Some(paste) = &mut self.paste {
                if let Some(end) = find(&self.pending, Self::END) {
                    paste.extend_from_slice(&self.pending[..end]);
                    self.pending.drain(..end + Self::END.len());
                    let mut framed = Self::START.to_vec();
                    framed.append(paste);
                    framed.extend_from_slice(Self::END);
                    events.push(InputEvent::Paste(framed));
                    self.paste = None;
                    continue;
                }
                let keep = suffix_prefix_len(&self.pending, Self::END);
                let count = self.pending.len() - keep;
                paste.extend_from_slice(&self.pending[..count]);
                self.pending.drain(..count);
                break;
            }
            if let Some(start) = find(&self.pending, Self::START) {
                if start > 0 {
                    events.push(InputEvent::Bytes(self.pending.drain(..start).collect()));
                }
                self.pending.drain(..Self::START.len());
                self.paste = Some(Vec::new());
                continue;
            }
            // Retain a possible split start delimiter; forward only safe prefix.
            let keep = suffix_prefix_len(&self.pending, Self::START);
            let count = self.pending.len().saturating_sub(keep);
            if count > 0 {
                events.push(InputEvent::Bytes(self.pending.drain(..count).collect()));
            }
            break;
        }
        events
    }

    /// Releases bytes retained solely because they might become a bracketed
    /// paste marker. Call this from the monotonic input deadline so a lone
    /// Escape is never swallowed while normal bytes remain immediate.
    pub fn flush_marker_prefix(&mut self) -> Vec<InputEvent> {
        if self.paste.is_some() || self.pending.is_empty() {
            return Vec::new();
        }
        vec![InputEvent::Bytes(std::mem::take(&mut self.pending))]
    }

    pub fn marker_prefix_pending(&self) -> bool {
        !self.pending.is_empty() && self.paste.is_none()
    }
}

fn find(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes
        .windows(needle.len())
        .position(|window| window == needle)
}

fn suffix_prefix_len(bytes: &[u8], marker: &[u8]) -> usize {
    (1..marker.len())
        .rev()
        .find(|&len| bytes.ends_with(&marker[..len]))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_events_preserve_shortcuts_text_mouse_and_paste() {
        use crossterm::event::{
            Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers as Mods, MouseEvent,
            MouseEventKind,
        };
        for (code, modifiers, expected) in [
            (KeyCode::Char('b'), Mods::CONTROL, b"\x02".as_slice()),
            (KeyCode::Char('1'), Mods::CONTROL | Mods::ALT, b"\x1b[49;7u"),
            (KeyCode::Char('h'), Mods::ALT, b"\x1bh"),
            (KeyCode::Char('é'), Mods::NONE, "é".as_bytes()),
            (
                KeyCode::Char('€'),
                Mods::CONTROL | Mods::ALT,
                "€".as_bytes(),
            ),
            (KeyCode::Char('@'), Mods::CONTROL | Mods::ALT, b"@"),
            (KeyCode::Char('A'), Mods::SHIFT, b"A"),
            (KeyCode::Enter, Mods::NONE, b"\r"),
            (KeyCode::Up, Mods::CONTROL, b"\x1b[1;5A"),
            (KeyCode::Delete, Mods::NONE, b"\x1b[3~"),
            (KeyCode::F(1), Mods::NONE, b"\x1bOP"),
            (KeyCode::F(12), Mods::SHIFT, b"\x1b[24;2~"),
            (KeyCode::BackTab, Mods::SHIFT, b"\x1b[Z"),
        ] {
            let bytes = console_event_bytes(Event::Key(KeyEvent::new(code, modifiers)));
            let mut keyboard = crate::keyboard::KeyboardDecoder::default();
            assert_eq!(keyboard.push(&bytes).concat(), expected, "{code:?}");
        }
        assert!(
            console_event_bytes(Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('b'),
                Mods::CONTROL,
                KeyEventKind::Release
            )))
            .is_empty()
        );
        assert_eq!(
            RawInput::default().push(&console_event_bytes(Event::Paste("\x02qy".into()))),
            vec![InputEvent::Paste(b"\x1b[200~\x02qy\x1b[201~".to_vec())]
        );
        // Windows can deliver bracketed paste as individual console chars.
        let mut input = RawInput::default();
        let mut pasted = Vec::new();
        for ch in "\x1b[200~\x02qy\x1b[201~".chars() {
            pasted.extend(input.push(&console_event_bytes(Event::Key(KeyEvent::new(
                KeyCode::Char(ch),
                Mods::NONE,
            )))));
        }
        assert_eq!(
            pasted,
            vec![InputEvent::Paste(b"\x1b[200~\x02qy\x1b[201~".to_vec())]
        );
        let mouse = console_event_bytes(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 7,
            row: 4,
            modifiers: Mods::SHIFT,
        }));
        assert!(matches!(
            &crate::mouse::SgrDecoder::default().push(&mouse)[0],
            crate::mouse::Frame::Mouse(event) if event.shift && event.code == 68 && event.x == 7 && event.y == 4
        ));
    }

    #[test]
    fn paste_is_atomic_across_reads() {
        let mut input = RawInput::default();
        assert!(input.push(b"\x1b[200~\x02y").is_empty());
        assert_eq!(
            input.push(b"\x1b[201~"),
            vec![InputEvent::Paste(b"\x1b[200~\x02y\x1b[201~".to_vec())]
        );
    }
    #[test]
    fn ordinary_bytes_do_not_wait_for_a_paste_prefix() {
        let mut input = RawInput::default();
        assert_eq!(input.push(b"abc"), vec![InputEvent::Bytes(b"abc".to_vec())]);
    }

    #[test]
    fn a_split_marker_prefix_flushes_as_literal_input_on_timeout() {
        let mut input = RawInput::default();
        assert!(input.push(b"\x1b").is_empty());
        assert_eq!(
            input.flush_marker_prefix(),
            vec![InputEvent::Bytes(vec![0x1b])]
        );
    }
}
