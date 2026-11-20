use super::*;
use std::path::Path;

fn abs(name: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!(r"C:\{name}"))
    } else {
        PathBuf::from(format!("/{name}"))
    }
}

#[test]
fn relative_and_empty_path_entries_are_never_searched() {
    let path = std::env::join_paths([
        PathBuf::from("."),
        abs("usr-bin"),
        PathBuf::from(""),
        PathBuf::from("bin"),
        abs("bin"),
    ])
    .unwrap();
    assert_eq!(
        absolute_entries(Some(&path)),
        vec![abs("usr-bin"), abs("bin")]
    );
    let child = absolute_path_value(Some(&path)).unwrap();
    assert_eq!(
        std::env::split_paths(&child).collect::<Vec<_>>(),
        vec![abs("usr-bin"), abs("bin")]
    );
    let only_relative = std::env::join_paths([".", "bin"]).unwrap();
    assert_eq!(absolute_path_value(Some(&only_relative)), None);
    assert!(absolute_entries(None).is_empty());
}

#[test]
fn a_tool_is_found_only_by_its_full_path_in_an_absolute_folder() {
    let dir = tempfile::tempdir().unwrap();
    let tool = dir.path().join(super::super::exe_name("ffmpeg"));
    std::fs::write(&tool, b"x").unwrap();
    let found = find_in(&[dir.path().to_path_buf()], "ffmpeg").unwrap();
    assert!(found.is_absolute());
    assert_eq!(found, tool);
    assert_eq!(find_in(&[dir.path().join("empty")], "ffmpeg"), None);
}

#[test]
fn only_absolute_settings_count() {
    assert_eq!(absolute_setting(None), None);
    assert_eq!(absolute_setting(Some(OsString::new())), None);
    assert_eq!(absolute_setting(Some(OsString::from("./ffmpeg"))), None);
    assert_eq!(absolute_setting(Some(OsString::from("ffmpeg"))), None);
    let full = abs("tools").join("ffmpeg");
    assert_eq!(
        absolute_setting(Some(full.clone().into_os_string())),
        Some(full)
    );
}

#[test]
fn a_missing_tool_resolves_to_an_absolute_place_that_does_not_exist() {
    for stem in ["ffmpeg", "ffprobe"] {
        let place = not_installed(stem);
        assert!(place.is_absolute(), "{}", place.display());
        assert_eq!(place.file_stem().unwrap(), stem);
        assert_ne!(place.as_path(), Path::new(stem));
    }
}
