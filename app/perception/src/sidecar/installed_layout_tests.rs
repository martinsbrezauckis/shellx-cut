//! An installed app (any package layout) never runs a Python from PATH; a source
//! build or a test binary still does, after the version probe.

use super::*;

fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"# payload").unwrap();
}

/// (layout name, engine executable, packaged instruments.py)
fn layouts(root: &Path) -> Vec<(&'static str, PathBuf, PathBuf)> {
    vec![
        (
            "Linux DEB/RPM",
            root.join("deb/usr/bin/cutd"),
            root.join("deb/usr/lib/ShellX Cut/perception/instruments.py"),
        ),
        (
            "Windows install folder",
            root.join("nsis/ShellX Cut/cutd.exe"),
            root.join("nsis/ShellX Cut/perception/instruments.py"),
        ),
        (
            "macOS bundle",
            root.join("mac/ShellX Cut.app/Contents/MacOS/cutd"),
            root.join("mac/ShellX Cut.app/Contents/Resources/perception/instruments.py"),
        ),
    ]
}

#[test]
fn every_packaged_layout_is_recognised_as_installed() {
    let dir = tempfile::tempdir().unwrap();
    for (name, exe, payload) in layouts(dir.path()) {
        touch(&exe);
        touch(&payload);
        assert!(is_installed_layout(&exe), "{name} must count as installed");
    }
}

#[test]
fn a_source_build_or_a_stray_engine_is_not_installed() {
    let dir = tempfile::tempdir().unwrap();
    // A cargo target directory, a copied cutd with no payload, and a payload that
    // is only reachable by an environment variable are all "not installed".
    for exe in [
        dir.path().join("repo/app/target/debug/cutd"),
        dir.path().join("downloads/cutd"),
        dir.path().join("usr/bin/cutd"),
    ] {
        touch(&exe);
        assert!(!is_installed_layout(&exe), "{}", exe.display());
    }
    touch(&dir.path().join("elsewhere/perception/instruments.py"));
    assert!(!is_installed_layout(&dir.path().join("usr/bin/cutd")));
}

/// A working `python3` first on PATH is never executed (not even to probe it)
/// by an installed engine, whatever the layout; a source build does use it.
#[cfg(unix)]
#[test]
fn an_installed_engine_never_runs_a_python_from_path() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let witness = dir.path().join("python-was-executed");
    let bin = dir.path().join("pathbin");
    std::fs::create_dir_all(&bin).unwrap();
    let python = bin.join("python3");
    std::fs::write(
        &python,
        // `: >` is a shell builtin: PATH holds only this directory.
        format!("#!/bin/sh\n: > '{}'\necho '(3, 12)'\n", witness.display()),
    )
    .unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
    let appdata = dir.path().join("appdata");
    let managed = venv_python(&appdata);

    for (name, exe, payload) in layouts(&dir.path().join("pkg")) {
        touch(&exe);
        touch(&payload);
        let chosen = fallback_python_with_path(
            is_installed_layout(&exe),
            Some(appdata.clone()),
            bin.as_os_str(),
        );
        assert_eq!(chosen, managed, "{name}: only the managed Python");
        assert!(!witness.exists(), "{name}: the PATH python was executed");
    }
    // A source build (no payload) does use the verified PATH Python.
    let source = dir.path().join("repo/target/debug/cutd");
    touch(&source);
    let chosen =
        fallback_python_with_path(is_installed_layout(&source), Some(appdata), bin.as_os_str());
    assert_eq!(chosen, python);
    assert!(witness.exists());
}

/// A hand-started engine (no SHELLX_CUT_SIDECAR_DIR) finds the captions payload
/// on every packaged layout through the same rule as `is_installed_layout`.
#[test]
fn the_sidecar_ladder_reaches_the_packaged_payload_on_every_layout_without_the_environment() {
    let dir = tempfile::tempdir().unwrap();
    let appdata = dir.path().join("appdata");
    for (name, exe, payload) in layouts(dir.path()) {
        touch(&exe);
        touch(&payload);
        let payload_dir = payload.parent().unwrap().to_path_buf();
        let dirs = sidecar_base_dirs_with(None, Some(&exe), Some(appdata.clone()));
        let found = dirs
            .iter()
            .find(|base| base.join("instruments.py").is_file());
        assert_eq!(found, Some(&payload_dir), "{name}: {dirs:?}");
        assert_eq!(
            packaged_resource_dir_for_exe(&exe, "perception", "instruments.py"),
            Some(payload_dir),
            "{name}"
        );
    }
}

/// The override stays first, and other resource folders use the same layouts.
#[test]
fn the_environment_override_stays_first_and_other_resources_share_the_layout_rule() {
    let dir = tempfile::tempdir().unwrap();
    let (_, exe, payload) = layouts(dir.path()).remove(0);
    touch(&exe);
    touch(&payload);
    let over = dir.path().join("override");
    let dirs = sidecar_base_dirs_with(Some(over.clone()), Some(&exe), None);
    assert_eq!(dirs.first(), Some(&over));
    let docs = packaged_resource_dirs_for_exe(&exe, "agent-docs");
    assert!(docs.contains(&dir.path().join("deb/usr/lib/ShellX Cut/agent-docs")));
    assert!(packaged_resource_dir_for_exe(&exe, "agent-docs", "SKILL.md").is_none());
}

fn tag(root: &Path, text: &str) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("CACHEDIR.TAG"), text).unwrap();
}

const CARGO_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n# This file is a cache directory tag created by cargo.\n";

/// Tauri copies `perception/` next to a cargo-built engine; that engine is a
/// source build and keeps using a PATH Python.
#[test]
fn an_engine_in_a_cargo_target_folder_is_a_source_build_even_with_the_payload_beside_it() {
    let dir = tempfile::tempdir().unwrap();
    for (name, rel) in [
        ("debug", "shared/target/debug/cutd"),
        ("release", "shared/target/release/cutd"),
        (
            "triple",
            "shared/target/x86_64-unknown-linux-gnu/release/cutd",
        ),
    ] {
        let exe = dir.path().join(rel);
        touch(&exe);
        touch(&exe.parent().unwrap().join("perception/instruments.py"));
        tag(&dir.path().join("shared/target"), CARGO_TAG);
        assert!(!is_installed_layout(&exe), "{name}: {}", exe.display());
    }
}

/// The marker cannot switch a real install: the executable must itself sit in a
/// `debug`/`release` folder, and the tag must be cargo's.
#[test]
fn a_planted_tag_does_not_turn_an_installed_engine_into_a_source_build() {
    let dir = tempfile::tempdir().unwrap();
    for (name, exe, payload) in layouts(dir.path()) {
        touch(&exe);
        touch(&payload);
        // Tags planted in the install folder and above it.
        for ancestor in exe.ancestors().skip(1).take(4) {
            if ancestor.starts_with(dir.path()) && ancestor != dir.path() {
                tag(ancestor, CARGO_TAG);
            }
        }
        assert!(is_installed_layout(&exe), "{name} must stay installed");
    }
}

/// Without cargo's tag (or with another one) a folder named release is an install.
#[test]
fn a_folder_named_release_without_cargos_tag_is_an_install() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("apps/release/cutd");
    touch(&exe);
    touch(&dir.path().join("apps/release/perception/instruments.py"));
    assert!(is_installed_layout(&exe));
    tag(&dir.path().join("apps"), "Signature: not-cargo\n");
    assert!(is_installed_layout(&exe));
    tag(&dir.path().join("apps"), CARGO_TAG);
    assert!(!is_installed_layout(&exe));
}
