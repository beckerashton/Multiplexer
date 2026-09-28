use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

const READY: Duration = Duration::from_secs(6);

struct Harness {
    child: Option<Box<dyn Child + Send + Sync>>,
    writer: Option<Box<dyn Write + Send>>,
    reader_done: Option<mpsc::Receiver<()>>,
    output: Arc<Mutex<Vec<u8>>>,
    directory: PathBuf,
}

impl Drop for Harness {
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

impl Harness {
    fn start() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "multiplexer-nvim-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create isolated Neovim directory");
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open controlling PTY");
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_multiplexer"));
        command.cwd(&directory);
        command.env("SHELL", "/bin/sh");
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

    fn send(&mut self, bytes: &[u8]) {
        let writer = self.writer.as_mut().expect("PTY writer is alive");
        writer.write_all(bytes).expect("write PTY input");
        writer.flush().expect("flush PTY input");
    }

    fn command(&mut self, command: &str) {
        let mut bytes = command.as_bytes().to_vec();
        bytes.push(b'\n');
        self.send(&bytes);
    }

    fn wait_for(&self, name: &str) {
        let path = self.directory.join(name);
        let start = Instant::now();
        while start.elapsed() < READY {
            if path.is_file() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {}", path.display());
    }

    fn wait_for_bytes(&self, name: &str, expected: &[u8]) {
        let path = self.directory.join(name);
        let start = Instant::now();
        while start.elapsed() < READY {
            if fs::read(&path).ok().as_deref() == Some(expected) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let output = self.output.lock().expect("capture lock");
        let tail_start = output.len().saturating_sub(2048);
        panic!(
            "timed out waiting for {:?} in {}; PTY tail: {:?}",
            expected,
            path.display(),
            String::from_utf8_lossy(&output[tail_start..])
        );
    }
}

#[test]
fn neovim_insert_escape_write_and_shell_recovery() {
    assert!(
        PathBuf::from("/usr/bin/nvim").is_file(),
        "/usr/bin/nvim is required for this compatibility gate"
    );
    let mut harness = Harness::start();

    // The VimEnter command is the readiness signal; it is emitted by Neovim,
    // not by a test-side sleep or by the multiplexer renderer.
    harness.command(
        "nvim -u NONE -i NONE --noplugin -c \"call writefile(['ready'], 'nvim-ready')\" nvim-file; printf nvim-exited > nvim-exited",
    );
    harness.wait_for("nvim-ready");

    // Bare h/j/k/l are application input once Neovim is focused. Insert them,
    // leave insert mode with Escape, and save/quit through the normal command.
    harness.send(b"ihello-hjkl");
    harness.send(&[0x1b]);
    harness.send(b":wq\n");
    harness.wait_for_bytes("nvim-file", b"hello-hjkl\n");
    // The shell-owned marker proves that :wq completed and Neovim released
    // the foreground PTY job before the next command is sent.
    harness.wait_for_bytes("nvim-exited", b"nvim-exited");

    // Neovim's exit must return control to the original shell in the same
    // PTY, which then proves it remains responsive before multiplexer quit.
    harness.command("printf shell-ready > shell-ready");
    harness.wait_for_bytes("shell-ready", b"shell-ready");

    harness.send(&[0x02, b'q', b'y']);
    let start = Instant::now();
    while start.elapsed() < READY {
        if harness
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
