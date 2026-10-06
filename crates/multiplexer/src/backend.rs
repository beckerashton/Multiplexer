use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use mux_core::{SessionId, SessionSpec};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

pub fn default_shell() -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        select_windows_shell(
            std::env::var_os("MULTIPLEXER_SHELL").as_deref(),
            std::env::var_os("ProgramFiles").as_deref(),
            std::env::var_os("PATH").as_deref(),
        )
    }
    #[cfg(not(windows))]
    {
        Ok(std::env::var_os("SHELL")
            .filter(|shell| !shell.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/bin/sh")))
    }
}

#[cfg(any(windows, test))]
fn select_windows_shell(
    explicit: Option<&std::ffi::OsStr>,
    program_files: Option<&std::ffi::OsStr>,
    search_path: Option<&std::ffi::OsStr>,
) -> io::Result<PathBuf> {
    if let Some(shell) = explicit.filter(|shell| !shell.is_empty()) {
        return find_windows_executable(shell, search_path).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound,
                format!("MULTIPLEXER_SHELL executable {:?} was not found; set it to an executable path or name on PATH", shell))
        });
    }
    if let Some(program_files) = program_files.filter(|path| !path.is_empty()) {
        let installed = PathBuf::from(program_files).join("PowerShell/7/pwsh.exe");
        if installed.is_file() {
            return Ok(installed);
        }
    }
    find_windows_executable(std::ffi::OsStr::new("pwsh.exe"), search_path).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound,
            "PowerShell 7 (pwsh.exe) was not found. Install it with `winget install Microsoft.PowerShell`, add it to PATH, or set MULTIPLEXER_SHELL to an explicit shell executable")
    })
}

