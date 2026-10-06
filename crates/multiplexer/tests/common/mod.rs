use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

#[cfg(windows)]
struct SharedWriter(Arc<Mutex<Box<dyn Write + Send>>>);

#[cfg(windows)]
impl Write for SharedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

pub struct PtyHarness {
    #[cfg(windows)]
    master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    pub child: Option<Box<dyn Child + Send + Sync>>,
    pub writer: Option<Box<dyn Write + Send>>,
    pub reader_done: Option<mpsc::Receiver<()>>,
    pub output: Arc<Mutex<Vec<u8>>>,
    pub directory: PathBuf,
}

impl Drop for PtyHarness {
    fn drop(&mut self) {
        self.writer.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        #[cfg(windows)]
        self.master.take();
        if let Some(done) = self.reader_done.take() {
            let _ = done.recv_timeout(Duration::from_secs(1));
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

impl PtyHarness {
    pub fn spawn(prefix: &str, rows: u16, cols: u16, env: &[(&str, &str)]) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create isolated test directory");
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open controlling PTY");
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_multiplexer"));
        command.cwd(&directory);
        #[cfg(unix)]
        command.env("SHELL", "/bin/sh");
        #[cfg(windows)]
        {
            // COMSPEC remains cmd.exe on Windows; it must not select pane shells.
            command.env("COMSPEC", "cmd.exe");
            command.env_remove("MULTIPLEXER_SHELL");
        }
        command.env("XDG_STATE_HOME", directory.join("state"));
        command.env("LOCALAPPDATA", directory.join("state"));
        command.arg("--fresh");
        for &(name, value) in env {
            command.env(name, value);
        }
        let writer = pair.master.take_writer().expect("take PTY writer");
        #[cfg(windows)]
        let (writer, cursor_reply) = {
            let shared = Arc::new(Mutex::new(writer));
            (
                Box::new(SharedWriter(Arc::clone(&shared))) as Box<dyn Write + Send>,
                shared,
            )
        };
        let mut reader = pair.master.try_clone_reader().expect("clone PTY reader");

        let output = Arc::new(Mutex::new(Vec::new()));
        let output_capture = Arc::clone(&output);
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let mut chunk = [0_u8; 4096];
            #[cfg(windows)]
            let mut query = Vec::new();
            #[cfg(windows)]
            let mut cursor_reply_pending = true;
            while let Ok(count) = reader.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                #[cfg(windows)]
                {
                    // ConPTY inherits the host cursor and waits for its position.
                    query.extend_from_slice(&chunk[..count]);
                    if cursor_reply_pending && query.windows(4).any(|bytes| bytes == b"\x1b[6n") {
                        cursor_reply_pending = false;
                        let mut writer = cursor_reply.lock().unwrap();
                        let _ = writer.write_all(b"\x1b[1;1R");
                        let _ = writer.flush();
                    }
                    if query.len() > 3 {
                        query.drain(..query.len() - 3);
                    }
                }
                output_capture
                    .lock()
                    .expect("capture lock")
                    .extend_from_slice(&chunk[..count]);
            }
            let _ = done_tx.send(());
        });
        let child = pair
            .slave
            .spawn_command(command)
            .expect("start multiplexer");
        #[cfg(windows)]
        let master = {
            drop(pair.slave);
            Some(pair.master)
        };
        #[cfg(not(windows))]
        drop(pair);
        Self {
            #[cfg(windows)]
            master,
            child: Some(child),
            writer: Some(writer),
            reader_done: Some(done_rx),
            output,
            directory,
        }
    }

    pub fn send(&mut self, bytes: &[u8]) {
        let writer = self.writer.as_mut().expect("PTY writer is alive");
        writer.write_all(bytes).expect("write PTY input");
        writer.flush().expect("flush PTY input");
    }

    pub fn command(&mut self, command: &str) {
        let mut bytes = command.as_bytes().to_vec();
        bytes.push(if cfg!(windows) { b'\r' } else { b'\n' });
        self.send(&bytes);
    }

    pub fn confirm_quit(&mut self, timeout: Duration) {
        self.send(&[0x02, b'q', b'y']);
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if self
                .child
                .as_mut()
                .expect("multiplexer child")
                .try_wait()
                .expect("poll multiplexer")
                .is_some()
            {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("multiplexer did not exit after confirmed quit");
    }
}
