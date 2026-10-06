#![cfg(windows)]

mod common;

use std::{
    fs, thread,
    time::{Duration, Instant},
};

use common::PtyHarness;

fn expect_marker(harness: &PtyHarness, name: &str) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if let Ok(marker) = fs::read_to_string(harness.directory.join(name)) {
            if let Some(version) = marker.strip_prefix("exact-marker:PS") {
                if let Ok(major) = version.parse::<u32>() {
                    assert!(major >= 7, "pane must run modern PowerShell");
                    return;
                }
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "PowerShell 7 did not receive input; output: {:?}",
        String::from_utf8_lossy(&harness.output.lock().unwrap())
    );
}

#[test]
fn missing_modern_powershell_reports_install_error_before_terminal_setup() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_multiplexer"))
        .env(
            "ProgramFiles",
            std::env::temp_dir().join("multiplexer-no-powershell-install"),
        )
        .env("PATH", "")
        .env_remove("MULTIPLEXER_SHELL")
        .output()
        .expect("start multiplexer without PowerShell");
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("PowerShell 7 (pwsh.exe) was not found"),
        "{error}"
    );
    assert!(error.contains("winget install Microsoft.PowerShell"));
    assert!(
        !output.stdout.contains(&0x1b),
        "terminal setup must not run"
    );
}

#[test]
fn conpty_shell_split_layout_and_confirmed_quit() {
    let mut harness = PtyHarness::spawn("multiplexer-windows-smoke", 30, 100, &[]);
    let layout_path = harness.directory.join("state/multiplexer/layout.toml");
    let start = Instant::now();
    while !layout_path.is_file() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "startup timed out"
        );
        thread::sleep(Duration::from_millis(20));
    }
    harness.command("[IO.File]::WriteAllText('first-marker', 'exact-marker:PS' + $PSVersionTable.PSVersion.Major)");
    expect_marker(&harness, "first-marker");

    harness.send(&[0x02, b'|']);
    harness.command("[IO.File]::WriteAllText('second-marker', 'exact-marker:PS' + $PSVersionTable.PSVersion.Major)");
    expect_marker(&harness, "second-marker");
    harness.confirm_quit(Duration::from_secs(10));

    let saved = fs::read_to_string(layout_path).expect("Windows layout file");
    let layout: mux_core::WorkspaceLayout = toml::from_str(&saved).unwrap();
    let (restored, _) = mux_core::Workspace::restore_layout(
        layout,
        mux_core::SessionSpec {
            program: "pwsh.exe".into(),
            args: Vec::new(),
            cwd: None,
        },
    )
    .unwrap();
    assert_eq!(restored.view().tabs.len(), 1);
    assert_eq!(restored.view().tabs[0].slots.len(), 2);
}
