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

/// First line cargo writes into the `CACHEDIR.TAG` of every target folder.
const CARGO_CACHEDIR_SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";

/// Is `exe` somewhere under a cargo target folder (the folder holding cargo's own
/// `CACHEDIR.TAG`)? A target folder may sit anywhere (`CARGO_TARGET_DIR`, a shared
/// folder above the checkout) and cargo puts test binaries in `deps/` or, in the
/// newer layout, in `build/<crate>/<hash>/out/`, so every folder above is checked.
fn runs_from_cargo_target(exe: &Path) -> bool {
    exe.ancestors().skip(1).any(|dir| {
        std::fs::read(dir.join("CACHEDIR.TAG"))
            .map(|bytes| bytes.starts_with(CARGO_CACHEDIR_SIGNATURE.as_bytes()))
            .unwrap_or(false)
    })
}

/// True when `exe` is unknown, runs from inside this source workspace, or is a
/// cargo build of any target folder: the cases where compile-time checkout paths
/// describe the running program. An installed engine is none of these.
fn runs_from_workspace(exe: Option<&Path>) -> bool {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent();
    match (exe, workspace) {
        (Some(exe), Some(workspace)) => exe.starts_with(workspace) || runs_from_cargo_target(exe),
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
    fn a_source_build_with_its_target_folder_outside_the_workspace_still_reaches_the_checkout() {
        let dir = tempfile::tempdir().unwrap();
        // A shared target folder (CARGO_TARGET_DIR) above or beside the checkout, test binary
        // in the newer `build/<crate>/<hash>/out` layout.
        let target = dir.path().join("target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(
            target.join("CACHEDIR.TAG"),
            format!("{CARGO_CACHEDIR_SIGNATURE}\n# tag\n"),
        )
        .unwrap();
        for exe in [
            target.join("debug/build/server/0123abcd/out/cutd-0123abcd"),
            target.join("debug/deps/cutd-0123abcd"),
            target.join("x86_64-pc-windows-msvc/release/cutd"),
        ] {
            touch(&exe);
            let docs = agent_docs_root_for(None, Some(&exe));
            assert!(
                docs.is_some_and(|root| root.join(AGENT_DOCS_MARKER).is_file()),
                "{}",
                exe.display()
            );
            assert!(third_party_notices_for(Some(&exe)).is_some());
        }
        // The same folders without cargo's tag are an installed or copied engine: no checkout.
        let untagged = dir.path().join("plain/debug/deps/cutd");
        touch(&untagged);
        assert_eq!(agent_docs_root_for(None, Some(&untagged)), None);
        assert_eq!(third_party_notices_for(Some(&untagged)), None);
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
