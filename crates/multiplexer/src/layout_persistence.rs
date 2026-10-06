use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process,
};

use mux_core::WorkspaceLayout;
use thiserror::Error;

const MAX_LAYOUT_FILE_SIZE: u64 = 1_048_576;

#[derive(Debug, Error)]
pub enum LayoutStoreError {
    #[error("cannot {operation} layout file {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    #[error("layout file {path} is larger than 1 MiB")]
    TooLarge { path: PathBuf },
    #[error("invalid layout file {path}: {source}")]
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    #[error("cannot encode layout: {0}")]
    Encode(#[from] toml::ser::Error),
    #[error("no absolute state directory is available")]
    NoStateDirectory,
}

pub fn default_path() -> Result<PathBuf, LayoutStoreError> {
    #[cfg(windows)]
    let directory = windows_state_directory(
        env::var_os("LOCALAPPDATA").map(PathBuf::from),
        env::var_os("USERPROFILE").map(PathBuf::from),
    )?;

    #[cfg(not(windows))]
    let directory = state_directory()?;

    Ok(directory.join("multiplexer/layout.toml"))
}

#[cfg(windows)]
fn windows_state_directory(
    local_app_data: Option<PathBuf>,
    user_profile: Option<PathBuf>,
) -> Result<PathBuf, LayoutStoreError> {
    local_app_data
        .filter(|path| path.is_absolute())
        .or_else(|| {
            user_profile
                .filter(|path| path.is_absolute())
                .map(|home| home.join("AppData/Local"))
        })
        .ok_or(LayoutStoreError::NoStateDirectory)
}

#[cfg(not(windows))]
fn state_directory() -> Result<PathBuf, LayoutStoreError> {
    if let Some(path) = env::var_os("XDG_STATE_HOME").map(PathBuf::from) {
        if path.is_absolute() {
            return Ok(path);
        }
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(LayoutStoreError::NoStateDirectory)?;
    Ok(home.join(".local/state"))
}

pub fn load(path: &Path) -> Result<Option<WorkspaceLayout>, LayoutStoreError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(LayoutStoreError::Io {
                operation: "inspect",
                path: path.to_owned(),
                source,
            });
        }
    };
    if metadata.len() > MAX_LAYOUT_FILE_SIZE {
        return Err(LayoutStoreError::TooLarge {
            path: path.to_owned(),
        });
    }
    let text = fs::read_to_string(path).map_err(|source| LayoutStoreError::Io {
        operation: "read",
        path: path.to_owned(),
        source,
    })?;
    toml::from_str(&text)
        .map(Some)
        .map_err(|source| LayoutStoreError::Parse {
            path: path.to_owned(),
            source: Box::new(source),
        })
}

pub struct LayoutStore {
    path: PathBuf,
    last_saved: Option<WorkspaceLayout>,
}

impl LayoutStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            last_saved: None,
        }
    }

    pub fn save_initial(&mut self, layout: &WorkspaceLayout) -> Result<(), LayoutStoreError> {
        write_atomic(&self.path, layout)?;
        self.last_saved = Some(layout.clone());
        Ok(())
    }

    pub fn save_if_changed(&mut self, layout: &WorkspaceLayout) -> Result<bool, LayoutStoreError> {
        if self.last_saved.as_ref() == Some(layout) {
            return Ok(false);
        }
        write_atomic(&self.path, layout)?;
        self.last_saved = Some(layout.clone());
        Ok(true)
    }
}

fn write_atomic(path: &Path, layout: &WorkspaceLayout) -> Result<(), LayoutStoreError> {
    let text = toml::to_string_pretty(layout)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| LayoutStoreError::Io {
        operation: "create state directory for",
        path: parent.to_owned(),
        source,
    })?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("layout.toml");
    let temporary = parent.join(format!("{name}.{}.tmp", process::id()));
    if let Err(source) = fs::write(&temporary, text) {
        let _ = fs::remove_file(&temporary);
        return Err(LayoutStoreError::Io {
            operation: "write temporary",
            path: temporary,
            source,
        });
    }
    if let Err(source) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(LayoutStoreError::Io {
            operation: "replace",
            path: path.to_owned(),
            source,
        });
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn state_directory_prefers_absolute_local_app_data_and_falls_back_to_profile() {
        let local = PathBuf::from(r"C:\Users\test\AppData\Local");
        let profile = PathBuf::from(r"C:\Users\test");
        assert_eq!(
            windows_state_directory(Some(local.clone()), Some(profile.clone())).unwrap(),
            local
        );
        for local in [None, Some(PathBuf::from("relative"))] {
            assert_eq!(
                windows_state_directory(local, Some(profile.clone())).unwrap(),
                profile.join("AppData/Local")
            );
        }
        assert!(matches!(
            windows_state_directory(None, Some(PathBuf::from("relative"))),
            Err(LayoutStoreError::NoStateDirectory)
        ));
        assert!(matches!(
            windows_state_directory(None, None),
            Err(LayoutStoreError::NoStateDirectory)
        ));
    }
}
