//! Where a title or shape movie of THIS project may be read and written.
//!
//! A project can come from someone else, so an asset's recorded `path` is
//! untrusted text. Cut draws a missing title again automatically (at open and
//! before any exact frame, render or export), so the place it writes must never
//! come from that text: it is always `<project>/titles/<name>`, built from the
//! project folder the store opened, where `<name>` is one plain file name of
//! the form Cut gives its titles. The recorded path only decides whether the
//! asset is one of those movies:
//! - relative: exactly `titles/<name>`;
//! - absolute: its folder, resolved, is the project's resolved `titles` folder
//!   (a linked folder resolves elsewhere and is not ours); when the folder is
//!   gone the two are compared as written (on Windows the `\\?\` spelling and
//!   letter case do not matter; elsewhere a backslash is an ordinary character);
//! - any `..` or `.` component, another folder, or another name: not ours
//!   (`Elsewhere`), never written to.
//!
//! Before writing, [`writable_titles_folder`] checks again that the real
//! `titles` folder is a plain folder inside the real project folder (a link
//! or junction there is refused); the movie itself is written by
//! `cut_media::render::encode_title_overlay`, which refuses a link or second
//! name at the destination and publishes through a staged temporary file.

use std::path::{Component, Path, PathBuf};

/// What an asset's recorded path is, for the title rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TitleMovie {
    /// One of this project's title movies, at this (project-built) path.
    Ours(PathBuf),
    /// Named like a title movie but recorded somewhere else: never written.
    Elsewhere,
    /// Not a title movie (footage, images, sound).
    NotATitle,
}

/// The file name Cut gives a title movie: `title_<letters, digits, _ or ->.mov`.
fn is_title_name(name: &str) -> bool {
    name.strip_prefix("title_")
        .and_then(|rest| rest.strip_suffix(".mov"))
        .is_some_and(|middle| {
            !middle.is_empty()
                && middle
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

pub(crate) fn title_movie(project_dir: &Path, asset_path: &str) -> TitleMovie {
    let path = Path::new(asset_path);
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return TitleMovie::NotATitle;
    };
    if !is_title_name(name) {
        return TitleMovie::NotATitle;
    }
    let Some(parent) = path.parent() else {
        return TitleMovie::Elsewhere;
    };
    if parent
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return TitleMovie::Elsewhere;
    }
    let titles = project_dir.join("titles");
    let ours = if path.is_relative() {
        parent == Path::new("titles")
    } else {
        // Both sides resolved when they exist (macOS /var -> /private/var, the
        // Windows \\?\ form; a linked titles folder resolves elsewhere and is
        // not ours); spelled alike otherwise (the titles folder is gone).
        match (parent.canonicalize(), project_dir.canonicalize()) {
            (Ok(real_parent), Ok(real_dir)) => real_parent == real_dir.join("titles"),
            _ => same_folder(parent, &titles),
        }
    };
    if ours {
        TitleMovie::Ours(titles.join(name))
    } else {
        TitleMovie::Elsewhere
    }
}

#[cfg(not(windows))]
fn same_folder(a: &Path, b: &Path) -> bool {
    a == b
}

#[cfg(windows)]
fn same_folder(a: &Path, b: &Path) -> bool {
    windows_spelling(a) == windows_spelling(b)
}

/// One spelling of a Windows path: `\\?\` and `\\?\UNC\` removed, one kind of
/// slash, no trailing slash, lower case (Windows paths ignore case).
#[cfg(windows)]
fn windows_spelling(path: &Path) -> String {
    let mut text = path.to_string_lossy().replace('/', "\\");
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        text = format!(r"\\{rest}");
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        text = rest.to_string();
    }
    while text.len() > 3 && text.ends_with('\\') {
        text.pop();
    }
    text.to_lowercase()
}

/// The real `titles` folder of the project, made if missing, when it is a
/// plain folder directly inside the real project folder; `Err` says why not.
/// Checked again right before a title movie is written.
pub(crate) fn writable_titles_folder(project_dir: &Path) -> Result<PathBuf, String> {
    let titles = project_dir.join("titles");
    match std::fs::symlink_metadata(&titles) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&titles).map_err(|e| e.to_string())?;
        }
        Err(error) => return Err(error.to_string()),
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            return Err("the project's titles folder is a link or not a folder".into());
        }
        Ok(_) => {}
    }
    let real_project = project_dir.canonicalize().map_err(|e| e.to_string())?;
    let real_titles = titles.canonicalize().map_err(|e| e.to_string())?;
    if real_titles != real_project.join("titles") {
        return Err("the project's titles folder leads outside the project".into());
    }
    Ok(titles)
}

#[cfg(test)]
#[path = "title_movie_path_tests.rs"]
mod tests;
