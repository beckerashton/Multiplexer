#[path = "../src/layout_persistence.rs"]
mod layout_persistence;

use std::{fs, path::PathBuf};

use layout_persistence::{LayoutStore, default_path, load};
use mux_core::{SessionSpec, Workspace, WorkspaceCommand};

#[test]
fn saved_layout_can_be_replaced_and_reloaded() {
    let directory = std::env::temp_dir().join(format!(
        "multiplexer-layout-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path = directory.join("nested/layout.toml");
    let shell = SessionSpec {
        program: PathBuf::from("test-shell"),
        args: Vec::new(),
        cwd: None,
    };
    let (mut workspace, _) = Workspace::new(shell.clone());
    let initial = workspace.layout_snapshot();
    let mut store = LayoutStore::new(path.clone());

    assert_eq!(load(&path).unwrap(), None);
    fs::create_dir_all(&path).unwrap();
    assert!(store.save_initial(&initial).is_err());
    fs::remove_dir(&path).unwrap();
    assert!(store.save_if_changed(&initial).unwrap());
    store.save_initial(&initial).unwrap();
    assert_eq!(load(&path).unwrap(), Some(initial.clone()));
    assert!(!store.save_if_changed(&initial).unwrap());

    workspace
        .execute(WorkspaceCommand::CreateTab { session: shell })
        .unwrap();
    let changed = workspace.layout_snapshot();
    // A failed replacement must remain retryable (for example, a Windows
    // application temporarily holding the destination without delete sharing).
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(store.save_if_changed(&changed).is_err());
    fs::remove_dir(&path).unwrap();
    assert!(store.save_if_changed(&changed).unwrap());
    assert_eq!(load(&path).unwrap(), Some(changed));
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn malformed_and_oversized_layout_files_are_reported() {
    let path = std::env::temp_dir().join(format!(
        "multiplexer-layout-invalid-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, "not a layout = [").unwrap();
    assert!(matches!(
        load(&path),
        Err(layout_persistence::LayoutStoreError::Parse { .. })
    ));
    fs::File::create(&path).unwrap().set_len(1_048_577).unwrap();
    assert!(matches!(
        load(&path),
        Err(layout_persistence::LayoutStoreError::TooLarge { .. })
    ));
    fs::remove_file(path).unwrap();
}

#[test]
fn default_layout_path_is_absolute() {
    assert!(default_path().unwrap().is_absolute());
}

#[cfg(windows)]
#[test]
fn default_layout_path_uses_local_app_data() {
    assert_eq!(
        default_path().unwrap(),
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("multiplexer/layout.toml")
    );
}
