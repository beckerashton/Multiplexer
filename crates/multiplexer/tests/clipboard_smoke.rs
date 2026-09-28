use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

const READY: Duration = Duration::from_secs(6);

struct HelperDirectory(PathBuf);

impl HelperDirectory {
    fn new(prefix: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        )))
    }
}

impl Drop for HelperDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

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
    fn start(env: &[(&str, &str)]) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "multiplexer-clipboard-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create isolated clipboard directory");
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

    fn wait_for_file(&self, name: &str, expected: Option<&[u8]>) {
        self.wait_for_path(&self.directory.join(name), expected);
    }

    fn wait_for_path(&self, path: &std::path::Path, expected: Option<&[u8]>) {
        let start = Instant::now();
        while start.elapsed() < READY {
            if let Ok(contents) = fs::read(path) {
                if expected.is_none_or(|wanted| contents == wanted) {
                    return;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        let output = self.output.lock().expect("capture lock");
        let tail_start = output.len().saturating_sub(2048);
        let tail = String::from_utf8_lossy(&output[tail_start..]).into_owned();
        drop(output);
        panic!(
            "timed out waiting for {} in {}; PTY tail: {:?}",
            path.display(),
            self.directory.display(),
            tail
        );
    }

    fn wait_for_pane_text(&self, text: &str) {
        let start = Instant::now();
        while start.elapsed() < READY {
            let mut parser = vt100::Parser::new(40, 120, 0);
            parser.process(&self.output.lock().expect("capture lock"));
            let actual: String = (0..text.chars().count())
                .filter_map(|col| parser.screen().cell(2, col as u16 + 1))
                .map(|cell| cell.contents())
                .collect();
            if actual == text {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for pane text {text:?}");
    }

    fn confirm_quit(&mut self) {
        self.send(&[0x02, b'q', b'y']);
        let start = Instant::now();
        while start.elapsed() < READY {
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

fn helper_environment(directory: &std::path::Path) -> String {
    let helper = directory.join("wl-copy");
    let capture = directory.join("clipboard-capture");
    fs::write(
        &helper,
        format!("#!/bin/sh\n/bin/cat > '{}'\n", capture.display()),
    )
    .expect("write fake wl-copy");
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755))
        .expect("make fake wl-copy executable");
    format!("{}:/usr/bin:/bin", directory.display())
}

#[test]
fn shift_drag_and_leader_copy_reaches_fake_wayland_helper() {
    let directory = HelperDirectory::new("multiplexer-clipboard-helper");
    fs::create_dir_all(&directory.0).expect("create helper directory");
    let path = helper_environment(&directory.0);
    let mut harness = Harness::start(&[("PATH", &path), ("WAYLAND_DISPLAY", "test-wayland")]);
    let capture = directory.0.join("clipboard-capture");

    // Clear the child screen and put four known cells at child row 0, col 0.
    harness.command("printf '\\033[2J\\033[HCLIP'; : > clip-ready");
    harness.wait_for_file("clip-ready", None);
    harness.wait_for_pane_text("CLIP");

    // The tab bar and top border put child row 0 at host row 3.
    // SGR coordinates are one-based; the decoder translates them to zero-based
    // selection points. Select exactly C..P with Shift-left press/drag/release.
    harness.send(b"\x1b[<4;2;3M\x1b[<36;5;3M\x1b[<4;5;3m");
    harness.send(&[0x02, b'y']);
    harness.wait_for_path(&capture, Some(b"CLIP"));
    harness.confirm_quit();
}

#[test]
fn ordinary_sgr_mouse_bytes_reach_child_when_mouse_mode_is_enabled() {
    let helper_dir = HelperDirectory::new("multiplexer-mouse-helper");
    fs::create_dir_all(&helper_dir.0).expect("create helper directory");
    let path = helper_environment(&helper_dir.0);
    let mut harness = Harness::start(&[("PATH", &path), ("WAYLAND_DISPLAY", "test-wayland")]);

    // Have the child enable SGR mouse mode, enter a raw reader, and signal
    // readiness before the ordinary (non-Shift) report is sent.
    harness.command(
        "stty raw -echo; printf '\\033[?1002h\\033[?1006h'; : > mouse-ready; dd bs=1 count=9 of=mouse-bytes 2>/dev/null; stty sane; : > mouse-done",
    );
    harness.wait_for_file("mouse-ready", None);
    // The marker is created by the shell before the PTY output has necessarily
    // reached vt100; allow that bounded backend update before the report.
    thread::sleep(Duration::from_millis(100));
    harness.send(b"\x1b[<0;3;3M");
    harness.wait_for_file("mouse-done", None);
    harness.wait_for_file("mouse-bytes", Some(b"\x1b[<0;2;1M"));
    harness.confirm_quit();
}
