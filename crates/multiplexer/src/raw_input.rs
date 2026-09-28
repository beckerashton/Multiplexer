use mux_core::InputEvent;

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
