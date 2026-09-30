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
        source: toml::de::Error,
    },
    #[error("cannot encode layout: {0}")]
    Encode(#[from] toml::ser::Error),
    #[error("no absolute XDG_STATE_HOME or HOME directory is available")]
    NoStateDirectory,
}

pub fn default_path() -> Result<PathBuf, LayoutStoreError> {
    if let Some(path) = env::var_os("XDG_STATE_HOME").map(PathBuf::from) {
        if path.is_absolute() {
            return Ok(path.join("multiplexer/layout.toml"));
        }
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(LayoutStoreError::NoStateDirectory)?;
    Ok(home.join(".local/state/multiplexer/layout.toml"))
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
            source,
        })
}

pub struct LayoutStore {
    path: PathBuf,
    last_attempted: WorkspaceLayout,
}

impl LayoutStore {
    pub fn new(path: PathBuf, initial: WorkspaceLayout) -> Self {
        Self {
            path,
            last_attempted: initial,
        }
    }

    pub fn save_initial(&mut self, layout: &WorkspaceLayout) -> Result<(), LayoutStoreError> {
        self.last_attempted = layout.clone();
        write_atomic(&self.path, layout)
    }

    pub fn save_if_changed(&mut self, layout: &WorkspaceLayout) -> Result<bool, LayoutStoreError> {
        if self.last_attempted == *layout {
            return Ok(false);
        }
        self.last_attempted = layout.clone();
        write_atomic(&self.path, layout)?;
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
