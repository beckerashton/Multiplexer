use std::{
    ffi::OsString,
    io::Write,
    process::{Command, Stdio},
};

/// Host clipboard selection. Wayland uses `wl-copy`; X11 attempts `xclip`
/// before `xsel`. The optional path is private and exists only to make helper
/// discovery deterministic in tests without changing the user's environment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardEnvironment {
    pub wayland: bool,
    search_path: Option<OsString>,
}

impl ClipboardEnvironment {
    #[must_use]
    pub fn detect() -> Self {
        Self {
            wayland: std::env::var_os("WAYLAND_DISPLAY").is_some(),
            search_path: None,
        }
    }

    #[cfg(test)]
    fn with_search_path(wayland: bool, search_path: OsString) -> Self {
        Self {
            wayland,
            search_path: Some(search_path),
        }
    }
}

impl Default for ClipboardEnvironment {
    fn default() -> Self {
        Self::detect()
    }
}

#[derive(Debug)]
pub enum ClipboardError {
    NoHelper {
        attempted: Vec<&'static str>,
    },
    HelperFailed {
        helper: &'static str,
        message: String,
    },
    Input(std::io::Error),
}

impl std::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoHelper { attempted } => write!(
                f,
                "no clipboard helper found (tried {})",
                attempted.join(", ")
            ),
            Self::HelperFailed { helper, message } => {
                write!(f, "clipboard helper {helper} failed: {message}")
            }
            Self::Input(error) => write!(f, "failed to write clipboard input: {error}"),
        }
    }
}

impl std::error::Error for ClipboardError {}

/// Copies UTF-8 text through a local helper. The text is written to stdin;
/// it is never included in a command line or evaluated by a shell. Selection
/// ownership stays with the caller, so every error leaves it available to
/// retry or inspect.
pub fn copy_with_helper(
    text: &str,
    environment: &ClipboardEnvironment,
) -> Result<(), ClipboardError> {
    let candidates: &[(&str, &[&str])] = if environment.wayland {
        &[("wl-copy", &[])]
    } else {
        &[
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    };
    let mut attempted = Vec::with_capacity(candidates.len());
    let mut last_failure = None;
    for &(helper, args) in candidates {
        attempted.push(helper);
        match invoke(helper, args, text, environment) {
            Ok(()) => return Ok(()),
            Err(InvokeError::NotFound) => {}
            Err(InvokeError::Failed(message)) => {
                last_failure = Some(ClipboardError::HelperFailed { helper, message })
            }
            Err(InvokeError::Input(error)) => return Err(ClipboardError::Input(error)),
        }
    }
    Err(last_failure.unwrap_or(ClipboardError::NoHelper { attempted }))
}

enum InvokeError {
    NotFound,
    Failed(String),
    Input(std::io::Error),
}

fn invoke(
    helper: &'static str,
    args: &[&str],
    text: &str,
    environment: &ClipboardEnvironment,
) -> Result<(), InvokeError> {
    let mut command = Command::new(helper);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(path) = &environment.search_path {
        command.env("PATH", path);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(InvokeError::NotFound);
        }
        Err(error) => return Err(InvokeError::Failed(error.to_string())),
    };
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(text.as_bytes())
            .map_err(InvokeError::Input)?;
    }
    let output = child.wait_with_output().map_err(InvokeError::Input)?;
    if output.status.success() {
        Ok(())
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(InvokeError::Failed(if message.is_empty() {
            output.status.to_string()
        } else {
            message
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct HelperDir(PathBuf);

    impl HelperDir {
        fn new() -> Self {
            let unique = format!(
                "multiplexer-clipboard-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn helper(&self, name: &str, exit_status: u8) -> PathBuf {
            let path = self.0.join(name);
            let capture = self.0.join(format!("{name}.stdin"));
            fs::write(
                &path,
                format!(
                    "#!/bin/sh\n/bin/cat > '{}'\nexit {exit_status}\n",
                    capture.display()
                ),
            )
            .unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        fn captured(&self, name: &str) -> String {
            fs::read_to_string(self.0.join(format!("{name}.stdin"))).unwrap()
        }
    }

    impl Drop for HelperDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn environment(dir: &Path, wayland: bool) -> ClipboardEnvironment {
        ClipboardEnvironment::with_search_path(wayland, dir.as_os_str().to_os_string())
    }

    #[test]
    fn wayland_writes_utf8_to_wl_copy_stdin() {
        let dir = HelperDir::new();
        dir.helper("wl-copy", 0);
        copy_with_helper("snowman ☃\n", &environment(&dir.0, true)).unwrap();
        assert_eq!(dir.captured("wl-copy"), "snowman ☃\n");
    }

    #[test]
    fn x11_falls_back_from_failed_xclip_to_xsel() {
        let dir = HelperDir::new();
        dir.helper("xclip", 1);
        dir.helper("xsel", 0);
        copy_with_helper("fallback", &environment(&dir.0, false)).unwrap();
        assert_eq!(dir.captured("xclip"), "fallback");
        assert_eq!(dir.captured("xsel"), "fallback");
    }

    #[test]
    fn missing_helper_is_reported_without_any_external_clipboard_write() {
        let dir = HelperDir::new();
        assert!(
            matches!(copy_with_helper("kept", &environment(&dir.0, true)), Err(ClipboardError::NoHelper { attempted }) if attempted == vec!["wl-copy"])
        );
    }
}
