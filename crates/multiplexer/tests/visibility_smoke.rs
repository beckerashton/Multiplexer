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

fn size(harness: &mut PtyHarness, name: &str, expected: &str) {
    harness.command(&format!("stty size > {name}"));
    let start = Instant::now();
    loop {
        if let Ok(actual) = fs::read_to_string(harness.directory.join(name)) {
            if !actual.is_empty() {
                assert_eq!(actual.trim(), expected, "{name}");
                return;
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "missing size: {name}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn visibility_changes_rendering_and_actual_pty_dimensions() {
    let mut harness = PtyHarness::spawn(
        "mux-visibility",
        24,
        80,
        &[("PS1", "vis> "), ("ENV", "/dev/null")],
    );
    wait(&harness, |s| s.contents().contains("vis>"));
    size(&mut harness, "initial", "21 78");
    harness.send(b"\x02tk\x1b[27u");
    wait(&harness, |s| {
        !s.contents().contains("Visibility") && s.cell(1, 0).unwrap().contents() == "│"
    });
    size(&mut harness, "no-top", "22 78");
    harness.send(b"\x02tb\x1b[27u");
    wait(&harness, |s| {
        !s.contents().contains("Visibility") && s.cell(0, 0).unwrap().contents() == "│"
    });
    size(&mut harness, "no-bar", "23 78");
    // All borders first restores the partially hidden set, then hides them.
    harness.send(b"\x02taa\x1b[27u");
    wait(&harness, |s| {
        !s.contents().contains('│') && !s.contents().contains('╰')
    });
    size(&mut harness, "no-borders", "24 80");
    harness.send(b"\x02a");
    wait(&harness, |s| s.cell(0, 0).unwrap().contents() == "╭");
    size(&mut harness, "stack", "23 80");
    harness.send(b"\x02ts\x1b[27u");
    wait(&harness, |s| !s.contents().contains('╭'));
    size(&mut harness, "no-stack-lines", "24 80");
    harness.confirm_quit(Duration::from_secs(5));
}

#[test]
fn visibility_menu_stays_open_across_toggles_and_pane_navigation() {
    let mut harness = PtyHarness::spawn(
        "mux-visibility-navigation",
        24,
        80,
        &[("PS1", "vis> "), ("ENV", "/dev/null")],
    );
    wait(&harness, |s| s.contents().contains("vis>"));
    harness.send(b"\x02\\");
    wait(&harness, |s| s.cell(1, 40).unwrap().contents() == "╭");
    // Toggle the right pane's top twice, then hide the left pane's bottom
    // and the right pane's right border without leaving the menu.
    harness.send(b"\x02tkk\x1bhj\x1bll");
    wait(&harness, |s| {
        s.contents().contains("Visibility — selected pane")
            && s.cell(23, 0).unwrap().contents() == "│"
            && s.cell(23, 79).unwrap().contents() == "─"
    });
    harness.send(b"\x1b[27u");
    wait(&harness, |s| {
        !s.contents().contains("Visibility — selected pane")
    });
    size(&mut harness, "right", "21 39");
    harness.send(b"\x1bh");
    size(&mut harness, "left", "22 38");
    harness.confirm_quit(Duration::from_secs(5));
}

#[test]
fn inside_and_outside_border_groups_reclaim_their_pty_space() {
    let mut harness = PtyHarness::spawn(
        "mux-border-groups",
        24,
        80,
        &[("PS1", "vis> "), ("ENV", "/dev/null")],
    );
    wait(&harness, |s| s.contents().contains("vis>"));
    harness.send(b"\x02\\");
    wait(&harness, |s| s.cell(1, 40).unwrap().contents() == "╭");
    harness.send(b"\x02Ti\x1b[27u");
    wait(&harness, |s| {
        !s.contents().contains("Visibility") && s.cell(2, 40).unwrap().contents() != "│"
    });
    size(&mut harness, "right-inside-hidden", "21 39");
    harness.send(b"\x1bh");
    size(&mut harness, "left-inside-hidden", "21 39");
    harness.send(b"\x02To\x1b[27u");
    wait(&harness, |s| {
        !s.contents().contains("Visibility") && !s.contents().contains('╭')
    });
    size(&mut harness, "all-hidden", "23 40");
    harness.send(b"\x02To\x1b[27u");
    wait(&harness, |s| {
        !s.contents().contains("Visibility") && s.cell(1, 0).unwrap().contents() == "╭"
    });
    size(&mut harness, "outside-restored", "21 39");
    harness.send(b"\x02ti\x1b[27u");
    wait(&harness, |s| {
        !s.contents().contains("Visibility") && s.cell(2, 39).unwrap().contents() == "│"
    });
    size(&mut harness, "left-restored", "21 38");
    harness.send(b"\x1bl");
    size(&mut harness, "right-still-hidden", "21 39");
    harness.confirm_quit(Duration::from_secs(5));
}
