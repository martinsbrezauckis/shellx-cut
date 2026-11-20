//! program_path.rs — external programs are only ever found in ABSOLUTE
//! folders.
//!
//! A relative or empty `PATH` entry (`.`, `bin`, `a::b`) is resolved against
//! the current folder, and an engine started by hand may be running inside a
//! received project: searching such an entry would run a program planted
//! there. So: a tool is looked up only in the absolute entries of `PATH`; an
//! explicit path setting counts only when it is absolute; a tool that is not
//! found is never started by its bare name; and the engine's own `PATH` (which
//! every child inherits, e.g. `tar` looking for `xz`) keeps only its absolute
//! entries.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// The absolute entries of a `PATH` value, in order. Relative and empty
/// entries are dropped.
pub fn absolute_entries(path: Option<&OsStr>) -> Vec<PathBuf> {
    path.map(|p| {
        std::env::split_paths(p)
            .filter(|dir| dir.is_absolute())
            .collect()
    })
    .unwrap_or_default()
}

/// A `PATH` value of only the absolute entries of `path` (None when no
/// absolute entry remains) — the `PATH` to give a child process.
pub fn absolute_path_value(path: Option<&OsStr>) -> Option<OsString> {
    let entries = absolute_entries(path);
    if entries.is_empty() {
        return None;
    }
    std::env::join_paths(entries).ok()
}

/// `PATH` for a child of this process: only the absolute entries of ours.
pub fn child_path_value() -> Option<OsString> {
    absolute_path_value(std::env::var_os("PATH").as_deref())
}

/// The program `stem` (platform executable name) in the first absolute `PATH`
/// entry that holds it.
pub fn find_on_path(stem: &str) -> Option<PathBuf> {
    find_in(&absolute_entries(std::env::var_os("PATH").as_deref()), stem)
}

/// The program `stem` in the first of `dirs` that holds it.
pub fn find_in(dirs: &[PathBuf], stem: &str) -> Option<PathBuf> {
    let want = super::exe_name(stem);
    dirs.iter()
        .map(|dir| dir.join(&want))
        .find(|candidate| candidate.is_file())
}

/// An explicit path setting (an environment variable, a saved choice) is used
/// only when it is absolute; a relative value would be read against the
/// current folder.
pub fn absolute_setting(value: Option<OsString>) -> Option<PathBuf> {
    value
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// The absolute place a tool would be when it is installed nowhere Cut looks.
/// It does not exist, so starting it fails with "not found" and the caller's
/// plain "not installed" refusal follows — a tool is never started by its bare
/// name. It is where the built-in download installs (the app-data tools
/// folder), else beside this program.
pub fn not_installed(stem: &str) -> PathBuf {
    let exe = super::exe_name(stem);
    if let Some(tools) = super::appdata_tools_dir() {
        return tools.join(stem).join("bin").join(exe);
    }
    if let Some(dir) = super::exe_dir() {
        return dir.join(stem).join(exe);
    }
    if cfg!(windows) {
        PathBuf::from(r"C:\shellx-cut-tool-not-installed").join(exe)
    } else {
        PathBuf::from("/shellx-cut-tool-not-installed").join(exe)
    }
}

/// Make this process's `PATH` absolute-only, and on Windows keep the current
/// folder out of every child's program search. Call first thing in `main`,
/// before any thread exists: every later program start of the engine, and every
/// program its children start by name, then searches only absolute folders.
pub fn harden_process_search_path() {
    let path = std::env::var_os("PATH");
    if let Some(original) = path.as_deref() {
        match absolute_path_value(Some(original)) {
            Some(clean) if clean.as_os_str() != original => std::env::set_var("PATH", clean),
            Some(_) => {}
            None => std::env::remove_var("PATH"),
        }
    }
    if cfg!(windows) {
        // CreateProcess, cmd.exe and the search functions skip the current
        // folder for a bare program name when this is set.
        std::env::set_var("NoDefaultCurrentDirectoryInExePath", "1");
    }
}

#[cfg(test)]
#[path = "program_path_tests.rs"]
mod tests;
