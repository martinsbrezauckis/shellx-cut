//! The exact files a render review hands to its adapter.
//!
//! The review checks its inputs when it is admitted, then runs as a detached
//! job whose adapter opens files by name. A project folder can be changed by
//! another program in between, so the adapter is never given a project file
//! that was checked: it gets a copy, written into the run's own new folder, of
//! exactly the bytes that passed the check.
//!
//! - The render's analysis file: the bytes read once and checked by
//!   `judge_files::admit_render_report`.
//! - A render outside the project: copied while it is hashed, and used only
//!   when that copy has the receipt's content hash.
//! - A render inside the project or a standing export folder: hashed against
//!   the receipt before the review and again after it. A verdict is attached
//!   only when both hashes are the receipt's. (It is not copied: a 4K hour of
//!   video is tens of gigabytes, and the two hashes cost reading it twice
//!   instead of doubling its disk space; a file swapped and restored between
//!   the two hashes needs a live program of the person's own account, which
//!   is outside what Cut defends against.)

use super::judge_files::JudgeRefusal;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Name of the analysis copy inside the run folder.
const REPORT_COPY: &str = "render.perception.json";

/// A new private file in the run folder, never reusing an existing name.
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Write the checked analysis bytes into the run folder and return that path.
pub(super) fn stage_report(run_dir: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let path = run_dir.join(REPORT_COPY);
    let mut file = create_private(&path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(path)
}

/// Copy a render that lies outside the project into the run folder, hashing
/// the bytes as they are copied, and return the copy only when its content
/// hash is the receipt's (full SHA-256, as render.final records it). Runs on
/// the caller's blocking thread.
pub(super) fn stage_render(
    render: &Path,
    run_dir: &Path,
    output_hash: &str,
) -> Result<PathBuf, JudgeRefusal> {
    let refusal = |detail: String| JudgeRefusal {
        message: "review not run: the render file outside the project does not match its receipt"
            .into(),
        cause: format!(
            "{} {detail}, so it was not sent to the reviewer",
            render.display()
        ),
        action: "render again, then review the new render".into(),
    };
    let name = render
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| extension.chars().all(|c| c.is_ascii_alphanumeric()))
        .map_or_else(
            || "render".to_string(),
            |extension| format!("render.{extension}"),
        );
    let copy = run_dir.join(name);
    let copied = (|| -> std::io::Result<String> {
        let mut source = std::fs::File::open(render)?;
        let mut target = create_private(&copy)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
            target.write_all(&buffer[..count])?;
        }
        target.sync_all()?;
        Ok(format!("sha256:{:x}", hasher.finalize()))
    })();
    let matches = match copied {
        Ok(hash) => hash == output_hash,
        Err(error) => {
            let _ = std::fs::remove_file(&copy);
            return Err(refusal(format!("could not be read ({error})")));
        }
    };
    if !matches {
        let _ = std::fs::remove_file(&copy);
        return Err(refusal(format!(
            "is not the file this project rendered (its content hash differs from {output_hash})"
        )));
    }
    Ok(copy)
}

/// Full SHA-256 of a render, in the receipt's `sha256:<hex>` form.
fn render_hash(render: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(render)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

/// A render inside the project matches the receipt that names it. `when` is
/// "before the review" or "after the review". Runs on the caller's blocking
/// thread.
pub(super) fn check_render(
    render: &Path,
    output_hash: &str,
    when: &str,
) -> Result<(), JudgeRefusal> {
    let differs = |detail: String| JudgeRefusal {
        message: format!("review not run: the render file does not match its receipt {when}"),
        cause: format!(
            "{} {detail}, so no verdict was attached to the receipt",
            render.display()
        ),
        action: "render again, then review the new render".into(),
    };
    match render_hash(render) {
        Ok(hash) if hash == output_hash => Ok(()),
        Ok(hash) => Err(differs(format!(
            "has content hash {hash}, but the receipt records {output_hash}"
        ))),
        Err(error) => Err(differs(format!("could not be read ({error})"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(bytes: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    #[test]
    fn the_review_gets_a_private_copy_of_the_checked_bytes() {
        let run = tempfile::tempdir().unwrap();
        let path = stage_report(run.path(), b"checked facts").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"checked facts");
        assert!(path.starts_with(run.path()));
        assert!(
            stage_report(run.path(), b"again").is_err(),
            "never reuses a name"
        );
    }

    #[test]
    fn a_render_outside_the_project_is_used_only_as_a_matching_copy() {
        let outside = tempfile::tempdir().unwrap();
        let render = outside.path().join("final.mp4");
        std::fs::write(&render, b"rendered bytes").unwrap();
        let run = tempfile::tempdir().unwrap();
        let copy = stage_render(&render, run.path(), &sha(b"rendered bytes")).unwrap();
        assert_eq!(copy, run.path().join("render.mp4"));
        // Changing the original afterwards does not change what is reviewed.
        std::fs::write(&render, b"swapped bytes").unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), b"rendered bytes");

        let run = tempfile::tempdir().unwrap();
        let error = stage_render(&render, run.path(), &sha(b"rendered bytes")).unwrap_err();
        assert!(error.cause.contains("content hash differs"), "{error:?}");
        assert!(
            !run.path().join("render.mp4").exists(),
            "a refused copy is removed"
        );
    }

    #[test]
    fn a_render_inside_the_project_must_have_its_receipts_hash() {
        let dir = tempfile::tempdir().unwrap();
        let render = dir.path().join("render_001.mp4");
        std::fs::write(&render, b"rendered bytes").unwrap();
        assert!(check_render(&render, &sha(b"rendered bytes"), "before the review").is_ok());
        let error = check_render(&render, &sha(b"other bytes"), "before the review").unwrap_err();
        assert!(
            error.cause.contains(&sha(b"rendered bytes"))
                && error.cause.contains(&sha(b"other bytes")),
            "the refusal names both hashes: {error:?}"
        );
    }
}
