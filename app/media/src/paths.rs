//! paths.rs — output-path fencing (public verb contract SECURITY the output-fencing contract).
//!
//! Role: the gate every file-WRITING media function passes its output path
//! through. Without this the REST/MCP surface is an arbitrary-file-overwrite
//! primitive (same bug class as the shellX fs-denylist finding).
//! Policy, exactly per the output-fencing contract:
//!   1. canonicalize (resolves `.`/`..`/symlinked ancestors against the fs);
//!   2. refuse path traversal (`..` components) BEFORE touching the fs;
//!   3. refuse symlinked targets (an existing out-file that is a symlink);
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
            CutError::new(
                error_codes::INVALID_ARGS,
                format!("output dir {} does not exist", parent.display()),
                e.to_string(),
            )
            .with_suggested_action(
                "create the directory first or use a path inside the project dir",
            )
        })?;
        let canonical = canonical_parent.join(&file_name);

        // (3) Refuse symlinked targets: writing "through" a link is how an
        // in-fence name overwrites an out-of-fence file.
        if canonical
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("output path {} is a symlink", canonical.display()),
                "symlinked targets are refused by the public output-fencing contract",
            ));
        }

        // (4) Must be inside the project dir or an explicit outputs root.
        let in_fence = std::iter::once(&self.project_dir)
            .chain(self.extra_roots.iter())
            .any(|root| canonical.starts_with(root));
        if !in_fence {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!(
                    "output path {} is outside the project/outputs dirs",
                    canonical.display()
                ),
                format!("fence roots: {:?}", {
                    let mut roots = vec![self.project_dir.display().to_string()];
                    roots.extend(self.extra_roots.iter().map(|r| r.display().to_string()));
                    roots
                }),
            )
            .with_suggested_action("write inside the project dir or configure an outputs dir"));
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
}
