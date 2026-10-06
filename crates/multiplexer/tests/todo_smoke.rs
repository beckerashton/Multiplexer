#![cfg(unix)]

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

struct Harness {
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: mpsc::Receiver<Vec<u8>>,
    screen: vt100::Parser,
    directory: PathBuf,
}
impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.directory);
    }
}
impl Harness {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "multiplexer-todo-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 67,
                cols: 240,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_multiplexer"));
        command.env("SHELL", "/bin/sh");
        command.env("ENV", "/dev/null");
        command.env("PS1", "todo-shell> ");
        command.env("TERM", "xterm-256color");
        command.env("XDG_STATE_HOME", directory.join("state"));
        command.arg("--fresh");
        let child = pair.slave.spawn_command(command).unwrap();
        let writer = pair.master.take_writer().unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let (tx, output) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = [0; 8192];
            while let Ok(n) = reader.read(&mut bytes) {
                if n == 0 || tx.send(bytes[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            writer,
            output,
            screen: vt100::Parser::new(67, 240, 0),
            directory,
        }
    }
    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }
    fn wait(&mut self, description: &str, condition: impl Fn(&vt100::Screen) -> bool) {
        let start = Instant::now();
        loop {
            if condition(self.screen.screen()) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "{description}: {}",
                self.screen.screen().contents()
            );
            if let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(20)) {
                self.screen.process(&bytes);
            }
        }
    }
    fn top(&mut self, expected: &str) {
        self.wait(expected, |screen| {
            let row: String = (0..240)
                .filter_map(|col| screen.cell(0, col))
                .map(|cell| cell.contents())
                .collect();
            row.trim_end() == expected
        });
    }
}

