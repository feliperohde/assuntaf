//! Moves user data from the previous app identifier (com.meetily.ai) to the
//! current one after the rename to Assunta. Tauri derives the app data folder
//! from the bundle identifier, so without this the renamed app would start empty.

use std::fs;
use std::io;
use std::path::Path;
use tauri::{AppHandle, Manager, Runtime};

pub const LEGACY_IDENTIFIER: &str = "com.meetily.ai";

/// Moves every entry of `legacy` into `target` that `target` does not already
/// have (never overwrites), then removes `legacy` if it ended up empty.
/// Returns how many entries were moved.
pub fn migrate_dir(legacy: &Path, target: &Path) -> io::Result<usize> {
    if !legacy.is_dir() || legacy == target {
        return Ok(0);
    }
    fs::create_dir_all(target)?;
    let mut moved = 0;
    for entry in fs::read_dir(legacy)? {
        let entry = entry?;
        let destination = target.join(entry.file_name());
        if destination.exists() {
            continue;
        }
        match fs::rename(entry.path(), &destination) {
            Ok(()) => moved += 1,
            // Different volume: copy, then remove the original
            Err(_) => {
                copy_recursive(&entry.path(), &destination)?;
                remove_recursive(&entry.path())?;
                moved += 1;
            }
        }
    }
    if fs::read_dir(legacy)?.next().is_none() {
        let _ = fs::remove_dir(legacy);
    }
    Ok(moved)
}

fn copy_recursive(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map(|_| ())
    }
}

fn remove_recursive(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// Runs the migration for the app's data folders. Call at the very start of setup,
/// before the database or any store is opened.
pub fn migrate_legacy_app_data<R: Runtime>(app: &AppHandle<R>) {
    let paths = app.path();
    let mut targets = Vec::new();
    for dir in [paths.app_data_dir(), paths.app_local_data_dir(), paths.app_config_dir()] {
        if let Ok(dir) = dir {
            if !targets.contains(&dir) {
                targets.push(dir);
            }
        }
    }
    for target in targets {
        let Some(parent) = target.parent() else { continue };
        let legacy = parent.join(LEGACY_IDENTIFIER);
        match migrate_dir(&legacy, &target) {
            Ok(0) => {}
            Ok(moved) => log::info!(
                "Migrated {} item(s) from {} to {}",
                moved,
                legacy.display(),
                target.display()
            ),
            Err(e) => log::error!(
                "Failed to migrate data from {} to {}: {}",
                legacy.display(),
                target.display(),
                e
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_missing_entries_without_overwriting() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join(LEGACY_IDENTIFIER);
        let target = root.path().join("com.assunta.app");
        fs::create_dir_all(legacy.join("models/summary")).unwrap();
        fs::write(legacy.join("meeting_minutes.sqlite"), b"db").unwrap();
        fs::write(legacy.join("models/summary/m.gguf"), b"model").unwrap();
        fs::write(legacy.join("settings.json"), b"old").unwrap();
        // The new folder may already exist with a freshly created file
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("settings.json"), b"new").unwrap();

        let moved = migrate_dir(&legacy, &target).unwrap();

        assert_eq!(moved, 2);
        assert_eq!(fs::read(target.join("meeting_minutes.sqlite")).unwrap(), b"db");
        assert_eq!(fs::read(target.join("models/summary/m.gguf")).unwrap(), b"model");
        assert_eq!(fs::read(target.join("settings.json")).unwrap(), b"new"); // not overwritten
        // Legacy folder keeps what wasn't moved
        assert!(legacy.join("settings.json").exists());
    }

    #[test]
    fn removes_legacy_folder_when_fully_moved_and_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join(LEGACY_IDENTIFIER);
        let target = root.path().join("com.assunta.app");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("meeting_minutes.sqlite"), b"db").unwrap();

        assert_eq!(migrate_dir(&legacy, &target).unwrap(), 1);
        assert!(!legacy.exists());
        assert_eq!(migrate_dir(&legacy, &target).unwrap(), 0);
        assert!(target.join("meeting_minutes.sqlite").exists());
    }
}
