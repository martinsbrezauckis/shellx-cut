//! Trusted, project-local Canvas -> Cut handback for copy-on-write Motion edits.
//!
//! Cut creates one immutable launch request under the open project. Canvas keeps
//! that path in its native/Node host boundary and publishes immutable ready
//! descriptors beside it after a verified render. The webview never receives a
//! filesystem path, and Cut revalidates package identity plus the exact authored
//! source revision before refreshing the stable linked clip.

use crate::motion_package::{
    identity as motion_package_identity, revision as motion_package_revision,
};
use crate::output_paths::{fence_output_path, write_output_atomic, OutputPathPolicy};
use cut_core::{error_codes, CutError};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const REQUEST_SCHEMA: &str = "shellx-cut/motion-edit-return-request@1";
const READY_SCHEMA: &str = "shellx-canvas/motion-edit-return-ready@1";
const MAX_DESCRIPTOR_BYTES: u64 = 64 * 1024;
const MAX_SESSIONS_TO_SCAN: usize = 16;
const MAX_READY_FILES_TO_SCAN: usize = 32;

#[derive(Debug, Clone)]
pub(crate) struct MotionEditReturnCandidate {
    pub package_dir: PathBuf,
    pub source_revision: String,
    pub session_token: String,
    pub revision_token: String,
    pub completed_at_unix_ms: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedMotionEditSource {
    pub package_dir: PathBuf,
    pub package_id: String,
    pub motion_id: String,
    pub source_revision: String,
    pub canvas_return: Option<MotionEditReturnCandidate>,
}

impl MotionEditReturnCandidate {
    pub(crate) fn public_evidence(&self) -> Value {
        json!({
            "applied": true,
            "sessionToken": self.session_token,
            "revisionToken": self.revision_token,
            "completedAtUnixMs": self.completed_at_unix_ms,
            "sourceRevision": self.source_revision,
        })
    }
}

pub(crate) fn resolve_latest_source(
    project_dir: &Path,
    clip: &str,
    safe_clip: &str,
    link: &Value,
) -> Result<ResolvedMotionEditSource, CutError> {
    let linked_package_id = required_link_id(link, "packageId")?;
    let linked_motion_id = required_link_id(link, "motionId")?;
    let canvas_return = latest_ready(
        project_dir,
        clip,
        safe_clip,
        linked_package_id,
        linked_motion_id,
    )?;
    let source_path = canvas_return
        .as_ref()
        .map(|candidate| candidate.package_dir.clone())
        .or_else(|| {
            link.get("sourcePath")
                .and_then(Value::as_str)
                .map(PathBuf::from)
        })
        .ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                "linked Motion source is not available",
                "this clip was imported without a local package path",
            )
            .with_suggested_action("use motion.link.relink before refreshing")
        })?;
    let package_dir = source_path.canonicalize().map_err(|error| {
        CutError::new(
            error_codes::NOT_FOUND,
            "linked Motion source is missing",
            error.to_string(),
        )
        .with_suggested_action("relink the clip to its original Motion package")
    })?;
    let (package_id, motion_id) = motion_package_identity(&package_dir)?;
    if package_id != linked_package_id || motion_id != linked_motion_id {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "linked Motion source identity changed",
            format!("resolved {package_id} / {motion_id}"),
        )
        .with_suggested_action("relink to the original package instead of rendering a different package into this clip"));
    }
    let source_revision = motion_package_revision(&package_dir)?;
    if canvas_return
        .as_ref()
        .is_some_and(|candidate| candidate.source_revision != source_revision)
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "Canvas Motion revision changed after verified handback",
            "the returned package no longer matches Canvas's verified source revision",
        )
        .with_suggested_action(
            "render the current revision in Canvas again before refreshing Cut",
        ));
    }
    Ok(ResolvedMotionEditSource {
        package_dir,
        package_id,
        motion_id,
        source_revision,
        canvas_return,
    })
}

