//! Which derived files under a project folder a render may reuse (items 139
//! and 155).
//!
//! Renders keep derived files in the project and reuse an existing one instead
//! of making it again: segment windows, mask and power-window alphas, matte
//! alphas, stabilization data, baked titles. Their names are hashes of data
//! that is in `project.json`, so a project received from someone else can carry
//! a file at exactly such a name with any content, and the render would have
//! used it: the output would show something the edit does not say.
//!
//! A derived file is reused only when THIS user's engine made it: after the
//! engine writes one, the file's content hash is recorded outside the project,
//! in the engine's app-data work root (`cache-trust/`, owner-only), under a name
//! derived from the file's full path. A file without a matching record (planted,
//! copied from another machine, or changed since) is made again; the normal
//! write replaces it. A project copied from another machine therefore makes its
//! derived files once more on its first render here: slower, nothing fails.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

fn index_dir() -> Option<PathBuf> {
    let dir = crate::private_work::work_root()?.join("cache-trust");
    std::fs::create_dir_all(&dir).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    Some(dir)
}

/// The record's file name: a hash of the window's canonical path, so the same
/// key in two projects (or two copies of one) never shares a record.
fn record_path(segment: &Path) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(segment).ok()?;
    let mut h = Sha256::new();
    h.update(canonical.to_string_lossy().as_bytes());
    Some(index_dir()?.join(format!("{:x}", h.finalize())))
}

/// SHA-256 of the file's bytes, read without following a link at the name
/// (the caller has already refused links via `plain_internal_file_exists`).
fn content_hash(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(format!("{:x}", h.finalize()))
}

/// Record a window this engine has just encoded. Best effort: without a
/// record the window is simply encoded again next time.
pub fn remember(segment: &Path) {
    remember_record(segment, None);
}

/// True when `bytes`, which the caller has read once from `path` through its
/// own ordinary-file checks, are exactly the content this engine recorded for
/// `path`. The caller then uses those bytes and never reads the file again, so
/// a file replaced after this check cannot reach its consumer.
pub fn trusted_bytes(path: &Path, bytes: &[u8]) -> bool {
    let Some(record) = record_path(path) else {
        return false;
    };
    let hash = format!("{:x}", Sha256::digest(bytes));
    std::fs::read_to_string(record).is_ok_and(|recorded| recorded_hash(&recorded) == hash)
}

/// The content hash of a record: its first line (a bound record carries what
/// the file was made for on a second line).
fn recorded_hash(record_text: &str) -> &str {
    record_text.lines().next().unwrap_or("").trim()
}

/// Record a file this engine has just written whose SHA-256 (lower-case hex)
/// the caller already computed from its bytes, so a large file is not read
/// twice. Best effort, like [`remember`].
pub fn remember_hash(path: &Path, sha256_hex: &str) {
    if let Some(record) = record_path(path) {
        let _ = std::fs::write(record, sha256_hex);
    }
}

/// True when this engine recorded exactly `sha256_hex` for the file at `path`.
/// The caller must have computed the hash from the file's current bytes; the
/// record (outside the project) is what proves this engine wrote them.
pub fn trusted_hash(path: &Path, sha256_hex: &str) -> bool {
    record_path(path)
        .and_then(|record| std::fs::read_to_string(record).ok())
        .is_some_and(|recorded| !sha256_hex.is_empty() && recorded_hash(&recorded) == sha256_hex)
}

/// True when the cached window is one this engine wrote and is unchanged.
pub fn trusted(segment: &Path) -> bool {
    recorded(segment).is_some_and(|(hash, current, _)| hash == current)
}

/// [`remember`] bound to what the file was made FOR (an asset id and its
/// source hash, a title's settings): a file this engine made for something
/// else, reached by another name or a link, does not count.
pub fn remember_bound(file: &Path, binding: &str) {
    remember_record(file, Some(binding));
}

/// Discard only this output's engine-origin record after failed publication.
/// The derived file itself is left for the caller's ownership/recovery protocol.
pub fn forget(file: &Path) {
    if let Some(record) = record_path(file) {
        let _ = std::fs::remove_file(record);
    }
}

/// True when this engine made the file for exactly `binding` and it is unchanged.
pub fn trusted_bound(file: &Path, binding: &str) -> bool {
    recorded(file).is_some_and(|(hash, current, bound)| {
        hash == current && bound.as_deref() == Some(binding_digest(binding).as_str())
    })
}

fn binding_digest(binding: &str) -> String {
    format!("{:x}", Sha256::digest(binding.as_bytes()))
}

fn remember_record(file: &Path, binding: Option<&str>) {
    if let (Some(record), Some(hash)) = (record_path(file), content_hash(file)) {
        let text = match binding {
            Some(binding) => format!("{hash}\n{}", binding_digest(binding)),
            None => hash,
        };
        let _ = std::fs::write(record, text);
    }
}

/// Record that this engine read `binding` (an asset's probe) from the source
/// file at `source` as it is now: its size and modification time, not its
/// content (a source can be gigabytes; hashing it at every open would freeze a
/// large project). Keyed apart from derived-file records.
pub fn remember_source(source: &Path, binding: &str) {
    if let (Some(record), Some(stat)) = (source_record_path(source), source_stat(source)) {
        let _ = std::fs::write(record, format!("{stat}\n{}", binding_digest(binding)));
    }
}

