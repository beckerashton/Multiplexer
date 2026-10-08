use std::{
    io::{Read, Write},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

struct ChildGuard(Box<dyn Child + Send + Sync>);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn settled_output(rx: &Receiver<Vec<u8>>) -> Vec<u8> {
    let start = Instant::now();
    let mut output = Vec::new();
    while let Ok(bytes) = rx.recv_timeout(Duration::from_millis(200)) {
        output.extend(bytes);
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "output never settled"
        );
    }
    output
}

#[test]
fn fullscreen_mouse_typing_and_resize_use_incremental_output() {
    for (cols, rows) in [(240, 67), (320, 90)] {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_multiplexer"));
        let state_dir = std::env::temp_dir().join(format!("mux-rendering-{}-{cols}", std::process::id()));
        command.env("XDG_STATE_HOME", &state_dir);
        command.arg("--fresh");
        command.env("SHELL", "/bin/sh");
        command.env("ENV", "/dev/null");
        command.env("PS1", "mux-test> ");
        command.env("TERM", "xterm-256color");
        let mut child = ChildGuard(pair.slave.spawn_command(command).unwrap());
        drop(pair.slave);
        let mut writer = pair.master.take_writer().unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0; 8192];
            while let Ok(n) = reader.read(&mut buffer) {
                if n == 0 || tx.send(buffer[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut host = vt100::Parser::new(rows, cols, 0);
        let start = Instant::now();
        loop {
            host.process(&settled_output(&rx));
            if host.screen().contents().contains("mux-test>") {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "shell prompt missing"
            );
        }
        assert!(
            settled_output(&rx).is_empty(),
            "idle terminal emitted output"
        );
        writer.write_all(b"\x02z").unwrap();
        writer.flush().unwrap();
        host.process(&settled_output(&rx));
        assert!(host.screen().contents().contains("mux-test>"));
        assert!(!host.screen().contents().contains('╭'));
        assert!(!host.screen().contents().contains("focused"));
        writer.write_all(b"stty size\r").unwrap();
        writer.flush().unwrap();
        host.process(&settled_output(&rx));
        assert!(host.screen().contents().contains(&format!("{rows} {cols}")), "{}", host.screen().contents());
        writer.write_all(b"\x02").unwrap();
        writer.flush().unwrap();
        host.process(&settled_output(&rx));
        assert!(host.screen().contents().contains("Key bindings"));
        writer.write_all(b"z").unwrap();
        writer.flush().unwrap();
        host.process(&settled_output(&rx));
        assert!(host.screen().contents().contains('╭'));
        assert!(host.screen().contents().contains("focused"));
        for x in 10..30 {
            write!(writer, "\x1b[<35;{x};10M").unwrap();
        }
        writer.flush().unwrap();
        assert!(
            settled_output(&rx).is_empty(),
            "ignored mouse motion repainted"
        );
        writer.write_all(b"a").unwrap();
        writer.flush().unwrap();
        let typed = settled_output(&rx);
        assert!(!typed.is_empty());
        assert!(
            typed.len() < 256,
            "{cols}x{rows}: typing emitted {} bytes",
            typed.len()
        );
        assert!(!typed.windows(4).any(|b| b == b"\x1b[2J"));
        assert!(typed.starts_with(b"\x1b[?2026h\x1b[?25l"));
        assert!(typed.ends_with(b"\x1b[?2026l"));
        host.process(&typed);
        assert!(host.screen().contents().contains("mux-test> a"));
        assert!(!host.screen().hide_cursor());
        eprintln!("{cols}x{rows}: mouse=0 bytes; typing={} bytes", typed.len());

        let (new_cols, new_rows) = (cols - 40, rows - 12);
        host.screen_mut().set_size(new_rows, new_cols);
        pair.master
            .resize(PtySize {
                rows: new_rows,
                cols: new_cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let resized = settled_output(&rx);
        assert!(!resized.is_empty(), "resize was not rendered");
        host.process(&resized);
        assert!(host.screen().contents().contains("mux-test> a"));
        let status: String = (0..new_cols)
            .filter_map(|col| host.screen().cell(new_rows - 1, col))
            .map(|cell| cell.contents())
            .collect();
        assert!(
            status.starts_with("tab 1"),
            "status row did not follow resize"
        );

        writer.write_all(b"\x02qy").unwrap();
        writer.flush().unwrap();
        let start = Instant::now();
        while child.0.try_wait().unwrap().is_none() {
            assert!(start.elapsed() < Duration::from_secs(5), "quit failed");
            thread::sleep(Duration::from_millis(10));
        }
        std::fs::remove_dir_all(state_dir).unwrap();
    }
}
