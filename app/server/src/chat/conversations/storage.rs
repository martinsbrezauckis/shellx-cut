//! Bounded, atomic, symlink-safe project storage for conversations under `<proj>.cutproj/chat/`.

use super::{
    ChatActionRef, ChatTurnRecord, ConversationRecord, ConversationSummary,
    CHAT_CONVERSATION_SCHEMA,
};
use crate::chat::redact::{redact_secrets, redact_value};
use cut_core::store::{read_bounded_cache, replace_project_file, BoundedCacheRead};
use cut_core::{error_codes, CutError, OpRecord};
use serde_json::Value;
use std::path::{Path, PathBuf};

const MAX_CONVERSATION_FILE_BYTES: u64 = 4 * 1024 * 1024;
pub(super) const MAX_CONVERSATION_FILES: usize = 4096;
pub(super) const MAX_CHAT_LIST_REPLY_BYTES: usize = 2 * 1024 * 1024;
pub(super) const MAX_TITLE_CHARS: usize = 200;
const MAX_ID_CHARS: usize = 128;
const MAX_SHORT_FIELD_CHARS: usize = 64;
const MAX_MODEL_CHARS: usize = 128;
const MAX_ERROR_CHARS: usize = 256;
const MAX_LEGACY_KEY_CHARS: usize = 512;
const MAX_TURN_TEXT_CHARS: usize = 65_536;
const MAX_TURN_ACTION_FIELD_CHARS: usize = 128;
const MAX_VALUE_STRING_CHARS: usize = 65_536;

/// Validate a safe single-component ASCII conversation identifier.
pub(super) fn validate_conversation_id(id: &str) -> Result<(), CutError> {
    let valid = !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !valid {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("invalid conversation_id '{id}'"),
            "pass an ASCII conversation_id using letters, digits, '-' or '_'",
        ));
    }
    Ok(())
}

pub(super) fn chat_dir(project_dir: &Path) -> PathBuf {
    project_dir.join("chat")
}

pub(super) fn conversation_path(project_dir: &Path, id: &str) -> PathBuf {
    chat_dir(project_dir).join(format!("{id}.json"))
}

fn refuse_symlink_dir(path: &Path) -> Result<(), CutError> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(CutError::new(
                error_codes::IO,
                format!("cannot inspect project chat folder: {error}"),
                "check permissions on the project folder",
            ));
        }
    };
    if meta.file_type().is_symlink() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "project chat folder cannot be a symbolic link",
            "replace the project's chat symlink with a normal directory",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "project chat folder cannot be a reparse point",
                "replace the project's chat junction with a normal directory",
            ));
        }
    }
    if !meta.is_dir() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "project chat path is not a folder",
            "remove the stray 'chat' file inside the project folder",
        ));
    }
    Ok(())
}

pub(super) fn ensure_chat_dir(project_dir: &Path) -> Result<PathBuf, CutError> {
    let dir = chat_dir(project_dir);
    refuse_symlink_dir(&dir)?;
    std::fs::create_dir_all(&dir).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("cannot create project chat directory: {e}"),
            "check write permissions on the project folder",
        )
    })?;
    refuse_symlink_dir(&dir)?;
    Ok(dir)
}

pub(super) enum LoadedRecord {
    Readable(ConversationRecord),
    Unreadable {
        id: String,
        title: String,
        reason: String,
    },
    Missing,
}

