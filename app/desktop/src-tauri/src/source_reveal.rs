//! Identity-bound source-file reveal for the desktop shell.
//!
//! The webview supplies only an asset id. The shell obtains the current project
//! snapshot from cutd, resolves that id in the authoritative asset registry,
//! and only then asks the platform file manager to reveal the file. No renderer
//! path, URL, receipt, or arbitrary caller-supplied location is accepted.

use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SourceRevealReply {
    Revealed { message: String },
    Refused { message: String },
}

impl SourceRevealReply {
    pub fn refused(message: impl Into<String>) -> Self {
        Self::Refused {
            message: message.into(),
        }
    }
}

/// Resolve only an exact entry from a `project.state` result. This intentionally
/// has no path parameter: the source path is server-owned project data, never a
/// renderer authority.
pub fn registered_source_path(
    project: &Value,
    asset_id: &str,
) -> Result<PathBuf, SourceRevealReply> {
    if asset_id.trim().is_empty() {
        return Err(SourceRevealReply::refused(
            "Choose a registered source before revealing its file",
        ));
    }
    let path = project
        .get("assets")
        .and_then(Value::as_object)
        .and_then(|assets| assets.get(asset_id))
        .and_then(|asset| asset.get("path"))
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| {
            SourceRevealReply::refused("This source is no longer registered in the open project")
        })?;
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(SourceRevealReply::refused(
            "The registered source does not have a local file location to reveal",
        ));
    }
    Ok(path)
}

fn require_regular_file(path: &Path) -> Result<(), SourceRevealReply> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        _ => Err(SourceRevealReply::refused(
            "The registered source file is offline or is not a local file",
        )),
    }
}

/// Ask the platform's file manager to show the resolved file. A successful
/// spawn means the OS accepted the hand-off, not that every desktop environment
/// provides an observable selected-row state (Linux file managers vary there).
pub fn reveal_registered_source(project: &Value, asset_id: &str) -> SourceRevealReply {
    let path = match registered_source_path(project, asset_id) {
        Ok(path) => path,
        Err(reply) => return reply,
    };
    if let Err(reply) = require_regular_file(&path) {
        return reply;
    }

    let result = reveal_in_file_manager(&path);
    match result {
        Ok(message) => SourceRevealReply::Revealed {
            message: message.to_string(),
        },
        Err(()) => SourceRevealReply::refused(
            "The desktop file manager could not reveal the registered source",
        ),
    }
}

#[cfg(windows)]
fn reveal_in_file_manager(path: &Path) -> Result<&'static str, ()> {
    Command::new("explorer.exe")
        .args(windows_reveal_args(path))
        .spawn()
        .map(|_| "Revealed the registered source file in File Explorer")
        .map_err(|_| ())
}

/// Keep Explorer's exact two-argument `/select, <path>` contract inspectable
/// in source tests even when those tests run on a non-Windows build host.
#[cfg(any(windows, test))]
fn windows_reveal_args(path: &Path) -> Vec<std::ffi::OsString> {
    vec![
        std::ffi::OsString::from("/select,"),
        path.as_os_str().to_owned(),
    ]
}

#[cfg(target_os = "macos")]
fn reveal_in_file_manager(path: &Path) -> Result<&'static str, ()> {
    Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
        .map(|_| "Revealed the registered source file in Finder")
        .map_err(|_| ())
}

#[cfg(target_os = "linux")]
fn reveal_in_file_manager(path: &Path) -> Result<&'static str, ()> {
    let parent = path.parent().ok_or(())?;
    Command::new("xdg-open")
        .arg(parent)
        .spawn()
        .map(|_| "Opened the registered source folder; this Linux file manager may not preselect the file")
        .map_err(|_| ())
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn reveal_in_file_manager(_path: &Path) -> Result<&'static str, ()> {
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn resolves_only_the_exact_registered_asset_and_refuses_missing_identity() {
        let root = std::env::temp_dir().join(format!(
            "shellx-cut-source-reveal-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("registered.mp4");
        fs::write(&source, b"fixture").unwrap();
        let project = serde_json::json!({
            "assets": {
                "asset-registered": { "path": source },
                "asset-folder": { "path": root }
            }
        });

        assert_eq!(
            registered_source_path(&project, "asset-registered").unwrap(),
            source,
            "only the server-returned asset entry supplies a path",
        );
        assert_eq!(
            reveal_registered_source(&project, "missing"),
            SourceRevealReply::refused("This source is no longer registered in the open project"),
            "an arbitrary caller identity never becomes a path",
        );
        assert_eq!(
            reveal_registered_source(&project, "asset-folder"),
            SourceRevealReply::refused(
                "The registered source file is offline or is not a local file"
            ),
            "registered directories are not passed to a file-manager reveal",
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reply_serializes_as_the_documented_tauri_invoke_shape() {
        assert_eq!(
            serde_json::to_value(SourceRevealReply::Revealed {
                message: "Revealed the registered source file in File Explorer".to_string(),
            })
            .unwrap(),
            serde_json::json!({
                "status": "revealed",
                "message": "Revealed the registered source file in File Explorer",
            }),
        );
        assert_eq!(
            serde_json::to_value(SourceRevealReply::refused("offline")).unwrap(),
            serde_json::json!({ "status": "refused", "message": "offline" }),
        );
    }

    #[test]
    fn explorer_uses_the_select_comma_argument_before_the_registered_file() {
        let path = Path::new("C:\\Cut Fixtures\\registered.mp4");
        let args = windows_reveal_args(path);
        assert_eq!(
            args,
            vec![
                std::ffi::OsString::from("/select,"),
                path.as_os_str().to_owned(),
            ]
        );
    }
}
