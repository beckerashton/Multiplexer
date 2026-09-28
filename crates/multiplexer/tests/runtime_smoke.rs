use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

fn temporary_directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "multiplexer-runtime-smoke-{}-{}",
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

fn wait_for_bytes(path: &PathBuf, expected: &[u8], deadline: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if fs::read(path).ok().as_deref() == Some(expected) {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn shell_receives_exact_input_then_confirmed_quit_exits() {
    let directory = temporary_directory();
    let marker = directory.join("marker");
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open controlling PTY");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_multiplexer"));
    command.cwd(&directory);
    command.env("SHELL", "/bin/sh");
    let mut child = pair
        .slave
        .spawn_command(command)
        .expect("start multiplexer");
    let mut writer = pair.master.take_writer().expect("PTY writer");
    let mut reader = pair.master.try_clone_reader().expect("PTY reader");
    let (output_tx, output_rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut output = Vec::new();
        let _ = reader.read_to_end(&mut output);
        let _ = output_tx.send(String::from_utf8_lossy(&output).into_owned());
    });

    writer
        .write_all(b"printf exact-marker > marker\n")
        .expect("write ordinary shell input");
    writer.flush().expect("flush shell input");
    let marked = wait_for(&marker, Duration::from_secs(3));
    let observed = fs::read_to_string(&marker).unwrap_or_default();

    writer
        .write_all(&[0x02, b'q', b'y'])
        .expect("request confirmed quit");
    writer.flush().expect("flush quit request");
    let start = Instant::now();
    let mut exited = false;
    while start.elapsed() < Duration::from_secs(3) {
        if child.try_wait().expect("poll child").is_some() {
            exited = true;
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    if !exited {
        let _ = child.kill();
    }
    drop(writer);
    let output = output_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap_or_default();
    let _ = fs::remove_dir_all(&directory);

    assert!(
        marked,
        "ordinary bytes did not reach shell; PTY output: {output:?}"
    );
    assert_eq!(observed, "exact-marker");
    assert!(
        exited,
        "leader-q then y did not exit multiplexer; PTY output: {output:?}"
    );
}

#[test]
fn standalone_escape_reaches_a_raw_shell_reader() {
    assert_raw_input(&[0x1b], &[0x1b]);
}

#[test]
fn extended_modifiers_reach_a_fullscreen_raw_shell_reader() {
    assert_raw_input(
        b"\x1b[99;133u\x1b[97;4u\x1b[99;6u\x1b[233;3u\x1b[9;2u\x1b[27;6;67~",
        "\x03\x1bA\x03\x1bé\x1b[Z\x03".as_bytes(),
    );
}

#[test]
fn arrows_follow_the_child_cursor_mode() {
    let input = b"\x1b[A\x1b[B\x1bOC\x1bOD\x1b[1;129A\x1b[1;133D";
    assert_raw_input_mode(input, b"\x1b[A\x1b[B\x1b[C\x1b[D\x1b[A\x1b[1;5D", false);
    assert_raw_input_mode(input, b"\x1bOA\x1bOB\x1bOC\x1bOD\x1bOA\x1b[1;5D", true);
}

fn assert_raw_input(input: &[u8], expected: &[u8]) {
    assert_raw_input_mode(input, expected, false);
}

fn assert_raw_input_mode(input: &[u8], expected: &[u8], application: bool) {
    let directory = temporary_directory();
    let marker = directory.join("escape-byte");
    let ready = directory.join("escape-ready");
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 67,
            cols: 240,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open controlling PTY");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_multiplexer"));
    command.cwd(&directory);
    command.env("SHELL", "/bin/sh");
    let mut child = pair
        .slave
        .spawn_command(command)
        .expect("start multiplexer");
    let mut writer = pair.master.take_writer().expect("PTY writer");
    let mut reader = pair.master.try_clone_reader().expect("PTY reader");
    thread::spawn(move || {
        let mut discard = [0_u8; 4096];
        while reader.read(&mut discard).is_ok() {}
    });

    // Canonical shell input would wait for a newline and `dd` creates its
    // destination before receiving a byte. Use a raw child reader and a
    // separate ready marker so the assertion measures the application byte.
    writer.write_all(format!("printf '\\033[?1{}'; stty raw -echo; : > escape-ready; dd bs=1 count={} of=escape-byte 2>/dev/null; stty sane\n", if application { "h" } else { "l" }, expected.len()).as_bytes()).unwrap();
    writer.flush().unwrap();
    assert!(
        wait_for(&ready, Duration::from_secs(2)),
        "raw child did not become ready"
    );
    writer.write_all(input).unwrap();
    writer.flush().unwrap();
    let arrived = wait_for_bytes(&marker, expected, Duration::from_secs(2));
    let observed = fs::read(&marker).unwrap_or_default();
    let _ = child.kill();
    let _ = fs::remove_dir_all(&directory);

    assert!(
        arrived,
        "input did not reach raw child reader within deadline"
    );
    assert_eq!(observed, expected);
}