pub(super) fn read_conversation_file(
    project_dir: &Path,
    id: &str,
) -> Result<LoadedRecord, CutError> {
    validate_conversation_id(id)?;
    let dir = chat_dir(project_dir);
    refuse_symlink_dir(&dir)?;
    match std::fs::symlink_metadata(&dir) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LoadedRecord::Missing);
        }
        Err(error) => {
            return Err(CutError::new(
                error_codes::IO,
                format!("cannot inspect project chat folder: {error}"),
                "check permissions on the project folder",
            ));
        }
    }
    let path = conversation_path(project_dir, id);
    if matches!(
        std::fs::symlink_metadata(&path),
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound
    ) {
        return Ok(LoadedRecord::Missing);
    }
    let bytes = match read_bounded_cache(&path, MAX_CONVERSATION_FILE_BYTES) {
        BoundedCacheRead::Bytes(b) => b,
        BoundedCacheRead::Oversized => {
            return Ok(LoadedRecord::Unreadable {
                id: id.to_string(),
                title: id.to_string(),
                reason: "This conversation is too large to display in the Chat tab.".into(),
            });
        }
        BoundedCacheRead::Unavailable => {
            return Ok(LoadedRecord::Unreadable {
                id: id.to_string(),
                title: id.to_string(),
                reason: "This conversation file cannot be read safely.".into(),
            });
        }
    };
    let Ok(val) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(LoadedRecord::Unreadable {
            id: id.to_string(),
            title: id.to_string(),
            reason: "This conversation file is incomplete or damaged.".into(),
        });
    };
    let Some(schema) = val.get("schema").and_then(Value::as_str) else {
        return Ok(LoadedRecord::Unreadable {
            id: id.to_string(),
            title: val_title(&val, id),
            reason: "This conversation format is unrecognized and cannot be opened here.".into(),
        });
    };
    if schema != CHAT_CONVERSATION_SCHEMA {
        let short_schema = bound_chars(schema, 48);
        return Ok(LoadedRecord::Unreadable {
            id: id.to_string(),
            title: val_title(&val, id),
            reason: format!(
                "Saved in an unsupported format ({short_schema}) and cannot be opened here."
            ),
        });
    }
    let Ok(mut record) = serde_json::from_value::<ConversationRecord>(val) else {
        return Ok(LoadedRecord::Unreadable {
            id: id.to_string(),
            title: id.to_string(),
            reason: "This conversation record is missing required fields.".into(),
        });
    };
    if let Some(reason) = oversized_record_reason(&record) {
        return Ok(LoadedRecord::Unreadable {
            id: id.to_string(),
            title: bound_unreadable_title(&record.title, id),
            reason,
        });
    }
    if record.id != id {
        record.id = id.to_string();
    }
    scrub_record(&mut record);
    Ok(LoadedRecord::Readable(record))
}

fn bound_chars(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut chars = input.chars();
    let mut out = String::new();
    for _ in 0..max_chars {
        match chars.next() {
            Some(ch) => out.push(ch),
            None => return out,
        }
    }
    if chars.next().is_some() {
        out.pop();
        out.push('…');
    }
    out
}

fn bound_unreadable_title(raw: &str, fallback: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    // Bound input before running secret regex/scanning so a multi-megabyte title
    // cannot trigger megabyte scans before summary construction.
    let pre_bounded: String = trimmed.chars().take(MAX_TITLE_CHARS * 4).collect();
    let redacted = redact_secrets(&pre_bounded);
    let collapsed = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    let bounded = bound_chars(&collapsed, MAX_TITLE_CHARS);
    if bounded.is_empty() {
        fallback.to_string()
    } else {
        bounded
    }
}

fn val_title(val: &Value, fallback: &str) -> String {
    val.get("title")
        .and_then(Value::as_str)
        .map(|s| bound_unreadable_title(s, fallback))
        .unwrap_or_else(|| fallback.to_string())
}

fn exceeds_chars(s: &str, max_chars: usize) -> bool {
    s.len() > max_chars && s.chars().count() > max_chars
}

fn value_has_oversized_string(val: &Value, max_chars: usize) -> bool {
    match val {
        Value::String(s) => exceeds_chars(s, max_chars),
        Value::Array(items) => items
            .iter()
            .any(|item| value_has_oversized_string(item, max_chars)),
        Value::Object(map) => map
            .values()
            .any(|v| value_has_oversized_string(v, max_chars)),
        _ => false,
    }
}

