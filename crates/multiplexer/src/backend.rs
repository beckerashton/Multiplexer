use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use mux_core::{SessionId, SessionSpec};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

#[derive(Debug)]
pub enum TerminalEvent {
    Output { session: SessionId, bytes: Vec<u8> },
    Exited { session: SessionId, code: u32 },
    ReadError { session: SessionId, message: String },
}

#[derive(Debug)]
pub enum TerminalError {
    DuplicateSession(SessionId),
    UnknownSession(SessionId),
    Pty(String),
    Io(io::Error),
}

impl std::fmt::Display for TerminalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateSession(id) => write!(f, "session {} already has a terminal", id.0),
            Self::UnknownSession(id) => write!(f, "no terminal for session {}", id.0),
            Self::Pty(error) => f.write_str(error),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for TerminalError {}

impl From<io::Error> for TerminalError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

struct TerminalInstance {
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    killer: Arc<Mutex<Box<dyn ChildKiller + Send + Sync>>>,
    parser: vt100::Parser,
    #[cfg(test)]
    pid: Option<u32>,
    live: Arc<AtomicBool>,
    #[cfg(unix)]
    process_group: Option<libc::pid_t>,
}

/// Owns all OS-facing terminal state. It deliberately has no layout, tab, or
/// broadcast policy: callers refer only to stable SessionId values.
pub struct TerminalBackend {
    terminals: BTreeMap<SessionId, TerminalInstance>,
    event_tx: mpsc::SyncSender<TerminalEvent>,
    event_rx: mpsc::Receiver<TerminalEvent>,
    scrollback_rows: usize,
}

impl TerminalBackend {
    pub fn new(scrollback_rows: usize) -> Self {
        // Bounds queued terminal output. Reader tasks block when the UI is not
        // draining rather than allocating without limit for a noisy hidden PTY.
        let (event_tx, event_rx) = mpsc::sync_channel(256);
        Self {
            terminals: BTreeMap::new(),
            event_tx,
            event_rx,
            scrollback_rows,
        }
    }

