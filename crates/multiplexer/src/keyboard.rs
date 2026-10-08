/// Frame extended keyboard reports before confirmation and command handling.
/// Ordinary and unknown sequences remain untouched. The host's disambiguated
/// control keys are converted back to their legacy bytes for hosted programs.
#[derive(Default)]
pub struct KeyboardDecoder {
    pending: Vec<u8>,
}

impl KeyboardDecoder {
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        for &byte in bytes {
            if self.pending.is_empty() {
                if byte == 0x1b {
                    self.pending.push(byte);
                } else {
                    frames.push(vec![byte]);
                }
                continue;
            }
            self.pending.push(byte);
            if self.pending.len() == 2 && matches!(byte, b'[' | b'O') {
                continue;
            }
            if self.pending.len() > 2 && !(0x40..=0x7e).contains(&byte) && self.pending.len() < 64 {
                continue;
            }
            let report = std::mem::take(&mut self.pending);
            frames.push(normalize(report));
        }
        frames
    }

    pub fn flush(&mut self) -> Vec<Vec<u8>> {
        if self.pending.is_empty() {
            Vec::new()
        } else {
            vec![std::mem::take(&mut self.pending)]
        }
    }
}

struct KeyReport {
    key: char,
    shifted: Option<char>,
    modifiers: u32,
    event: u32,
}

fn parse_report(bytes: &[u8]) -> Option<KeyReport> {
    let body = std::str::from_utf8(bytes.strip_prefix(b"\x1b[")?).ok()?;
    let (key, shifted, modifiers, event) = if let Some(body) = body.strip_suffix('u') {
        let mut fields = body.split(';');
        let mut keys = fields.next()?.split(':');
        let key = keys.next()?.parse::<u32>().ok()?;
        let shifted = keys
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .and_then(char::from_u32);
        let mut mods = fields.next().unwrap_or("1").split(':');
        let modifiers = mods.next()?.parse::<u32>().ok()?;
        let event = mods.next().unwrap_or("1").parse::<u32>().ok()?;
        (key, shifted, modifiers, event)
    } else {
        let mut fields = body.strip_suffix('~')?.split(';');
        if fields.next()? != "27" {
            return None;
        }
        let modifiers = fields.next()?.parse::<u32>().ok()?;
        let key = fields.next()?.parse::<u32>().ok()?;
        if fields.next().is_some() {
            return None;
        }
        (key, None, modifiers, 1)
    };
    Some(KeyReport {
        key: char::from_u32(key)?,
        shifted,
        modifiers: modifiers.checked_sub(1)?,
        event,
    })
}

// Cursor reports can include lock-state bits and press/repeat/release fields
// even when the application only understands legacy cursor sequences.
fn normalize_cursor(report: &[u8]) -> Option<Vec<u8>> {
    let body = report.strip_prefix(b"\x1b[")?;
    let (&final_byte, params) = body.split_last()?;
    if !b"ABCDHF".contains(&final_byte) {
        return None;
    }
    let params = std::str::from_utf8(params).ok()?;
    let mut fields = params.split(';');
    if !matches!(fields.next()?, "" | "1") {
        return None;
    }
    let mut mods = fields.next().unwrap_or("1").split(':');
    let modifier = mods.next()?.parse::<u32>().ok()?.checked_sub(1)? & !(64 | 128);
    let event = mods.next().unwrap_or("1");
    if fields.next().is_some() || modifier > 7 {
        return None;
    }
    if event == "3" {
        return Some(Vec::new());
    }
    if event != "1" && event != "2" {
        return None;
    }
    Some(if modifier == 0 {
        vec![27, b'[', final_byte]
    } else {
        format!("\x1b[1;{}{}", modifier + 1, final_byte as char).into_bytes()
    })
}

/// Encode an unmodified cursor key using the recipient application's mode.
/// Called on a complete routed key, never on pasted content.
pub fn application_cursor_key(bytes: &[u8], application: bool) -> Vec<u8> {
    let mut bytes = bytes.to_vec();
    if bytes.len() == 3
        && bytes[0] == 27
        && matches!(bytes[1], b'[' | b'O')
        && b"ABCDHF".contains(&bytes[2])
    {
        bytes[1] = if application { b'O' } else { b'[' };
    }
    bytes
}