fn oversized_record_reason(record: &ConversationRecord) -> Option<String> {
    if exceeds_chars(&record.id, MAX_ID_CHARS)
        || exceeds_chars(&record.title, MAX_TITLE_CHARS)
        || exceeds_chars(&record.origin, MAX_SHORT_FIELD_CHARS)
        || exceeds_chars(&record.created_at, MAX_SHORT_FIELD_CHARS)
        || exceeds_chars(&record.updated_at, MAX_SHORT_FIELD_CHARS)
        || record
            .source_legacy_key
            .as_deref()
            .is_some_and(|s| exceeds_chars(s, MAX_LEGACY_KEY_CHARS))
    {
        return Some(
            "This conversation record has a metadata field that exceeds the maximum length.".into(),
        );
    }
    if record.turns.len() > super::MAX_TURNS_PER_CONVERSATION {
        return Some("This conversation has more turns than this version can display.".into());
    }
    for turn in &record.turns {
        if exceeds_chars(&turn.id, MAX_ID_CHARS)
            || exceeds_chars(&turn.role, MAX_SHORT_FIELD_CHARS)
            || exceeds_chars(&turn.text, MAX_TURN_TEXT_CHARS)
            || exceeds_chars(&turn.created_at, MAX_SHORT_FIELD_CHARS)
            || turn
                .origin
                .as_deref()
                .is_some_and(|s| exceeds_chars(s, MAX_SHORT_FIELD_CHARS))
            || turn
                .status
                .as_deref()
                .is_some_and(|s| exceeds_chars(s, MAX_SHORT_FIELD_CHARS))
            || turn
                .agent
                .as_deref()
                .is_some_and(|s| exceeds_chars(s, MAX_SHORT_FIELD_CHARS))
            || turn
                .model
                .as_deref()
                .is_some_and(|s| exceeds_chars(s, MAX_MODEL_CHARS))
            || turn
                .error
                .as_deref()
                .is_some_and(|s| exceeds_chars(s, MAX_ERROR_CHARS))
            || turn
                .evidence_index_id
                .as_deref()
                .is_some_and(|s| exceeds_chars(s, MAX_ID_CHARS))
            || turn.actions.iter().any(|a| {
                exceeds_chars(&a.op_id, MAX_TURN_ACTION_FIELD_CHARS)
                    || exceeds_chars(&a.verb, MAX_TURN_ACTION_FIELD_CHARS)
            })
            || turn
                .attachments
                .iter()
                .any(|s| exceeds_chars(s, MAX_ID_CHARS))
            || turn
                .evidence_ids
                .iter()
                .any(|s| exceeds_chars(s, MAX_ID_CHARS))
            || turn
                .target
                .as_ref()
                .is_some_and(|v| value_has_oversized_string(v, MAX_VALUE_STRING_CHARS))
            || turn
                .plan
                .as_ref()
                .is_some_and(|v| value_has_oversized_string(v, MAX_VALUE_STRING_CHARS))
            || turn
                .review
                .as_ref()
                .is_some_and(|v| value_has_oversized_string(v, MAX_VALUE_STRING_CHARS))
        {
            return Some(
                "This conversation contains a turn field that exceeds the maximum length.".into(),
            );
        }
    }
    None
}

fn bound_string_in_place(s: &mut String, max_chars: usize) {
    if exceeds_chars(s, max_chars) {
        *s = bound_chars(s, max_chars);
    }
}

fn bound_value_strings(val: &mut Value, max_chars: usize) {
    match val {
        Value::String(s) => bound_string_in_place(s, max_chars),
        Value::Array(items) => {
            for item in items {
                bound_value_strings(item, max_chars);
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                bound_value_strings(v, max_chars);
            }
        }
        _ => {}
    }
}

pub(super) fn scrub_record(record: &mut ConversationRecord) {
    record.title = redact_secrets(&record.title);
    bound_string_in_place(&mut record.id, MAX_ID_CHARS);
    bound_string_in_place(&mut record.title, MAX_TITLE_CHARS);
    bound_string_in_place(&mut record.origin, MAX_SHORT_FIELD_CHARS);
    bound_string_in_place(&mut record.created_at, MAX_SHORT_FIELD_CHARS);
    bound_string_in_place(&mut record.updated_at, MAX_SHORT_FIELD_CHARS);
    if let Some(key) = &mut record.source_legacy_key {
        bound_string_in_place(key, MAX_LEGACY_KEY_CHARS);
    }
    for turn in &mut record.turns {
        scrub_turn(turn);
    }
}