#[test]
fn fullscreen_todo_interactions_and_extended_keyboard_work_together() {
    let mut app = Harness::new();
    app.top(" 1 ▪");
    app.wait("shell and rounded border", |screen| {
        screen.contents().contains("todo-shell>")
            && screen.cell(1, 0).unwrap().contents() == "╭"
            && screen.cell(65, 239).unwrap().contents() == "╯"
    });

    // A disambiguated Ctrl-b opens the same popup as the legacy leader byte.
    app.send(b"\x1b[98;5u");
    app.wait("leader popup", |screen| {
        screen.contents().contains("Key bindings")
            && screen.contents().contains("<alt> hjkl ~ Change Focus")
            && screen
                .contents()
                .contains("<ctrl><alt> 1-9 ~ Carry Pane to Tab")
            && screen.hide_cursor()
    });
    // Process output beyond the old one-second deadline; an unbound key must
    // also leave command mode open without leaking into the shell.
    app.send(b"!\x1b[A\x1bOB");
    let until = Instant::now() + Duration::from_millis(1400);
    while Instant::now() < until {
        if let Ok(bytes) = app.output.recv_timeout(Duration::from_millis(20)) {
            app.screen.process(&bytes);
        }
    }
    assert!(app.screen.screen().contents().contains("Key bindings"));
    assert!(app.screen.screen().contents().contains("Manage Panes"));
    assert!(app.screen.screen().contents().contains("Broadcast"));
    app.send(b"\x1b[27u");
    app.wait("Escape dismisses popup", |screen| {
        !screen.contents().contains("Key bindings") && !screen.hide_cursor()
    });

    app.send(b"\x02|");
    app.top(" 1 ▫▪");
    app.send(b"\x1bh");
    app.top(" 1 ▪▫");
    // Ctrl-Alt-l moves the focused left pane into the right position.
    app.send(b"\x1b[108;7u");
    app.top(" 1 ▫▪");
    // Carry to an absent numbered tab creates the destination without a shell.
    app.send(b"\x1b[57;7u");
    app.top(" 1 ▫  │  9 ▪");
    app.send(b"\x1b5");
    app.top(" 1 ▫  │  5 ▪  │  9 ▫");
    app.wait("new tab shell", |screen| {
        screen.contents().contains("todo-shell>")
    });
    app.send(b"exit\r");
    app.top(" 1 ▫  │  5 ∅  │  9 ▫");
    app.send(b"\x1b1");
    app.top(" 1 ▪  │  9 ▫");
    // xterm modifyOtherKeys uses the same direct carry binding.
    app.send(b"\x1b[27;7;57~");
    app.top(" 9 ▫▪");
    app.send(b"\x1b[98;5uqy");
    let start = Instant::now();
    while app.child.try_wait().unwrap().is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "extended leader quit failed"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn stack_edges_follow_active_member_and_resize_terminal_content() {
    let mut app = Harness::new();
    app.wait("initial shell", |screen| {
        screen.contents().contains("todo-shell>")
    });
    app.send(b"\x02a");
    app.wait("second member", |screen| screen.contents().contains("2/2"));
    app.send(b"\x02a");
    app.wait("two edges above third member", |screen| {
        screen.cell(1, 2).unwrap().contents() == "1"
            && screen.cell(2, 2).unwrap().contents() == "2"
            && screen.contents().contains("3/3")
    });
    app.top(" 1 ▪");
    app.send(b"\x022");
    app.wait("edges above and below middle member", |screen| {
        screen.cell(1, 2).unwrap().contents() == "1"
            && screen.cell(65, 2).unwrap().contents() == "3"
            && screen.contents().contains("2/3")
    });
    app.send(b"stty size\r");
    app.wait(
        "terminal content accounts for both collapsed edges",
        |screen| screen.contents().contains("61 238"),
    );
    app.send(b"\x021");
    app.wait("two edges below first member", |screen| {
        screen.cell(64, 2).unwrap().contents() == "2"
            && screen.cell(65, 2).unwrap().contents() == "3"
            && screen.contents().contains("1/3")
    });
}

#[test]
fn legacy_pane_jumps_and_horizontal_edges_cross_populated_tabs() {
    let mut app = Harness::new();
    app.top(" 1 ▪");
    app.send(b"\x02a");
    app.wait("stack member created", |screen| {
        screen.contents().contains("2/2")
    });
    app.send(b"\x1bk");
    app.wait("Alt-k visits earlier member", |screen| {
        screen.contents().contains("1/2")
    });
    app.send(b"\x02|");
    app.top(" 1 ▫▪");
    app.send(b"\x1b[");
    app.top(" 1 ▪▫");
    app.wait("jump preserves displayed member", |screen| {
        screen.contents().contains("1/2")
    });
    app.send(b"\x1b]");
    app.top(" 1 ▫▪");
    app.send(b"\x1b3");
    app.top(" 1 ▫▫  │  3 ▪");
    app.send(b"\x1bh");
    app.top(" 1 ▫▪  │  3 ▫");
    app.send(b"\x1bl");
    app.top(" 1 ▫▫  │  3 ▪");
    app.send(b"\x1b]");
    app.top(" 1 ▫▪  │  3 ▫");
}

#[test]
fn resize_mode_and_broadcast_submenu_render_and_route_live_input() {
    let mut app = Harness::new();
    app.wait("initial shell", |s| s.contents().contains("todo-shell>"));
    app.send(b"\x02|");
    app.top(" 1 ▫▪");
    app.wait("split boundary", |s| {
        s.cell(65, 120).unwrap().contents() == "╰"
    });
    app.send(b"\x02r");
    app.wait("resize popup", |s| {
        s.contents().contains("Resize mode")
            && s.contents().contains("(5 cells)")
            && s.hide_cursor()
    });
    app.send(b"h");
    app.wait("one cell left", |s| {
        s.cell(65, 119).unwrap().contents() == "╰"
    });
    // Extended Shift-h must match a legacy uppercase H.
    app.send(b"\x1b[104;2u");
    app.wait("five cells left", |s| {
        s.cell(65, 114).unwrap().contents() == "╰"
    });
    app.send(b"\x1bh");
    app.top(" 1 ▪▫");
    app.send(b"l");
    app.wait("resize newly focused pane", |s| {
        s.cell(65, 115).unwrap().contents() == "╰" && s.contents().contains("Resize mode")
    });
    app.send(b"\x1b");
    app.wait("Escape ends resize mode", |s| {
        !s.contents().contains("Resize mode") && !s.hide_cursor()
    });
    app.send(b"printf 'MODE-EXIT-OK\\n'\r");
    app.wait("input resumes", |s| s.contents().contains("MODE-EXIT-OK"));

    app.send(b"\x02b");
    app.wait("broadcast menu", |s| {
        s.contents().contains("Toggle Visible Broadcast")
            && s.contents().contains("Reset Broadcast")
            && !s.contents().contains("BROADCAST ALL")
    });
    app.send(b"!");
    app.send(b"\x1b");
    app.wait("broadcast cancellation", |s| {
        !s.contents().contains("Toggle Visible Broadcast")
            && !s.contents().contains("BROADCAST ALL")
            && !s.hide_cursor()
    });
    app.send(b"\x02bb");
    app.wait("visible broadcast enabled", |s| {
        s.contents().contains("BROADCAST ALL") && !s.contents().contains("Toggle Visible Broadcast")
    });
    app.send(b"\x02br");
    app.wait("broadcast reset", |s| {
        !s.contents().contains("BROADCAST ALL") && !s.contents().contains("Reset Broadcast")
    });
}
