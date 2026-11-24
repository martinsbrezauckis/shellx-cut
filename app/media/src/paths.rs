//! paths.rs — output-path fencing (public verb contract SECURITY the output-fencing contract).
//!
//! Role: the gate every file-WRITING media function passes its output path
//! through. Without this the REST/MCP surface is an arbitrary-file-overwrite
//! primitive (same bug class as the shellX fs-denylist finding).
//! Policy, exactly per the output-fencing contract:
//!   1. canonicalize (resolves `.`/`..`/symlinked ancestors against the fs);
//!   2. refuse path traversal (`..` components) BEFORE touching the fs;
//!   3. refuse symlink, reparse and hardlink targets before direct writes;
//!   4. refuse writes outside the project dir / configured outputs dirs;
//!   5. refuse overwriting an EXISTING file whose suffix is not one of the
//!      media/sidecar suffixes ShellX Cut writes through export/render paths.
//! Dependencies: std only. Primary callers: render::render_final,
//! export::export_xml (and any future export.* writer).

use cut_core::{error_codes, CutError};
use std::path::{Component, Path, PathBuf};

/// File suffixes a fenced write may OVERWRITE (lowercase, no dot). Creating a
/// new file is allowed with any suffix as long as it is inside the fence;
/// overwriting is restricted so a verb can never clobber project.json,
/// ops.jsonl, shell rc files, etc. This list must cover every user-facing
/// export/render suffix so a confirmed Save As overwrite works like a normal
/// desktop app while non-export files remain protected.
///
/// Audio render outputs (.mp3/.m4a/.wav/.flac/.opus/.aac/.ogg) are equally
/// regenerable and MUST be overwritable: the live preview audio MONITOR
/// re-renders the timeline mix to a fixed `exports/audio.mp3` on every play
/// after an edit (Preview/index.tsx ensureMix → export.audio). Without these,
/// the second monitor render onward fails ("refusing to overwrite … audio.mp3")
/// and the in-app preview goes SILENT while the real render still has sound —
/// These are the formats audio_format_args
/// emits, plus aac/ogg for completeness; none names a config/system file.
pub const OVERWRITABLE_SUFFIXES: &[&str] = &[
    "mp4", "mov", "webm", "mkv", "gif", "srt", "vtt", "xml", "fcpxml", "otio", "edl", "ass", "jpg",
    "jpeg", "png", "mp3", "m4a", "wav", "flac", "opus", "aac", "ogg",
];

/// Text outputs are only overwrite-safe for the generated export filenames Cut
/// owns. Generic .txt/.md files can be user notes and must not be clobbered.
pub const OVERWRITABLE_TEXT_FILENAMES: &[&str] =
    &["transcript.txt", "transcript.md", "chapters.txt"];

/// The fence: the set of directory roots a media write is allowed to land in.
/// Constructed once per project by the server (project dir + optional
/// explicitly configured outputs dir) and passed to every writing function.
#[derive(Debug, Clone)]
pub struct PathFence {
    /// Canonicalized project dir (`<name>.cutproj`). Also used by the
    /// renderer to resolve project-relative asset paths.
    project_dir: PathBuf,
    /// Additional canonicalized roots writes may target (e.g. a user-chosen
    /// exports directory). Empty by default.
    extra_roots: Vec<PathBuf>,
}

impl PathFence {
    /// Build a fence rooted at `project_dir`. Errors if the dir does not
    /// exist — a fence over a nonexistent root would canonicalize to nothing.
    pub fn new(project_dir: &Path) -> Result<Self, CutError> {
        let project_dir = project_dir.canonicalize().map_err(|e| {
            CutError::new(
                error_codes::IO,
                format!(
                    "project dir {} cannot be canonicalized",
                    project_dir.display()
                ),
                e.to_string(),
            )
        })?;
        Ok(Self {
            project_dir,
            extra_roots: Vec::new(),
        })
    }

    /// Add an explicitly configured outputs root (canonicalized; must exist).
    pub fn with_extra_root(mut self, root: &Path) -> Result<Self, CutError> {
        let root = root.canonicalize().map_err(|e| {
            CutError::new(
                error_codes::IO,
                format!("outputs dir {} cannot be canonicalized", root.display()),
                e.to_string(),
            )
        })?;
        self.extra_roots.push(root);
        Ok(self)
    }

