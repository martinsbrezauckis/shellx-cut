//! Admission of the project files a render review reads or writes.
//!
//! A project, including its receipts and `.scratch` folder, may come from
//! someone else. The review therefore:
//!
//! - never reads the render through a link at the render path itself, and
//!   reads a render outside the project and its export folders (for example
//!   a one-off Save As destination) only when its content hash equals the
//!   receipt's, which a project cannot arrange for a file it does not know;
//! - reads the render's analysis file only when it is an ordinary single-link
//!   file of the project that records this render's content hash;
//! - gives every adapter run a folder the engine has just created, so nothing
//!   the project contains can sit at a path the adapter writes;
//! - refuses a link or shared file at the saved review path before the
//!   adapter starts, and replaces that path by name when saving the copy.
//!
//! The adapters apply the same rules again on their side.

use crate::output_paths::{authorized_export_read_roots, write_output_atomic};
use cut_core::store::{plain_file_state, read_bounded_cache, BoundedCacheRead, PlainFileState};
use cut_core::{error_codes, CutError};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Analysis reports and review results share the perception-report ceiling.
const MAX_REVIEW_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// The saved copy of the latest review, kept beside the per-run folders.
const REVIEW_FILE: &str = "envelope.json";

static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A project file the review will not use, described for the person.
#[derive(Debug)]
pub(super) struct JudgeRefusal {
    pub(super) message: String,
    pub(super) cause: String,
    pub(super) action: String,
}

impl JudgeRefusal {
    /// The text recorded on the receipt.
    pub(super) fn reason(&self) -> String {
        format!("{}: {}", self.message, self.cause)
    }
}

fn shown(project_dir: &Path, path: &Path) -> String {
    path.strip_prefix(project_dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn delete_and_retry(shown: &str) -> String {
    format!("delete {shown} from the project folder, then run the review again")
}

/// A regular file that does not name another entry. A Windows render in a
/// synced folder may carry a cloud-files reparse point; only a name-surrogate
/// reparse point (symbolic link, junction) is refused.
fn ordinary_render(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.file_type().is_file() && !cut_core::reparse::names_another_entry(&metadata)
    })
}

/// Where an admitted render sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RenderPlacement {
    /// Inside the project or one of its standing export folders.
    Exported,
    /// Elsewhere; reviewed only as a copy that matches the receipt
    /// (`judge_inputs::stage_render`).
    Elsewhere,
}

/// Admit the render named by the receipt: an ordinary file, never reached
/// through a link at the render path itself.
pub(super) fn admit_render(
    project_dir: &Path,
    output_path: &str,
) -> Result<RenderPlacement, CutError> {
    let declared = Path::new(output_path);
    let declared = if declared.is_absolute() {
        declared.to_path_buf()
    } else {
        project_dir.join(declared)
    };
    let unsafe_render = || {
        CutError::new(
            error_codes::CONFLICT,
            "review not run: the render file is a link or not an ordinary file",
            format!(
                "{} must be the rendered file itself; the review does not follow links",
                declared.display()
            ),
        )
        .with_suggested_action("render again, then review the new render")
    };
    if !ordinary_render(&declared) {
        return Err(unsafe_render());
    }
    let resolved = declared.canonicalize().map_err(|error| {
        CutError::new(
            error_codes::NOT_FOUND,
            "review not run: the render file could not be resolved",
            format!("{}: {error}", declared.display()),
        )
    })?;
    let in_project = project_dir
        .canonicalize()
        .is_ok_and(|project| resolved.starts_with(project));
    let in_export_folder = authorized_export_read_roots(project_dir)
        .iter()
        .any(|root| resolved.starts_with(root));
    if !ordinary_render(&resolved) {
        return Err(unsafe_render());
    }
    Ok(if in_project || in_export_folder {
        RenderPlacement::Exported
    } else {
        RenderPlacement::Elsewhere
    })
}

/// The render's analysis file the reviewer may be given.
#[derive(Debug, Default)]
pub(super) struct RenderReport {
    /// The checked bytes to hand to the reviewer (as a private copy), or
    /// `None`: the adapter analyses the render itself.
    pub(super) bytes: Option<Vec<u8>>,
    /// Why an existing file was not passed, said to the person with the result.
    pub(super) note: Option<String>,
}

