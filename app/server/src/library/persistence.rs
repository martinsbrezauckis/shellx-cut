//! Complete Library index replacement and preservation of damaged state.

use super::{io_err, LibraryManifest};
use cut_core::{error_codes, CutError};
use std::io::Write;
use std::path::Path;

pub(super) fn load_at(path: &Path) -> Result<LibraryManifest, CutError> {
    load_at_with(path, crate::replace_temporary::read_optional_text)
}

fn load_at_with(
    path: &Path,
    read: impl FnOnce(&Path) -> std::io::Result<Option<String>>,
) -> Result<LibraryManifest, CutError> {
    let Some(text) = read(path).map_err(|error| io_err("read library manifest", error))? else {
        return Ok(LibraryManifest::default());
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
    crate::replace_temporary::replace_temporary(file, path)
        .map_err(|error| io_err("replace library manifest", error))?;
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
    fn concurrent_reader_never_observes_empty_or_partial_manifest_during_save() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Barrier,
        };
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("library.json");
        save_at(&path, &named_library("Before")).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let started = Arc::new(Barrier::new(2));
        let reader = {
            let path = path.clone();
            let done = done.clone();
            let started = started.clone();
            std::thread::spawn(move || {
                let mut reads = 0;
                started.wait();
                loop {
                    let manifest = load_at(&path).unwrap();
                    assert_eq!(manifest.items.len(), 1);
                    assert_eq!(manifest.items[0].name, "demo.mp4");
                    reads += 1;
                    if done.load(Ordering::Acquire) {
                        return reads;
                    }
                }
            })
        };
        started.wait();
        let result =
            (0..600).try_for_each(|tick| save_at(&path, &named_library(&format!("Folder {tick}"))));
        done.store(true, Ordering::Release);
        assert!(reader.join().unwrap() > 0);
        result.unwrap();
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

    #[cfg(windows)]
    #[test]
    fn transient_sharing_violation_retries_and_loads_manifest() {
        use std::os::windows::fs::OpenOptionsExt;
        use std::sync::{Arc, Barrier};

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("library.json");
        let expected = named_library("Keep");
        save_at(&path, &expected).unwrap();

        let locked = Arc::new(Barrier::new(2));
        let holder = {
            let path = path.clone();
            let locked = Arc::clone(&locked);
            std::thread::spawn(move || {
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(&path)
                    .unwrap();
                locked.wait();
                std::thread::sleep(std::time::Duration::from_millis(60));
                drop(file);
            })
        };

        locked.wait();
        let loaded = load_at(&path).expect("transient sharing violation must retry and succeed");
        holder.join().unwrap();
        assert_eq!(loaded, expected);
    }

    #[test]
    fn windows_transient_read_errors_recover_and_exhausted_refusals_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("library.json");
        let expected = named_library("Keep");
        let json = serde_json::to_string_pretty(&expected).unwrap();

        for make_err in [
            || std::io::Error::new(std::io::ErrorKind::NotFound, "replacement gap"),
            || std::io::Error::from_raw_os_error(5),
            || std::io::Error::from_raw_os_error(32),
        ] {
            let mut attempts = 0;
            let loaded = load_at_with(&path, |p| {
                crate::replace_temporary::read_optional_text_with(
                    p,
                    true,
                    |_| {
                        attempts += 1;
                        if attempts <= 2 {
                            Err(make_err())
                        } else {
                            Ok(json.clone())
                        }
                    },
                    |_| {},
                )
            })
            .unwrap();
            assert_eq!(loaded, expected);
            assert_eq!(attempts, 3);
        }

        let refused = load_at_with(&path, |p| {
            crate::replace_temporary::read_optional_text_with(
                p,
                true,
                |_| Err(std::io::Error::from_raw_os_error(5)),
                |_| {},
            )
        });
        assert!(
            refused.is_err(),
            "exhausted access refusal must return error, not empty LibraryManifest"
        );
    }
}
