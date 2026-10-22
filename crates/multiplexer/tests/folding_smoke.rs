mod common;

use common::PtyHarness;
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn wait(harness: &PtyHarness, condition: impl Fn(&vt100::Screen) -> bool) {
    let start = Instant::now();
    loop {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(&harness.output.lock().unwrap());
        if condition(parser.screen()) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "screen: {}",
            parser.screen().contents()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn fold_and_unfold_keep_the_shell_alive_and_restore_its_display() {
    let mut harness = PtyHarness::spawn(
        "mux-fold",
        24,
        80,
        &[("PS1", "fold-shell> "), ("ENV", "/dev/null")],
    );
    wait(&harness, |s| s.contents().contains("fold-shell>"));
    harness.send(b"\x02\\");
    wait(&harness, |s| s.cell(1, 40).unwrap().contents() == "╭");
    harness.command("echo $$ > before; echo FOLD_READY; sleep 1; echo alive > alive");
    wait(&harness, |s| s.contents().contains("FOLD_READY"));
    // Wait for actual command execution rather than its input echo.
    let start = Instant::now();
    while !harness.directory.join("before").exists() {
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(20));
    }
    harness.send(b"\x02f");
    wait(&harness, |s| s.cell(1, 79).unwrap().contents() == "▸");
    harness.command("echo unexpected > unwanted");
    let start = Instant::now();
    while !harness.directory.join("alive").exists() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "folded shell stopped running"
        );
        thread::sleep(Duration::from_millis(20));
    }
    let layout =
        fs::read_to_string(harness.directory.join("state/multiplexer/layout.toml")).unwrap();
    assert!(layout.contains("folded = true"));
    harness.send(b"\x02f");
    wait(&harness, |s| {
        s.cell(1, 40).unwrap().contents() == "╭" && s.contents().contains("fold-shell>")
    });
    harness.command("echo $$ > after; echo UNFOLDED_OK");
    let start = Instant::now();
    while !harness.directory.join("after").exists() {
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        fs::read(harness.directory.join("before")).unwrap(),
        fs::read(harness.directory.join("after")).unwrap()
    );
    assert!(!harness.directory.join("unwanted").exists());
    wait(&harness, |s| s.contents().contains("UNFOLDED_OK"));
    harness.confirm_quit(Duration::from_secs(5));
}