    /// The canonical project dir (asset-path resolution + default out paths).
    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    /// The refusal for a target that is not inside the project or an outputs
    /// folder.
    fn outside_fence_error(&self, target: &Path) -> CutError {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "output path {} is outside the project/outputs dirs",
                target.display()
            ),
            format!("fence roots: {:?}", {
                let mut roots = vec![self.project_dir.display().to_string()];
                roots.extend(self.extra_roots.iter().map(|r| r.display().to_string()));
                roots
            }),
        )
        .with_suggested_action("write inside the project dir or configure an outputs dir")
    }

    /// Validate `candidate` as an output path. Returns the canonical absolute
    /// path to actually write to. Relative candidates resolve against the
    /// project dir. See module docs for the exact policy.
    pub fn fence_output_path(&self, candidate: &Path) -> Result<PathBuf, CutError> {
        // (2) Traversal is refused on the RAW path, before canonicalization —
        // ".." is never a legitimate way to name an output file via the API.
        if candidate
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("output path {} contains '..'", candidate.display()),
                "path traversal is refused by the public output-fencing contract",
            )
            .with_suggested_action("pass an absolute path inside the project or outputs dir"));
        }

        let absolute = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.project_dir.join(candidate)
        };

        // (1) Canonicalize. The file itself may not exist yet, so canonicalize
        // the PARENT (which must exist) and re-attach the file name. This also
        // resolves symlinked ancestor dirs, so a symlink-dir escape cannot
        // smuggle the write outside the fence.
        let file_name = absolute
            .file_name()
            .ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("output path {} has no file name", absolute.display()),
                    "a directory cannot be a render/export target",
                )
            })?
            .to_owned();
        if file_name.to_string_lossy().contains(':') {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("output file name {} contains ':'", file_name.to_string_lossy()),
                "colon is refused in output file names to avoid Windows alternate-data-stream targets",
            )
            .with_suggested_action("choose a normal export file name without ':'"));
        }
        let parent = absolute.parent().unwrap_or(Path::new("/"));
        let canonical_parent = parent.canonicalize().map_err(|e| {
            // A folder that cannot be resolved is first judged by where it
            // would be: a target outside the project and outputs folders is
            // refused as outside, whether or not that folder exists (on
            // Windows `/tmp` is usually missing; the answer must not depend on
            // it, and must not tell a caller which outside folders exist).
            // Resolve the nearest folder that does exist and re-attach the
            // missing rest, so a project reached through a linked path (a
            // macOS temp folder, a mapped drive) still counts as inside.
            let would_be = resolve_existing_prefix(&absolute);
            let named_inside = std::iter::once(&self.project_dir)
                .chain(self.extra_roots.iter())
                .any(|root| would_be.starts_with(root));
            if named_inside {
                crate::folder_access::folder_error("output folder", parent, &e)
            } else {
                self.outside_fence_error(&absolute)
            }
        })?;
        if !canonical_parent.is_dir() {
            return Err(crate::folder_access::folder_error(
                "output folder",
                parent,
                &std::io::Error::from(std::io::ErrorKind::NotADirectory),
            ));
        }
        let canonical = canonical_parent.join(&file_name);

        // (3) Static imported links can redirect direct encoder writes even
        // when the parent is fenced. Admit only missing or single-link files.
        cut_core::matte_cache::plain_file_exists(&canonical).map_err(|error| {
            CutError::new(
                &error.code,
                format!(
                    "output path {} is not a plain file (symlink/reparse/hardlink refused)",
                    canonical.display()
                ),
                format!(
                    "symlink, reparse and hardlink targets are refused: {}",
                    error.cause
                ),
            )
        })?;

        // (4) Must be inside the project dir or an explicit outputs root.
        let in_fence = std::iter::once(&self.project_dir)
            .chain(self.extra_roots.iter())
            .any(|root| canonical.starts_with(root));
        if !in_fence {
            return Err(self.outside_fence_error(&canonical));
        }

        // (5) Overwrites only for media/sidecar suffixes.
        if canonical.exists() {
            let file_name = canonical
                .file_name()
                .and_then(|e| e.to_str())
                .map(|name| name.to_ascii_lowercase());
            let text_name_ok = file_name
                .as_deref()
                .map(|name| OVERWRITABLE_TEXT_FILENAMES.contains(&name))
                .unwrap_or(false);
            let suffix_ok = canonical
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| OVERWRITABLE_SUFFIXES.contains(&e.to_ascii_lowercase().as_str()))
                .unwrap_or(false);
            if !suffix_ok && !text_name_ok {
                return Err(CutError::new(
                    error_codes::CONFLICT,
                    format!(
                        "refusing to overwrite {} — not a media/sidecar file",
                        canonical.display()
                    ),
                    format!(
                        "overwrite allowlist: suffixes={OVERWRITABLE_SUFFIXES:?}; text filenames={OVERWRITABLE_TEXT_FILENAMES:?}"
                    ),
                )
                .with_suggested_action(
                    "choose a fresh file name or an export media/sidecar target",
                ));
            }
        }

        Ok(canonical)
    }
}

