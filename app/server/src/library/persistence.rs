//! Complete Library index replacement and preservation of damaged state.

use super::{io_err, LibraryManifest};
use cut_core::{error_codes, CutError};
use std::io::Write;
use std::path::Path;

pub(super) fn load_at(path: &Path) -> Result<LibraryManifest, CutError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LibraryManifest::default());
        }
        Err(error) => return Err(io_err("read library manifest", error)),
    };
    serde_json::from_str(&text).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "the saved Library index is damaged; its existing bytes were preserved",
            error.to_string(),
        )
        .with_suggested_action(
            "restore library.json from a known complete backup before changing the Library",
        )
    })
}

pub(super) fn save_at(path: &Path, manifest: &LibraryManifest) -> Result<(), CutError> {
    save_before_replace(path, manifest, || Ok(()))
}

fn save_before_replace(
    path: &Path,
    manifest: &LibraryManifest,
    before_replace: impl FnOnce() -> std::io::Result<()>,
) -> Result<(), CutError> {
    let parent = path.parent().ok_or_else(|| {
        io_err(
            "resolve library directory",
            std::io::Error::other("manifest has no parent"),
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|error| io_err("create library dir", error))?;
    let body = serde_json::to_vec_pretty(manifest)
        .map_err(|error| io_err("serialize library manifest", std::io::Error::other(error)))?;
    let mut file = tempfile::Builder::new()
        .prefix(".library-manifest-")
        .tempfile_in(parent)
        .map_err(|error| io_err("create temporary library manifest", error))?;
    file.write_all(&body)
        .map_err(|error| io_err("write library manifest", error))?;
    file.as_file()
        .sync_all()
        .map_err(|error| io_err("flush library manifest", error))?;
    before_replace().map_err(|error| io_err("publish library manifest", error))?;
    // tempfile uses a same-directory rename on Unix and MoveFileExW with
    // replacement on Windows. Never remove the old index before replacement.
    file.persist(path)
        .map_err(|error| io_err("replace library manifest", error.error))?;
    #[cfg(unix)]
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_err("flush library directory", error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named_library(name: &str) -> LibraryManifest {
        serde_json::from_value(serde_json::json!({
            "version": 1, "folders": [name],
            "items": [{"id":"a", "type":"video", "name":"demo.mp4",
                "src_path":"/media/demo.mp4", "added_ms":1, "source":"user",
                "favorite":true, "tags":["demo"]}]
        }))
        .unwrap()
    }

    #[test]
    fn missing_manifest_is_first_run_but_invalid_existing_state_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("library.json");
        assert_eq!(load_at(&path).unwrap(), LibraryManifest::default());
        for damaged in ["{\"items\": [", "null", "{\"version\": 1}"] {
            std::fs::write(&path, damaged).unwrap();
            assert!(load_at(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), damaged);
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(
            load_at(&path).is_err(),
            "I/O failures must not look like an empty Library"
        );
    }

    #[test]
    fn replacing_manifest_retains_complete_items_and_organization() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("library.json");
        save_at(&path, &named_library("Before")).unwrap();
        let next = named_library("After");
        save_at(&path, &next).unwrap();
        assert_eq!(load_at(&path).unwrap(), next);
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn interrupted_replacement_preserves_previous_index_and_cleans_temp() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("library.json");
        save_at(&path, &named_library("Before")).unwrap();
        let before = std::fs::read(&path).unwrap();
        let error = save_before_replace(&path, &named_library("After"), || {
            Err(std::io::Error::other("simulated publication failure"))
        });
        assert!(error.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(load_at(&path).unwrap(), named_library("Before"));
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }
}