pub(super) fn scrub_turn(turn: &mut ChatTurnRecord) {
    turn.text = redact_secrets(&turn.text);
    turn.error = turn.error.as_deref().map(redact_secrets);
    bound_string_in_place(&mut turn.id, MAX_ID_CHARS);
    bound_string_in_place(&mut turn.role, MAX_SHORT_FIELD_CHARS);
    bound_string_in_place(&mut turn.text, MAX_TURN_TEXT_CHARS);
    bound_string_in_place(&mut turn.created_at, MAX_SHORT_FIELD_CHARS);
    if let Some(origin) = &mut turn.origin {
        bound_string_in_place(origin, MAX_SHORT_FIELD_CHARS);
    }
    if let Some(status) = &mut turn.status {
        bound_string_in_place(status, MAX_SHORT_FIELD_CHARS);
    }
    if let Some(agent) = &mut turn.agent {
        bound_string_in_place(agent, MAX_SHORT_FIELD_CHARS);
    }
    if let Some(model) = &mut turn.model {
        bound_string_in_place(model, MAX_MODEL_CHARS);
    }
    if let Some(error) = &mut turn.error {
        bound_string_in_place(error, MAX_ERROR_CHARS);
    }
    if let Some(index_id) = &mut turn.evidence_index_id {
        bound_string_in_place(index_id, MAX_ID_CHARS);
    }
    for action in &mut turn.actions {
        bound_string_in_place(&mut action.op_id, MAX_TURN_ACTION_FIELD_CHARS);
        bound_string_in_place(&mut action.verb, MAX_TURN_ACTION_FIELD_CHARS);
    }
    for item in &mut turn.attachments {
        bound_string_in_place(item, MAX_ID_CHARS);
    }
    for item in &mut turn.evidence_ids {
        bound_string_in_place(item, MAX_ID_CHARS);
    }
    if let Some(plan) = &mut turn.plan {
        redact_value(plan);
        bound_value_strings(plan, MAX_VALUE_STRING_CHARS);
    }
    if let Some(review) = &mut turn.review {
        redact_value(review);
        bound_value_strings(review, MAX_VALUE_STRING_CHARS);
    }
    if let Some(target) = &mut turn.target {
        redact_value(target);
        bound_value_strings(target, MAX_VALUE_STRING_CHARS);
    }
}

pub(super) fn write_conversation_file(
    project_dir: &Path,
    record: &mut ConversationRecord,
) -> Result<(), CutError> {
    validate_conversation_id(&record.id)?;
    scrub_record(record);
    ensure_chat_dir(project_dir)?;
    let path = conversation_path(project_dir, &record.id);
    let bytes = serde_json::to_vec_pretty(record).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("cannot serialize conversation: {e}"),
            "report this conversation serialization error",
        )
    })?;
    if bytes.len() as u64 > MAX_CONVERSATION_FILE_BYTES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "conversation record exceeds the maximum file size",
            "reduce conversation turn payload size or start a new conversation",
        ));
    }
    replace_project_file(&path, &bytes).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("cannot save project conversation: {e}"),
            "check disk space and write permissions in the project folder",
        )
    })
}

pub(super) fn delete_conversation_file(project_dir: &Path, id: &str) -> Result<bool, CutError> {
    validate_conversation_id(id)?;
    let dir = chat_dir(project_dir);
    refuse_symlink_dir(&dir)?;
    match std::fs::symlink_metadata(&dir) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(CutError::new(
                error_codes::IO,
                format!("cannot inspect project chat folder: {error}"),
                "check permissions on the project folder",
            ));
        }
    }
    let filename = format!("{id}.json");
    delete_plain_single_link_file(&dir, &filename)
}

