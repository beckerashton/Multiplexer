use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

pub struct PtyHarness {
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
        command.env("SHELL", "/bin/sh");
        for &(name, value) in env {
            command.env(name, value);
        }
        let child = pair
            .slave
            .spawn_command(command)
            .expect("start multiplexer");
        let writer = pair.master.take_writer().expect("take PTY writer");
        let mut reader = pair.master.try_clone_reader().expect("clone PTY reader");
        drop(pair);

        let output = Arc::new(Mutex::new(Vec::new()));
        let output_capture = Arc::clone(&output);
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let mut chunk = [0_u8; 4096];
            while let Ok(count) = reader.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                output_capture
                    .lock()
                    .expect("capture lock")
                    .extend_from_slice(&chunk[..count]);
            }
            let _ = done_tx.send(());
        });
        Self {
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
        bytes.push(b'\n');
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
