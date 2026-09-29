use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

mod common;
use common::PtyHarness as Harness;

const READY: Duration = Duration::from_secs(6);

impl Harness {
    fn start() -> Self {
        common::PtyHarness::spawn("multiplexer-nvim", 40, 120, &[])
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

    harness.confirm_quit(READY);
}