    pub fn spawn(
        &mut self,
        id: SessionId,
        spec: &SessionSpec,
        size: (u16, u16),
    ) -> Result<(), TerminalError> {
        if self.terminals.contains_key(&id) {
            return Err(TerminalError::DuplicateSession(id));
        }

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(pty_size(size))
            .map_err(|error| TerminalError::Pty(error.to_string()))?;
        let mut command = CommandBuilder::new(&spec.program);
        command.args(&spec.args);
        if let Some(cwd) = &spec.cwd {
            command.cwd(cwd);
        }
        command.env("TERM", "xterm-256color");

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| TerminalError::Pty(error.to_string()))?;
        #[cfg(test)]
        let pid = child.process_id();
        let killer = Arc::new(Mutex::new(child.clone_killer()));
        let reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(error) => {
                let _ = killer.lock().map(|mut killer| killer.kill());
                return Err(TerminalError::Pty(error.to_string()));
            }
        };
        let writer = match pair.master.take_writer() {
            Ok(writer) => Arc::new(Mutex::new(writer)),
            Err(error) => {
                let _ = killer.lock().map(|mut killer| killer.kill());
                return Err(TerminalError::Pty(error.to_string()));
            }
        };
        #[cfg(unix)]
        let process_group = pair.master.process_group_leader();

        spawn_reader(id, reader, self.event_tx.clone());
        let live = Arc::new(AtomicBool::new(true));
        spawn_waiter(id, child, self.event_tx.clone(), live.clone());
        self.terminals.insert(
            id,
            TerminalInstance {
                master: pair.master,
                writer,
                killer,
                parser: vt100::Parser::new(size.1.max(1), size.0.max(1), self.scrollback_rows),
                #[cfg(test)]
                pid,
                live,
                #[cfg(unix)]
                process_group,
            },
        );
        Ok(())
    }

    pub fn resize(&mut self, id: SessionId, size: (u16, u16)) -> Result<(), TerminalError> {
        let terminal = self
            .terminals
            .get_mut(&id)
            .ok_or(TerminalError::UnknownSession(id))?;
        if terminal.parser.screen().size() == (size.1.max(1), size.0.max(1)) {
            return Ok(());
        }
        terminal
            .master
            .resize(pty_size(size))
            .map_err(|error| TerminalError::Pty(error.to_string()))?;
        terminal
            .parser
            .screen_mut()
            .set_size(size.1.max(1), size.0.max(1));
        Ok(())
    }

    pub fn write(&self, id: SessionId, bytes: &[u8]) -> Result<(), TerminalError> {
        let terminal = self
            .terminals
            .get(&id)
            .ok_or(TerminalError::UnknownSession(id))?;
        let mut writer = terminal
            .writer
            .lock()
            .map_err(|_| TerminalError::Pty("PTY writer lock poisoned".into()))?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    pub fn terminate(&mut self, id: SessionId) -> Result<(), TerminalError> {
        let terminal = self
            .terminals
            .get_mut(&id)
            .ok_or(TerminalError::UnknownSession(id))?;
        if !terminal.live.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        #[cfg(unix)]
        if let Some(group) = terminal.process_group {
            // A foreground job can have descendants beyond the spawned shell.
            // The PTY session group is the contained, exact kill target.
            unsafe {
                libc::kill(-group, libc::SIGTERM);
            }
        }
        terminal
            .killer
            .lock()
            .map_err(|_| TerminalError::Pty("PTY killer lock poisoned".into()))?
            .kill()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn process_id(&self, id: SessionId) -> Option<u32> {
        self.terminals.get(&id).and_then(|terminal| terminal.pid)
    }

    pub fn screen(&self, id: SessionId) -> Option<&vt100::Screen> {
        self.terminals
            .get(&id)
            .map(|terminal| terminal.parser.screen())
    }

    pub fn set_scrollback(&mut self, id: SessionId, rows: usize) -> bool {
        let Some(terminal) = self.terminals.get_mut(&id) else {
            return false;
        };
        let screen = terminal.parser.screen_mut();
        let previous = screen.scrollback();
        screen.set_scrollback(rows);
        previous != screen.scrollback()
    }

    /// Applies all queued output to each independent parser. Call this even
    /// when a session is hidden so a noisy background program cannot block.
    pub fn drain_events(&mut self, maximum: usize) -> Vec<TerminalEvent> {
        let mut observed = Vec::new();
        while observed.len() < maximum {
            let Ok(event) = self.event_rx.try_recv() else {
                break;
            };
            if let TerminalEvent::Output { session, bytes } = &event {
                if let Some(terminal) = self.terminals.get_mut(session) {
                    terminal.parser.process(bytes);
                }
            }
            if let TerminalEvent::Exited { session, .. } = &event {
                if let Some(terminal) = self.terminals.get_mut(session) {
                    terminal.live.store(false, Ordering::Release);
                }
            }
            observed.push(event);
        }
        observed
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        for terminal in self.terminals.values() {
            if !terminal.live.swap(false, Ordering::AcqRel) {
                continue;
            }
            if let Ok(mut killer) = terminal.killer.lock() {
                let _ = killer.kill();
            }
        }
    }
}

fn pty_size((cols, rows): (u16, u16)) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn spawn_reader(
    session: SessionId,
    mut reader: Box<dyn Read + Send>,
    sender: mpsc::SyncSender<TerminalEvent>,
) {
    thread::spawn(move || {
        let mut buffer = vec![0_u8; 16 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if sender
                        .send(TerminalEvent::Output {
                            session,
                            bytes: buffer[..count].to_vec(),
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    let _ = sender.send(TerminalEvent::ReadError {
                        session,
                        message: error.to_string(),
                    });
                    break;
                }
            }
        }
    });
}

fn spawn_waiter(
    session: SessionId,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    sender: mpsc::SyncSender<TerminalEvent>,
    live: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        if let Ok(status) = child.wait() {
            live.store(false, Ordering::Release);
            let _ = sender.send(TerminalEvent::Exited {
                session,
                code: status.exit_code(),
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, thread, time::Duration};

    use mux_core::{SessionId, SessionSpec};

    use super::{TerminalBackend, TerminalEvent};

    fn shell(command: &str) -> SessionSpec {
        SessionSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), command.into()],
            cwd: None,
        }
    }

    #[test]
    fn drains_output_and_preserves_the_pty_identity_until_explicit_termination() {
        let id = SessionId(7);
        let mut backend = TerminalBackend::new(100);
        backend
            .spawn(id, &shell("printf 'ready'; sleep 5"), (80, 24))
            .expect("spawn shell in PTY");
        let pid = backend.process_id(id).expect("child pid");

        let mut saw_ready = false;
        for _ in 0..50 {
            for event in backend.drain_events(256) {
                if matches!(event, TerminalEvent::Output { session, .. } if session == id) {
                    saw_ready = backend
                        .screen(id)
                        .expect("screen")
                        .contents()
                        .contains("ready");
                }
            }
            if saw_ready {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_ready, "PTY output must be parsed while not rendered");
        assert_eq!(backend.process_id(id), Some(pid));
        backend.terminate(id).expect("terminate");
    }

    #[test]
    fn writes_exact_bytes_to_the_child() {
        let id = SessionId(8);
        let mut backend = TerminalBackend::new(100);
        backend
            .spawn(
                id,
                &shell("IFS= read -r line; printf '<%s>' \"$line\"; sleep 1"),
                (80, 24),
            )
            .expect("spawn shell in PTY");
        backend.write(id, b"alpha beta\n").expect("write");

        let mut saw_echo = false;
        for _ in 0..50 {
            for _ in backend.drain_events(256) {
                saw_echo = backend
                    .screen(id)
                    .expect("screen")
                    .contents()
                    .contains("<alpha beta>");
            }
            if saw_echo {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_echo);
    }

    #[test]
    fn reaped_child_is_never_signalled_again_while_scrollback_is_retained() {
        let id = SessionId(9);
        let mut backend = TerminalBackend::new(10);
        backend.spawn(id, &shell("printf done"), (80, 24)).unwrap();
        for _ in 0..50 {
            if !backend.terminals[&id]
                .live
                .load(std::sync::atomic::Ordering::Acquire)
            {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !backend.terminals[&id]
                .live
                .load(std::sync::atomic::Ordering::Acquire)
        );
        assert!(
            backend.screen(id).is_some(),
            "parser/scrollback remains available"
        );
        backend.terminate(id).unwrap();
    }
}
