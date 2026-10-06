use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

mod common;
use common::PtyHarness as Harness;

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

impl Harness {
    fn start(env: &[(&str, &str)]) -> Self {
        let mut variables = vec![("TERM", "xterm-256color"), ("NO_COLOR", "")];
        variables.extend_from_slice(env);
        common::PtyHarness::spawn("multiplexer-clipboard", 40, 120, &variables)
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

    fn wait_for_screen(&self, description: &str, condition: impl Fn(&vt100::Screen) -> bool) {
        let start = Instant::now();
        while start.elapsed() < READY {
            let mut parser = vt100::Parser::new(40, 120, 0);
            parser.process(&self.output.lock().unwrap());
            if condition(parser.screen()) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let mut parser = vt100::Parser::new(40, 120, 0);
        parser.process(&self.output.lock().unwrap());
        panic!(
            "timed out waiting for {description}: cursor={:?}, hidden={}, row={:?}, text={}",
            parser.screen().cursor_position(),
            parser.screen().hide_cursor(),
            (1..=4)
                .map(|col| parser.screen().cell(2, col).unwrap().bgcolor())
                .collect::<Vec<_>>(),
            parser.screen().contents()
        );
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
fn vim_selection_yanks_to_clipboard_and_restores_shell_input() {
    let directory = HelperDirectory::new("multiplexer-clipboard-helper");
    fs::create_dir_all(&directory.0).expect("create helper directory");
    let path = helper_environment(&directory.0);
    let mut harness = Harness::start(&[("PATH", &path), ("WAYLAND_DISPLAY", "test-wayland")]);
    let capture = directory.0.join("clipboard-capture");

    // Clear the child screen and put four known cells at child row 0, col 0.
    harness.command("printf '\\033[2J\\033[HCLIP'; : > clip-ready");
    harness.wait_for_file("clip-ready", None);
    harness.wait_for_pane_text("CLIP");

    // Entry starts at the application cursor; select four cells from the
    // beginning of its line without sending any selection keys to the shell.
    harness.send(b"\x02y0v3l");
    harness.wait_for_screen("visible selection cursor and highlight", |s| {
        s.cursor_position() == (2, 4)
            && !s.hide_cursor()
            && (1..=4)
                .all(|col| s.cell(2, col).unwrap().bgcolor() == vt100::Color::Rgb(0x44, 0x47, 0x5a))
    });
    harness.send(b"y");
    harness.wait_for_path(&capture, Some(b"CLIP"));
    harness.command(": > after-yank");
    harness.wait_for_file("after-yank", None);
    harness.confirm_quit(READY);
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
    harness.command(
        "stty raw -echo; printf '\\033[?1002h\\033[?1006h'; : > wheel-ready; dd bs=1 count=10 of=wheel-bytes 2>/dev/null; stty sane; : > wheel-done",
    );
    harness.wait_for_file("wheel-ready", None);
    thread::sleep(Duration::from_millis(100));
    harness.send(b"\x1b[<64;3;3M");
    harness.wait_for_file("wheel-done", None);
    harness.wait_for_file("wheel-bytes", Some(b"\x1b[<64;2;1M"));
    harness.confirm_quit(READY);
}

#[test]
fn shell_wheel_scrollback_and_typing_returns_to_live_output() {
    let mut harness = Harness::start(&[]);
    harness.command("printf '\\033[2J\\033[H'; i=0; while [ $i -lt 80 ]; do printf 'HISTORY-%03d\\n' $i; i=$((i+1)); done; : > history-ready");
    harness.wait_for_file("history-ready", None);
    thread::sleep(Duration::from_millis(100));
    let live_top = {
        let mut parser = vt100::Parser::new(40, 120, 0);
        parser.process(&harness.output.lock().unwrap());
        (1..12)
            .map(|col| parser.screen().cell(2, col).unwrap().contents())
            .collect::<String>()
    };
    assert!(live_top.starts_with("HISTORY-"));
    assert_ne!(live_top, "HISTORY-000");
    // Reach the oldest retained line, including with Shift held.
    for _ in 0..40 {
        harness.send(b"[<68;3;3M");
    }
    harness.wait_for_pane_text("HISTORY-000");
    {
        let mut parser = vt100::Parser::new(40, 120, 0);
        parser.process(&harness.output.lock().unwrap());
        assert!(parser.screen().hide_cursor());
    }
    // Downward wheel returns to the live viewport.
    for _ in 0..40 {
        harness.send(b"[<65;3;3M");
    }
    harness.wait_for_pane_text(&live_top);
    for _ in 0..40 {
        harness.send(b"[<64;3;3M");
    }
    harness.wait_for_pane_text("HISTORY-000");
    harness.command("printf '\\033[2J\\033[HLIVE'; : > live-ready");
    harness.wait_for_file("live-ready", None);
    harness.wait_for_pane_text("LIVE");
    harness.confirm_quit(READY);
}

#[test]
fn selection_freezes_history_consumes_paste_and_copies_across_viewports() {
    let directory = HelperDirectory::new("multiplexer-selection-history");
    fs::create_dir_all(&directory.0).unwrap();
    let path = helper_environment(&directory.0);
    let capture = directory.0.join("clipboard-capture");
    let mut harness = Harness::start(&[("PATH", &path), ("WAYLAND_DISPLAY", "test-wayland")]);
    harness.command("printf '\\033[2J\\033[H'; i=0; while [ $i -lt 80 ]; do printf 'ROW-%03d   \\n' $i; i=$((i+1)); done; : > history-ready");
    harness.wait_for_file("history-ready", None);
    harness.wait_for_screen("last history row", |s| s.contents().contains("ROW-079"));
    harness.send(b"\x02yggV40j");
    harness.wait_for_screen("selection mode", |s| s.contents().contains("SELECT LINE"));
    // Pasted motion-looking input, Alt navigation and mouse reports must not
    // change the selection or reach the underlying shell.
    harness.send(b"\x1b[200~Gjunk\x1b[201~\x1bh\x1b[<64;3;3M");
    harness.send(b"y");
    use std::fmt::Write as _;
    let mut expected = String::new();
    for i in 0..=40 {
        writeln!(expected, "ROW-{i:03}").unwrap();
    }
    harness.wait_for_path(&capture, Some(expected.as_bytes()));
    harness.command(": > after-history-yank");
    harness.wait_for_file("after-history-yank", None);
    harness.confirm_quit(READY);
}

#[test]
fn selection_snapshot_stays_fixed_and_escape_returns_to_live_output() {
    let mut harness = Harness::start(&[]);
    harness.command("printf '\\033[2J\\033[HFROZEN'; : > frozen-ready; while [ ! -e release-output ]; do sleep 0.02; done; printf '\\r\\nNEW-LIVE-OUTPUT'; : > output-done");
    harness.wait_for_file("frozen-ready", None);
    harness.wait_for_pane_text("FROZEN");
    harness.send(b"\x02y");
    harness.wait_for_screen("selection started", |s| {
        s.contents().contains("SELECT NORMAL")
    });
    fs::write(harness.directory.join("release-output"), b"").unwrap();
    harness.wait_for_file("output-done", None);
    // Force another rendered frame after the output has arrived.
    thread::sleep(Duration::from_millis(100));
    harness.send(b"v");
    harness.wait_for_screen("frozen selection", |s| {
        s.contents().contains("SELECT VISUAL") && !s.contents().contains("NEW-LIVE-OUTPUT")
    });
    harness.send(b"\x1b");
    harness.wait_for_screen("live output restored", |s| {
        s.contents().contains("NEW-LIVE-OUTPUT") && !s.contents().contains("SELECT VISUAL")
    });
    harness.confirm_quit(READY);
}

#[test]
fn failed_yank_keeps_selection_and_can_retry() {
    let directory = HelperDirectory::new("multiplexer-selection-retry");
    fs::create_dir_all(&directory.0).unwrap();
    let path = helper_environment(&directory.0);
    // Fail every clipboard backend without writing to a real clipboard.
    for helper in ["wl-copy", "xclip", "xsel"] {
        let file = directory.0.join(helper);
        fs::write(&file, "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut harness = Harness::start(&[("PATH", &path), ("WAYLAND_DISPLAY", "test-wayland")]);
    harness.command("printf '\\033[2J\\033[HRETRY'; : > retry-ready");
    harness.wait_for_file("retry-ready", None);
    harness.wait_for_pane_text("RETRY");
    harness.send(b"\x02y0v4ly");
    harness.wait_for_screen("clipboard error", |s| s.contents().contains("copy failed:"));
    helper_environment(&directory.0);
    harness.send(b"y");
    harness.wait_for_path(&directory.0.join("clipboard-capture"), Some(b"RETRY"));
    harness.command(": > after-retry");
    harness.wait_for_file("after-retry", None);
    harness.confirm_quit(READY);
}