/// Admit `receipts/<render_id>.output.perception.json`. Only an analysis
/// this engine made on this computer counts (`cut_media::cache_trust`): its
/// own `asset_hash` field cannot prove which video it describes, because a
/// project from someone else can write any value there.
pub(super) fn admit_render_report(
    project_dir: &Path,
    receipts: &Path,
    render_id: &str,
    output_hash: &str,
) -> Result<RenderReport, JudgeRefusal> {
    let path = receipts.join(format!("{render_id}.output.perception.json"));
    let name = shown(project_dir, &path);
    let not_plain = |what: &str| {
        JudgeRefusal {
        message: format!("review not run: this render's analysis file {what}"),
        cause: format!(
            "{name} must be an ordinary file of this project; it was not read, so no other file's facts or transcript were sent to the reviewer"
        ),
        action: delete_and_retry(&name),
    }
    };
    match plain_file_state(&path) {
        PlainFileState::Missing => return Ok(RenderReport::default()),
        PlainFileState::NotPlain => return Err(not_plain("is a link or not an ordinary file")),
        PlainFileState::Plain => {}
    }
    // Read once; every check below and the reviewer's copy use these bytes.
    let bytes = match read_bounded_cache(&path, MAX_REVIEW_FILE_BYTES) {
        BoundedCacheRead::Bytes(bytes) => bytes,
        BoundedCacheRead::Oversized => return Err(not_plain("is too large")),
        BoundedCacheRead::Unavailable => {
            return Err(not_plain("is a link or not an ordinary file"))
        }
    };
    if !cut_media::cache_trust::trusted_bytes(&path, &bytes) {
        return Ok(RenderReport {
            bytes: None,
            note: Some(format!(
                "{name} was not made by this engine on this computer, so it was not given to the reviewer; the reviewer analysed the render itself. Render again on this computer to keep an analysis with the render."
            )),
        });
    }
    let recorded = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|report| {
            report
                .get("asset_hash")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    if recorded.as_deref() != Some(output_hash) {
        return Err(JudgeRefusal {
            message: "review not run: this render's analysis file describes a different video"
                .into(),
            cause: format!(
                "{name} does not record this render's content hash {output_hash}; its facts and transcript were not sent to the reviewer"
            ),
            action: delete_and_retry(&name),
        });
    }
    Ok(RenderReport {
        bytes: Some(bytes),
        note: None,
    })
}

/// Admit the saved review path `<bundle>/envelope.json` before any review
/// work: absent or an ordinary single-link file.
pub(super) fn admit_review_file(
    project_dir: &Path,
    bundle_dir: &Path,
) -> Result<PathBuf, JudgeRefusal> {
    let path = bundle_dir.join(REVIEW_FILE);
    if plain_file_state(&path) == PlainFileState::NotPlain {
        let name = shown(project_dir, &path);
        return Err(JudgeRefusal {
            message:
                "review not run: the project's saved review file is a link or not an ordinary file"
                    .into(),
            cause: format!("{name} was left untouched, so nothing outside the project was changed"),
            action: delete_and_retry(&name),
        });
    }
    Ok(path)
}

/// Create a new folder for one adapter run inside the admitted bundle folder.
/// Creation fails if anything already uses the name, so the folder is always
/// empty and owned by this run.
pub(super) fn create_run_dir(bundle_dir: &Path) -> std::io::Result<PathBuf> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    for _ in 0..64 {
        let sequence = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = bundle_dir.join(format!("run-{millis}-{}-{sequence}", std::process::id()));
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no unused review folder name was available",
    ))
}

/// Read the result an adapter wrote in its own run folder.
pub(super) fn read_review_result(path: &Path) -> Option<String> {
    match read_bounded_cache(path, MAX_REVIEW_FILE_BYTES) {
        BoundedCacheRead::Bytes(bytes) => String::from_utf8(bytes).ok(),
        BoundedCacheRead::Oversized | BoundedCacheRead::Unavailable => None,
    }
}

/// Save the review copy at the admitted path by replacing the name.
pub(super) fn save_review_copy(path: &Path, envelope: &Value) -> Result<(), CutError> {
    if plain_file_state(path) == PlainFileState::NotPlain {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the saved review file is no longer an ordinary file",
            path.display().to_string(),
        ));
    }
    write_output_atomic(path, serde_json::to_vec_pretty(envelope)?)
}
