//! packaged_resources.rs — where the engine finds the resources a package ships
//! (agent documents, the UI bundle), from its own executable location.
//!
//! The desktop shell names these folders in environment variables when it
//! starts the engine, but an engine started by hand (an agent running
//! `/usr/bin/cutd serve --headless` for MCP is a main use of the product) has
//! no such variables and must find the same files. The layout rule is the one
//! the captions payload uses (`cut_perception::sidecar::packaged_resource_dirs_for_exe`):
//!   - Linux DEB/RPM: `/usr/bin/cutd` + `/usr/lib/ShellX Cut/<name>`
//!   - Windows install folder: `<install>\cutd.exe` + `<install>\<name>`
//!   - macOS bundle: `Contents/MacOS/cutd` + `Contents/Resources/<name>`
//! An environment variable stays as an explicit override. The source checkout
//! (a compile-time path) is used only by an engine that itself runs from inside
//! the build workspace, never by an installed one.

use cut_perception::sidecar::{packaged_resource_dir_for_exe, packaged_resource_dirs_for_exe};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub(crate) const ENV_AGENT_DOCS_DIR: &str = "SHELLX_CUT_AGENT_DOCS_DIR";
const AGENT_DOCS_MARKER: &str = "skill/shellx-cut/SKILL.md";
const THIRD_PARTY_NOTICES: &str = "THIRD-PARTY-NOTICES.txt";

/// True when `exe` is unknown or runs from inside this source workspace, the
/// only case where compile-time checkout paths describe the running program.
fn runs_from_workspace(exe: Option<&Path>) -> bool {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent();
    match (exe, workspace) {
        (Some(exe), Some(workspace)) => exe.starts_with(workspace),
        _ => true,
    }
}

/// The agent documents root: the override, else the packaged `agent-docs`, else
/// (source builds only) the checkout.
pub(crate) fn agent_docs_root_for(
    env_dir: Option<OsString>,
    exe: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(dir) = env_dir.filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) =
        exe.and_then(|exe| packaged_resource_dir_for_exe(exe, "agent-docs", AGENT_DOCS_MARKER))
    {
        return Some(dir);
    }
    if !runs_from_workspace(exe) {
        return None;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dev = manifest.join("../../agent-docs");
    if dev.join(AGENT_DOCS_MARKER).is_file() {
        return Some(dev.components().collect());
    }
    Some(manifest.join("../..").components().collect())
}

pub(crate) fn agent_docs_root() -> Option<PathBuf> {
    agent_docs_root_for(
        std::env::var_os(ENV_AGENT_DOCS_DIR),
        std::env::current_exe().ok().as_deref(),
    )
}

/// Locate ui/dist: explicit flag wins; else the package's own `ui-dist` beside
/// the engine; else <repo>/ui/dist relative to the workspace (CARGO_MANIFEST_DIR
/// = app/server at compile time, a source build).
pub(crate) fn resolve_ui_dist(explicit: Option<PathBuf>) -> PathBuf {
    explicit.unwrap_or_else(|| ui_dist_for(std::env::current_exe().ok().as_deref()))
}

