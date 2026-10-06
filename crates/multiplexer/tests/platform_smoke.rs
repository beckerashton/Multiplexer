use std::process::Command;

#[test]
fn help_and_argument_errors_work_without_a_controlling_terminal() {
    let help = Command::new(env!("CARGO_BIN_EXE_multiplexer"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--fresh"));

    let error = Command::new(env!("CARGO_BIN_EXE_multiplexer"))
        .arg("--config")
        .output()
        .unwrap();
    assert!(!error.status.success());
    assert!(String::from_utf8_lossy(&error.stderr).contains("--config requires a path"));
}
