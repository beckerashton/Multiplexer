use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

const READY: Duration = Duration::from_secs(5);

struct Harness {
    child: Option<Box<dyn Child + Send + Sync>>,
    writer: Option<Box<dyn Write + Send>>,
    reader_done: Option<mpsc::Receiver<()>>,
    directory: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Close the input side first, then terminate and reap the multiplexer.
        // This path also runs when an assertion panics.
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
            "multiplexer-live-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create isolated live-session directory");
        let pty = native_pty_system();
        let pair = pty
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
        let mut writer = pair.master.take_writer().expect("take PTY writer");
        let mut reader = pair.master.try_clone_reader().expect("clone PTY reader");
        drop(pair);

        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let mut output = Vec::new();
            let _ = reader.read_to_end(&mut output);
            let _ = done_tx.send(());
        });
        writer.flush().expect("flush initial PTY writer");
        Self {
            child: Some(child),
            writer: Some(writer),
            reader_done: Some(done_rx),
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

    fn wait_for_file(&self, name: &str) -> String {
        let path = self.directory.join(name);
        let start = Instant::now();
        while start.elapsed() < READY {
            if let Ok(contents) = fs::read_to_string(&path) {
                if !contents.is_empty() {
                    return contents;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for nonempty marker {}", path.display());
    }

    fn assert_absent(&self, name: &str) {
        let path = self.directory.join(name);
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(250) {
            assert!(
                !path.exists(),
                "unexpected marker appeared: {}",
                path.display()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
}

#[test]
fn live_processes_survive_stack_carry_and_broadcast_scope_is_exact() {
    let mut harness = Harness::start();

    // Initial tab, slot 1: shell A.
    harness.command("printf '%s' \"$$\" > pid-a; : > ready-a");
    let pid_a = harness.wait_for_file("pid-a");
    assert!(harness.path("ready-a").exists());

    // Split slot 2 and create shell B, then add shell C to the same stack.
    harness.send(&[0x02, b'|']);
    harness.command("printf '%s' \"$$\" > pid-b; : > ready-b");
    let pid_b = harness.wait_for_file("pid-b");
    harness.send(&[0x02, b'a']);
    harness.command("printf '%s' \"$$\" > pid-c; : > ready-c");
    let pid_c = harness.wait_for_file("pid-c");
    assert_ne!(pid_b, pid_c, "stack members must be distinct processes");

    // Create tab 2 with shell D and prove its startup before switching back.
    harness.send(&[0x02, b't']);
    harness.command("printf '%s' \"$$\" > pid-d; : > ready-d");
    let pid_d = harness.wait_for_file("pid-d");
    assert!(!pid_d.is_empty());
    harness.send(&[0x1b, b'1']);

    // Tab 1 has A and the visible C. B is hidden in the stack and D is in tab 2.
    harness.send(&[0x02, b'b', b'b']);
    harness.command("printf broadcast > broadcast-$$");
    harness.wait_for_file(&format!("broadcast-{pid_a}"));
    harness.wait_for_file(&format!("broadcast-{pid_c}"));
    harness.assert_absent(&format!("broadcast-{pid_b}"));
    harness.assert_absent(&format!("broadcast-{pid_d}"));

    // Carry the focused slot and its complete [B,C] stack to tab 2. Carrying
    // resets broadcast and focuses the carried slot, whose active member is C.
    harness.send(b"\x02\x1b[50;7u");
    harness.command("printf '%s' \"$$\" > pid-c-after");
    assert_eq!(harness.wait_for_file("pid-c-after"), pid_c);

    // Move up to hidden B and down to C, proving both members are the same PTYs
    // after the move rather than newly spawned replacements.
    harness.send(&[0x1b, b'k']);
    harness.command("printf '%s' \"$$\" > pid-b-after");
    assert_eq!(harness.wait_for_file("pid-b-after"), pid_b);
    harness.send(&[0x1b, b'j']);
    harness.command("printf '%s' \"$$\" > pid-c-after-2");
    assert_eq!(harness.wait_for_file("pid-c-after-2"), pid_c);

    // Direct carry moves C alone, retaining B and both PTY identities.
    harness.send(b"\x1b[51;7u");
    harness.command("printf '%s' \"$$\" > pid-c-alone");
    assert_eq!(harness.wait_for_file("pid-c-alone"), pid_c);
    harness.send(b"\x1b2");
    harness.command("printf '%s' \"$$\" > pid-b-retained");
    assert_eq!(harness.wait_for_file("pid-b-retained"), pid_b);

    // Quit through the explicit confirmation path; Drop remains the fallback
    // cleanup if any assertion above fails.
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