#[cfg(unix)]
#[allow(clippy::unnecessary_cast)]
fn delete_plain_single_link_file(dir: &Path, filename: &str) -> Result<bool, CutError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};

    let c_dir = CString::new(dir.as_os_str().as_bytes()).map_err(|_| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "invalid project chat folder path",
            "project chat folder path cannot contain NUL bytes",
        )
    })?;
    // SAFETY: NUL-terminated path; O_DIRECTORY | O_NOFOLLOW refuses a symlinked chat folder.
    let raw_dir = unsafe {
        libc::open(
            c_dir.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw_dir < 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::NotFound {
            return Ok(false);
        }
        if matches!(err.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENOTDIR)) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "project chat folder cannot be a symbolic link",
                "replace the project's chat symlink with a normal directory",
            ));
        }
        return Err(CutError::new(
            error_codes::IO,
            format!("cannot open project chat folder: {err}"),
            "check permissions on the project folder",
        ));
    }
    // SAFETY: `raw_dir` is a newly opened owned descriptor.
    let dir_fd = unsafe { OwnedFd::from_raw_fd(raw_dir) };
    let c_name = CString::new(filename.as_bytes()).map_err(|_| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "invalid conversation filename",
            "conversation filename cannot contain NUL bytes",
        )
    })?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::zeroed();
    // SAFETY: `dir_fd` is an open directory descriptor, `c_name` is NUL-terminated,
    // `AT_SYMLINK_NOFOLLOW` inspects the entry itself without following symlinks.
    if unsafe {
        libc::fstatat(
            dir_fd.as_raw_fd(),
            c_name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::NotFound {
            return Ok(false);
        }
        return Err(CutError::new(
            error_codes::IO,
            format!("cannot inspect conversation file: {err}"),
            "check permissions on the project chat folder",
        ));
    }
    // SAFETY: `fstatat` succeeded and initialized `stat`.
    let stat = unsafe { stat.assume_init() };
    let file_type = stat.st_mode as u32 & libc::S_IFMT as u32;
    if file_type == libc::S_IFLNK as u32 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "conversation file cannot be a symbolic link",
            "remove the symlinked conversation entry manually",
        ));
    }
    if file_type != libc::S_IFREG as u32 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "conversation entry is not a regular file",
            "remove the non-regular entry in the project chat folder manually",
        ));
    }
    if (stat.st_nlink as u64) != 1 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "conversation file cannot have multiple hard links",
            "remove extra hard links or delete the conversation file manually",
        ));
    }
    // SAFETY: `dir_fd` is the pinned non-symlink chat folder and `c_name` is a single component.
    if unsafe { libc::unlinkat(dir_fd.as_raw_fd(), c_name.as_ptr(), 0) } != 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::NotFound {
            return Ok(false);
        }
        return Err(CutError::new(
            error_codes::IO,
            format!("cannot delete conversation: {err}"),
            "check permissions on the project chat folder",
        ));
    }
    Ok(true)
}

#[cfg(windows)]
fn delete_plain_single_link_file(dir: &Path, filename: &str) -> Result<bool, CutError> {
    use std::fs::OpenOptions;
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::*;

    let dir_handle = match OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(dir)
    {
        Ok(handle) => handle,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(CutError::new(
                error_codes::IO,
                format!("cannot open project chat folder: {err}"),
                "check permissions on the project folder",
            ));
        }
    };
    let dir_tag = cut_core::reparse::handle_reparse_tag(&dir_handle).map_err(|err| {
        CutError::new(
            error_codes::IO,
            format!("cannot inspect project chat folder: {err}"),
            "check permissions on the project folder",
        )
    })?;
    if dir_tag.is_some() || !dir_handle.metadata().is_ok_and(|m| m.is_dir()) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "project chat folder cannot be a symbolic link or junction",
            "replace the project's chat link with a normal directory",
        ));
    }

    let path = dir.join(filename);
    let file = match OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(CutError::new(
                error_codes::IO,
                format!("cannot open conversation for delete: {err}"),
                "check permissions on the project chat folder",
            ));
        }
    };
    let file_tag = cut_core::reparse::handle_reparse_tag(&file).map_err(|err| {
        CutError::new(
            error_codes::IO,
            format!("cannot inspect conversation file: {err}"),
            "check permissions on the project chat folder",
        )
    })?;
    if file_tag.is_some_and(cut_core::reparse::is_name_surrogate_tag) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "conversation file cannot be a symbolic link or reparse point",
            "remove the linked conversation entry manually",
        ));
    }
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
        let err = std::io::Error::last_os_error();
        return Err(CutError::new(
            error_codes::IO,
            format!("cannot inspect conversation file: {err}"),
            "check permissions on the project chat folder",
        ));
    }
    if (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "conversation entry is not a regular file",
            "remove the folder in the project chat directory manually",
        ));
    }
    if info.nNumberOfLinks != 1 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "conversation file cannot have multiple hard links",
            "remove extra hard links or delete the conversation file manually",
        ));
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle() as _,
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    } == 0
    {
        let err = std::io::Error::last_os_error();
        return Err(CutError::new(
            error_codes::IO,
            format!("cannot delete conversation: {err}"),
            "check permissions on the project chat folder",
        ));
    }
    drop(file);
    drop(dir_handle);
    Ok(true)
}

