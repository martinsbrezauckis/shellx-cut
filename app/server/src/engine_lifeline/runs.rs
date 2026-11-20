//! Each serving engine leaves a small record of its run's mark in the app's
//! data folder (`<cut home>/jobs/running/run-<pid>.json`). If an engine and
//! its watcher were both killed, the next start finds the record of a run
//! whose engine is gone and stops whatever still carries that mark, also
//! helpers of work no one is told about (a proxy, a waveform). The record
//! is removed then; a live engine's record is never touched.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const SCHEMA: &str = "shellx-cut/engine-run/1";

#[derive(Serialize, Deserialize)]
struct Run {
    schema: String,
    mark: String,
    owner_pid: u32,
    owner_identity: String,
}

fn folder() -> Option<PathBuf> {
    crate::userdata::shellx_cut_home().map(|home| home.join("jobs").join("running"))
}

/// Remember this run's mark for a later start.
pub(super) fn record(mark: &str, owner_identity: &str) {
    let Some(dir) = folder() else { return };
    let run = Run {
        schema: SCHEMA.into(),
        mark: mark.into(),
        owner_pid: std::process::id(),
        owner_identity: owner_identity.into(),
    };
    let path = dir.join(format!("run-{}.json", std::process::id()));
    let written = std::fs::create_dir_all(&dir).and_then(|()| {
        let bytes = serde_json::to_vec(&run).map_err(std::io::Error::other)?;
        record_recovery::replace_synced(&path, &bytes)
    });
    if let Err(error) = written {
        eprintln!("[cutd] could not record this run's helper mark: {error}");
    }
}

fn owner_gone(run: &Run) -> bool {
    if run.owner_pid == std::process::id() {
        return false;
    }
    let start = record_recovery::CaptureStart {
        schema: String::new(),
        capture_id: String::new(),
        owner_pid: run.owner_pid,
        owner_identity: run.owner_identity.clone(),
        owner_nonce: String::new(),
        started_unix_ms: 0,
        checkpoint_interval_ms: 0,
        output_cadence: None,
    };
    record_recovery::owner_state(&start) == record_recovery::OwnerState::Dead
}

/// Stop the leftovers of every run whose engine is gone; returns how many
/// processes were stopped.
pub(crate) fn stop_dead_runs() -> usize {
    let Some(dir) = folder() else { return 0 };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut stopped = 0;
    for entry in entries.flatten().take(256) {
        let path = entry.path();
        let named = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("run-") && name.ends_with(".json"));
        if !named
            || cut_core::store::plain_file_state(&path) != cut_core::store::PlainFileState::Plain
        {
            continue;
        }
        let Some(run) = std::fs::read(&path)
            .ok()
            .filter(|bytes| bytes.len() <= 4096)
            .and_then(|bytes| serde_json::from_slice::<Run>(&bytes).ok())
            .filter(|run| run.schema == SCHEMA && super::valid_mark(&run.mark))
        else {
            continue;
        };
        if owner_gone(&run) {
            stopped += super::stop_marked(&run.mark);
            let _ = std::fs::remove_file(&path);
        }
    }
    stopped
}