pub(crate) fn create_request(
    project_dir: &Path,
    clip: &str,
    safe_clip: &str,
    package_id: &str,
    motion_id: &str,
    source_revision: &str,
) -> Result<PathBuf, CutError> {
    let created_at_unix_ms = now_unix_ms()?;
    let mut digest = Sha256::new();
    digest.update(created_at_unix_ms.to_le_bytes());
    digest.update(std::process::id().to_le_bytes());
    digest.update(clip.as_bytes());
    digest.update(package_id.as_bytes());
    digest.update(motion_id.as_bytes());
    digest.update(source_revision.as_bytes());
    let session_token = format!("{created_at_unix_ms}-{:x}", digest.finalize());
    let relative =
        format!(".shellx-cut/motion-edit-returns/{safe_clip}/{session_token}/request.json");
    let path = fence_output_path(
        project_dir,
        Some(&relative),
        &relative,
        OutputPathPolicy::JSON,
    )?;
    let document = json!({
        "schema": REQUEST_SCHEMA,
        "state": "pending",
        "sessionToken": session_token,
        "clip": clip,
        "packageId": package_id,
        "motionId": motion_id,
        "sourceRevision": source_revision,
        "createdAtUnixMs": created_at_unix_ms,
        "localOnly": true,
        "remotePublish": false,
    });
    write_output_atomic(
        &path,
        [serde_json::to_vec_pretty(&document)?, b"\n".to_vec()].concat(),
    )?;
    Ok(path.to_path_buf())
}

pub(crate) fn latest_ready(
    project_dir: &Path,
    clip: &str,
    safe_clip: &str,
    package_id: &str,
    motion_id: &str,
) -> Result<Option<MotionEditReturnCandidate>, CutError> {
    let root = project_dir
        .join(".shellx-cut")
        .join("motion-edit-returns")
        .join(safe_clip);
    if !root.is_dir() {
        return Ok(None);
    }
    let mut sessions = bounded_directories(&root, MAX_SESSIONS_TO_SCAN)?;
    sessions.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    let mut best: Option<MotionEditReturnCandidate> = None;
    for session in sessions {
        let request = match read_json_file(&session.join("request.json")) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if request.get("schema").and_then(Value::as_str) != Some(REQUEST_SCHEMA)
            || request.get("state").and_then(Value::as_str) != Some("pending")
            || request.get("clip").and_then(Value::as_str) != Some(clip)
            || request.get("packageId").and_then(Value::as_str) != Some(package_id)
            || request.get("motionId").and_then(Value::as_str) != Some(motion_id)
        {
            continue;
        }
        let Some(session_token) = request
            .get("sessionToken")
            .and_then(Value::as_str)
            .filter(|value| valid_session_token(value))
        else {
            continue;
        };
        let mut ready_files = bounded_ready_files(&session, MAX_READY_FILES_TO_SCAN)?;
        ready_files.sort();
        for ready_path in ready_files {
            let ready = match read_json_file(&ready_path) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let Some(candidate) = parse_ready(&ready, clip, session_token, package_id, motion_id)
            else {
                continue;
            };
            if best
                .as_ref()
                .map(|current| candidate.completed_at_unix_ms > current.completed_at_unix_ms)
                .unwrap_or(true)
            {
                best = Some(candidate);
            }
        }
    }
    Ok(best)
}

