use std::{
    fs,
    io::Write,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

mod common;
use common::PtyHarness as Harness;

const READY: Duration = Duration::from_secs(5);

impl Harness {
    fn start() -> Self {
        let mut harness = common::PtyHarness::spawn("multiplexer-live", 40, 120, &[]);
        harness
            .writer
            .as_mut()
            .expect("PTY writer is alive")
            .flush()
            .expect("flush initial PTY writer");
        harness
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
        let output = self.output.lock().expect("capture lock");
        let tail_start = output.len().saturating_sub(2048);
        panic!(
            "timed out waiting for nonempty marker {}; PTY tail: {:?}",
            path.display(),
            String::from_utf8_lossy(&output[tail_start..])
        );
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
fn pane_navigation_uses_the_rendered_cursor_row_in_a_stack() {
    let mut harness = Harness::start();
    harness.command("printf ready > source-ready");
    harness.wait_for_file("source-ready");
    harness.send(&[0x02, b'\\']);
    harness.command("PANE=top; printf ready > top-ready");
    harness.wait_for_file("top-ready");
    harness.send(&[0x02, b'-']);
    harness.command("PANE=bottom; printf ready > bottom-ready");
    harness.wait_for_file("bottom-ready");
    harness.send(b"\x1bh\x02a");
    harness.command("PANE=source; printf ready > stacked-ready");
    harness.wait_for_file("stacked-ready");

    // The right panes meet at workspace row 20. The expanded left member's
    // content starts at row 3, after its collapsed member and top border.
    for (row, expected) in [(17, "top"), (18, "bottom")] {
        harness.command(&format!(
            "PS1=; printf '\\033[{row};1H'; printf ready > cursor-{row}"
        ));
        harness.wait_for_file(&format!("cursor-{row}"));
        let start = Instant::now();
        loop {
            let mut host = vt100::Parser::new(40, 120, 0);
            host.process(&harness.output.lock().expect("capture lock"));
            if host.screen().cursor_position() == (row + 2, 1) && !host.screen().hide_cursor() {
                break;
            }
            assert!(
                start.elapsed() < READY,
                "positioned cursor was not rendered"
            );
            thread::sleep(Duration::from_millis(10));
        }
        harness.send(b"\x1bl");
        harness.command(&format!("printf '%s' \"$PANE\" > selected-{row}"));
        assert_eq!(harness.wait_for_file(&format!("selected-{row}")), expected);
        harness.send(b"\x1bh");
    }
    harness.confirm_quit(READY);
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
    harness.confirm_quit(READY);
}