#[cfg(not(any(unix, windows)))]
fn delete_plain_single_link_file(dir: &Path, filename: &str) -> Result<bool, CutError> {
    let path = dir.join(filename);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(CutError::new(
                error_codes::IO,
                format!("cannot inspect conversation file: {err}"),
                "check permissions",
            ));
        }
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "conversation entry must be a regular file",
            "remove the non-regular entry manually",
        ));
    }
    std::fs::remove_file(&path).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("cannot delete conversation: {e}"),
            "check permissions",
        )
    })?;
    Ok(true)
}

pub(super) fn scan_chat_ids(project_dir: &Path) -> Result<Vec<String>, CutError> {
    scan_chat_ids_bounded(project_dir, MAX_CONVERSATION_FILES).map(|(ids, _)| ids)
}

pub(super) fn scan_chat_ids_bounded(
    project_dir: &Path,
    limit: usize,
) -> Result<(Vec<String>, usize), CutError> {
    let dir = chat_dir(project_dir);
    refuse_symlink_dir(&dir)?;
    match std::fs::symlink_metadata(&dir) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), 0));
        }
        Err(error) => {
            return Err(CutError::new(
                error_codes::IO,
                format!("cannot inspect project chat folder: {error}"),
                "check permissions on the project folder",
            ));
        }
    }
    let entries = std::fs::read_dir(&dir).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("cannot read project chat folder: {e}"),
            "check permissions on the project folder",
        )
    })?;
    let mut ids = Vec::new();
    let mut extra = 0usize;
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() || !ft.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if let Some(stem) = name_str.strip_suffix(".json") {
            if validate_conversation_id(stem).is_ok() {
                if ids.len() < limit {
                    ids.push(stem.to_string());
                } else {
                    extra += 1;
                }
            }
        }
    }
    ids.sort();
    Ok((ids, extra))
}

pub(super) fn interrupted_turn(ts: &str) -> ChatTurnRecord {
    ChatTurnRecord {
        id: format!("interrupted_{ts}"),
        role: "agent".into(),
        text: "The engine stopped before this turn finished.".into(),
        created_at: ts.to_string(),
        origin: None,
        status: Some("interrupted".into()),
        agent: None,
        model: None,
        error: Some("interrupted".into()),
        actions: Vec::<ChatActionRef>::new(),
        attachments: Vec::new(),
        evidence_ids: Vec::new(),
        evidence_index_id: None,
        target: None,
        plan: None,
        review: None,
        cost_usd: None,
        reverted: false,
    }
}

pub(super) fn make_title(text: &str) -> String {
    let single = redact_secrets(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut out = String::new();
    for ch in single.chars().take(72) {
        out.push(ch);
    }
    if single.chars().count() > 72 {
        out.push('…');
    }
    if out.is_empty() {
        "Conversation".into()
    } else {
        out
    }
}

pub(super) fn summary_from_record(
    record: &ConversationRecord,
    turns: &[ChatTurnRecord],
    running: bool,
) -> ConversationSummary {
    let last_preview = turns
        .iter()
        .rev()
        .map(|t| t.text.trim())
        .find(|t| !t.is_empty())
        .map(make_title);
    ConversationSummary {
        id: record.id.clone(),
        title: record.title.clone(),
        origin: record.origin.clone(),
        readable: true,
        unreadable_reason: None,
        running,
        turn_count: turns.len(),
        created_at: Some(record.created_at.clone()),
        updated_at: Some(record.updated_at.clone()),
        last_preview,
    }
}

pub(super) fn now_ts() -> String {
    OpRecord::now_ts()
}