fn normalize(report: Vec<u8>) -> Vec<u8> {
    if let Some(cursor) = normalize_cursor(&report) {
        return cursor;
    }
    let Some(key_report) = parse_report(&report) else {
        return report;
    };
    let KeyReport {
        key,
        shifted,
        modifiers,
        event,
    } = key_report;
    // Release and modifier-only reports have no legacy input equivalent.
    if event == 3 || (57441..=57454).contains(&(key as u32)) {
        return Vec::new();
    }
    if event != 1 && event != 2 {
        return report;
    }
    if (57344..=63743).contains(&(key as u32)) {
        return report;
    }
    // Lock state is independent of the shortcut modifiers. The key code (or
    // shifted alternate) already supplies the character chosen by the host.
    let mods = modifiers & !(64 | 128);
    if mods & !7 != 0 {
        return report;
    }
    let shift = mods & 1 != 0;
    let alt = mods & 2 != 0;
    let ctrl = mods & 4 != 0;
    // Preserve the otherwise ambiguous Ctrl-Alt digits for workspace routing.
    if mods == 6 && key.is_ascii_digit() {
        return format!("\x1b[{};7u", key as u32).into_bytes();
    }
    let mut key = if shift {
        shifted.unwrap_or_else(|| {
            if key.is_ascii_lowercase() {
                key.to_ascii_uppercase()
            } else {
                key
            }
        })
    } else {
        key
    };
    let mut bytes = if key == '\t' && shift {
        b"\x1b[Z".to_vec()
    } else {
        if ctrl {
            key = match key {
                'a'..='z' => char::from(key as u8 & 0x1f),
                '@'..='_' => char::from(key as u8 & 0x1f),
                ' ' | '2' => '\0',
                '3' => '\x1b',
                '4' => '\x1c',
                '5' => '\x1d',
                '6' => '\x1e',
                '7' | '/' => '\x1f',
                '8' | '?' | '\x7f' => '\x7f',
                _ => key,
            };
        }
        key.to_string().into_bytes()
    };
    if alt {
        bytes.insert(0, 0x1b);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arrows_are_framed_and_lock_flags_removed() {
        let mut decoder = KeyboardDecoder::default();
        assert!(decoder.push(b"\x1bO").is_empty());
        assert_eq!(decoder.push(b"A"), vec![b"\x1bOA".to_vec()]);
        assert_eq!(decoder.push(b"\x1b[1;129A"), vec![b"\x1b[A".to_vec()]);
        assert_eq!(decoder.push(b"\x1b[1;133D"), vec![b"\x1b[1;5D".to_vec()]);
        assert!(decoder.push(b"\x1b[1;1:3A").concat().is_empty());
    }

    #[test]
    fn locks_preserve_shortcuts_and_release_does_not_type() {
        let mut decoder = KeyboardDecoder::default();
        assert_eq!(decoder.push(b"\x1b[49;199u").concat(), b"\x1b[49;7u");
        assert_eq!(decoder.push(b"\x1b[104;131u").concat(), b"\x1bh");
        assert!(decoder.push(b"\x1b[99;5:3u").concat().is_empty());
        assert!(decoder.push(b"\x1b[57442;2u").concat().is_empty());
        assert_eq!(decoder.push(b"\x1b[99;5:2u").concat(), vec![3]);
        assert_eq!(decoder.push(b"\x1b[49:33;2u").concat(), b"!");
        assert_eq!(decoder.push(b"\x1b[45:95;2u").concat(), b"_");
        assert_eq!(decoder.push(b"\x1b[92:124;2u").concat(), b"|");
    }
    #[test]
    fn lock_bits_and_combined_modifiers_become_shell_input() {
        for (report, expected) in [
            ("\x1b[103;7u", "\x1b\x07"), // Ctrl-Alt-G jump mark
            ("\x1b[103;3u", "\x1bg"), // Alt-G jump
            ("\x1b[99;133u", "\x03"), // Ctrl-C with Num Lock
            ("\x1b[99;6u", "\x03"),   // Ctrl-Shift-C
            ("\x1b[97;4u", "\x1bA"),  // Alt-Shift-A
            ("\x1b[233;3u", "\x1bé"),
            ("\x1b[97:65;2u", "A"),
            ("\x1b[9;2u", "\x1b[Z"),
            ("\x1b[27;6;67~", "\x03"),
        ] {
            let mut decoder = KeyboardDecoder::default();
            assert_eq!(
                decoder.push(report.as_bytes()).concat(),
                expected.as_bytes(),
                "{report:?}"
            );
        }
    }
    #[test]
    fn fragmented_extended_keys_normalize_without_touching_arrows() {
        let mut decoder = KeyboardDecoder::default();
        assert!(decoder.push(b"\x1b[98;").is_empty());
        assert_eq!(decoder.push(b"5u"), vec![vec![2]]);
        assert_eq!(decoder.push(b"\x1b[27u"), vec![vec![27]]);
        assert_eq!(decoder.push(b"\x1b[104;3u"), vec![b"\x1bh".to_vec()]);
        assert_eq!(decoder.push(b"\x1b[104;7u"), vec![vec![27, 8]]);
        assert_eq!(decoder.push(b"\x1b[49;7u"), vec![b"\x1b[49;7u".to_vec()]);
        assert_eq!(decoder.push(b"\x1b[A"), vec![b"\x1b[A".to_vec()]);
        assert_eq!(decoder.push(b"\x1b[27;5;99~"), vec![vec![3]]);
    }
}