/// The UI bundle for `cutd serve` without `--ui-dist`: the packaged `ui-dist`,
/// else the checkout's `ui/dist` (a source build).
pub(crate) fn ui_dist_for(exe: Option<&Path>) -> PathBuf {
    if let Some(dir) =
        exe.and_then(|exe| packaged_resource_dir_for_exe(exe, "ui-dist", "index.html"))
    {
        return dir;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../ui/dist")
        .components()
        .collect() // normalize the ../.. without touching the filesystem
}

/// The generated third-party notices file the installers ship next to `NOTICE`: the packaged one, else (an engine
/// that runs from inside the build workspace) the checkout's copy.
pub(crate) fn third_party_notices_for(exe: Option<&Path>) -> Option<PathBuf> {
    if let Some(file) = exe
        .map(|exe| packaged_resource_dirs_for_exe(exe, THIRD_PARTY_NOTICES))
        .and_then(|candidates| candidates.into_iter().find(|file| file.is_file()))
    {
        return Some(file);
    }
    if !runs_from_workspace(exe) {
        return None;
    }
    let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(THIRD_PARTY_NOTICES)
        .components()
        .collect::<PathBuf>();
    dev.is_file().then_some(dev)
}

pub(crate) fn third_party_notices() -> Option<PathBuf> {
    third_party_notices_for(std::env::current_exe().ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    /// (layout name, engine exe, where the resource folder `name` sits)
    fn layouts(root: &Path, name: &str) -> Vec<(&'static str, PathBuf, PathBuf)> {
        vec![
            (
                "Linux package",
                root.join("linux/usr/bin/cutd"),
                root.join("linux/usr/lib/ShellX Cut").join(name),
            ),
            (
                "Windows install folder",
                root.join("nsis/ShellX Cut/cutd.exe"),
                root.join("nsis/ShellX Cut").join(name),
            ),
            (
                "macOS bundle",
                root.join("mac/ShellX Cut.app/Contents/MacOS/cutd"),
                root.join("mac/ShellX Cut.app/Contents/Resources")
                    .join(name),
            ),
        ]
    }

    #[test]
    fn a_hand_started_engine_finds_the_agent_docs_on_every_packaged_layout() {
        let dir = tempfile::tempdir().unwrap();
        for (name, exe, docs) in layouts(dir.path(), "agent-docs") {
            touch(&exe);
            touch(&docs.join(AGENT_DOCS_MARKER));
            assert_eq!(agent_docs_root_for(None, Some(&exe)), Some(docs), "{name}");
        }
    }

    #[test]
    fn the_environment_variable_still_overrides_the_packaged_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (_, exe, docs) = layouts(dir.path(), "agent-docs").remove(0);
        touch(&exe);
        touch(&docs.join(AGENT_DOCS_MARKER));
        let chosen = agent_docs_root_for(Some(OsString::from("/elsewhere/docs")), Some(&exe));
        assert_eq!(chosen, Some(PathBuf::from("/elsewhere/docs")));
        // An empty value is no override.
        assert_eq!(
            agent_docs_root_for(Some(OsString::new()), Some(&exe)),
            Some(docs)
        );
    }

    #[test]
    fn an_installed_engine_without_docs_does_not_fall_back_to_a_build_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("usr/bin/cutd");
        touch(&exe);
        assert_eq!(agent_docs_root_for(None, Some(&exe)), None);
        // The engine of a source build still reaches the checkout.
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let dev_exe = manifest.join("../../app/target/debug/cutd");
        assert!(agent_docs_root_for(None, Some(&dev_exe)).is_some());
    }

    #[test]
    fn the_third_party_notices_are_found_on_every_packaged_layout_and_not_invented_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        for (name, exe, file) in layouts(dir.path(), THIRD_PARTY_NOTICES) {
            touch(&exe);
            touch(&file);
            assert_eq!(third_party_notices_for(Some(&exe)), Some(file), "{name}");
        }
        // An installed engine without the file does not read a build checkout's copy.
        let bare = dir.path().join("bare/usr/bin/cutd");
        touch(&bare);
        assert_eq!(third_party_notices_for(Some(&bare)), None);
    }

    #[test]
    fn the_ui_bundle_is_found_on_every_packaged_layout_and_falls_back_to_the_checkout() {
        let dir = tempfile::tempdir().unwrap();
        for (name, exe, ui) in layouts(dir.path(), "ui-dist") {
            touch(&exe);
            touch(&ui.join("index.html"));
            assert_eq!(ui_dist_for(Some(&exe)), ui, "{name}");
        }
        let stray = dir.path().join("downloads/cutd");
        touch(&stray);
        assert!(ui_dist_for(Some(&stray)).ends_with("ui/dist"));
    }
}