fn parse_ready(
    value: &Value,
    clip: &str,
    session_token: &str,
    package_id: &str,
    motion_id: &str,
) -> Option<MotionEditReturnCandidate> {
    if value.get("schema")?.as_str()? != READY_SCHEMA
        || value.get("state")?.as_str()? != "ready"
        || value.get("sessionToken")?.as_str()? != session_token
        || value.get("clip")?.as_str()? != clip
        || value.get("packageId")?.as_str()? != package_id
        || value.get("motionId")?.as_str()? != motion_id
        || !value.get("localOnly")?.as_bool()?
        || value.get("remotePublish")?.as_bool()?
    {
        return None;
    }
    let package_dir = value.get("packageDir")?.as_str()?;
    let source_revision = value.get("sourceRevision")?.as_str()?;
    let revision_token = value.get("revisionToken")?.as_str()?;
    if package_dir.len() > 4096
        || !Path::new(package_dir).is_absolute()
        || !is_sha256(source_revision)
        || !valid_revision_token(revision_token)
    {
        return None;
    }
    Some(MotionEditReturnCandidate {
        package_dir: PathBuf::from(package_dir),
        source_revision: source_revision.to_string(),
        session_token: session_token.to_string(),
        revision_token: revision_token.to_string(),
        completed_at_unix_ms: value.get("completedAtUnixMs")?.as_u64()?,
    })
}

fn bounded_directories(root: &Path, limit: usize) -> Result<Vec<PathBuf>, CutError> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let info = entry.file_type()?;
        if info.is_dir() && !info.is_symlink() {
            paths.push(entry.path());
        }
    }
    paths.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    paths.truncate(limit);
    Ok(paths)
}

fn bounded_ready_files(root: &Path, limit: usize) -> Result<Vec<PathBuf>, CutError> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let info = entry.file_type()?;
        if info.is_file()
            && !info.is_symlink()
            && name.starts_with("ready-")
            && name.ends_with(".json")
        {
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH);
            paths.push((modified, entry.path()));
        }
    }
    paths.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    paths.truncate(limit);
    Ok(paths.into_iter().map(|(_, path)| path).collect())
}

fn read_json_file(path: &Path) -> Result<Value, CutError> {
    let info = std::fs::symlink_metadata(path)?;
    if !info.is_file() || info.file_type().is_symlink() || info.len() > MAX_DESCRIPTOR_BYTES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "Motion edit return descriptor is invalid",
            path.display().to_string(),
        ));
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn now_unix_ms() -> Result<u64, CutError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CutError::new(error_codes::IO, "read system clock", error.to_string()))?
        .as_millis();
    u64::try_from(millis)
        .map_err(|_| CutError::new(error_codes::IO, "read system clock", "timestamp overflow"))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_session_token(value: &str) -> bool {
    (16..=96).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_revision_token(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn required_link_id<'a>(link: &'a Value, field: &str) -> Result<&'a str, CutError> {
    link.get(field).and_then(Value::as_str).ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("linked Motion {field} is missing"),
            format!("motion_link.{field} is required"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_resolves_latest_ready_return_without_exposing_it_elsewhere() {
        let root = tempfile::tempdir().unwrap();
        let request = create_request(
            root.path(),
            "clip-1",
            "clip-1",
            "pkg-1",
            "motion-1",
            &"a".repeat(64),
        )
        .unwrap();
        let request_json: Value =
            serde_json::from_slice(&std::fs::read(&request).unwrap()).unwrap();
        let token = request_json["sessionToken"].as_str().unwrap();
        let revision_token = "r".repeat(32);
        let ready = request
            .parent()
            .unwrap()
            .join(format!("ready-{revision_token}.json"));
        std::fs::write(
            &ready,
            serde_json::to_vec(&json!({
                "schema": READY_SCHEMA,
                "state": "ready",
                "sessionToken": token,
                "clip": "clip-1",
                "packageId": "pkg-1",
                "motionId": "motion-1",
                "packageDir": root.path().join("revision-package"),
                "sourceRevision": "b".repeat(64),
                "revisionToken": revision_token.clone(),
                "completedAtUnixMs": 42,
                "localOnly": true,
                "remotePublish": false,
            }))
            .unwrap(),
        )
        .unwrap();
        let candidate = latest_ready(root.path(), "clip-1", "clip-1", "pkg-1", "motion-1")
            .unwrap()
            .unwrap();
        assert_eq!(candidate.source_revision, "b".repeat(64));
        assert_eq!(candidate.session_token, token);
        assert_eq!(candidate.revision_token, revision_token);
    }
}
