#![cfg(windows)]

#[path = "../src/clipboard.rs"]
mod clipboard;

use std::process::Command;

#[test]
#[ignore = "overwrites the Windows clipboard; run only in a disposable desktop/session"]
fn system_clipboard_roundtrip_preserves_unicode_and_literal_text() {
    let samples = [
        String::new(),
        "snowman ☃, emoji 🦀, 日本語, café".into(),
        "first\nsecond\r\nthird\n".into(),
        "'\"; $(throw 'not code'); `n; $env:USERNAME\n".into(),
        "long Unicode line 🦀\n".repeat(8192),
    ];
    for (index, text) in samples.iter().enumerate() {
        clipboard::copy_with_helper(text, &clipboard::ClipboardEnvironment::default())
            .expect("copy through actual Windows clipboard helper");
        let output = Command::new("powershell.exe")
            .args([
                "-NoLogo", "-NoProfile", "-NonInteractive", "-STA", "-Command",
                "$ErrorActionPreference = 'Stop'; $text = [string](Get-Clipboard -Raw); $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes($text); [Console]::OpenStandardOutput().Write($bytes, 0, $bytes.Length)",
            ])
            .output()
            .expect("read actual Windows clipboard");
        assert!(
            output.status.success(),
            "clipboard read failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout == text.as_bytes(),
            "sample {index}: clipboard bytes differ (expected {}, received {})",
            text.len(),
            output.stdout.len()
        );
    }
}
