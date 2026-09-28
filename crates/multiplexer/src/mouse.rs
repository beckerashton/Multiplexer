/// Decoded SGR mouse report with the original terminal bytes retained for
/// exact forwarding when the multiplexer does not consume the event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MouseEvent {
    pub code: u16,
    pub button: u16,
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub motion: bool,
    pub x: u16,
    pub y: u16,
    pub press: bool,
    pub raw: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Frame {
    Bytes(Vec<u8>),
    Mouse(MouseEvent),
}

#[derive(Default)]
pub struct SgrDecoder {
    pending: Vec<u8>,
}

impl SgrDecoder {
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<Frame> {
        self.pending.extend_from_slice(bytes);
        let mut frames = Vec::new();
        loop {
            let Some(start) = self.pending.windows(3).position(|part| part == b"\x1b[<") else {
                if self.pending == [0x1b] {
                    frames.push(Frame::Bytes(std::mem::take(&mut self.pending)));
                    break;
                }
                let keep = prefix_len(&self.pending, b"\x1b[<");
                let count = self.pending.len().saturating_sub(keep);
                if count > 0 {
                    frames.push(Frame::Bytes(self.pending.drain(..count).collect()));
                }
                break;
            };
            if start > 0 {
                frames.push(Frame::Bytes(self.pending.drain(..start).collect()));
                continue;
            }
            let Some(end) = self
                .pending
                .iter()
                .position(|byte| *byte == b'M' || *byte == b'm')
            else {
                break;
            };
            let raw: Vec<u8> = self.pending.drain(..=end).collect();
            match parse(&raw) {
                Some(event) => frames.push(Frame::Mouse(event)),
                None => frames.push(Frame::Bytes(raw)),
            }
        }
        frames
    }

    pub fn flush(&mut self) -> Vec<Frame> {
        if self.pending.is_empty() {
            Vec::new()
        } else {
            vec![Frame::Bytes(std::mem::take(&mut self.pending))]
        }
    }
}

fn parse(raw: &[u8]) -> Option<MouseEvent> {
    let press = matches!(raw.last(), Some(b'M'));
    let body = std::str::from_utf8(raw.get(3..raw.len().checked_sub(1)?)?).ok()?;
    let mut values = body.split(';').map(str::parse::<u16>);
    let code = values.next()?.ok()?;
    let x = values.next()?.ok()?.checked_sub(1)?;
    let y = values.next()?.ok()?.checked_sub(1)?;
    if values.next().is_some() {
        return None;
    }
    Some(MouseEvent {
        code,
        button: code & 3,
        shift: code & 4 != 0,
        alt: code & 8 != 0,
        control: code & 16 != 0,
        motion: code & 32 != 0,
        x,
        y,
        press,
        raw: raw.to_vec(),
    })
}

fn prefix_len(bytes: &[u8], marker: &[u8]) -> usize {
    (1..marker.len())
        .rev()
        .find(|len| bytes.ends_with(&marker[..*len]))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragments_and_shift_drag_decode() {
        let mut decoder = SgrDecoder::default();
        assert!(decoder.push(b"\x1b[<4;2").is_empty());
        let frames = decoder.push(b";3M\x1b[<36;4;3M\x1b[<4;4;3m");
        assert!(
            matches!(&frames[0], Frame::Mouse(event) if event.shift && event.button == 0 && event.press && (event.x,event.y)==(1,2))
        );
        assert!(matches!(&frames[1], Frame::Mouse(event) if event.shift && event.motion));
        assert!(matches!(&frames[2], Frame::Mouse(event) if !event.press));
    }
    #[test]
    fn malformed_report_is_preserved() {
        let mut decoder = SgrDecoder::default();
        assert_eq!(
            decoder.push(b"a\x1b[<oopsM"),
            vec![
                Frame::Bytes(b"a".to_vec()),
                Frame::Bytes(b"\x1b[<oopsM".to_vec())
            ]
        );
    }

    #[test]
    fn lone_escape_is_never_buffered() {
        assert_eq!(
            SgrDecoder::default().push(b"\x1b"),
            vec![Frame::Bytes(vec![0x1b])]
        );
    }
}