#[cfg(any(windows, test))]
fn find_windows_executable(
    program: &std::ffi::OsStr,
    search_path: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    let program = PathBuf::from(program);
    if program.is_absolute() || program.components().count() > 1 {
        return program.is_file().then_some(program);
    }
    for directory in std::env::split_paths(search_path?) {
        let candidate = directory.join(&program);
        if candidate.is_file() {
            return Some(candidate);
        }
        if candidate.extension().is_none() {
            let candidate = candidate.with_extension("exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

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
    #[cfg(any(test, windows))]
    pid: Option<u32>,
    live: Arc<AtomicBool>,
    #[cfg(unix)]
    process_group: Option<libc::pid_t>,
}

impl TerminalInstance {
    fn kill(&self) -> Result<(), TerminalError> {
        #[cfg(unix)]
        if let Some(group) = self.process_group {
            // A foreground job can have descendants beyond the spawned shell.
            unsafe {
                libc::kill(-group, libc::SIGTERM);
            }
        }
        #[cfg(windows)]
        if let Some(pid) = self.pid {
            use std::os::windows::process::CommandExt;
            use std::process::{Command, Stdio};

            // portable-pty's Windows killer targets only the shell. taskkill
            // also closes its descendants; the handle-based killer is fallback.
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            if Command::new("taskkill.exe")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .is_ok_and(|status| status.success())
            {
                return Ok(());
            }
        }
        self.killer
            .lock()
            .map_err(|_| TerminalError::Pty("PTY killer lock poisoned".into()))?
            .kill()?;
        Ok(())
    }
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

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| TerminalError::Pty(error.to_string()))?;
        let writer = Arc::new(Mutex::new(
            pair.master
                .take_writer()
                .map_err(|error| TerminalError::Pty(error.to_string()))?,
        ));
        // ConPTY inherits the cursor asynchronously. Its reader must be ready
        // to answer the cursor query even while CreateProcess is running.
        spawn_reader(
            id,
            reader,
            self.event_tx.clone(),
            #[cfg(windows)]
            writer.clone(),
        );
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| TerminalError::Pty(error.to_string()))?;
        #[cfg(any(test, windows))]
        let pid = child.process_id();
        let killer = Arc::new(Mutex::new(child.clone_killer()));
        #[cfg(unix)]
        let process_group = pair.master.process_group_leader();

        let live = Arc::new(AtomicBool::new(true));
        spawn_waiter(id, child, self.event_tx.clone(), live.clone());
        self.terminals.insert(
            id,
            TerminalInstance {
                master: pair.master,
                writer,
                killer,
                parser: vt100::Parser::new(size.1.max(1), size.0.max(1), self.scrollback_rows),
                #[cfg(any(test, windows))]
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
        terminal.kill()
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
        // Release readers blocked on the bounded UI queue before closing PTYs.
        // Windows readers keep draining so ClosePseudoConsole can finish.
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(sender);
        self.event_rx = receiver;
        for terminal in self.terminals.values() {
            if !terminal.live.swap(false, Ordering::AcqRel) {
                continue;
            }
            let _ = terminal.kill();
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
    #[cfg(windows)] writer: Arc<Mutex<Box<dyn Write + Send>>>,
) {
    thread::spawn(move || {
        let mut buffer = vec![0_u8; 16 * 1024];
        #[cfg(windows)]
        let mut cursor_query = CursorInheritance::default();
        #[cfg(windows)]
        let mut sender = Some(sender);
        #[cfg(not(windows))]
        let sender = Some(sender);
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    #[cfg(windows)]
                    if cursor_query.push(&buffer[..count]) {
                        if let Ok(mut writer) = writer.lock() {
                            // Each pane begins at its own top-left position.
                            let _ = writer.write_all(b"\x1b[1;1R");
                            let _ = writer.flush();
                        }
                    }
                    if sender.as_ref().is_some_and(|sender| {
                        sender
                            .send(TerminalEvent::Output {
                                session,
                                bytes: buffer[..count].to_vec(),
                            })
                            .is_err()
                    }) {
                        #[cfg(not(windows))]
                        break;
                        #[cfg(windows)]
                        {
                            sender = None;
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                #[cfg(windows)]
                Err(error) if error.kind() == io::ErrorKind::BrokenPipe => break,
                Err(error) => {
                    if let Some(sender) = &sender {
                        let _ = sender.send(TerminalEvent::ReadError {
                            session,
                            message: error.to_string(),
                        });
                    }
                    break;
                }
            }
        }
    });
}

/// Only ConPTY's initial inheritance query is answered; later application
/// queries must not receive a fabricated cursor position.
#[cfg(any(windows, test))]
#[derive(Default)]
struct CursorInheritance {
    matched: usize,
    answered: bool,
}

#[cfg(any(windows, test))]
impl CursorInheritance {
    fn push(&mut self, bytes: &[u8]) -> bool {
        if self.answered {
            return false;
        }
        const QUERY: &[u8] = b"\x1b[6n";
        for &byte in bytes {
            self.matched = if byte == QUERY[self.matched] {
                self.matched + 1
            } else {
                usize::from(byte == QUERY[0])
            };
            if self.matched == QUERY.len() {
                self.answered = true;
                return true;
            }
        }
        false
    }
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
    #[cfg(unix)]
    use std::path::PathBuf;
    use std::{thread, time::Duration};

    use mux_core::{SessionId, SessionSpec};

    use super::{CursorInheritance, TerminalBackend, TerminalEvent, select_windows_shell};

    fn shell(command: &str) -> SessionSpec {
        #[cfg(unix)]
        let spec = SessionSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), command.into()],
            cwd: None,
        };
        #[cfg(windows)]
        let spec = SessionSpec {
            program: super::default_shell()
                .expect("PowerShell 7 must be installed for Windows tests"),
            args: vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                command.into(),
            ],
            cwd: None,
        };
        spec
    }

    #[test]
    fn windows_shell_prefers_modern_install_and_requires_explicit_alternatives() {
        use std::{
            ffi::OsStr,
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };
        let directory = std::env::temp_dir().join(format!(
            "multiplexer-shell-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let installed = directory.join("ProgramFiles/PowerShell/7/pwsh.exe");
        let path_dir = directory.join("bin");
        fs::create_dir_all(installed.parent().unwrap()).unwrap();
        fs::create_dir_all(&path_dir).unwrap();
        let from_path = path_dir.join("pwsh.exe");
        let alternative = path_dir.join("cmd.exe");
        for path in [&installed, &from_path, &alternative] {
            fs::write(path, "").unwrap();
        }
        let program_files = directory.join("ProgramFiles");
        let search_path = std::env::join_paths([&path_dir]).unwrap();
        let select = |explicit| {
            select_windows_shell(
                explicit,
                Some(program_files.as_os_str()),
                Some(&search_path),
            )
        };
        assert_eq!(select(None).unwrap(), installed);
        assert_eq!(select(Some(OsStr::new("cmd"))).unwrap(), alternative);
        assert_eq!(select(Some(alternative.as_os_str())).unwrap(), alternative);
        assert_eq!(
            select(Some(OsStr::new("missing.exe"))).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        fs::remove_file(&installed).unwrap();
        assert_eq!(select(None).unwrap(), from_path);
        fs::remove_file(&from_path).unwrap();
        let error = select(None).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(
            error
                .to_string()
                .contains("winget install Microsoft.PowerShell")
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn conpty_cursor_inheritance_handles_fragments_and_answers_only_once() {
        let mut query = CursorInheritance::default();
        assert!(!query.push(b"output\x1b["));
        assert!(!query.push(b"3m\x1b\x1b[6"));
        assert!(query.push(b"nmore output"));
        assert!(!query.push(b"\x1b[6n"));
    }

    #[test]
    fn drains_output_and_preserves_the_pty_identity_until_explicit_termination() {
        let id = SessionId(7);
        let mut backend = TerminalBackend::new(100);
        #[cfg(unix)]
        let command = "printf 'ready'; sleep 5";
        #[cfg(windows)]
        let command = "[Console]::Write('ready'); Start-Sleep -Seconds 5";
        backend
            .spawn(id, &shell(command), (80, 24))
            .expect("spawn shell in PTY");
        let pid = backend.process_id(id).expect("child pid");

        let mut saw_ready = false;
        for _ in 0..500 {
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
        backend.resize(id, (100, 30)).expect("resize");
        assert_eq!(backend.screen(id).unwrap().size(), (30, 100));
        backend.terminate(id).expect("terminate");
    }

    #[test]
    fn writes_exact_bytes_to_the_child() {
        let id = SessionId(8);
        let mut backend = TerminalBackend::new(100);
        #[cfg(unix)]
        let command = "IFS= read -r line; printf '<%s>' \"$line\"; sleep 1";
        #[cfg(windows)]
        let command = "[Console]::Write('<' + [Console]::ReadLine() + '>'); Start-Sleep -Seconds 1";
        backend
            .spawn(id, &shell(command), (80, 24))
            .expect("spawn shell in PTY");
        #[cfg(unix)]
        let input = b"alpha beta\n";
        #[cfg(windows)]
        let input = b"alpha beta\r";
        backend.write(id, input).expect("write");

        let mut saw_echo = false;
        for _ in 0..500 {
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
        #[cfg(unix)]
        let command = "printf done";
        #[cfg(windows)]
        let command = "[Console]::Write('done')";
        backend.spawn(id, &shell(command), (80, 24)).unwrap();
        for _ in 0..500 {
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
