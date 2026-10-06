#![cfg(unix)]

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

fn temporary_directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "multiplexer-terminal-restore-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    fs::create_dir_all(&path).expect("create isolated test directory");
    path
}

fn wait_for(path: &Path, deadline: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if path.exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

fn wait_for_output(output: &Arc<Mutex<Vec<u8>>>, needle: &[u8], deadline: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if output
            .lock()
            .expect("lock captured output")
            .windows(needle.len())
            .any(|bytes| bytes == needle)
        {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

fn output_tail(output: &[u8]) -> String {
    let start = output.len().saturating_sub(4 * 1024);
    String::from_utf8_lossy(&output[start..]).into_owned()
}

fn termios(fd: libc::c_int) -> libc::termios {
    let mut value = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { libc::tcgetattr(fd, &mut value) },
        0,
        "read termios"
    );
    value
}

#[test]
fn confirmed_quit_restores_controlling_terminal_and_leaves_alternate_screen() {
    let directory = temporary_directory();
    let ready_marker = directory.join("shell-ready");
    let system = native_pty_system();
    let pair = system
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let fd = pair.master.as_raw_fd().expect("unix master fd");
    let before = termios(fd);
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_multiplexer"));
    command.cwd(&directory);
    command.env("SHELL", "/bin/sh");
    command.env("XDG_STATE_HOME", directory.join("state"));
    command.arg("--fresh");
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut writer = pair.master.take_writer().unwrap();
    let mut reader = pair.master.try_clone_reader().unwrap();
    let output = Arc::new(Mutex::new(Vec::new()));
    let output_reader = output.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    let drain = thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        let _ = ready_tx.send(());
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            let mut captured = output_reader.lock().expect("lock captured output");
            const MAX_CAPTURED_OUTPUT: usize = 128 * 1024;
            let excess = captured
                .len()
                .saturating_add(count)
                .saturating_sub(MAX_CAPTURED_OUTPUT);
            if excess > 0 {
                let drain_count = excess.min(captured.len());
                captured.drain(..drain_count);
            }
            captured.extend_from_slice(&buffer[..count]);
        }
    });
    ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let application_ready = wait_for_output(&output, b"\x1b[?1049h", Duration::from_secs(2));
    writer
        .write_all(b"printf ready > shell-ready\n")
        .expect("write shell readiness marker");
    writer.flush().expect("flush shell readiness marker");
    let shell_ready = wait_for(&ready_marker, Duration::from_secs(2));
    let marker_contents = fs::read(&ready_marker).unwrap_or_default();
    writer.write_all(&[0x02, b'q', b'y']).unwrap();
    writer.flush().unwrap();
    let start = Instant::now();
    let mut exited = false;
    while start.elapsed() < Duration::from_secs(3) {
        if child.try_wait().unwrap().is_some() {
            exited = true;
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    if !exited {
        let _ = child.kill();
    }
    let _ = child.wait();
    let after = termios(fd);
    drop(writer);
    drop(pair.master);
    let _ = drain.join();
    let output = output.lock().unwrap().clone();
    let _ = fs::remove_dir_all(&directory);

    assert!(
        application_ready,
        "multiplexer did not enter alternate screen; output_tail={:?}",
        output_tail(&output)
    );
    assert!(
        shell_ready,
        "shell did not become ready; output_tail={:?}",
        output_tail(&output)
    );
    assert_eq!(marker_contents, b"ready", "readiness marker content");
    assert!(
        exited,
        "confirmed quit did not exit; output_tail={:?}",
        output_tail(&output)
    );
    assert_eq!(
        before.c_lflag, after.c_lflag,
        "local terminal flags restored"
    );
    assert_eq!(
        before.c_iflag, after.c_iflag,
        "input terminal flags restored"
    );
    assert_eq!(
        &before.c_cc[..],
        &after.c_cc[..],
        "terminal control characters restored"
    );
    assert!(
        output
            .windows(b"\x1b[?1049l".len())
            .any(|bytes| bytes == b"\x1b[?1049l")
            || output
                .windows(b"\x1b[?47l".len())
                .any(|bytes| bytes == b"\x1b[?47l"),
        "expected alternate-screen leave sequence; output_tail={:?}",
        output_tail(&output)
    );
}
