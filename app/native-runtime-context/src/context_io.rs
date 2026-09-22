//! Bounded locator reads shared by the typed Runner context consumers.

use crate::{regular_no_link, CONTEXT_ENV, CONTEXT_MAX_BYTES};
use serde::Deserialize;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Read a bounded, regular, absolute context locator before either typed parser
/// inspects it. This does not authorize the schema; each consumer validates its
/// full typed payload independently.
pub fn context_schema_from_env() -> Result<Option<String>, String> {
    match std::env::var_os(CONTEXT_ENV) {
        None => Ok(None),
        Some(path) if path.is_empty() => Err("native runtime context locator is empty".into()),
        Some(path) => {
            let bytes = read_context(Path::new(&path))?;
            #[derive(Deserialize)]
            struct Envelope {
                schema: String,
            }
            let envelope: Envelope = serde_json::from_slice(&bytes)
                .map_err(|e| format!("parse native runtime context schema: {e}"))?;
            Ok(Some(envelope.schema))
        }
    }
}

pub(crate) fn read_context(path: &Path) -> Result<Vec<u8>, String> {
    if !path.is_absolute() {
        return Err("native runtime context locator must be absolute".into());
    }
    regular_no_link(path, "native runtime context")?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| format!("open native runtime context: {e}"))?
        .take(CONTEXT_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("read native runtime context: {e}"))?;
    if bytes.len() as u64 > CONTEXT_MAX_BYTES {
        return Err("native runtime context exceeds 1 MiB".into());
    }
    Ok(bytes)
}