/// `path` with its longest existing leading part made canonical and the
/// missing rest re-attached unchanged. Used only to decide which refusal a
/// target with a missing folder gets; nothing is written through the result.
fn resolve_existing_prefix(path: &Path) -> PathBuf {
    let mut missing: Vec<&std::ffi::OsStr> = Vec::new();
    let mut current = path;
    loop {
        if let Ok(resolved) = current.canonicalize() {
            return missing
                .iter()
                .rev()
                .fold(resolved, |joined, part| joined.join(part));
        }
        match (current.parent(), current.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                current = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[cfg(unix)]
    fn symlink_file(original: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(original, link).unwrap();
        true
    }

    #[cfg(windows)]
    fn symlink_file(original: &Path, link: &Path) -> bool {
        match std::os::windows::fs::symlink_file(original, link) {
            Ok(()) => true,
            Err(error) if error.raw_os_error() == Some(1314) => false,
            Err(error) => panic!("failed to create test symlink: {error}"),
        }
    }

    #[cfg(unix)]
    fn symlink_dir(original: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(original, link).unwrap();
        true
    }

    #[cfg(windows)]
    fn symlink_dir(original: &Path, link: &Path) -> bool {
        match std::os::windows::fs::symlink_dir(original, link) {
            Ok(()) => true,
            Err(error) if error.raw_os_error() == Some(1314) => false,
            Err(error) => panic!("failed to create test directory symlink: {error}"),
        }
    }

    /// Build a fence over a temp "project dir" for each rejection case.
    fn setup() -> (tempfile::TempDir, PathFence) {
        let dir = tempfile::tempdir().unwrap();
        let fence = PathFence::new(dir.path()).unwrap();
        (dir, fence)
    }

    #[test]
    fn accepts_in_project_new_file() {
        let (_dir, fence) = setup();
        let out = fence.fence_output_path(Path::new("final.mp4")).unwrap();
        assert!(out.ends_with("final.mp4"));
    }

    #[test]
    fn rejects_traversal() {
        let (_dir, fence) = setup();
        let err = fence
            .fence_output_path(Path::new("../escape.mp4"))
            .unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_ARGS);
        assert!(err.message.contains(".."));
    }

    #[test]
    fn rejects_outside_project_absolute() {
        let (_dir, fence) = setup();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("evil.mp4");
        let err = fence.fence_output_path(&outside_file).unwrap_err();
        assert!(err.message.contains("outside"));
    }

    #[test]
    fn rejects_symlink_target() {
        let (dir, fence) = setup();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let link = dir.path().join("looks_safe.mp4");
        if !symlink_file(outside.path(), &link) {
            eprintln!("skipping: Windows symlink creation privilege is unavailable");
            return;
        }
        let err = fence.fence_output_path(&link).unwrap_err();
        assert!(err.message.contains("symlink"));
    }

    #[test]
    fn rejects_symlinked_dir_escape() {
        // A symlinked SUBDIR pointing outside must not fence-pass via the raw
        // (uncanonicalized) prefix.
        let (dir, fence) = setup();
        let outside = tempfile::tempdir().unwrap();
        if !symlink_dir(outside.path(), &dir.path().join("exports")) {
            eprintln!("skipping: Windows symlink creation privilege is unavailable");
            return;
        }
        let err = fence
            .fence_output_path(&dir.path().join("exports/out.mp4"))
            .unwrap_err();
        assert!(err.message.contains("outside"));
    }

    #[test]
    fn rejects_overwrite_of_non_media_file() {
        let (dir, fence) = setup();
        fs::write(dir.path().join("project.json"), "{}").unwrap();
        let err = fence
            .fence_output_path(Path::new("project.json"))
            .unwrap_err();
        assert_eq!(err.code, error_codes::CONFLICT);
    }

    #[test]
    fn rejects_colon_in_output_filename() {
        let (_dir, fence) = setup();
        let err = fence
            .fence_output_path(Path::new("out.mp4:ads"))
            .unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_ARGS);
        assert!(err.message.contains(':'));
    }

    #[test]
    fn allows_overwrite_of_export_media_and_sidecar_files() {
        let (dir, fence) = setup();
        for ext in [
            "mp4", "mov", "webm", "mkv", "gif", "srt", "vtt", "xml", "fcpxml", "otio", "edl",
            "ass", "jpg", "jpeg", "png", "mp3", "m4a", "wav", "flac", "opus", "aac", "ogg",
        ] {
            let name = format!("export.{ext}");
            fs::write(dir.path().join(&name), "x").unwrap();
            fence.fence_output_path(Path::new(&name)).unwrap();
        }
        for name in ["transcript.txt", "transcript.md", "chapters.txt"] {
            fs::write(dir.path().join(name), "x").unwrap();
            fence.fence_output_path(Path::new(name)).unwrap();
        }
        fs::write(dir.path().join("notes.txt"), "x").unwrap();
        let err = fence.fence_output_path(Path::new("notes.txt")).unwrap_err();
        assert_eq!(err.code, error_codes::CONFLICT);

        // Extra outputs root accepts writes too.
        let outputs = tempfile::tempdir().unwrap();
        let fence = PathFence::new(dir.path())
            .unwrap()
            .with_extra_root(outputs.path())
            .unwrap();
        fence
            .fence_output_path(&outputs.path().join("out.mov"))
            .unwrap();
    }

    /// A target outside the project is refused as outside whether or not its
    /// folder exists: the answer must not depend on (or reveal) what exists
    /// elsewhere on the machine. On Windows `/tmp` is usually missing.
    #[test]
    fn a_missing_folder_outside_the_project_is_refused_as_outside() {
        let (_dir, fence) = setup();
        let elsewhere = tempfile::tempdir().unwrap();
        let target = elsewhere.path().join("not-there").join("out.mp4");
        let error = fence.fence_output_path(&target).unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_ARGS, "{error:?}");
        assert!(
            error.message.contains("outside the project/outputs dirs"),
            "{}",
            error.message
        );
        assert!(!error.message.contains("not found"), "{}", error.message);
    }

    #[test]
    fn a_missing_output_folder_is_named_as_missing() {
        let (dir, fence) = setup();
        let error = fence
            .fence_output_path(&dir.path().join("nope").join("out.mp4"))
            .unwrap_err();
        assert_eq!(error.code, error_codes::NOT_FOUND);
        assert!(
            error.message.starts_with("output folder not found"),
            "{}",
            error.message
        );
        assert_eq!(
            error.suggested_action.as_deref(),
            Some("create the folder first, or choose an existing folder")
        );
    }

    #[test]
    fn an_output_folder_that_is_a_file_is_named_as_a_file() {
        let (dir, fence) = setup();
        let file = dir.path().join("plain.txt");
        fs::write(&file, b"x").unwrap();
        let error = fence.fence_output_path(&file.join("out.mp4")).unwrap_err();
        assert!(
            error.message.starts_with("output folder is a file"),
            "{}",
            error.message
        );
        assert!(!error.message.contains("not found"), "{}", error.message);
    }

    #[test]
    fn access_denied_and_full_disk_are_never_reported_as_missing() {
        use std::io::{Error, ErrorKind};
        let folder = Path::new(r"\\?\C:\work\readonly");
        let denied = crate::folder_access::folder_error(
            "output folder",
            folder,
            &Error::new(ErrorKind::PermissionDenied, "Access is denied."),
        );
        assert_eq!(denied.code, error_codes::IO);
        assert_eq!(
            denied.message,
            r"output folder cannot be written: C:\work\readonly (access is denied)"
        );
        assert!(denied.cause.contains("Access is denied"));
        assert!(!denied.suggested_action.unwrap().contains("create"));
        let full = crate::folder_access::folder_error(
            "output folder",
            folder,
            &Error::new(ErrorKind::StorageFull, "disk full"),
        );
        assert!(full.message.contains("is full"), "{}", full.message);
        assert!(full
            .suggested_action
            .unwrap()
            .contains("free up disk space"));
    }
}