/// True when this engine recorded `binding` for this source file and the file
/// has the same size and modification time since.
pub fn source_unchanged_for(source: &Path, binding: &str) -> bool {
    let (Some(record), Some(stat)) = (source_record_path(source), source_stat(source)) else {
        return false;
    };
    std::fs::read_to_string(record)
        .is_ok_and(|text| text == format!("{stat}\n{}", binding_digest(binding)))
}

fn source_record_path(source: &Path) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(source).ok()?;
    let mut h = Sha256::new();
    h.update(b"source\n");
    h.update(canonical.to_string_lossy().as_bytes());
    Some(index_dir()?.join(format!("{:x}", h.finalize())))
}

fn source_stat(source: &Path) -> Option<String> {
    let meta = std::fs::metadata(source).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!("stat:{}:{modified}", meta.len()))
}

/// (recorded hash, current hash, recorded binding digest).
fn recorded(file: &Path) -> Option<(String, String, Option<String>)> {
    let (record, current) = (record_path(file)?, content_hash(file)?);
    let text = std::fs::read_to_string(record).ok()?;
    let mut lines = text.lines();
    let hash = lines.next()?.trim().to_string();
    let bound = lines.next().map(|line| line.trim().to_string());
    Some((hash, current, bound))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_window_this_engine_wrote_and_left_unchanged_is_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let seg = dir.path().join("seg_0123456789abcdef.mp4");
        std::fs::write(&seg, b"planted by the sender").unwrap();
        assert!(!trusted(&seg), "a file nobody recorded is not reused");
        std::fs::write(&seg, b"encoded here").unwrap();
        remember(&seg);
        assert!(trusted(&seg));
        std::fs::write(&seg, b"changed afterwards").unwrap();
        assert!(!trusted(&seg), "a changed window is encoded again");
        assert!(!trusted(&dir.path().join("missing.mp4")));
    }

    #[test]
    fn checked_bytes_count_only_when_they_are_what_this_engine_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let report = dir.path().join("a1.perception.json");
        std::fs::write(&report, b"made here").unwrap();
        remember(&report);
        assert!(trusted_bytes(&report, b"made here"));
        assert!(!trusted_bytes(&report, b"other bytes"));
        assert!(!trusted_bytes(
            &dir.path().join("missing.json"),
            b"made here"
        ));
    }

    #[test]
    fn a_hash_the_caller_computed_is_trusted_only_as_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("part.mp4");
        std::fs::write(&part, b"made here").unwrap();
        let hash = content_hash(&part).unwrap();
        assert!(!trusted_hash(&part, &hash), "nothing recorded yet");
        remember_hash(&part, &hash);
        assert!(trusted_hash(&part, &hash));
        assert!(trusted(&part), "the same record serves both checks");
        assert!(
            !trusted_hash(&part, "0000"),
            "another content is not this record"
        );
        assert!(!trusted_hash(&part, ""), "an empty hash proves nothing");
    }

    #[test]
    fn a_source_record_counts_only_for_the_unchanged_file_and_its_binding() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("clip.mp4");
        std::fs::write(&source, b"source").unwrap();
        assert!(!source_unchanged_for(&source, "probe one"));
        remember_source(&source, "probe one");
        assert!(source_unchanged_for(&source, "probe one"));
        assert!(!source_unchanged_for(&source, "probe two"), "another probe");
        std::fs::write(&source, b"a longer source").unwrap();
        assert!(
            !source_unchanged_for(&source, "probe one"),
            "a changed file"
        );
        assert!(!trusted(&source), "not a derived-file record");
    }

    #[test]
    fn a_bound_record_counts_only_for_what_the_file_was_made_for() {
        let dir = tempfile::tempdir().unwrap();
        let proxy = dir.path().join("b1.mp4");
        std::fs::write(&proxy, b"made for b1").unwrap();
        remember_bound(&proxy, "proxy b1 sha256:bb");
        assert!(trusted_bound(&proxy, "proxy b1 sha256:bb"));
        assert!(
            !trusted_bound(&proxy, "proxy a1 sha256:aa"),
            "made for another asset"
        );
        assert!(trusted(&proxy), "still this engine's own file");
        assert!(
            trusted_bytes(&proxy, b"made for b1"),
            "a bound record still answers for bytes already read"
        );
        assert!(trusted_hash(&proxy, &content_hash(&proxy).unwrap()));
        remember(&proxy);
        assert!(
            !trusted_bound(&proxy, "proxy b1 sha256:bb"),
            "an unbound record binds nothing"
        );
        #[cfg(unix)]
        {
            // A link at another asset's name resolves to b1's file and record.
            remember_bound(&proxy, "proxy b1 sha256:bb");
            let link = dir.path().join("a1.mp4");
            std::os::unix::fs::symlink(&proxy, &link).unwrap();
            assert!(!trusted_bound(&link, "proxy a1 sha256:aa"));
        }
    }
}
