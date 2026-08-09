//! store.rs — project-dir persistence + log replay (timeline/op-log contract "Project dir").
//!
//! Role: owns the `<name>.cutproj/` layout — project.json (cache), ops.jsonl
//! (truth), receipts/, proxies/ — and the load/save/rebuild lifecycle, plus
//! the SINGLE commit path every mutation takes:
//!
//!   live:   ProjectStore::apply(verb, args, actor, rationale)
//!             → apply_edit_verb on a CLONE (transactional: errors can't
//!               half-mutate), append OpRecord, save cache
//!   replay: rebuild_from_log → apply_record per op (uses recorded args
//!             AND effects — checkpoint/import payloads live in effects)
//!
//! UNDO model — recompute-by-replay, replacing per-op snapshots.
//! Mutating ops carry NO inverse (the old full-timeline snapshot per op was
//! O(N²) disk growth in ops.jsonl). Undo/restore/revert RECOMPUTE the needed
//! pre-op timeline by replaying the log prefix (the determinism contract makes
//! that exactly the snapshot the old model stored), and the restore/rebase op
//! records its computed RESULT timeline in its effect so replay reproduces it
//! directly. "Is this a timeline op?" is answered by OpRecord::mutates_timeline()
//! (verb-based), not by the presence of an inverse. Historic logs whose ops
//! still carry snapshot inverses replay unchanged (apply_record falls back to
//! the recorded snapshot when a restore op has no computed-result effect).
//!
//! Determinism contract (timeline/op-log contract + tests/roundtrip.rs): replaying the same
//! ops.jsonl always yields a byte-identical project.json. That is why
//! project.create and media.import are THEMSELVES ops (the append-only operation-log contract) — name,
//! settings and asset payloads must come from the log, never from wall-clock
//! or caller state. The recompute-by-replay undo model leans entirely on this
//! contract (undo == replay), so the determinism gate IS the undo-correctness gate.
//!
//! Lowering escape hatch: verbs core doesn't know (e.g. transcript.cut_words)
//! replay via a `lowered` effects entry — an array of {verb, args} core edit
//! ops recorded by the layer that lowered them. The log keeps the honest verb
//! name; replay stays deterministic.
//!
//! Dependencies: types.rs, ops.rs, edit.rs, error.rs. Primary callers: server
//! (project.* verbs) and integration tests.

use crate::edit;
use crate::error::{codes, CutError, VerbWarning};
use crate::ops::{Actor, InverseOp, OpEffect, OpLog, OpRecord, OpStatus};
use crate::types::{
    Asset, BrandKit, Checkpoint, Clip, Project, ProjectSettings, Sequence, Track,
    DEFAULT_SEQUENCE_ID,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

mod commit_state;
mod history;
mod name_policy;
mod open_health;
#[cfg(test)]
mod open_health_tests;
mod snapshots;
#[cfg(test)]
mod speed_ramp_replay_tests;

use name_policy::{validate_logged_project_name, validate_project_name};
pub use open_health::{ProjectCacheHealth, ProjectOpenHealth, ProjectSnapshotHealth};

/// A project on disk: the materialized state + its op-log + dir layout.
#[derive(Debug)]
pub struct ProjectStore {
    /// Absolute path of the `<name>.cutproj` directory.
    pub dir: PathBuf,
    /// Materialized state (cache of the log).
    pub project: Project,
    /// The append-only log handle.
    pub log: OpLog,
    /// Linear undo/redo history: op ids in edit order.
    /// Element 0 is the `project.create` baseline (empty timeline); each later
    /// entry is one FORWARD timeline edit ([`Self::is_history_edit`]). The
    /// history vector is reconstructed from forward edits in the immutable
    /// log; the logical cursor is reconstructed from the appended
    /// `project.undo` / `project.redo` navigation ops. See [`Self::commit`],
    /// [`Self::undo`], [`Self::redo`].
    pub undo_history: Vec<String>,
    /// Cursor into [`Self::undo_history`]: index of the edit whose timeline is
    /// currently live. A new edit truncates the redo future then pushes (cursor
    /// → tip); `undo` decrements; `redo` increments.
    pub undo_pos: usize,
    /// The `group_id` of the op the cursor tip currently points at
    /// (or `None` for a standalone edit). When a new committed edit carries the
    /// SAME group tag, it EXTENDS the tip entry instead of pushing a new one, so
    /// a linked A/V paste / linked delete is ONE Ctrl+Z. Reset to `None` after an
    /// undo/redo (a post-undo edit never merges into a pre-undo group; group tags
    /// are fresh per action anyway). In-memory only.
    pub tip_group: Option<String>,
    /// Non-fatal cache-refresh warnings keyed by the durably committed op.
    /// The server drains only warnings belonging to the current response's
    /// `op_ids`, so concurrent callers cannot steal or misattribute them.
    commit_warnings: BTreeMap<String, Vec<VerbWarning>>,
    /// The last strict project creation/open outcome. It is disclosure-only:
    /// callers must still ask `OpLog` to validate live journal identity before
    /// treating materialized state as current.
    open_health: ProjectOpenHealth,
}

#[derive(Debug, Clone)]
pub struct AtomicMediaInsertPlanResult {
    pub checkpoint: Checkpoint,
    pub asset_ids: Vec<String>,
    pub clip_ids: Vec<String>,
    pub op: OpRecord,
    pub already_applied: bool,
}

#[derive(Debug, Clone)]
pub struct MotionLinkRefreshResult {
    pub asset_id: String,
    pub op: OpRecord,
}

fn asset_id_number(id: &str) -> Option<u64> {
    id.strip_prefix('a').and_then(|x| x.parse::<u64>().ok())
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn imported_asset_ids(op: &OpRecord) -> Vec<String> {
    match op.verb.as_str() {
        "media.import" => op
            .effects
            .iter()
            .find_map(|e| e.detail.get("asset_id")?.as_str().map(str::to_string))
            .into_iter()
            .collect(),
        "import.otio" | "motion.apply_import" | "motion.link.refresh" => op
            .effects
            .iter()
            .find_map(|e| e.detail.get("assets"))
            .and_then(Value::as_object)
            .map(|assets| assets.keys().cloned().collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn motion_import_detail(op: &OpRecord) -> Result<&serde_json::Map<String, Value>, CutError> {
    op.effects
        .iter()
        .map(|effect| &effect.detail)
        .find(|detail| {
            detail.get("atomic_media_insert").is_some()
                || detail.get("motion_editable_import").is_some()
        })
        .ok_or_else(|| replay_corrupt(op, "Motion import effect is missing"))
}

fn motion_assets_from_record(op: &OpRecord) -> Result<Option<BTreeMap<String, Asset>>, CutError> {
    if op.verb == "motion.link.refresh" {
        let assets = op
            .effects
            .iter()
            .find_map(|effect| effect.detail.get("assets"))
            .cloned()
            .ok_or_else(|| replay_corrupt(op, "Motion refresh assets are missing"))?;
        return Ok(Some(serde_json::from_value(assets)?));
    }
    if op.verb != "motion.apply_import" {
        return Ok(None);
    }
    let detail = motion_import_detail(op)?;
    if detail.get("motion_editable_import").is_some() {
        return Ok(Some(BTreeMap::new()));
    }
    let assets = detail
        .get("assets")
        .cloned()
        .ok_or_else(|| replay_corrupt(op, "Motion import assets are missing"))?;
    Ok(Some(serde_json::from_value(assets)?))
}

fn motion_assets_for_op(
    ops: &[OpRecord],
    op_id: &str,
) -> Result<Option<BTreeMap<String, Asset>>, CutError> {
    let Some(op) = ops.iter().find(|op| op.op_id == op_id) else {
        return Ok(None);
    };
    motion_assets_from_record(op)
}

fn motion_checkpoint_for_op(ops: &[OpRecord], op_id: &str) -> Result<Option<Checkpoint>, CutError> {
    let Some(op) = ops
        .iter()
        .find(|op| op.op_id == op_id && op.verb == "motion.apply_import")
    else {
        return Ok(None);
    };
    let detail = motion_import_detail(op)?;
    let checkpoint = detail
        .get("checkpoint")
        .cloned()
        .ok_or_else(|| replay_corrupt(op, "Motion checkpoint is missing"))?;
    Ok(Some(serde_json::from_value(checkpoint)?))
}

fn motion_assets_for_checkpoint(
    ops: &[OpRecord],
    checkpoint_id: &str,
) -> Result<Option<BTreeMap<String, Asset>>, CutError> {
    for op in ops.iter().filter(|op| op.verb == "motion.apply_import") {
        let detail = motion_import_detail(op)?;
        if detail
            .get("checkpoint")
            .and_then(|checkpoint| checkpoint.get("id"))
            .and_then(Value::as_str)
            == Some(checkpoint_id)
        {
            return motion_assets_from_record(op);
        }
    }
    Ok(None)
}

fn op_sequence_assignments(ops: &[OpRecord]) -> Result<Vec<String>, CutError> {
    let mut active = DEFAULT_SEQUENCE_ID.to_string();
    let mut assignments = Vec::new();
    for op in ops {
        match op.verb.as_str() {
            "project.sequence_create" => {
                active = op
                    .effects
                    .iter()
                    .find_map(|effect| effect.detail.get("sequence"))
                    .and_then(|sequence| sequence.get("id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CutError::new(
                            codes::INVALID_ARGS,
                            "sequence-create op is missing its recorded sequence id",
                            format!("op '{}' cannot be assigned to a sequence", op.op_id),
                        )
                    })?
                    .to_string();
            }
            "project.sequence_switch" => {
                active = op
                    .args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CutError::new(
                            codes::INVALID_ARGS,
                            "sequence-switch op is missing id",
                            format!("op '{}' cannot be assigned to a sequence", op.op_id),
                        )
                    })?
                    .to_string();
            }
            _ => {}
        }
        assignments.push(active.clone());
    }
    Ok(assignments)
}

fn project_without_derived_asset_cache(project: &Project) -> Project {
    let mut project = project.clone();
    for asset in project.assets.values_mut() {
        asset.probe = None;
        asset.transcript = None;
        asset.perception = None;
        asset.proxy = None;
        asset.filmstrip = None;
    }
    project
}

fn cache_matches_log(cached: &Project, rebuilt: &Project) -> bool {
    project_without_derived_asset_cache(cached) == project_without_derived_asset_cache(rebuilt)
}

impl ProjectStore {
    /// Create a fresh `<name>.cutproj` under `parent_dir` with default tracks,
    /// the receipts/ + proxies/ subdirs, and op_000001 = `project.create`
    /// (the append-only operation-log contract spirit: the log alone must reproduce the project, including
    /// its name and settings). Errors `conflict` if the directory exists.
    pub fn create(
        parent_dir: &Path,
        name: &str,
        settings: Option<ProjectSettings>,
    ) -> Result<Self, CutError> {
        Self::create_with_actor(parent_dir, name, settings, Actor::system())
    }

    /// Create a project while preserving the external caller identity on the
    /// initial `project.create` operation.
    pub fn create_with_actor(
        parent_dir: &Path,
        name: &str,
        settings: Option<ProjectSettings>,
        actor: Actor,
    ) -> Result<Self, CutError> {
        validate_project_name(name)?;
        let dir = parent_dir.join(format!("{name}.cutproj"));
        if dir.exists() {
            return Err(CutError::new(
                codes::CONFLICT,
                format!("project dir already exists: {}", dir.display()),
                "use project.open to open it, or pick another name",
            ));
        }
        let cleanup_dir = dir.clone();
        let result = (|| -> Result<Self, CutError> {
            std::fs::create_dir_all(dir.join("receipts"))?;
            std::fs::create_dir_all(dir.join("proxies"))?;
            std::fs::create_dir_all(dir.join("filmstrip"))?;
            // Canonicalize the dir to an absolute path NOW, at the create boundary,
            // exactly as open() does (the project-path contract, regression. A relative `dir`
            // (e.g. ".scratch/nested/x.cutproj") otherwise stays relative on the
            // store, and every project-internal path derived from it — exports/,
            // receipts/<id>.output.perception.json — gets resolved RELATIVE to the
            // server cwd by downstream consumers. Worse, the output PathFence
            // canonicalizes the project root but joins a still-relative candidate
            // against it, DOUBLING the path (<proj>/<relative-proj>/exports). The
            // dirs were just created above so canonicalize() resolves cleanly here;
            // it only fails on exotic FS states, where we fall back to the joined
            // path rather than refuse to create (parity with open()'s fallback).
            let dir = dir.canonicalize().unwrap_or(dir);
            let settings = settings.unwrap_or_default();
            let project = Project::new(name, settings.clone());
            let log = OpLog::open(&dir.join("ops.jsonl"))?;
            let mut store = Self {
                dir,
                project,
                log,
                undo_history: Vec::new(),
                undo_pos: 0,
                tip_group: None,
                commit_warnings: BTreeMap::new(),
                open_health: ProjectOpenHealth::new_project(),
            };
            let rec = OpRecord {
                op_id: store.log.next_id()?,
                ts: OpRecord::now_ts(),
                actor,
                verb: "project.create".into(),
                args: json!({"name": name, "settings": settings}),
                rationale: None,
                effects: vec![],
                inverse: None, // creation is not undoable
                status: OpStatus::Applied,
            };
            store.commit(&rec)?;
            // Seed the undo history with the create op as element 0 (the empty-
            // timeline baseline). project.create is NOT an is_history_edit, so the
            // commit above left the history empty — set it explicitly here.
            store.undo_history = vec![rec.op_id.clone()];
            store.undo_pos = 0;
            Ok(store)
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&cleanup_dir);
        }
        result
    }

    /// Open an existing project dir. Loads project.json; if it is missing or
    /// corrupt, rebuilds it from ops.jsonl (the log is the truth).
    pub fn open(dir: &Path) -> Result<Self, CutError> {
        if !dir.is_dir() {
            return Err(CutError::new(
                codes::NOT_FOUND,
                format!("no project dir at {}", dir.display()),
                "pass the .cutproj directory path",
            ));
        }
        // Derived-output roots are part of the live project layout. Background
        // workers require these directories to already exist so a late worker
        // cannot recreate a project after project.delete removes it.
        std::fs::create_dir_all(dir.join("receipts"))?;
        std::fs::create_dir_all(dir.join("proxies"))?;
        std::fs::create_dir_all(dir.join("filmstrip"))?;
        let log = OpLog::open(&dir.join("ops.jsonl"))?;
        let pj = dir.join("project.json");
        let cached = std::fs::read_to_string(&pj)
            .ok()
            .and_then(|s| serde_json::from_str::<Project>(&s).ok());
        // ops.jsonl is the source of truth. A syntactically-valid project.json
        // can still be stale after a crash/cache-write failure, so verify the
        // cache against a replay before trusting it. Derived asset pointers are
        // cache-only enrichment and are ignored for freshness; if the cache is
        // stale/missing/corrupt, rebuild and re-point what can be recovered from
        // deterministic sidecar files.
        let journal = log.replay_view()?;
        let ops = journal.records();
        let (mut rebuilt, replay) = snapshots::rebuild(dir, &journal, ops.len())?;
        // A malformed or stale snapshot is never truth. Rebuild from the
        // journal and refresh this disposable cache for the next open.
        if replay.rejected_snapshot {
            let _ = snapshots::write(dir, &journal, &rebuilt);
        }
        reconcile_derived_assets(&mut rebuilt, dir);
        let (project, cache) = match cached {
            Some(p) if cache_matches_log(&p, &rebuilt) => (p, ProjectCacheHealth::Matched),
            _ => (rebuilt, ProjectCacheHealth::Rebuilt),
        };
        // Heal track grouping at LOAD for BOTH paths (cached project.json and
        // log-rebuild): projects saved before track grouping may have interleaved
        // lanes (`[v1, a1t, v2, a2t]`). normalize_track_order stably partitions
        // them into `[Video…, Audio…, Caption…]` WITHOUT changing within-kind
        // order, so the video compositing z-order (first video = base canvas) is
        // preserved. Idempotent — a no-op on already-grouped projects.
        let mut project = project;
        project.normalize_track_order();
        // Canonicalize so a relative `--project` CLI path can't double up when
        // file-writing verbs join this dir with default rel paths and the path
        // fence re-resolves the still-relative result against the absolute
        // project root (regression: render.final default output dir
        // became <proj>/<relative-proj>/exports). is_dir() passed above, so
        // canonicalize only fails on exotic FS states; fall back rather than
        // refuse to open.
        // Reconstruct both the linear history and its logical cursor from the
        // journal. Navigation ops are durable state: reopening after an undo
        // must make the next undo step immediately older, never a no-op.
        let (undo_history, undo_pos) =
            Self::history_state_from_log(&log, &project.active_sequence)?;
        let journal_tail_recovery = log.recovery().cloned();
        Ok(Self {
            dir: dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()),
            project,
            log,
            undo_history,
            undo_pos,
            // A post-open edit never merges into a pre-open group (group tags are
            // fresh per action); start with no open group.
            tip_group: None,
            commit_warnings: BTreeMap::new(),
            open_health: ProjectOpenHealth {
                cache,
                snapshot: if replay.rejected_snapshot {
                    ProjectSnapshotHealth::Rejected
                } else if replay.snapshot_prefix > 0 {
                    ProjectSnapshotHealth::Verified {
                        prefix_ops: replay.snapshot_prefix,
                    }
                } else {
                    ProjectSnapshotHealth::NotPresent
                },
                journal_tail_recovery,
            },
        })
    }

    /// Recovery outcome from the most recent strict create/open. This does not
    /// replace live journal identity validation; use `log.current_revision()`
    /// before reporting current project membership.
    pub fn open_health(&self) -> &ProjectOpenHealth {
        &self.open_health
    }

    fn write_project_json(&self, encoded: &str) -> Result<(), CutError> {
        let dst = self.dir.join("project.json");
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let tmp = self
            .dir
            .join(format!("project.json.tmp.{}.{}", std::process::id(), nonce));
        let result = (|| -> Result<(), CutError> {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            f.write_all(encoded.as_bytes())?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp, &dst)?;
            if let Ok(dir) = std::fs::File::open(&self.dir) {
                let _ = dir.sync_all();
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    fn save_project(&self, project: &Project) -> Result<(), CutError> {
        self.write_project_json(&serde_json::to_string_pretty(project)?)
    }

    /// Write project.json atomically (tmp + rename) — the cache must never be
    /// observable half-written by a concurrent UI/agent read.
    pub fn save(&self) -> Result<(), CutError> {
        self.save_project(&self.project)
    }

    /// Apply a mutating verb end-to-end: validate + mutate (on a clone, so an
    /// error can never half-mutate the cache), record replayable effects,
    /// append the OpRecord, save. Returns the appended record (op_id, effects).
    ///
    /// Handles every `edit.*` verb incl. `edit.restore{op_id}` (rationale per
    /// the rationale-preservation contract is threaded onto the record verbatim).
    pub fn apply(
        &mut self,
        verb: &str,
        args: Value,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<OpRecord, CutError> {
        let mut next = self.project.clone(); // transactional: swap only on success
        let effects = if verb == "edit.restore" {
            #[derive(serde::Deserialize)]
            struct A {
                op_id: String,
                /// "tip" (default, today's behavior) | "rebase" (selective
                /// non-tip undo). serde default "tip" so legacy callers and the
                /// review-rail reject action keep the tip-only semantics.
                #[serde(default = "default_restore_mode")]
                mode: String,
            }
            fn default_restore_mode() -> String {
                "tip".into()
            }
            let a: A = serde_json::from_value(args.clone())?;
            // mode:"rebase" → selective non-tip undo. Delegated to rebase_out,
            // which runs the dependency gate + skip-replay + verify, then
            // appends its own op. We return that op directly (it already
            // committed), bypassing the recompute path below.
            if a.mode == "rebase" {
                let (rec, _rebased_over) = self.rebase_out(&a.op_id, actor, rationale)?;
                return Ok(rec);
            }
            if a.mode != "tip" {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!("unknown edit.restore mode '{}'", a.mode),
                    "mode must be \"tip\" (default) or \"rebase\"",
                ));
            }
            let journal = self.log.replay_view()?;
            let all = journal.records();
            let pos = all.iter().position(|o| o.op_id == a.op_id).ok_or_else(|| {
                CutError::new(
                    codes::NOT_FOUND,
                    format!("no op '{}' in the log", a.op_id),
                    "op ids come from project.ops",
                )
            })?;
            let assignments = op_sequence_assignments(all)?;
            let target_sequence = &assignments[pos];
            if target_sequence != &self.project.active_sequence {
                return Err(CutError::new(
                    codes::GUARDRAIL,
                    format!(
                        "op '{}' belongs to sequence '{}', not the active sequence '{}'",
                        a.op_id, target_sequence, self.project.active_sequence
                    ),
                    "undo and restore are sequence-scoped",
                )
                .with_suggested_action(format!(
                    "switch to sequence '{}' before restoring this op",
                    target_sequence
                )));
            }
            // selective-undo guardrail (mode:"tip" — the DEFAULT): a tip restore
            // recomputes the pre-op timeline by REPLAYING the log prefix, so a
            // tip restore of a non-tip op would silently roll the whole timeline
            // back to its pre-op state — discarding every later edit with
            // ok:true and zero warnings (the regression case lost a 16-op session
            // exactly this way). Refuse unless the target is the LATEST timeline
            // op. The HONEST path for selective non-tip undo is mode:"rebase"
            // (above), which is gated + verified; the tip guardrail STAYS the
            // default so the safe behavior never changes under existing callers.
            // Non-timeline ops (checkpoint, import) don't block.
            let later: Vec<(&str, &str)> = all[pos + 1..]
                .iter()
                .zip(&assignments[pos + 1..])
                .map(|(op, sequence)| {
                    if op.mutates_timeline()? && sequence == &self.project.active_sequence {
                        Ok(Some((op.op_id.as_str(), op.verb.as_str())))
                    } else {
                        Ok(None)
                    }
                })
                .collect::<Result<Vec<_>, CutError>>()?
                .into_iter()
                .flatten()
                .collect();
            if let Some(&(tip_id, tip_verb)) = later.last() {
                let n = later.len();
                return Err(CutError::new(
                    codes::GUARDRAIL,
                    format!(
                        "op '{}' is {n} timeline op(s) deep — restore would discard the {n} later op(s)",
                        a.op_id
                    ),
                    format!(
                        "tip restore recomputes the pre-op timeline by replay: restoring '{}' would \
                         roll the WHOLE timeline back to its pre-op state, silently discarding every \
                         later edit (latest timeline op: '{tip_id}' {tip_verb})",
                        a.op_id
                    ),
                )
                .with_suggested_action(format!(
                    "edit.restore (mode:\"tip\") undoes the LATEST timeline op only ('{tip_id}'); \
                     to selectively undo THIS older op while keeping the later ones, retry with \
                     mode:\"rebase\" (refused if a later op depends on it); for a full rollback to \
                     a point use project.revert{{to}}"
                )));
            }
            // Recompute-by-replay: the pre-target timeline is exactly the log
            // replayed up to (NOT including) the target op (the determinism gate
            // guarantees this equals the snapshot the old model stored). Apply
            // it, and record it on the op so replay reproduces the restore
            // directly without re-deriving (same pattern as a rebase op).
            let (pre, _) = snapshots::rebuild(&self.dir, &journal, pos)?;
            let snap = timeline_snapshot(&pre);
            let undid_verb = all[pos].verb.clone();
            edit::apply_set_timeline(&mut next, &snap.args)?;
            // `commit_staged` appends a navigation record. Release the replay
            // view before it so JournalIndex never COW-clones the full record
            // and prefix vectors on a history action.
            drop(journal);
            vec![edit::fx(
                None,
                json!({
                    "restored_op": a.op_id,
                    "mode": "tip",
                    "undid_verb": undid_verb,
                    "restored_timeline": snap.args,
                }),
            )]
        } else {
            apply_edit_verb(&mut next, verb, &args)?
        };
        // Carry the optional `group_id` meta-arg as an effect so the
        // undo cursor collapses the linked action (read before `args` is moved).
        let mut effects = effects;
        if let Some(ge) = group_effect(&args) {
            effects.push(ge);
        }
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: verb.to_string(),
            args,
            rationale,
            effects,
            // Recompute-by-replay: mutating ops carry NO snapshot
            // inverse — that per-op full-timeline copy was the O(N²) disk
            // growth. Undo state is recomputed from the log on demand; "is this
            // a timeline op?" is answered by OpRecord::mutates_timeline(), and
            // edit.restore/rebase carry their computed result timeline in their
            // effects instead.
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok(rec)
    }

    /// Apply a HIGHER-LAYER verb (transcript.*, captions.*) by its lowered
    /// core-edit steps — the store-header escape hatch made first-class. Runs
    /// each step transactionally and records
    /// the steps in a `lowered` effects entry so replay (apply_record) can
    /// reproduce the op while the log keeps the honest verb name.
    /// `extra_effects` carries human-readable findings (counts, ranges) for
    /// the review rail; they ride before the `lowered` entry.
    pub fn apply_lowered(
        &mut self,
        verb: &str,
        args: Value,
        actor: Actor,
        rationale: Option<String>,
        steps: Vec<InverseOp>,
        extra_effects: Vec<OpEffect>,
    ) -> Result<OpRecord, CutError> {
        let mut next = self.project.clone(); // transactional: swap only on success
        let mut effects = Vec::new();
        for s in &steps {
            effects.extend(apply_edit_verb(&mut next, &s.verb, &s.args)?);
        }
        effects.extend(extra_effects);
        effects.push(edit::fx(None, json!({"lowered": steps})));
        // Carry the optional `group_id` meta-arg (see apply()).
        if let Some(ge) = group_effect(&args) {
            effects.push(ge);
        }
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: verb.to_string(),
            args,
            rationale,
            effects,
            // Recompute-by-replay: no per-op snapshot (see apply()).
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok(rec)
    }

    /// Record a checkpoint pointing at the current log head (timeline/op-log contract;
    /// the append-only operation-log contract: `project.checkpoint` IS an op — `at_op` is the checkpoint
    /// op's own id, so the checkpointed state includes everything up to and
    /// including this record). The full Checkpoint object is stored in the
    /// effects so replay reproduces id/ts byte-identically.
    ///
    /// COMMITS the op itself and returns (checkpoint, record) — callers must
    /// NOT append their own "project.checkpoint" op on top (the server once
    /// double-committed exactly that way, which duplicated checkpoints on
    /// replay and broke replay==live determinism). `rationale` is stored on
    /// the op per the rationale-preservation contract.
    pub fn checkpoint(
        &mut self,
        name: &str,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<(Checkpoint, OpRecord), CutError> {
        let n = self
            .project
            .checkpoints
            .iter()
            .filter_map(|c| c.id.strip_prefix("cp").and_then(|x| x.parse::<u64>().ok()))
            .max()
            .unwrap_or(0);
        let cp = Checkpoint {
            id: format!("cp{}", n + 1),
            name: name.to_string(),
            sequence_id: Some(self.project.active_sequence.clone()),
            at_op: self.log.next_id()?,
            ts: OpRecord::now_ts(),
        };
        let mut next = self.project.clone();
        next.checkpoints.push(cp.clone());
        let rec = OpRecord {
            op_id: cp.at_op.clone(),
            ts: cp.ts.clone(),
            actor,
            verb: "project.checkpoint".into(),
            args: json!({"name": name}),
            rationale,
            effects: vec![edit::fx(None, json!({"checkpoint": cp}))],
            inverse: None, // checkpoints are pointers, not timeline mutations
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok((cp, rec))
    }

    fn validate_sequence_name(
        &self,
        name: &str,
        except_id: Option<&str>,
    ) -> Result<String, CutError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "sequence name must contain 1 to 80 characters",
                "use a short descriptive name such as Main edit or Social cut",
            ));
        }
        let conflicts_with_implicit_main = self.project.sequences.is_empty()
            && except_id != Some(DEFAULT_SEQUENCE_ID)
            && name.eq_ignore_ascii_case("Main");
        if conflicts_with_implicit_main
            || self.project.sequences.iter().any(|sequence| {
                Some(sequence.id.as_str()) != except_id && sequence.name.eq_ignore_ascii_case(name)
            })
        {
            return Err(CutError::new(
                codes::CONFLICT,
                format!("a sequence named '{name}' already exists"),
                "choose a unique sequence name",
            ));
        }
        Ok(name.to_string())
    }

    /// Create and activate an empty sequence or a duplicate of the active one.
    /// The full allocated snapshot rides in the effect for deterministic replay.
    pub fn sequence_create(
        &mut self,
        name: &str,
        duplicate_active: bool,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<(Sequence, OpRecord), CutError> {
        let name = self.validate_sequence_name(name, None)?;
        let historical_max = {
            let journal = self.log.replay_view()?;
            journal
                .records()
                .iter()
                .filter(|op| op.verb == "project.sequence_create")
                .filter_map(|op| {
                    op.effects
                        .iter()
                        .find_map(|effect| effect.detail.get("sequence"))
                        .and_then(|sequence| sequence.get("id"))
                        .and_then(Value::as_str)
                        .and_then(|id| id.strip_prefix("seq"))
                        .and_then(|id| id.parse::<u64>().ok())
                })
                .max()
                .unwrap_or(1)
        };
        let id = format!("seq{}", historical_max + 1);
        let mut next = self.project.clone();
        let sequence = next.create_sequence_snapshot(id, name, duplicate_active);
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "project.sequence_create".into(),
            args: json!({
                "name": sequence.name,
                "from": if duplicate_active { "active" } else { "empty" },
            }),
            rationale,
            effects: vec![edit::fx(None, json!({"sequence": sequence}))],
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        let sequence = self
            .project
            .sequences
            .iter()
            .find(|candidate| candidate.id == self.project.active_sequence)
            .cloned()
            .expect("created sequence is active");
        Ok((sequence, rec))
    }

    pub fn sequence_switch(
        &mut self,
        id: &str,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<OpRecord, CutError> {
        if id == self.project.active_sequence {
            return Err(CutError::new(
                codes::GUARDRAIL,
                format!("sequence '{id}' is already active"),
                "choose a different sequence",
            ));
        }
        let mut next = self.project.clone();
        if !next.switch_sequence(id) {
            return Err(CutError::new(
                codes::NOT_FOUND,
                format!("sequence '{id}' does not exist"),
                "call project.sequence_list to inspect sequence ids",
            ));
        }
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "project.sequence_switch".into(),
            args: json!({"id": id}),
            rationale,
            effects: vec![edit::fx(None, json!({"active_sequence": id}))],
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok(rec)
    }

    pub fn sequence_rename(
        &mut self,
        id: &str,
        name: &str,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<OpRecord, CutError> {
        let name = self.validate_sequence_name(name, Some(id))?;
        let mut next = self.project.clone();
        next.ensure_sequence_bank();
        let sequence = next
            .sequences
            .iter_mut()
            .find(|sequence| sequence.id == id)
            .ok_or_else(|| {
                CutError::new(
                    codes::NOT_FOUND,
                    format!("sequence '{id}' does not exist"),
                    "call project.sequence_list to inspect sequence ids",
                )
            })?;
        sequence.name = name.clone();
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "project.sequence_rename".into(),
            args: json!({"id": id, "name": name}),
            rationale,
            effects: vec![edit::fx(None, json!({"sequence_id": id, "name": name}))],
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok(rec)
    }

    pub fn sequence_delete(
        &mut self,
        id: &str,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<OpRecord, CutError> {
        if id == self.project.active_sequence {
            return Err(CutError::new(
                codes::GUARDRAIL,
                "the active sequence cannot be deleted",
                "switch to another sequence first",
            ));
        }
        let mut next = self.project.clone();
        next.ensure_sequence_bank();
        let before = next.sequences.len();
        next.sequences.retain(|sequence| sequence.id != id);
        if next.sequences.len() == before {
            return Err(CutError::new(
                codes::NOT_FOUND,
                format!("sequence '{id}' does not exist"),
                "call project.sequence_list to inspect sequence ids",
            ));
        }
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "project.sequence_delete".into(),
            args: json!({"id": id}),
            rationale,
            effects: vec![edit::fx(None, json!({"deleted_sequence": id}))],
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok(rec)
    }

    /// Rename the project's DISPLAY NAME (label). It IS an op (the append-only operation-log contract): the
    /// name is set by the project.create op, so `rebuild_from_log` would reset a
    /// plain field mutation back to the create-op name — logging the rename makes
    /// it survive reopen/revert. No timeline effect (inverse:None, excluded from
    /// mutates_timeline). The .cutproj directory on disk is NOT renamed (the
    /// project path is fixed at create); this is the human-facing label only.
    pub fn rename(
        &mut self,
        name: &str,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<OpRecord, CutError> {
        validate_project_name(name)?;
        let mut next = self.project.clone();
        next.name = name.to_string();
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "project.rename".into(),
            args: json!({ "name": name }),
            rationale,
            effects: vec![edit::fx(None, json!({ "name": name }))],
            inverse: None, // metadata, not a timeline mutation
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok(rec)
    }

    /// Set the project's output FORMAT — timeline resolution (width×height) and
    /// frame rate. The render reads these for output geometry + fps, so LOWERING
    /// them can make renders and proxies faster on heavy footage ask. Frame-aware
    /// speed ramps are deterministically regridded to the new output grid here;
    /// legacy ramps with no persisted timebase retain historic millisecond
    /// semantics. Like `rename`, this has no inverse — the recorded setting
    /// snapshot remains the audit source of truth. Any of width/height/fps may be
    /// None to leave it unchanged; values are validated.
    pub fn set_format(
        &mut self,
        width: Option<u32>,
        height: Option<u32>,
        fps: Option<f64>,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<OpRecord, CutError> {
        let mut next = self.project.clone();
        if let Some(w) = width {
            if !(16..=7680).contains(&w) {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    "width out of range",
                    "width must be 16..=7680 px",
                ));
            }
            next.settings.width = w;
        }
        if let Some(h) = height {
            if !(16..=4320).contains(&h) {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    "height out of range",
                    "height must be 16..=4320 px",
                ));
            }
            next.settings.height = h;
        }
        if let Some(f) = fps {
            if !(f.is_finite() && f > 0.0 && f <= 240.0) {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    "fps out of range",
                    "fps must be in (0, 240]",
                ));
            }
            next.settings.fps = f;
        }
        let (grid_fps, grid_audio_rate) = (next.settings.fps, next.settings.audio_rate);
        crate::speed_ramp_timing::regrid_timebased_speed_ramps(
            &mut next,
            grid_fps,
            grid_audio_rate,
        );
        let s = &next.settings;
        let snapshot = json!({ "width": s.width, "height": s.height, "fps": s.fps });
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "project.format".into(),
            args: snapshot.clone(),
            rationale,
            effects: vec![edit::fx(None, snapshot)],
            inverse: None, // metadata, not a timeline mutation
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok(rec)
    }

    /// Set the project's COLOR MANAGEMENT (`project.color`): the working and/or
    /// output color space. Like `set_format`, this is project METADATA (not a
    /// timeline edit, inverse None) — the new config lives in project.json (saved by
    /// commit), so replay starts from it; the op is recorded for audit. Either of
    /// working/output may be None to leave it unchanged. The full post-change
    /// snapshot {working, output} is recorded in BOTH args + effect so replay is a
    /// direct settings assignment (see `apply_record` "project.color"). Setting the
    /// config back to rec709/rec709 restores the byte-identical default render.
    pub fn set_color(
        &mut self,
        working: Option<crate::types::ColorSpace>,
        output: Option<crate::types::ColorSpace>,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<OpRecord, CutError> {
        let mut next = self.project.clone();
        if let Some(w) = working {
            next.settings.color.working = w;
        }
        if let Some(o) = output {
            next.settings.color.output = o;
        }
        let c = &next.settings.color;
        let snapshot = json!({ "working": c.working.as_str(), "output": c.output.as_str() });
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "project.color".into(),
            args: snapshot.clone(),
            rationale,
            effects: vec![edit::fx(None, snapshot)],
            inverse: None, // metadata, not a timeline mutation
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok(rec)
    }

    /// Replace or clear the project-owned brand kit. This is project metadata,
    /// not a timeline edit. The full post-change snapshot is logged so replay
    /// never depends on prior cache state or partial-update semantics.
    pub fn set_brand(
        &mut self,
        brand: Option<BrandKit>,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<OpRecord, CutError> {
        let brand = brand
            .map(BrandKit::normalized)
            .transpose()
            .map_err(|cause| CutError::new(codes::INVALID_ARGS, "invalid brand kit", cause))?;
        let mut next = self.project.clone();
        next.brand = brand.clone();
        let snapshot = json!({ "brand": brand });
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "project.brand".into(),
            args: snapshot.clone(),
            rationale,
            effects: vec![edit::fx(None, snapshot)],
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok(rec)
    }

    /// Save a named GRADE PRESET into the project's grade gallery (`grade.save`).
    /// Snapshots `grade` under `name` in `Project::grade_presets`; a re-save under an
    /// existing name REPLACES it (the gallery is name-keyed). Project METADATA, not a
    /// timeline edit — like `set_color`, the FULL preset {name, grade} is recorded in
    /// BOTH args + the effect so replay is a direct push/replace (see the `grade.save`
    /// arm in `apply_record`). Returns (the stored preset, the op record).
    pub fn save_grade_preset(
        &mut self,
        name: &str,
        grade: crate::types::ClipGrade,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<(crate::types::GradePreset, OpRecord), CutError> {
        let preset = crate::types::GradePreset {
            name: name.to_string(),
            grade,
        };
        // Name-keyed: a re-save replaces the existing preset, else append.
        let mut next = self.project.clone();
        if let Some(existing) = next
            .grade_presets
            .iter_mut()
            .find(|p| p.name == preset.name)
        {
            *existing = preset.clone();
        } else {
            next.grade_presets.push(preset.clone());
        }
        let snapshot = json!({ "name": preset.name, "grade": preset.grade });
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "grade.save".into(),
            args: snapshot.clone(),
            rationale,
            effects: vec![edit::fx(None, json!({ "preset": preset }))],
            inverse: None, // metadata, not a timeline mutation
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok((preset, rec))
    }

    /// Save (upsert) a CAPTION STYLE PRESET as an op (`captions.save_style` —
    /// The caption analog of `save_grade_preset`, byte-for-byte the
    /// same metadata pattern (full preset in the effect, name-keyed replace,
    /// off the undo cursor). Returns (replaced, rec).
    pub fn save_caption_style_preset(
        &mut self,
        preset: crate::types::CaptionStylePreset,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<(bool, OpRecord), CutError> {
        let mut next = self.project.clone();
        let replaced = if let Some(existing) = next
            .caption_style_presets
            .iter_mut()
            .find(|p| p.name == preset.name)
        {
            *existing = preset.clone();
            true
        } else {
            next.caption_style_presets.push(preset.clone());
            false
        };
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "captions.save_style".into(),
            args: serde_json::to_value(&preset).unwrap_or_default(),
            rationale,
            effects: vec![edit::fx(
                None,
                json!({ "preset": preset, "replaced": replaced }),
            )],
            inverse: None, // metadata, not a timeline mutation
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok((replaced, rec))
    }

    /// Save (upsert) a SMART BIN as an op (`media.bin_save`), the
    /// exact grade-gallery metadata pattern: the full bin rides in the effect
    /// so replay reproduces the bin list; name-keyed (re-save REPLACES);
    /// non-timeline metadata op off the undo cursor. Returns (replaced, rec).
    pub fn save_smart_bin(
        &mut self,
        bin: crate::types::SmartBin,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<(bool, OpRecord), CutError> {
        let mut next = self.project.clone();
        let replaced =
            if let Some(existing) = next.smart_bins.iter_mut().find(|b| b.name == bin.name) {
                *existing = bin.clone();
                true
            } else {
                next.smart_bins.push(bin.clone());
                false
            };
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "media.bin_save".into(),
            args: serde_json::to_value(&bin).unwrap_or_default(),
            rationale,
            effects: vec![edit::fx(None, json!({ "bin": bin, "replaced": replaced }))],
            inverse: None, // metadata, not a timeline mutation
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok((replaced, rec))
    }

    /// Delete a SMART BIN as an op (`media.bin_delete`). NOT_FOUND when no bin
    /// carries the name. Replay drops it by name (idempotent).
    pub fn delete_smart_bin(
        &mut self,
        name: &str,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<OpRecord, CutError> {
        let mut next = self.project.clone();
        let before = next.smart_bins.len();
        next.smart_bins.retain(|b| b.name != name);
        if next.smart_bins.len() == before {
            return Err(CutError::new(
                codes::NOT_FOUND,
                format!("no smart bin '{name}'"),
                "list bins via media.bin_list".to_string(),
            ));
        }
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "media.bin_delete".into(),
            args: json!({ "name": name }),
            rationale,
            effects: vec![edit::fx(None, json!({ "name": name }))],
            inverse: None, // metadata, not a timeline mutation
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok(rec)
    }

    /// Register an imported asset as an op (the append-only operation-log contract: `media.import` IS an
    /// op). The full Asset payload goes in the effects so replay reproduces
    /// the assets map — replay REQUIRES this exact `{asset_id, asset}` effect
    /// shape (apply_record), so importing must go through here, never through
    /// a hand-rolled op. `asset_id` None ⇒ deterministic "aN" allocation.
    /// `rationale` is stored on the op per the rationale-preservation contract. Returns (asset_id, record).
    pub fn record_import(
        &mut self,
        asset_id: Option<String>,
        asset: Asset,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<(String, OpRecord), CutError> {
        let id = match asset_id {
            Some(id) => id,
            None => {
                let live_n = self
                    .project
                    .assets
                    .keys()
                    .filter_map(|k| asset_id_number(k))
                    .max()
                    .unwrap_or(0);
                let log_n = {
                    let journal = self.log.replay_view()?;
                    journal
                        .records()
                        .iter()
                        .flat_map(imported_asset_ids)
                        .filter_map(|id| asset_id_number(&id))
                        .max()
                        .unwrap_or(0)
                };
                format!("a{}", live_n.max(log_n) + 1)
            }
        };
        let mut next = self.project.clone();
        if next.assets.contains_key(&id) {
            return Err(CutError::new(
                codes::CONFLICT,
                format!("asset '{id}' already exists"),
                "asset ids must be unique; omit asset_id for auto-allocation",
            ));
        }
        next.assets.insert(id.clone(), asset.clone());
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "media.import".into(),
            args: json!({"path": asset.path}),
            rationale,
            effects: vec![edit::fx(None, json!({"asset_id": id, "asset": asset}))],
            inverse: None, // assets persist; revert is timeline-scoped
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok((id, rec))
    }

    /// Reserve deterministic asset ids for one atomic interchange import.
    /// The log maximum matters even when an older import was undone or removed:
    /// reusing one of its ids would make a later replay bind clips to the wrong
    /// asset. This only computes ids; [`Self::replace_timeline_from_otio`] checks
    /// them again while committing the complete import.
    pub fn next_asset_ids(&self, count: usize) -> Result<Vec<String>, CutError> {
        let live_n = self
            .project
            .assets
            .keys()
            .filter_map(|id| asset_id_number(id))
            .max()
            .unwrap_or(0);
        let journal = self.log.replay_view()?;
        let log_n = journal
            .records()
            .iter()
            .flat_map(imported_asset_ids)
            .filter_map(|id| asset_id_number(&id))
            .max()
            .unwrap_or(0);
        let start = live_n.max(log_n);
        (1..=count)
            .map(|offset| {
                let offset = u64::try_from(offset).map_err(|_| {
                    CutError::new(
                        codes::INVALID_ARGS,
                        "too many assets in interchange import",
                        format!("asset count {count} exceeds the supported id range"),
                    )
                })?;
                let number = start.checked_add(offset).ok_or_else(|| {
                    CutError::new(
                        codes::INVALID_ARGS,
                        "interchange asset id range overflowed",
                        format!("starting id a{start}, count {count}"),
                    )
                })?;
                Ok(format!("a{number}"))
            })
            .collect()
    }

    /// Atomically register media assets and insert all of them into the active
    /// timeline as one replayable Motion plan op. Every asset and edit step is
    /// validated against a cloned project before the single log/cache commit,
    /// so a failure at any position leaves both live state and the op log
    /// unchanged. The exact plan hash is the idempotency key.
    pub fn apply_atomic_media_insert_plan(
        &mut self,
        idempotency_key: &str,
        assets: Vec<Asset>,
        insert_args: Vec<Value>,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<AtomicMediaInsertPlanResult, CutError> {
        self.apply_atomic_media_insert_plan_with_links(
            idempotency_key,
            assets,
            insert_args,
            None,
            actor,
            rationale,
        )
    }

    /// The rendered-media Motion import with optional per-clip source/render
    /// provenance. Link templates must align one-to-one with the verified assets;
    /// the store adds the replay-stable Cut clip and asset ids after allocation.
    pub fn apply_atomic_media_insert_plan_with_links(
        &mut self,
        idempotency_key: &str,
        assets: Vec<Asset>,
        insert_args: Vec<Value>,
        motion_link_templates: Option<Vec<Value>>,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<AtomicMediaInsertPlanResult, CutError> {
        if !is_lower_hex_sha256(idempotency_key) {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "Motion import idempotency key is invalid",
                "expected the lowercase SHA-256 of the exact attested import plan",
            ));
        }
        if assets.is_empty() || assets.len() != insert_args.len() {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "Motion import assets and inserts do not align",
                format!(
                    "received {} assets and {} insert operations",
                    assets.len(),
                    insert_args.len()
                ),
            ));
        }
        if motion_link_templates
            .as_ref()
            .is_some_and(|links| links.len() != assets.len())
        {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "Motion link provenance does not align with imported media",
                "motion link templates must match the verified asset count",
            ));
        }
        let journal = self.log.replay_view()?;
        let all = journal.records();
        if let Some(op) = all.iter().find(|op| {
            op.verb == "motion.apply_import"
                && op.args.get("idempotency_key").and_then(Value::as_str) == Some(idempotency_key)
        }) {
            let detail = op
                .effects
                .iter()
                .map(|effect| &effect.detail)
                .find(|detail| detail.get("atomic_media_insert").is_some())
                .ok_or_else(|| replay_corrupt(op, "atomic Motion import effect is missing"))?;
            let asset_ids: Vec<String> = serde_json::from_value(
                detail
                    .get("asset_ids")
                    .cloned()
                    .ok_or_else(|| replay_corrupt(op, "Motion asset ids are missing"))?,
            )?;
            let clip_ids: Vec<String> = serde_json::from_value(
                detail
                    .get("clip_ids")
                    .cloned()
                    .ok_or_else(|| replay_corrupt(op, "Motion clip ids are missing"))?,
            )?;
            let clips_are_live = clip_ids.iter().all(|clip_id| {
                self.project
                    .all_sequence_tracks()
                    .flat_map(|track| track.clips.iter())
                    .any(|clip| clip.id() == Some(clip_id.as_str()))
            });
            let assets_are_live = asset_ids
                .iter()
                .all(|asset_id| self.project.assets.contains_key(asset_id));
            if !clips_are_live || !assets_are_live {
                return Err(CutError::new(
                    codes::CONFLICT,
                    "Motion import plan was already applied and is not currently active",
                    format!(
                        "op '{}' owns this idempotency key but its imported assets or clips were later undone",
                        op.op_id
                    ),
                )
                .with_suggested_action(
                    "use project.redo to restore the original apply, or generate a new import plan for an intentional second insertion",
                ));
            }
            return Ok(AtomicMediaInsertPlanResult {
                checkpoint: serde_json::from_value(
                    detail
                        .get("checkpoint")
                        .cloned()
                        .ok_or_else(|| replay_corrupt(op, "Motion checkpoint is missing"))?,
                )?,
                asset_ids,
                clip_ids,
                op: op.clone(),
                already_applied: true,
            });
        }

        let prior_op = all.last().ok_or_else(|| {
            CutError::new(
                codes::CONFLICT,
                "Motion import has no project baseline",
                "the project log must contain project.create before applying a plan",
            )
        })?;
        let checkpoint_number = self
            .project
            .checkpoints
            .iter()
            .filter_map(|checkpoint| {
                checkpoint
                    .id
                    .strip_prefix("cp")
                    .and_then(|value| value.parse::<u64>().ok())
            })
            .max()
            .unwrap_or(0)
            + 1;
        let checkpoint = Checkpoint {
            id: format!("cp{checkpoint_number}"),
            name: format!("Before Motion import {}", &idempotency_key[..12]),
            sequence_id: Some(self.project.active_sequence.clone()),
            at_op: prior_op.op_id.clone(),
            ts: OpRecord::now_ts(),
        };

        // The idempotency and baseline lookups are complete. Drop this view
        // before the atomic operation eventually commits its one durable op.
        drop(journal);
        let asset_ids = self.next_asset_ids(assets.len())?;
        let mut staged_assets = BTreeMap::new();
        let mut next = self.project.clone();
        next.checkpoints.push(checkpoint.clone());
        for (asset_id, asset) in asset_ids.iter().cloned().zip(assets) {
            if next
                .assets
                .insert(asset_id.clone(), asset.clone())
                .is_some()
            {
                return Err(CutError::new(
                    codes::CONFLICT,
                    format!("asset '{asset_id}' already exists"),
                    "Motion import asset ids must be allocated from the current project log",
                ));
            }
            staged_assets.insert(asset_id, asset);
        }

        let mut steps = Vec::with_capacity(insert_args.len());
        let mut effects = Vec::new();
        let mut clip_ids = Vec::with_capacity(insert_args.len());
        for (asset_id, raw_args) in asset_ids.iter().zip(insert_args) {
            let mut args = raw_args.as_object().cloned().ok_or_else(|| {
                CutError::new(
                    codes::INVALID_ARGS,
                    "Motion insert operation is not an object",
                    raw_args.to_string(),
                )
            })?;
            args.insert("asset".into(), json!(asset_id));
            args.entry("ripple").or_insert(Value::Bool(false));
            let args = Value::Object(args);
            let step = InverseOp {
                verb: "edit.insert".into(),
                args: args.clone(),
            };
            let step_effects = apply_edit_verb(&mut next, &step.verb, &step.args)?;
            let clip_id = step_effects
                .iter()
                .find_map(|effect| effect.detail.get("added_clip").and_then(Value::as_str))
                .ok_or_else(|| {
                    CutError::new(
                        codes::CONFLICT,
                        "Motion insert produced no clip identity",
                        format!("asset '{asset_id}' could not be bound to a timeline clip"),
                    )
                })?
                .to_string();
            clip_ids.push(clip_id);
            effects.extend(step_effects);
            steps.push(step);
        }
        let motion_links = motion_link_templates
            .unwrap_or_default()
            .into_iter()
            .zip(asset_ids.iter().zip(clip_ids.iter()))
            .map(|(template, (asset_id, clip_id))| {
                let mut link = template.as_object().cloned().ok_or_else(|| {
                    CutError::new(
                        codes::INVALID_ARGS,
                        "Motion link provenance must be an object",
                        "received a non-object Motion link template",
                    )
                })?;
                link.insert("clipId".into(), json!(clip_id));
                link.insert("assetId".into(), json!(asset_id));
                Ok(Value::Object(link))
            })
            .collect::<Result<Vec<_>, CutError>>()?;
        let mut atomic_detail = json!({
            "atomic_media_insert": true,
            "idempotency_key": idempotency_key,
            "checkpoint": checkpoint,
            "assets": staged_assets,
            "asset_ids": asset_ids,
            "clip_ids": clip_ids,
            "lowered": steps,
        });
        if !motion_links.is_empty() {
            atomic_detail["motion_links"] = Value::Array(motion_links);
        }
        effects.push(edit::fx(None, atomic_detail));
        let op = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "motion.apply_import".into(),
            args: json!({
                "idempotency_key": idempotency_key,
                "operation_count": asset_ids.len(),
            }),
            rationale,
            effects,
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &op)?;
        Ok(AtomicMediaInsertPlanResult {
            checkpoint,
            asset_ids,
            clip_ids,
            op,
            already_applied: false,
        })
    }

    /// Atomically register a verified immutable rerender and replace one linked
    /// clip in place. The clip id, timeline slot, and Cut-owned look survive;
    /// the new asset plus lowering ride one replayable/undoable refresh op.
    pub fn apply_motion_link_refresh(
        &mut self,
        clip_id: &str,
        asset: Asset,
        mut motion_link: Value,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<MotionLinkRefreshResult, CutError> {
        let (track_id, clip_index) = self.project.find_clip(clip_id).ok_or_else(|| {
            CutError::new(
                codes::NOT_FOUND,
                format!("linked Motion clip '{clip_id}' is not on the timeline"),
                "use project.state to select a live linked clip",
            )
        })?;
        if !matches!(
            self.project
                .track(track_id)
                .and_then(|track| track.clips.get(clip_index)),
            Some(Clip::Media(_))
        ) {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("linked Motion target '{clip_id}' is not a media clip"),
                "only rendered-media Motion links can be refreshed",
            ));
        }
        let link = motion_link.as_object_mut().ok_or_else(|| {
            CutError::new(
                codes::INVALID_ARGS,
                "Motion refresh link is not an object",
                "the connector must provide shellx-cut/motion-link@1 provenance",
            )
        })?;
        if link.get("schema").and_then(Value::as_str) != Some("shellx-cut/motion-link@1") {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "Motion refresh link schema is unsupported",
                "expected shellx-cut/motion-link@1",
            ));
        }
        if link.get("clipId").and_then(Value::as_str) != Some(clip_id) {
            return Err(CutError::new(
                codes::CONFLICT,
                "Motion refresh link targets another clip",
                "the durable link clipId must match the requested live clip",
            ));
        }

        let asset_id = self.next_asset_ids(1)?.remove(0);
        let mut next = self.project.clone();
        if next
            .assets
            .insert(asset_id.clone(), asset.clone())
            .is_some()
        {
            return Err(CutError::new(
                codes::CONFLICT,
                format!("asset '{asset_id}' already exists"),
                "Motion refresh asset ids must be allocated from the current project log",
            ));
        }
        let step = InverseOp {
            verb: "edit.replace".into(),
            args: json!({
                "clip": clip_id,
                "asset": asset_id,
                "source_in_ms": 0,
                "source_out_ms": null,
            }),
        };
        let mut effects = apply_edit_verb(&mut next, &step.verb, &step.args)?;
        link.insert("assetId".into(), json!(asset_id));
        link.insert("state".into(), json!("linked-current"));
        let assets = BTreeMap::from([(asset_id.clone(), asset)]);
        effects.push(edit::fx(
            None,
            json!({
                "motion_link_refresh": true,
                "assets": assets,
                "asset_ids": [asset_id.clone()],
                "lowered": [step],
                "motion_links": [motion_link],
            }),
        ));
        let op = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "motion.link.refresh".into(),
            args: json!({ "clip": clip_id }),
            rationale,
            effects,
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &op)?;
        Ok(MotionLinkRefreshResult { asset_id, op })
    }

    /// Persist a validated package-path repair without changing pixels. Local
    /// availability remains a transient `project.state` projection.
    pub fn record_motion_link_source_update(
        &mut self,
        verb: &str,
        clip_id: &str,
        motion_link: Value,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<OpRecord, CutError> {
        if !matches!(
            verb,
            "motion.link.relink"
                | "motion.link.tracking.request"
                | "motion.link.tracking.apply"
                | "motion.link.tracking.detach"
        ) {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "Motion source-update verb is unsupported",
                verb,
            ));
        }
        if self.project.find_clip(clip_id).is_none() {
            return Err(CutError::new(
                codes::NOT_FOUND,
                format!("linked Motion clip '{clip_id}' is not on the timeline"),
                "use project.state to select a live linked clip",
            ));
        }
        if motion_link.get("schema").and_then(Value::as_str) != Some("shellx-cut/motion-link@1")
            || motion_link.get("clipId").and_then(Value::as_str) != Some(clip_id)
        {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "Motion relink provenance is invalid",
                "schema and stable clipId must match the existing link",
            ));
        }
        let op = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: verb.into(),
            args: json!({ "clip": clip_id }),
            rationale,
            effects: vec![edit::fx(None, json!({ "motion_links": [motion_link] }))],
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit(&op)?;
        Ok(op)
    }

    /// Record the identity binding for a group of Cut-native edits lowered from
    /// one ShellX Motion editable import plan. The child title/shape/media ops
    /// already carry the actual timeline mutations; this final grouped op keeps
    /// package/layer identity and the pre-plan checkpoint replayable so a single
    /// undo crosses the complete import and a later reimport can update by source
    /// layer instead of duplicating objects.
    pub fn record_motion_editable_import(
        &mut self,
        idempotency_key: &str,
        package_id: &str,
        motion_id: &str,
        checkpoint: Checkpoint,
        layer_bindings: Value,
        operation_count: usize,
        group_id: &str,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<OpRecord, CutError> {
        if !is_lower_hex_sha256(idempotency_key) {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "Motion editable import idempotency key is invalid",
                "expected the lowercase SHA-256 of the exact import plan",
            ));
        }
        if package_id.trim().is_empty() || motion_id.trim().is_empty() || group_id.trim().is_empty()
        {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "Motion editable import identity is incomplete",
                "package_id, motion_id, and group_id must be non-empty",
            ));
        }
        self.apply_lowered(
            "motion.apply_import",
            json!({
                "idempotency_key": idempotency_key,
                "package_id": package_id,
                "motion_id": motion_id,
                "mode": "editable_lowering",
                "operation_count": operation_count,
                "group_id": group_id,
            }),
            actor,
            rationale,
            vec![],
            vec![edit::fx(
                None,
                json!({
                    "motion_editable_import": true,
                    "idempotency_key": idempotency_key,
                    "checkpoint": checkpoint,
                    "layer_bindings": layer_bindings,
                }),
            )],
        )
    }

    /// Atomically replace the active timeline from a fully validated OTIO plan.
    /// New assets and the complete timeline ride in ONE op effect, so a failed
    /// append/save commits nothing, replay needs no filesystem access, and one
    /// undo restores the prior composition instead of peeling a partial rebuild.
    pub fn replace_timeline_from_otio(
        &mut self,
        tracks: Vec<Track>,
        assets: BTreeMap<String, Asset>,
        source_hash: String,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<OpRecord, CutError> {
        if tracks.is_empty() {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "OTIO import contains no tracks",
                "preflight must produce at least one video or audio track",
            ));
        }
        if source_hash.trim().is_empty() {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "OTIO import source hash is empty",
                "bind the import to the exact preflighted file bytes",
            ));
        }
        let mut next = self.project.clone();
        for (id, asset) in &assets {
            if next.assets.contains_key(id) {
                return Err(CutError::new(
                    codes::CONFLICT,
                    format!("asset '{id}' already exists"),
                    "interchange asset ids must be allocated from the current project log",
                ));
            }
            next.assets.insert(id.clone(), asset.clone());
        }
        let timeline = json!({
            "tracks": tracks,
            "markers": [],
            "adjustments": [],
            "nests": [],
            "transcript_ignores": [],
        });
        edit::apply_set_timeline(&mut next, &timeline)?;
        for track in &next.tracks {
            for clip in &track.clips {
                if let crate::types::Clip::Media(media) = clip {
                    if !next.assets.contains_key(&media.asset) {
                        return Err(CutError::new(
                            codes::INVALID_ARGS,
                            "OTIO timeline references an unknown asset",
                            format!(
                                "clip '{}' on track '{}' references '{}'",
                                media.id, track.id, media.asset
                            ),
                        ));
                    }
                }
            }
        }
        let clip_count: usize = next.tracks.iter().map(|track| track.clips.len()).sum();
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "import.otio".into(),
            args: json!({
                "source_hash": source_hash,
                "tracks": next.tracks.len(),
                "items": clip_count,
            }),
            rationale,
            effects: vec![edit::fx(
                None,
                json!({
                    "source_hash": source_hash,
                    "timeline": timeline,
                    "assets": assets,
                }),
            )],
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok(rec)
    }

    /// Remove an imported asset as an op — the namespace-inverse of
    /// `record_import`. REPLAY-SAFE: the effect carries `{asset_id}` and
    /// `rebuild_from_log` drops it from the assets map, so a removed asset stays
    /// removed across replay/revert. (A plain `assets.remove` + save would NOT
    /// be enough: the log still holds the `media.import` op, so the next
    /// rebuild_from_log would resurrect the asset — the exact failure mode the
    /// import path warns about.) Non-timeline metadata op: `inverse:None` and
    /// excluded from the undo stack (OpRecord::mutates_timeline), like
    /// media.import — re-import to restore. The caller MUST have verified no
    /// timeline clip still references the asset (the server refuses otherwise);
    /// this fn does not re-check, it only drops the record + commits. Returns the
    /// removed Asset (so the caller can unlink its derived proxy/filmstrip/etc.)
    /// and the OpRecord.
    pub fn record_remove_asset(
        &mut self,
        asset_id: &str,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<(Asset, OpRecord), CutError> {
        let mut next = self.project.clone();
        let removed = next.assets.remove(asset_id).ok_or_else(|| {
            CutError::new(
                codes::NOT_FOUND,
                format!("no asset '{asset_id}'"),
                "list assets via project.state".to_string(),
            )
        })?;
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "media.remove".into(),
            args: json!({"asset": asset_id}),
            rationale,
            effects: vec![edit::fx(None, json!({"asset_id": asset_id}))],
            inverse: None, // assets persist outside the undo stack (re-import to restore)
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok((removed, rec))
    }

    /// Repoint an imported asset at a new source file as an op (`media.relink`).
    /// REPLAY-SAFE: the effect carries the full outcome `{asset_id, path,
    /// old_path, hash, clear_derived}` and the replay arm applies exactly those
    /// recorded values — replay never touches the filesystem, so a rebuild on a
    /// machine where neither path exists still reproduces the project state.
    /// `clear_derived=true` (content hash changed) also drops the probe /
    /// transcript / perception / proxy / filmstrip pointers — they described the
    /// OLD content; the server regenerates them via the import chain.
    /// `clear_derived=false` (same hash ⇒ the file merely moved) keeps them.
    /// Non-timeline metadata op like import/remove: `inverse:None`, off the undo
    /// cursor — relink back to the old path to restore. Returns (old_path, rec).
    pub fn record_relink_asset(
        &mut self,
        asset_id: &str,
        new_path: &str,
        new_hash: &str,
        clear_derived: bool,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<(String, OpRecord), CutError> {
        let mut next = self.project.clone();
        let asset = next.assets.get_mut(asset_id).ok_or_else(|| {
            CutError::new(
                codes::NOT_FOUND,
                format!("no asset '{asset_id}'"),
                "list assets via project.state".to_string(),
            )
        })?;
        let old_path = asset.path.clone();
        asset.path = new_path.to_string();
        asset.hash = new_hash.to_string();
        if clear_derived {
            asset.probe = None;
            asset.transcript = None;
            asset.perception = None;
            asset.proxy = None;
            asset.filmstrip = None;
        }
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "media.relink".into(),
            args: json!({"asset": asset_id, "path": new_path}),
            rationale,
            effects: vec![edit::fx(
                None,
                json!({
                    "asset_id": asset_id,
                    "path": new_path,
                    "old_path": old_path,
                    "hash": new_hash,
                    "clear_derived": clear_derived,
                }),
            )],
            inverse: None, // metadata op; relink back to the old path to restore
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok((old_path, rec))
    }

    /// Append a timecoded review comment. Like checkpoint/import, this is
    /// a NON-TIMELINE metadata op: it carries no inverse and is NOT in the undo
    /// stack (OpRecord::mutates_timeline excludes the comment verbs). The full
    /// Comment rides in the effects so replay reproduces it byte-identically
    /// (id + ts come from the recorded payload, never wall-clock at replay).
    /// Returns (comment, record).
    pub fn add_comment(
        &mut self,
        at_ms: u64,
        end_ms: Option<u64>,
        text: &str,
        author: &str,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<(crate::types::Comment, OpRecord), CutError> {
        let n = self
            .project
            .comments
            .iter()
            .filter_map(|c| c.id.strip_prefix("cm").and_then(|x| x.parse::<u64>().ok()))
            .max()
            .unwrap_or(0);
        let cm = crate::types::Comment {
            id: format!("cm{}", n + 1),
            at_ms,
            end_ms,
            anchor: self.project.comment_anchor_at(at_ms),
            text: text.to_string(),
            author: author.to_string(),
            status: "open".into(),
            ts: OpRecord::now_ts(),
            review_source: None,
            draft: None,
        };
        let mut next = self.project.clone();
        next.comments.push(cm.clone());
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: cm.ts.clone(),
            actor,
            verb: "comment.add".into(),
            args: json!({"at_ms": at_ms, "end_ms": end_ms, "text": text, "author": author}),
            rationale,
            effects: vec![edit::fx(None, json!({"comment": cm}))],
            inverse: None, // review metadata — not a timeline op
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok((cm, rec))
    }

    /// Import a validated batch of notes from one portable review package.
    /// The entire batch is one metadata op and one cache commit: a crash or
    /// validation failure can never leave a partially imported review behind.
    pub fn import_review_comments(
        &mut self,
        notes: Vec<crate::types::ReviewFeedbackNote>,
        source: crate::types::CommentReviewSource,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<(Vec<crate::types::Comment>, OpRecord), CutError> {
        if notes.is_empty() || notes.len() > 500 {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "review feedback must contain between 1 and 500 comments",
                format!("got {} comments", notes.len()),
            ));
        }
        if source.source_op_id.trim().is_empty()
            || source.render_id.trim().is_empty()
            || source.render_hash.trim().is_empty()
        {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "review feedback provenance is incomplete",
                "source_op_id, render_id, and render_hash are required",
            ));
        }

        let mut next_id = self
            .project
            .comments
            .iter()
            .filter_map(|c| c.id.strip_prefix("cm").and_then(|x| x.parse::<u64>().ok()))
            .max()
            .unwrap_or(0);
        let ts = OpRecord::now_ts();
        let mut comments = Vec::new();
        for note in notes {
            let text = note.text.trim();
            let author = note.author.trim();
            if text.is_empty() || text.chars().count() > 2000 {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    "review comment text must be between 1 and 2000 characters",
                    format!("got {} characters", text.chars().count()),
                ));
            }
            if author.is_empty() || author.chars().count() > 80 {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    "review comment author must be between 1 and 80 characters",
                    format!("got {} characters", author.chars().count()),
                ));
            }
            if note.end_ms.is_some_and(|end_ms| end_ms <= note.at_ms) {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    "review comment end_ms must be greater than at_ms",
                    format!("got at_ms={} end_ms={:?}", note.at_ms, note.end_ms),
                ));
            }
            next_id += 1;
            comments.push(crate::types::Comment {
                id: format!("cm{next_id}"),
                at_ms: note.at_ms,
                end_ms: note.end_ms,
                anchor: self.project.comment_anchor_at(note.at_ms),
                text: text.to_string(),
                author: author.to_string(),
                status: "open".into(),
                ts: ts.clone(),
                review_source: Some(source.clone()),
                draft: None,
            });
        }

        let mut next = self.project.clone();
        next.comments.extend(comments.iter().cloned());
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts,
            actor,
            verb: "comment.import".into(),
            args: json!({"source": source, "count": comments.len()}),
            rationale,
            effects: vec![edit::fx(None, json!({"comments": comments}))],
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok((comments, rec))
    }

    /// Update a comment's status: "addressed" (its drafted change was
    /// applied), "dismissed", or back to "open". A non-timeline metadata op like
    /// add_comment; the updated Comment rides in the effects for replay. Errors
    /// on an unknown id or a status outside the lifecycle.
    pub fn resolve_comment(
        &mut self,
        comment_id: &str,
        status: &str,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<(crate::types::Comment, OpRecord), CutError> {
        if !matches!(status, "open" | "addressed" | "dismissed") {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("unknown comment status '{status}'"),
                "status must be \"open\", \"addressed\", or \"dismissed\"",
            ));
        }
        let mut next = self.project.clone();
        let cm = {
            let c = next
                .comments
                .iter_mut()
                .find(|c| c.id == comment_id)
                .ok_or_else(|| {
                    CutError::new(
                        codes::NOT_FOUND,
                        format!("no comment '{comment_id}'"),
                        "comment ids come from comment.list",
                    )
                })?;
            c.status = status.to_string();
            c.clone()
        };
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "comment.resolve".into(),
            args: json!({"comment_id": comment_id, "status": status}),
            rationale,
            effects: vec![edit::fx(None, json!({"comment": cm}))],
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok((cm, rec))
    }

    /// Store the agent's DRAFTED change on a comment (`comment.draft`). A
    /// non-timeline metadata op; the updated Comment (with `draft`) rides
    /// in the effects for replay. `draft` is the proposed `{verbs, rationale,
    /// confidence}` — recorded for review, NOT applied (comment.apply executes
    /// it). Errors on an unknown comment id.
    pub fn set_comment_draft(
        &mut self,
        comment_id: &str,
        draft: serde_json::Value,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<(crate::types::Comment, OpRecord), CutError> {
        let mut next = self.project.clone();
        let cm = {
            let c = next
                .comments
                .iter_mut()
                .find(|c| c.id == comment_id)
                .ok_or_else(|| {
                    CutError::new(
                        codes::NOT_FOUND,
                        format!("no comment '{comment_id}'"),
                        "comment ids come from comment.list",
                    )
                })?;
            c.draft = Some(draft);
            c.clone()
        };
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "comment.draft".into(),
            args: json!({"comment_id": comment_id}),
            rationale,
            effects: vec![edit::fx(None, json!({"comment": cm}))],
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok((cm, rec))
    }

    /// Revert to a checkpoint/op by APPENDING one ATOMIC `project.revert` op
    /// (never rewriting the log — timeline/op-log contract + the append-only operation-log contract). The reverted timeline is
    /// the log replayed THROUGH the target op (the state as it was when the
    /// checkpoint was taken); it rides in the op's `restored_timeline` effect so
    /// replay reproduces it directly — the same mechanism `edit.restore` uses.
    ///
    /// WHY ATOMIC (changed, Option B): revert used to expand into one
    /// `edit.restore` peel per undone op. That reached the target correctly, but
    /// a single tip-undo of a revert reversed only the LAST peel, leaving a
    /// confusing partial state. As ONE timeline op, undoing the revert (a tip
    /// restore) recomputes the pre-revert prefix = the FULL edit before the
    /// revert, byte-exact — a revert is now cleanly, atomically undoable. The
    /// target state is identical to the old peel macro's (it landed on
    /// `rebuild_from_log(&all[..=target_idx])`), so this is behavior-preserving
    /// for the reached state and replay; only the op SHAPE (1 vs N) changed.
    /// `comment.apply` still rolls back a partial apply in one `project.revert`.
    /// Returns the appended op's id (a Vec for caller/event-publish symmetry).
    pub fn revert(&mut self, to: &str, actor: Actor) -> Result<Vec<String>, CutError> {
        // Resolve a checkpoint/op ref to a concrete op id; the timeline is then
        // the log replayed up to AND INCLUDING that op (a checkpoint op doesn't
        // mutate the timeline, so this is the state at the checkpoint). The
        // op-existence check + replay live in set_timeline_as_of.
        let target = crate::diff::resolve_ref(&self.project, &to.to_string())?;
        let mut extra = serde_json::Map::new();
        extra.insert("reverted_to".into(), json!(target));
        let target_cursor = self.undo_history.iter().position(|id| id == &target);
        if let Some(cursor) = target_cursor {
            extra.insert("cursor".into(), json!(cursor));
        }
        let removed_assets = {
            let journal = self.log.replay_view()?;
            motion_assets_for_checkpoint(journal.records(), to)?
        };
        if let Some(assets) = removed_assets {
            extra.insert(
                "removed_assets".into(),
                serde_json::to_value(assets.keys().collect::<Vec<_>>())?,
            );
        }
        let rec = self.set_timeline_as_of(
            &target,
            "project.revert",
            json!({ "to": to }),
            Some(format!("revert to {to}")),
            actor,
            extra,
        )?;
        // Cursor-sync nicety (documented boundary): a rail revert is the
        // "deliberate jump" tool, separate from the Ctrl+Z cursor. If its
        // target op IS in the linear history, move the cursor onto it so a
        // following Ctrl+Z / redo stays coherent when the rail and keyboard are
        // mixed. If the target is not a history element (e.g. a checkpoint op),
        // the cursor is left as-is — revert is not a history edit, so it never
        // pushes either way.
        if let Some(p) = target_cursor {
            self.undo_pos = p;
        }
        Ok(vec![rec.op_id])
    }

    /// Set the live timeline to the state AS OF the (already-resolved, must-
    /// exist) op `at_op` — the log replayed up to AND INCLUDING it — and APPEND
    /// one nav op `verb` carrying the recomputed timeline in its
    /// `restored_timeline` effect (so replay reproduces it directly via the
    /// shared `apply_record` arm; the log is never rewritten — the append-only operation-log contract). The
    /// `extra` map is merged into the op's effect detail (e.g. `reverted_to`,
    /// `to_op`). Shared core of `revert` / `undo` / `redo`; only the verb label,
    /// args, rationale and extra detail differ.
    fn set_timeline_as_of(
        &mut self,
        at_op: &str,
        verb: &str,
        args: Value,
        rationale: Option<String>,
        actor: Actor,
        mut extra: serde_json::Map<String, Value>,
    ) -> Result<OpRecord, CutError> {
        let journal = self.log.replay_view()?;
        let all = journal.records();
        let idx = all.iter().position(|o| o.op_id == at_op).ok_or_else(|| {
            CutError::new(
                codes::NOT_FOUND,
                format!("op '{at_op}' not found in the log"),
                "checkpoints resolve to their at_op; that op must exist",
            )
        })?;
        let assignments = op_sequence_assignments(all)?;
        let target_sequence = &assignments[idx];
        if target_sequence != &self.project.active_sequence {
            return Err(CutError::new(
                codes::GUARDRAIL,
                format!(
                    "op '{at_op}' belongs to sequence '{}', not the active sequence '{}'",
                    target_sequence, self.project.active_sequence
                ),
                "checkpoints, undo, redo, revert, and restore are sequence-scoped",
            )
            .with_suggested_action(format!(
                "switch to sequence '{}' before using this history position",
                target_sequence
            )));
        }
        // Deterministic by the roundtrip gate: replaying up to AND INCLUDING the
        // target op IS exactly its timeline state.
        let (rebuilt, _) = snapshots::rebuild(&self.dir, &journal, idx + 1)?;
        let snap = timeline_snapshot(&rebuilt);
        let mut next = self.project.clone();
        edit::apply_set_timeline(&mut next, &snap.args)?;
        next.sync_active_sequence();
        if let Some(removed) = extra.get("removed_assets") {
            let ids: Vec<String> = serde_json::from_value(removed.clone())?;
            if let Some((asset_id, clip_id)) = ids.iter().find_map(|asset_id| {
                next.all_sequence_tracks()
                    .flat_map(|track| track.clips.iter())
                    .find_map(|clip| match clip {
                        Clip::Media(media) if media.asset == *asset_id => {
                            Some((asset_id.clone(), media.id.clone()))
                        }
                        _ => None,
                    })
            }) {
                return Err(CutError::new(
                    codes::GUARDRAIL,
                    format!("cannot remove Motion asset '{asset_id}' while clip '{clip_id}' uses it"),
                    "another sequence still references media owned by this Motion plan",
                )
                .with_suggested_action(
                    "remove or replace dependent clips in the other sequence before undoing/reverting the Motion plan",
                ));
            }
            for id in ids {
                next.assets.remove(&id);
            }
        }
        if let Some(restored) = extra.get("restored_assets") {
            let assets: BTreeMap<String, Asset> = serde_json::from_value(restored.clone())?;
            for (id, asset) in assets {
                next.assets.insert(id, asset);
            }
        }
        if let Some(removed) = extra.get("removed_checkpoints") {
            let ids: Vec<String> = serde_json::from_value(removed.clone())?;
            next.checkpoints
                .retain(|checkpoint| !ids.contains(&checkpoint.id));
        }
        if let Some(restored) = extra.get("restored_checkpoints") {
            let checkpoints: Vec<Checkpoint> = serde_json::from_value(restored.clone())?;
            for checkpoint in checkpoints {
                if !next
                    .checkpoints
                    .iter()
                    .any(|existing| existing.id == checkpoint.id)
                {
                    next.checkpoints.push(checkpoint);
                }
            }
        }
        extra.insert("restored_timeline".into(), snap.args);
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: verb.to_string(),
            args,
            rationale,
            effects: vec![edit::fx(None, Value::Object(extra))],
            inverse: None,
            status: OpStatus::Applied,
        };
        // The record owns every value derived from `journal`; release the
        // immutable view before its append to avoid full-vector Arc COW.
        drop(journal);
        self.commit_staged(next, &rec)?;
        Ok(rec)
    }

    /// True when there is an earlier edit the cursor can step back to.
    pub fn undo_available(&self) -> bool {
        self.undo_pos > 0
    }

    /// True when there is an undone edit the cursor can step forward to.
    pub fn redo_available(&self) -> bool {
        self.undo_pos + 1 < self.undo_history.len()
    }

    /// Step the linear history cursor back one edit and set the timeline to that
    /// older state. Fixes the oscillation bug: undoing twice lands on a
    /// strictly OLDER state — never the redo — because the cursor moves, the tip
    /// does not. Appends one `project.undo` nav op (itself not a history edit).
    /// Refuses with a clean guardrail at the baseline (nothing to undo).
    pub fn undo(&mut self, actor: Actor) -> Result<OpRecord, CutError> {
        if self.undo_pos == 0 {
            return Err(CutError::new(
                codes::GUARDRAIL,
                "nothing to undo — already at the start of the edit history",
                "make an edit first; use redo to step forward through undone edits",
            ));
        }
        let leaving = self.undo_history[self.undo_pos].clone();
        let (removed_assets, removed_checkpoint) = {
            let journal = self.log.replay_view()?;
            let all = journal.records();
            (
                motion_assets_for_op(all, &leaving)?,
                motion_checkpoint_for_op(all, &leaving)?,
            )
        };
        let target_pos = self.undo_pos - 1;
        let target = self.undo_history[target_pos].clone();
        let mut extra = serde_json::Map::new();
        extra.insert("to_op".into(), json!(target));
        extra.insert("cursor".into(), json!(target_pos));
        if let Some(assets) = removed_assets {
            extra.insert(
                "removed_assets".into(),
                serde_json::to_value(assets.keys().collect::<Vec<_>>())?,
            );
        }
        if let Some(checkpoint) = removed_checkpoint {
            extra.insert("removed_checkpoints".into(), json!([checkpoint.id]));
        }
        let result = self.set_timeline_as_of(
            &target,
            "project.undo",
            json!({ "to_op": target }),
            Some("undo".into()),
            actor,
            extra,
        )?;
        self.undo_pos = target_pos;
        self.tip_group = None;
        Ok(result)
    }

    /// Step the linear history cursor FORWARD one edit (re-applying an undone
    /// edit) and set the timeline to that state. Appends one `project.redo`
    /// nav op. Refuses with a clean guardrail at the tip (nothing to redo).
    pub fn redo(&mut self, actor: Actor) -> Result<OpRecord, CutError> {
        if self.undo_pos + 1 >= self.undo_history.len() {
            return Err(CutError::new(
                codes::GUARDRAIL,
                "nothing to redo — already at the latest edit",
                "redo only steps forward through edits you have undone",
            ));
        }
        let target_pos = self.undo_pos + 1;
        let target = self.undo_history[target_pos].clone();
        let (restored_assets, restored_checkpoint) = {
            let journal = self.log.replay_view()?;
            let all = journal.records();
            (
                motion_assets_for_op(all, &target)?,
                motion_checkpoint_for_op(all, &target)?,
            )
        };
        let mut extra = serde_json::Map::new();
        extra.insert("to_op".into(), json!(target));
        extra.insert("cursor".into(), json!(target_pos));
        if let Some(assets) = restored_assets {
            extra.insert("restored_assets".into(), serde_json::to_value(assets)?);
        }
        if let Some(checkpoint) = restored_checkpoint {
            extra.insert("restored_checkpoints".into(), json!([checkpoint]));
        }
        let result = self.set_timeline_as_of(
            &target,
            "project.redo",
            json!({ "to_op": target }),
            Some("redo".into()),
            actor,
            extra,
        )?;
        self.undo_pos = target_pos;
        self.tip_group = None;
        Ok(result)
    }

    /// Selective non-tip undo ("op rebase"): reproduce the timeline AS IF the
    /// op `target_op_id` never happened, WITHOUT discarding the ops after it
    /// APPENDS a new `edit.restore{mode:"rebase"}`
    /// op carrying the recomputed timeline (the log is never rewritten —
    /// the append-only operation-log contract / timeline/op-log contract). Its
    /// computed result timeline is recorded in the effect, so the rebase is
    /// itself tip-undoable by recomputing its pre-op journal prefix.
    ///
    /// SAFETY (the prime rule: a half-working rebase is worse than none):
    /// 1. The target must be a timeline-mutating op. Refuses
    ///    project.create / checkpoint / import (nothing to rebase).
    /// 2. DEPENDENCY GATE — if ANY later op references an id the target created,
    ///    REFUSE with a structured guardrail error naming the blockers
    ///    (rebase::rebase_refusal). Only provably-independent ops are rebased.
    /// 3. VERIFY-REPLAY — the post-skip timeline is computed by skip-replay
    ///    (rebuild_skipping, with every surviving id PINNED). Before committing,
    ///    we re-run skip-replay a SECOND time and require byte-identical output
    ///    (determinism self-check) — a belt-and-braces guard that the recomputed
    ///    state is reproducible. If the two diverge we refuse (never commit a
    ///    non-deterministic rebase).
    ///
    /// Returns the appended op's id + the ids of the later ops it rebased over.
    pub fn rebase_out(
        &mut self,
        target_op_id: &str,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<(OpRecord, Vec<String>), CutError> {
        // A selective skip cannot safely start from a snapshot that already
        // contains the target op, so rebase keeps its two full semantic
        // skip-replays. Its durable input is nevertheless the open-time
        // journal view: no extra journal reread or prefix rehash is needed.
        let journal = self.log.replay_view()?;
        let all = journal.records();
        let pos = all
            .iter()
            .position(|o| o.op_id == target_op_id)
            .ok_or_else(|| {
                CutError::new(
                    codes::NOT_FOUND,
                    format!("no op '{target_op_id}' in the log"),
                    "op ids come from project.ops",
                )
            })?;
        let assignments = op_sequence_assignments(all)?;
        let target_sequence = &assignments[pos];
        if target_sequence != &self.project.active_sequence {
            return Err(CutError::new(
                codes::GUARDRAIL,
                format!(
                    "op '{target_op_id}' belongs to sequence '{}', not the active sequence '{}'",
                    target_sequence, self.project.active_sequence
                ),
                "selective restore is sequence-scoped",
            )
            .with_suggested_action(format!(
                "switch to sequence '{}' before rebasing this op",
                target_sequence
            )));
        }
        // (1) The target must be a timeline mutation. Non-timeline ops
        // (project.create/checkpoint/import) own no timeline state to rebase
        // out — refuse with an actionable message.
        if all[pos].verb == "import.otio" {
            return Err(CutError::new(
                codes::GUARDRAIL,
                format!("op '{target_op_id}' is an atomic timeline import"),
                "selectively removing the import would orphan every later imported track and asset reference",
            )
            .with_suggested_action("use project.undo or project.revert to remove the whole import and its dependent edits"));
        }
        if !all[pos].mutates_timeline()? {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("op '{target_op_id}' ({}) is not a timeline op — nothing to rebase out", all[pos].verb),
                "rebase removes a timeline edit; project.create / checkpoint / import have no timeline effect",
            )
            .with_suggested_action("pick a timeline-mutating op (edit.*, transcript.*, captions.*)"));
        }
        // (2) Dependency gate — refuse (loudly, naming the dependents) if any
        // later op consumes an id this op created.
        let blockers = crate::rebase::rebase_blockers(all, pos);
        if !blockers.is_empty() {
            return Err(crate::rebase::rebase_refusal(target_op_id, &blockers));
        }
        // (3) Compute the post-skip timeline by id-pinned skip-replay, then
        // re-run it once and require an identical result (determinism check).
        // If skip-replay itself fails, the dependency gate missed a real
        // dependency — wrap the raw verb-replay error in an honest guardrail
        // refusal rather than leaking e.g. a bare "no cut at 4000ms" that names
        // neither the rebase target nor the actual blocker.
        let rebuilt = rebuild_skipping(all, pos)
            .map_err(|e| crate::rebase::rebase_unreproducible(target_op_id, &e))?;
        let rebuilt_again = rebuild_skipping(all, pos)
            .map_err(|e| crate::rebase::rebase_unreproducible(target_op_id, &e))?;
        if serde_json::to_string(&rebuilt)? != serde_json::to_string(&rebuilt_again)? {
            return Err(CutError::new(
                codes::CONFLICT,
                format!("rebase of '{target_op_id}' is not reproducible (skip-replay diverged)"),
                "two skip-replays of the same log produced different timelines — refusing to commit a non-deterministic rebase",
            )
            .with_suggested_action("this is a bug in the rebase machinery; report the op log"));
        }
        // The ids of the later ops the rebase reorders over (for the result +
        // an audit trail on the op).
        let rebased_over: Vec<String> = all[pos + 1..]
            .iter()
            .map(|op| {
                op.mutates_timeline()
                    .map(|is_timeline| is_timeline.then(|| op.op_id.clone()))
            })
            .collect::<Result<Vec<_>, CutError>>()?
            .into_iter()
            .flatten()
            .collect();

        // APPEND the rebase op. The recomputed timeline rides in
        // `rebase_new_timeline` so replay reproduces it without re-running the
        // dependency analysis; the fresh op carries no legacy inverse
        // (recompute-by-replay), and is itself tip-undoable (a tip restore recomputes the
        // pre-rebase timeline from the log prefix).
        // Stage the recomputed timeline for the live cache.
        let mut next = self.project.clone();
        next.tracks = rebuilt.tracks.clone();
        next.markers = rebuilt.markers.clone();
        next.caption_styles = rebuilt.caption_styles.clone();
        next.adjustments = rebuilt.adjustments.clone();
        next.nests = rebuilt.nests.clone();
        let new_snapshot = json!({
            "tracks": rebuilt.tracks,
            "markers": rebuilt.markers,
            "caption_styles": rebuilt.caption_styles,
            "adjustments": rebuilt.adjustments,
            "nests": rebuilt.nests,
        });
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "edit.restore".into(),
            args: json!({"op_id": target_op_id, "mode": "rebase"}),
            rationale,
            effects: vec![edit::fx(
                None,
                json!({
                    "restored_op": target_op_id,
                    "mode": "rebase",
                    "rebased_over": rebased_over,
                    // The recomputed timeline — replay applies this directly.
                    "rebase_new_timeline": new_snapshot,
                }),
            )],
            inverse: None, // recompute-by-replay: result rides in the effect
            status: OpStatus::Applied,
        };
        // Rebase has finished its in-memory dependency and skip-replay work;
        // do not retain its immutable view across the durable append.
        drop(journal);
        self.commit_staged(next, &rec)?;
        Ok((rec, rebased_over))
    }

    /// receipts/ subdir path (perception.json, render receipts live here).
    pub fn receipts_dir(&self) -> PathBuf {
        self.dir.join("receipts")
    }

    /// proxies/ subdir path (gitignored, regenerable).
    pub fn proxies_dir(&self) -> PathBuf {
        self.dir.join("proxies")
    }
}

/// A timeline snapshot as an `edit._set_timeline` op: dispatching it with these
/// args sets the timeline (tracks, markers, caption styles, adjustments, nests)
/// to `project`'s. Optional fields stay optional in apply_set_timeline so older
/// snapshots still apply.
///
/// Recompute-by-replay model: this is NO LONGER stored on every
/// mutating op (that was the O(N²) disk hog). It is now used to build the
/// RESULT-timeline effect that a restore/revert op records (computed from a
/// `rebuild_from_log` of the log prefix), so replay can reproduce the restore
/// directly. Still re-exported because the rebase path and historic-log replay
/// reference the same `edit._set_timeline` shape.
/// Extract the optional `group_id` meta-arg from a verb's args and
/// turn it into a `group_id` effect, so consecutive ops of ONE linked user
/// action (linked A/V paste, linked delete) carry the same tag and the undo
/// cursor steps over them together. The tag rides in the effects (NOT a new
/// OpRecord field, avoiding churn across every literal); it is persisted in the
/// log and survives reopen. Returns `None` if absent or blank. The typed verb
/// arg-parsers ignore this extra key (serde ignores unknown fields), so it is a
/// pure meta-arg with no schema change.
fn group_effect(args: &Value) -> Option<OpEffect> {
    let g = args.get("group_id").and_then(|v| v.as_str())?;
    let g = g.trim();
    if g.is_empty() {
        return None;
    }
    Some(edit::fx(None, json!({ "group_id": g })))
}

pub fn timeline_snapshot(project: &Project) -> InverseOp {
    InverseOp {
        verb: "edit._set_timeline".into(),
        args: json!({
            "tracks": project.tracks,
            "markers": project.markers,
            "caption_styles": project.caption_styles,
            "adjustments": project.adjustments,
            "nests": project.nests,
            "transcript_ignores": project.transcript_ignores,
        }),
    }
}

/// Apply one `edit.*` verb's args to a project — the shared verb→function
/// table used by BOTH the live path (ProjectStore::apply) and replay
/// (apply_record). Pure + deterministic: same project + verb + args ⇒ same
/// mutation and same allocated ids.
///
/// This is the LIVE entry: it allocates ids positionally (and the edit fns
/// record what they allocated). The REPLAY / skip-replay path calls
/// [`apply_edit_verb_pinned`] with the op's recorded ids so allocation order no
/// longer matters — the prerequisite that lets a rebase skip an earlier op
/// without renumbering later ids (rebase.rs).
pub fn apply_edit_verb(
    project: &mut Project,
    verb: &str,
    args: &Value,
) -> Result<Vec<OpEffect>, CutError> {
    apply_edit_verb_pinned(project, verb, args, None)
}

/// Id-pinning variant of [`apply_edit_verb`]: when `pinned` is `Some`, every
/// allocating verb (split/insert/move/ripple_delete/add_track/add_marker)
/// consumes the id it RECORDED in its effects instead of re-deriving it
/// positionally. `pinned` is built from an op's recorded effects
/// ([`crate::rebase::PinnedIds::from_effects`]).
///
/// Determinism contract preserved: in a NORMAL (no-skip) replay the recorded id
/// EQUALS what positional allocation would mint, so pinning is byte-identical to
/// not pinning — tests/roundtrip.rs proves this. Pinning only DIVERGES from
/// positional allocation when an earlier op was skipped (the rebase case),
/// which is exactly where positional allocation would corrupt the id graph.
///
/// `pinned: None` ⇒ identical behavior to the legacy live path (positional).
///
/// `pinned` is `&mut` because the QUEUE roles (markers, ripple range-edge
/// splits) are consumed POSITIONALLY: when one op's effects feed several
/// lowered steps (audio.add_music → many add_marker), each step pops the next
/// recorded id. Single-allocation roles (split/insert/move/add_track) just read.
pub fn apply_edit_verb_pinned(
    project: &mut Project,
    verb: &str,
    args: &Value,
    mut pinned: Option<&mut crate::rebase::PinnedIds>,
) -> Result<Vec<OpEffect>, CutError> {
    /// serde default for `ripple` flags whose pre-flag behavior was close-gap
    /// (`edit.ripple_delete`). A missing key in an old op MUST replay as
    /// true (the original behavior), so the default is true, not Rust's false.
    fn default_true() -> bool {
        true
    }
    match verb {
        "edit.split" => {
            #[derive(serde::Deserialize)]
            struct A {
                track: String,
                at_ms: u64,
            }
            let a: A = serde_json::from_value(args.clone())?;
            // Pin the right-half clip id on replay (rebase id-stability).
            let pin = pinned.as_ref().and_then(|p| p.split_right.clone());
            edit::split_pinned(project, &a.track, a.at_ms, pin.as_deref())
        }
        "edit.ripple_delete" => {
            #[derive(serde::Deserialize)]
            struct A {
                track: Option<String>,
                range_ms: [u64; 2],
                /// Ripple/lift selector. serde DEFAULT true: ops
                /// logged before the flag existed (every cut + the transcript
                /// verbs' lowered deletes) replay as close-gap, their original
                /// behavior — replay determinism over new defaults.
                #[serde(default = "default_true")]
                ripple: bool,
            }
            let a: A = serde_json::from_value(args.clone())?;
            // Pin range-edge split ids on replay (rebase id-stability). A
            // track=None ripple can split ONE clip PER TRACK; the recorded
            // `split_clip`s ride in the queue IN TRACK ORDER. INVARIANT: in this
            // codebase a single op runs at most ONE ripple_delete (direct ops,
            // and the only lowered ripple producer — transcript.cut_words —
            // lowers to exactly one), so draining the queue here consumes
            // precisely this op's recorded splits. (A hypothetical future
            // lowered op with multiple ripple steps would need a per-step split
            // tag; the dependency gate + rebase_out verify-replay backstop any
            // mis-pin by refusing rather than corrupting.)
            let split_ids: Vec<String> = match pinned.as_mut() {
                Some(p) => {
                    let mut v = Vec::new();
                    while let Some(id) = p.next_ripple_split() {
                        v.push(id);
                    }
                    v
                }
                None => Vec::new(),
            };
            let pin = if split_ids.is_empty() {
                None
            } else {
                Some(split_ids.as_slice())
            };
            edit::ripple_delete_pinned(project, a.track.as_deref(), a.range_ms, a.ripple, pin)
        }
        "edit.trim" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                src_in_ms: Option<u64>,
                src_out_ms: Option<u64>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::trim(project, &a.clip, a.src_in_ms, a.src_out_ms)
        }
        "edit.speed" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                factor: f64,
                // preserve_pitch rides in args for the audit trail but is NOT
                // read by core (v1 always preserves pitch at render; the verb
                // layer rejects preserve_pitch:false). serde ignores it here.
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::speed(project, &a.clip, a.factor)
        }
        "edit.speed_ramp" => {
            // The dispatch layer validates points (≥2, sorted, factor range) and
            // resolves `segments` (default + clamp) and the output timebase BEFORE
            // commit, so replay reproduces the exact curve, granularity, and frame
            // grid. Historic ops omit both timebase fields and keep millisecond
            // semantics. An EMPTY points list clears the ramp. preserve_pitch rides
            // in args for the audit trail (rejected if false at the verb layer; the
            // render always preserves pitch) and is ignored by core.
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                #[serde(default)]
                points: Vec<crate::types::SpeedRampPoint>,
                #[serde(default = "default_ramp_segments")]
                segments: usize,
                #[serde(default)]
                timebase_fps: Option<f64>,
                #[serde(default)]
                timebase_audio_rate: Option<u32>,
            }
            fn default_ramp_segments() -> usize {
                crate::types::DEFAULT_RAMP_SEGMENTS
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::speed_ramp(
                project,
                &a.clip,
                a.points,
                a.segments,
                a.timebase_fps,
                a.timebase_audio_rate,
            )
        }
        "edit.move" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                to_track: String,
                at_ms: u64,
                /// AV-sync ripple at the destination. serde DEFAULT
                /// false: ops logged before the flag replay as the original
                /// float move (only the dest track changes).
                #[serde(default)]
                ripple: bool,
            }
            let a: A = serde_json::from_value(args.clone())?;
            // Pin the destination splice-split id on replay (rebase id-stability).
            let pin = pinned.as_mut().and_then(|p| p.next_split_clip());
            edit::move_clip_pinned(
                project,
                &a.clip,
                &a.to_track,
                a.at_ms,
                a.ripple,
                pin.as_deref(),
            )
        }
        // Internal final step of one lowered linked A/V move. The two clip
        // destinations are already populated, so ripple every other media
        // track exactly once and remap captions/markers/windows once.
        "edit._ripple_open_gap" => {
            #[derive(serde::Deserialize)]
            struct A {
                exclude_tracks: Vec<String>,
                at_ms: u64,
                duration_ms: u64,
            }
            let a: A = serde_json::from_value(args.clone())?;
            let exclude_tracks: Vec<&str> = a.exclude_tracks.iter().map(String::as_str).collect();
            let mut effects = Vec::new();
            edit::ripple_open_gap_at_excluding(
                project,
                &exclude_tracks,
                a.at_ms,
                a.duration_ms,
                &mut effects,
            );
            Ok(effects)
        }
        "edit.insert" => {
            #[derive(serde::Deserialize)]
            struct A {
                asset: String,
                track: String,
                at_ms: u64,
                src_range_ms: Option<[u64; 2]>,
                /// Sibling-track ripple (the ripple-sync contract). serde DEFAULT false: ops
                /// logged before the flag existed replay with their original
                /// single-track behavior — replay determinism over new
                /// defaults. The live path (dispatch) always records the
                /// resolved value explicitly.
                #[serde(default)]
                ripple: bool,
            }
            let a: A = serde_json::from_value(args.clone())?;
            // Pin the added clip id AND the splice-split id on replay.
            let pin_clip = pinned.as_mut().and_then(|p| p.next_added_clip());
            let pin_split = pinned.as_mut().and_then(|p| p.next_split_clip());
            edit::insert_pinned(
                project,
                &a.asset,
                &a.track,
                a.at_ms,
                a.src_range_ms,
                a.ripple,
                pin_clip.as_deref(),
                pin_split.as_deref(),
            )
        }
        "edit.duplicate" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                /// Sibling-track ripple (the ripple-sync contract), mirrors edit.insert. serde
                /// DEFAULT false: the dispatch layer always records the resolved
                /// value explicitly (a lone clip resolves from its track; the
                /// linked-A/V pair forces false on both halves so they stay
                /// aligned). The default only governs a hand-written op with the
                /// key omitted — false is the conservative no-cross-track-shift.
                #[serde(default)]
                ripple: bool,
            }
            let a: A = serde_json::from_value(args.clone())?;
            // Pin the cloned clip id on replay (rebase id-stability). One id per
            // op ⇒ a single Option (the linked-audio half is a separate op).
            let pin = pinned.as_ref().and_then(|p| p.added_clip.clone());
            edit::duplicate_pinned(project, &a.clip, a.ripple, pin.as_deref())
        }
        "edit.nest" => {
            // Compound clip / nest: MOVE a contiguous run of clips into a sub-timeline
            // (Project::nests) and replace them on the parent track with a single nest
            // clip. The dispatch layer passes the selection verbatim; core validates
            // contiguity/same-track/media-only and allocates the sub-timeline.
            #[derive(serde::Deserialize)]
            struct A {
                clips: Vec<String>,
                #[serde(default)]
                name: Option<String>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            // Pin the nest CLIP id on replay (rebase id-stability); the nest id itself
            // is allocated deterministically (max nest index + 1). One id per op.
            let pin = pinned.as_ref().and_then(|p| p.added_clip.clone());
            edit::nest_pinned(project, &a.clips, a.name.as_deref(), pin.as_deref())
        }
        "edit.replace" => {
            // 3-point "replace edit": swap a clip's SOURCE in place, preserving its
            // slot. The dispatch layer resolves the source descriptor (asset +
            // source window) and records it verbatim; core re-derives the slot,
            // usable window, and any pad gap deterministically. NO new clip id is
            // allocated (the target keeps its id), so there is nothing to pin.
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                asset: String,
                source_in_ms: Option<u64>,
                source_out_ms: Option<u64>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::replace(project, &a.clip, &a.asset, a.source_in_ms, a.source_out_ms)
        }
        "edit.fit_to_fill" => {
            // "Fit to Fill": place speed-adjusted footage into an empty
            // slot so it exactly fills it. The dispatch layer resolves the slot
            // duration (explicit, or the gap at at_ms) and the source window and
            // records them explicitly, so core re-derives the same speed on replay.
            #[derive(serde::Deserialize)]
            struct A {
                track: String,
                at_ms: u64,
                slot_ms: u64,
                asset: String,
                src_range_ms: [u64; 2],
            }
            let a: A = serde_json::from_value(args.clone())?;
            // Pin the placed clip id on replay (rebase id-stability), like insert.
            let pin = pinned.as_ref().and_then(|p| p.added_clip.clone());
            edit::fit_to_fill_pinned(
                project,
                &a.track,
                a.at_ms,
                a.slot_ms,
                &a.asset,
                a.src_range_ms[0],
                a.src_range_ms[1],
                pin.as_deref(),
            )
        }
        "edit.gain" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: Option<String>,
                track: Option<String>,
                db: f64,
            }
            let a: A = serde_json::from_value(args.clone())?;
            let target = match (a.clip, a.track) {
                (Some(c), None) => edit::GainTarget::Clip(c),
                (None, Some(t)) => edit::GainTarget::Track(t),
                _ => {
                    return Err(CutError::new(
                        codes::INVALID_ARGS,
                        "edit.gain needs exactly one of `clip` or `track`",
                        "both or neither were given",
                    ))
                }
            };
            edit::gain(project, target, a.db)
        }
        "edit.add_track" => {
            #[derive(serde::Deserialize)]
            struct A {
                kind: crate::types::TrackKind,
                id: Option<String>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            // On replay, pin the recorded track id (effect `added_track`) so a
            // skip-replay keeps an auto-allocated `vN`/`aNt` stable. The
            // explicit `id` arg (if the live op supplied one) still wins via
            // the recorded effect, which equals it.
            let pin = pinned.as_ref().and_then(|p| p.added_track.clone());
            edit::add_track(project, a.kind, pin.as_deref().or(a.id.as_deref()))
        }
        "edit.reorder_track" => {
            #[derive(serde::Deserialize)]
            struct A {
                track: String,
                index: usize,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::reorder_track(project, &a.track, a.index)
        }
        "edit.remove_track" => {
            #[derive(serde::Deserialize)]
            struct A {
                track: String,
                #[serde(default)]
                force: bool,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::remove_track(project, &a.track, a.force)
        }
        "edit.blend" => {
            #[derive(serde::Deserialize)]
            struct A {
                track: String,
                #[serde(default = "default_blend_mode")]
                mode: String,
            }
            fn default_blend_mode() -> String {
                "normal".to_string()
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::set_track_blend(project, &a.track, &a.mode)
        }
        "edit.track_visible" => {
            #[derive(serde::Deserialize)]
            struct A {
                track: String,
                on: bool,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::set_track_visible(project, &a.track, a.on)
        }
        "edit.track_lock" => {
            #[derive(serde::Deserialize)]
            struct A {
                track: String,
                on: bool,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::set_track_locked(project, &a.track, a.on)
        }
        "edit.mute" => {
            #[derive(serde::Deserialize)]
            struct A {
                track: String,
                on: bool,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::set_track_muted(project, &a.track, a.on)
        }
        "edit.solo" => {
            #[derive(serde::Deserialize)]
            struct A {
                track: String,
                on: bool,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::set_track_solo(project, &a.track, a.on)
        }
        "edit.pan" => {
            #[derive(serde::Deserialize)]
            struct A {
                track: String,
                pan: f64,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::set_track_pan(project, &a.track, a.pan)
        }
        "edit.slip" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                by_ms: i64,
            }
            let a: A = serde_json::from_value(args.clone())?;
            crate::trim_edit::slip(project, &a.clip, a.by_ms)
        }
        "edit.roll" => {
            #[derive(serde::Deserialize)]
            struct A {
                track: String,
                at_ms: u64,
                by_ms: i64,
            }
            let a: A = serde_json::from_value(args.clone())?;
            crate::trim_edit::roll(project, &a.track, a.at_ms, a.by_ms)
        }
        "edit.slide_edit" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                by_ms: i64,
            }
            let a: A = serde_json::from_value(args.clone())?;
            crate::trim_edit::slide_edit(project, &a.clip, a.by_ms)
        }
        "edit.transform" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                #[serde(default)]
                x: f64,
                #[serde(default)]
                y: f64,
                #[serde(default = "default_scale")]
                scale: f64,
                #[serde(default = "crate::types::default_opacity")]
                opacity: f64,
            }
            fn default_scale() -> f64 {
                1.0
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::transform(
                project,
                &a.clip,
                crate::types::ClipTransform {
                    x: a.x,
                    y: a.y,
                    scale: a.scale,
                    opacity: a.opacity,
                },
            )
        }
        "edit.crop" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                x: u32,
                y: u32,
                w: u32,
                h: u32,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::crop(
                project,
                &a.clip,
                crate::types::ClipCrop {
                    x: a.x,
                    y: a.y,
                    w: a.w,
                    h: a.h,
                },
            )
        }
        "edit.grade" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                #[serde(default = "grade_one")]
                contrast: f64,
                #[serde(default)]
                brightness: f64,
                #[serde(default = "grade_one")]
                saturation: f64,
                #[serde(default = "grade_one")]
                gamma: f64,
                #[serde(default)]
                temperature_k: Option<u32>,
                #[serde(default)]
                lut: Option<String>,
            }
            fn grade_one() -> f64 {
                1.0
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::grade(
                project,
                &a.clip,
                crate::types::ClipGrade {
                    contrast: a.contrast,
                    brightness: a.brightness,
                    saturation: a.saturation,
                    gamma: a.gamma,
                    temperature_k: a.temperature_k,
                    lut: a.lut,
                },
            )
        }
        "edit.grade_stack" => {
            // Layered grade stack: a list of ClipGrade layers applied in order. Each
            // element deserializes directly into ClipGrade (its fields carry serde
            // defaults — contrast/saturation/gamma=1, brightness=0), so a layer may
            // specify only the knobs it changes. Pure replay (clip fields only).
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                #[serde(default)]
                grades: Vec<crate::types::ClipGrade>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::grade_stack(project, &a.clip, a.grades)
        }
        "edit.grade_window" => {
            // GEOMETRIC POWER WINDOW: a region (shape/points/feather/invert — the
            // edit.add_mask geometry) + the same ClipGrade edit.grade takes, applied
            // ONLY inside the region. remove_index removes one window atomically;
            // enabled:false CLEARS all windows; else APPEND one window. The op log stores
            // the resolved request so replay never depends on defaults. Pure replay
            // (clip fields only).
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                #[serde(default = "default_true")]
                enabled: bool,
                #[serde(default)]
                remove_index: Option<usize>,
                shape: Option<crate::types::MaskShape>,
                #[serde(default)]
                points: Vec<[f64; 2]>,
                #[serde(default)]
                feather: f64,
                #[serde(default)]
                invert: bool,
                // The grade applied inside the window — same shape + defaults as edit.grade.
                #[serde(default = "grade_one")]
                contrast: f64,
                #[serde(default)]
                brightness: f64,
                #[serde(default = "grade_one")]
                saturation: f64,
                #[serde(default = "grade_one")]
                gamma: f64,
                #[serde(default)]
                temperature_k: Option<u32>,
                #[serde(default)]
                lut: Option<String>,
            }
            fn grade_one() -> f64 {
                1.0
            }
            let a: A = serde_json::from_value(args.clone())?;
            if a.remove_index.is_some() {
                const APPEND_OR_CLEAR_KEYS: &[&str] = &[
                    "shape",
                    "points",
                    "feather",
                    "invert",
                    "contrast",
                    "brightness",
                    "saturation",
                    "gamma",
                    "temperature_k",
                    "lut",
                    "enabled",
                ];
                if let Some(key) = APPEND_OR_CLEAR_KEYS
                    .iter()
                    .find(|key| args.get(**key).is_some())
                {
                    return Err(crate::error::CutError::new(
                        crate::error::codes::INVALID_ARGS,
                        format!("edit.grade_window remove_index cannot be combined with '{key}'"),
                        "pass only clip + remove_index (and optional rationale) to remove one window",
                    ));
                }
            }
            let window = if a.remove_index.is_some() {
                None
            } else if a.enabled {
                let shape = a.shape.ok_or_else(|| {
                    crate::error::CutError::new(
                        crate::error::codes::INVALID_ARGS,
                        "edit.grade_window needs a shape (rect|ellipse|polygon) unless enabled:false",
                        "pass shape + points + grade params, or enabled:false to clear all windows",
                    )
                })?;
                Some(crate::types::WindowShape {
                    shape,
                    points: a.points,
                    feather: a.feather,
                    invert: a.invert,
                })
            } else {
                None
            };
            edit::grade_window(
                project,
                &a.clip,
                window,
                crate::types::ClipGrade {
                    contrast: a.contrast,
                    brightness: a.brightness,
                    saturation: a.saturation,
                    gamma: a.gamma,
                    temperature_k: a.temperature_k,
                    lut: a.lut,
                },
                a.remove_index,
            )
        }
        "edit.color_space" => {
            // Tag a clip's INPUT color space. `input` omitted / null clears the tag.
            // Pure replay: just a clip field (the dispatch layer validated the name).
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                #[serde(default)]
                input: Option<String>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            let space = match a.input.as_deref() {
                None => None,
                Some(s) => Some(crate::types::ColorSpace::parse(s).ok_or_else(|| {
                    CutError::new(
                        codes::INVALID_ARGS,
                        format!("unknown color space '{s}'"),
                        format!(
                            "supported input spaces: {}",
                            crate::types::ColorSpace::SUPPORTED
                        ),
                    )
                })?),
            };
            edit::set_color_space(project, &a.clip, space)
        }
        "edit.matte" => {
            // Pure replay: build the matte INTENT and store it. The alpha is baked
            // (network) at the dispatch layer only — replay never touches the
            // sidecar/filesystem. enabled:false clears the matte.
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                #[serde(default)]
                enabled: Option<bool>,
                #[serde(default)]
                mode: Option<crate::types::MatteMode>,
                #[serde(default)]
                model: Option<crate::types::MatteModel>,
                #[serde(default)]
                bg: Option<crate::types::MatteBg>,
                #[serde(default)]
                quality: Option<crate::types::MatteQuality>,
                #[serde(default)]
                seed: Option<crate::types::MatteSeed>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            let matte = if a.enabled == Some(false) {
                None
            } else {
                Some(crate::types::ClipMatte {
                    mode: a.mode.unwrap_or_default(),
                    model: a.model.unwrap_or_default(),
                    bg: a.bg,
                    quality: a.quality.unwrap_or_default(),
                    seed: a.seed,
                })
            };
            edit::matte(project, &a.clip, matte)
        }
        "edit.effect" => {
            // effects deserialize through the typed ClipEffect enum, so an unknown
            // `type` or a bad field is a structured parse error (no broken filter
            // can reach the renderer). SET semantics: replaces the clip's list.
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                #[serde(default)]
                effects: Vec<crate::types::ClipEffect>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::set_effects(project, &a.clip, a.effects)
        }
        "edit.adjustment" => {
            // Non-destructive ADJUSTMENT LAYER: a grade/effect band over range_ms,
            // applied to the composite beneath it. `grade` is the ClipGrade object
            // shape; `effect` (single) and/or `effects` (list) carry look effects —
            // both accepted, merged in order (effect first). Pure replay: stores the
            // layer; no clip is mutated, no id derived from clips. The deterministic
            // adjN id is allocated inside add_adjustment (pure project-state fn).
            #[derive(serde::Deserialize)]
            struct A {
                range_ms: [u64; 2],
                #[serde(default)]
                grade: Option<crate::types::ClipGrade>,
                #[serde(default)]
                effect: Option<crate::types::ClipEffect>,
                #[serde(default)]
                effects: Vec<crate::types::ClipEffect>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            let mut effects = Vec::new();
            if let Some(e) = a.effect {
                effects.push(e);
            }
            effects.extend(a.effects);
            edit::add_adjustment(project, a.range_ms, a.grade, effects)
        }
        "edit.reverse" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                // enabled defaults TRUE: edit.reverse{clip} turns reverse ON;
                // pass enabled:false to clear. serde-default keeps the verb
                // ergonomic (the common call is "reverse this clip").
                #[serde(default = "default_true")]
                enabled: bool,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::reverse(project, &a.clip, a.enabled)
        }
        "edit.stabilize" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                #[serde(default = "default_stab_smoothing")]
                smoothing: f64,
                #[serde(default = "default_true")]
                enabled: bool,
            }
            // module-local mirror of the type default so the verb is ergonomic.
            fn default_stab_smoothing() -> f64 {
                15.0
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::stabilize(project, &a.clip, a.smoothing, a.enabled)
        }
        "edit.freeze" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                // at_ms = offset into the clip's visible range of the frame to
                // hold (default 0 = the clip's first frame).
                #[serde(default)]
                at_ms: u64,
                // enabled defaults TRUE: edit.freeze{clip} freezes on frame 0;
                // pass enabled:false to clear.
                #[serde(default = "default_true")]
                enabled: bool,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::freeze(project, &a.clip, a.at_ms, a.enabled)
        }
        "edit.animate" => {
            // The dispatch layer resolves any `preset` into explicit from/to BEFORE
            // commit, so the op log stores resolved coordinates and replay never
            // depends on the preset table. Here we read from/to (+ enabled).
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                #[serde(default)]
                from: Option<crate::types::AnimState>,
                #[serde(default)]
                to: Option<crate::types::AnimState>,
                #[serde(default = "default_true")]
                enabled: bool,
            }
            let a: A = serde_json::from_value(args.clone())?;
            // enabled:false → identity (clears); else the given from/to (defaults
            // = identity per AnimState::default).
            let anim = if a.enabled {
                crate::types::ClipAnimation {
                    from: a.from.unwrap_or_default(),
                    to: a.to.unwrap_or_default(),
                }
            } else {
                crate::types::ClipAnimation {
                    from: Default::default(),
                    to: Default::default(),
                }
            };
            edit::animate(project, &a.clip, anim)
        }
        "edit.keyframe" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                param: crate::types::KfParam,
                #[serde(default)]
                points: Vec<crate::types::KfPoint>,
                #[serde(default)]
                interp: crate::types::KfInterp,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::keyframe(project, &a.clip, a.param, a.points, a.interp)
        }
        "edit.add_mask" => {
            // enabled:false (or a null mask) CLEARS the mask; else build a ClipMask
            // from shape/points/feather/invert/effect/strength. The op log stores the
            // resolved mask so replay never depends on defaults.
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                #[serde(default = "default_true")]
                enabled: bool,
                shape: Option<crate::types::MaskShape>,
                #[serde(default)]
                points: Vec<[f64; 2]>,
                #[serde(default)]
                feather: f64,
                #[serde(default)]
                invert: bool,
                #[serde(default)]
                effect: crate::types::MaskEffect,
                #[serde(default)]
                strength: Option<f64>,
                #[serde(default)]
                range_ms: Option<[u64; 2]>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            let mask = if a.enabled {
                let shape = a.shape.ok_or_else(|| {
                    crate::error::CutError::new(
                        crate::error::codes::INVALID_ARGS,
                        "edit.add_mask needs a shape (rect|ellipse|polygon) unless enabled:false",
                        "pass shape + points, or enabled:false to clear",
                    )
                })?;
                Some(crate::types::ClipMask {
                    shape,
                    points: a.points,
                    feather: a.feather,
                    invert: a.invert,
                    effect: a.effect,
                    strength: a.strength,
                    range_ms: a.range_ms,
                    track: None,
                    regions: Vec::new(),
                })
            } else {
                None
            };
            edit::add_mask(project, &a.clip, mask)
        }
        "edit.redact" => {
            // REDACTION — a time-bounded region effect for privacy. Shares the
            // `mask` field + render path with edit.add_mask, but framed for security:
            // `mode` (blur|pixelate|box→black; default blur), an over-blur FAIL-SAFE
            // default sigma when blur strength is unset (never under-redact), and the
            // `range_ms` time window (the secret is only on screen briefly). enabled:
            // false (or a null region) CLEARS. The op log stores the resolved mask so
            // replay never depends on defaults.
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                #[serde(default = "default_true")]
                enabled: bool,
                shape: Option<crate::types::MaskShape>,
                #[serde(default)]
                points: Vec<[f64; 2]>,
                #[serde(default)]
                feather: f64,
                #[serde(default)]
                invert: bool,
                /// blur (default) | pixelate | box (= solid black censor).
                mode: Option<String>,
                #[serde(default)]
                strength: Option<f64>,
                #[serde(default)]
                range_ms: Option<[u64; 2]>,
                /// MOTION TRACK: region centre over time → the redaction FOLLOWS
                /// the subject. rect/ellipse only. Feed from edit.track's `points`.
                #[serde(default)]
                track: Option<Vec<crate::types::MaskTrackPoint>>,
                /// ADDITIONAL regions: blur N faces / boxes at once,
                /// all sharing this `mode`/`strength`/`feather`/`range_ms`. Each box is
                /// {shape, points, track?}; a tracked box needs at least two samples.
                #[serde(default)]
                boxes: Vec<BoxArg>,
            }
            #[derive(serde::Deserialize)]
            struct BoxArg {
                shape: crate::types::MaskShape,
                #[serde(default)]
                points: Vec<[f64; 2]>,
                #[serde(default)]
                track: Option<Vec<crate::types::MaskTrackPoint>>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            let mask = if a.enabled {
                let shape = a.shape.ok_or_else(|| {
                    crate::error::CutError::new(
                        crate::error::codes::INVALID_ARGS,
                        "edit.redact needs a shape (rect|ellipse|polygon) unless enabled:false",
                        "pass shape + points (+ optional range_ms), or enabled:false to clear",
                    )
                })?;
                // A motion track follows the region; v1 procedural geq supports
                // rect/ellipse only (polygon-follow is a follow-up). Need ≥2 points.
                if let Some(tr) = &a.track {
                    if matches!(shape, crate::types::MaskShape::Polygon) {
                        return Err(crate::error::CutError::new(
                            crate::error::codes::INVALID_ARGS,
                            "tracked redaction supports rect/ellipse only (polygon-follow is a follow-up)",
                            "use a rect or ellipse shape with track, or drop track for a static polygon",
                        ));
                    }
                    if tr.len() < 2 {
                        return Err(crate::error::CutError::new(
                            crate::error::codes::INVALID_ARGS,
                            "a redaction track needs ≥2 points (the region centre over time)",
                            "feed edit.track's `points` (cx,cy,t_ms)",
                        ));
                    }
                }
                let effect = match a.mode.as_deref() {
                    None | Some("blur") => crate::types::MaskEffect::Blur,
                    Some("pixelate") => crate::types::MaskEffect::Pixelate,
                    Some("box") | Some("black") => crate::types::MaskEffect::Black,
                    Some(other) => {
                        return Err(crate::error::CutError::new(
                            crate::error::codes::INVALID_ARGS,
                            format!("unknown redact mode '{other}'"),
                            "mode: blur | pixelate | box",
                        ))
                    }
                };
                // FAIL-SAFE: an unset blur/pixelate strength over-redacts (heavy) so an
                // uncertain region is never left readable. blur → sigma 25 (vs the
                // creative-mask default 15); pixelate → 24px blocks (vs 16).
                let strength = a.strength.or(match effect {
                    crate::types::MaskEffect::Blur => Some(25.0),
                    crate::types::MaskEffect::Pixelate => Some(24.0),
                    crate::types::MaskEffect::Black => None,
                });
                // Static and tracked multi-region boxes share this
                // mask's effect/strength. A FULLY-static mask bakes the union into one
                // alpha PNG; if ANY region is tracked the renderer takes the procedural
                // geq path, which paints rect/ellipse only — so when any region is
                // tracked, EVERY region (primary + boxes) must be rect/ellipse, and each
                // tracked box needs ≥2 track points. (The primary track is validated
                // above.)
                let any_tracked = a.track.is_some() || a.boxes.iter().any(|b| b.track.is_some());
                for b in &a.boxes {
                    if let Some(tr) = &b.track {
                        if tr.len() < 2 {
                            return Err(crate::error::CutError::new(
                                crate::error::codes::INVALID_ARGS,
                                "a tracked redaction box needs ≥2 track points (the centre over time)",
                                "feed edit.track's `points` (cx,cy,t_ms) per box",
                            ));
                        }
                    }
                }
                if any_tracked
                    && (matches!(shape, crate::types::MaskShape::Polygon)
                        || a.boxes
                            .iter()
                            .any(|b| matches!(b.shape, crate::types::MaskShape::Polygon)))
                {
                    return Err(crate::error::CutError::new(
                        crate::error::codes::INVALID_ARGS,
                        "tracked multi-region redaction supports rect/ellipse only (polygon-follow is a follow-up)",
                        "use rect/ellipse for every region when any region is tracked",
                    ));
                }
                let regions: Vec<crate::types::MaskRegion> = a
                    .boxes
                    .into_iter()
                    .map(|b| crate::types::MaskRegion {
                        shape: b.shape,
                        points: b.points,
                        track: b.track,
                    })
                    .collect();
                Some(crate::types::ClipMask {
                    shape,
                    points: a.points,
                    feather: a.feather,
                    invert: a.invert,
                    effect,
                    strength,
                    range_ms: a.range_ms,
                    track: a.track,
                    regions,
                })
            } else {
                None
            };
            edit::add_mask(project, &a.clip, mask)
        }
        "edit.eq" => {
            // The dispatch layer resolves any `preset` into explicit high_pass/
            // low_pass/bands BEFORE commit, so the op log stores resolved values and
            // replay never depends on the preset table. Here we read those + enabled.
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                #[serde(default)]
                high_pass_hz: Option<f32>,
                #[serde(default)]
                low_pass_hz: Option<f32>,
                #[serde(default)]
                bands: Vec<crate::types::EqBand>,
                #[serde(default = "default_true")]
                enabled: bool,
            }
            let a: A = serde_json::from_value(args.clone())?;
            // enabled:false → identity (clears); else the given high/low-pass + bands.
            let eq = if a.enabled {
                crate::types::ClipEq {
                    high_pass_hz: a.high_pass_hz,
                    low_pass_hz: a.low_pass_hz,
                    bands: a.bands,
                }
            } else {
                crate::types::ClipEq {
                    high_pass_hz: None,
                    low_pass_hz: None,
                    bands: vec![],
                }
            };
            edit::eq(project, &a.clip, eq)
        }
        "edit.crossfade" => {
            #[derive(serde::Deserialize)]
            struct A {
                track: String,
                at_ms: u64,
                duration_ms: u64,
                #[serde(default)]
                transition: Option<String>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            // Validate the transition style against the exposed xfade set so an
            // unknown name fails fast (with the list) instead of producing a broken
            // filtergraph at render time.
            if let Some(t) = a.transition.as_deref() {
                if !crate::types::is_valid_transition(t) {
                    return Err(CutError::new(
                        codes::INVALID_ARGS,
                        format!("unknown transition '{t}'"),
                        format!(
                            "transition must be one of: {}",
                            crate::types::TRANSITIONS.join(", ")
                        ),
                    ));
                }
            }
            edit::crossfade(
                project,
                &a.track,
                a.at_ms,
                a.duration_ms,
                a.transition.as_deref(),
            )
        }
        "edit.move_marker" => {
            #[derive(serde::Deserialize)]
            struct A {
                id: String,
                at_ms: u64,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::marker_move(project, &a.id, a.at_ms)
        }
        "edit.update_marker" => {
            #[derive(serde::Deserialize)]
            struct A {
                id: String,
                label: Option<String>,
                color: Option<String>,
                note: Option<String>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::marker_update(
                project,
                &a.id,
                a.label.as_deref(),
                a.color.as_deref(),
                a.note.as_deref(),
            )
        }
        // captions.set_range is a single direct timeline mutation;
        // it replays through this table like an edit.* verb (apply_record
        // routes it here explicitly — see the dispatch precedence there).
        "captions.set_range" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                range_ms: [u64; 2],
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::caption_set_range(project, &a.clip, a.range_ms)
        }
        // captions.set_text edits an existing caption's words/style in place
        // (caption-editing regression fix) — same single-direct-mutation shape as set_range.
        "captions.set_text" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                text: String,
                #[serde(default)]
                style_ref: Option<String>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::caption_set_text(project, &a.clip, &a.text, a.style_ref.as_deref())
        }
        // edit.set_asset repoints a media clip at a different registered asset in
        // place — the core step behind title.update (title-editing regression). Never a
        // top-level verb: it rides only as a LOWERED step of title.update (like
        // edit.insert inside title.add), so it is reached via the lowered-replay
        // escape hatch in apply_record, which routes each step back through this
        // table. No id allocation, so no pinning needed.
        "edit.set_asset" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                asset: String,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::clip_set_asset(project, &a.clip, &a.asset)
        }
        "edit.duck" => {
            // Core sees RESOLVED windows only (the `windows` arg is written
            // by dispatch from perception facts before commit — ops must be
            // self-contained for replay). music_track/against_track/db ride
            // in args for the audit trail but core reads the windows.
            #[derive(serde::Deserialize)]
            struct A {
                music_track: String,
                windows: Vec<crate::types::GainWindow>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::duck(project, &a.music_track, a.windows)
        }
        "edit.fade" => {
            #[derive(serde::Deserialize)]
            struct A {
                clip: Option<String>,
                track: Option<String>,
                in_ms: Option<u64>,
                out_ms: Option<u64>,
                #[serde(default = "default_fade_kind")]
                kind: crate::types::FadeKind,
            }
            fn default_fade_kind() -> crate::types::FadeKind {
                crate::types::FadeKind::Both
            }
            let a: A = serde_json::from_value(args.clone())?;
            let target = match (a.clip, a.track) {
                (Some(c), None) => edit::FadeTarget::Clip(c),
                (None, Some(t)) => edit::FadeTarget::Track(t),
                _ => {
                    return Err(CutError::new(
                        codes::INVALID_ARGS,
                        "edit.fade needs exactly one of `clip` or `track`",
                        "both or neither were given",
                    ))
                }
            };
            edit::fade(project, target, a.in_ms, a.out_ms, a.kind)
        }
        "edit.mute_range" => {
            // Args-driven like edit.fade: one arm serves live apply AND replay
            // (edit::mute_range validates + normalizes identically both times).
            #[derive(serde::Deserialize)]
            struct A {
                clip: String,
                range_ms: Option<[u64; 2]>,
                remove_ms: Option<[u64; 2]>,
                #[serde(default)]
                clear: bool,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::mute_range(project, &a.clip, a.range_ms, a.remove_ms, a.clear)
        }
        "edit.add_marker" => {
            #[derive(serde::Deserialize)]
            struct A {
                at_ms: u64,
                label: String,
                note: Option<String>,
            }
            let a: A = serde_json::from_value(args.clone())?;
            // Pin the marker id on replay (rebase id-stability). POP the next
            // recorded marker id — a lowered audio.add_music runs many
            // add_marker steps (one per beat), each consuming the next id in
            // order, so a queue (not a single value) is required for correct
            // replay (regression: without this every beat marker pinned to m1).
            let pin = pinned.as_mut().and_then(|p| p.next_marker());
            edit::marker_add_pinned(
                project,
                a.at_ms,
                &a.label,
                a.note.as_deref(),
                pin.as_deref(),
            )
        }
        "edit.remove_marker" => {
            #[derive(serde::Deserialize)]
            struct A {
                id: String,
            }
            let a: A = serde_json::from_value(args.clone())?;
            edit::marker_remove(project, &a.id)
        }
        // Internal snapshot verb — only ever appears as an op INVERSE, but
        // accepting it here keeps the table total over everything edit.rs does.
        "edit._set_timeline" => {
            edit::apply_set_timeline(project, args)?;
            Ok(vec![])
        }
        other => Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{other}' is not a core edit verb"),
            "apply_edit_verb handles edit.* only; project/media verbs have their own paths",
        )),
    }
}

/// Replay ONE op record onto `project`. `prior` is the log prefix BEFORE this
/// op (needed by edit.restore to find the original record's inverse).
/// Dispatch precedence: known core verbs → recorded `lowered` effects (the
/// escape hatch for higher-layer verbs like transcript.cut_words) → error.
pub fn apply_record(
    project: &mut Project,
    op: &OpRecord,
    prior: &[OpRecord],
) -> Result<(), CutError> {
    match op.verb.as_str() {
        "project.create" => {
            let name = op
                .args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| replay_corrupt(op, "project.create args missing name"))?;
            validate_logged_project_name(name).map_err(|_| {
                replay_corrupt(
                    op,
                    format!("project.create replay name '{name}' is invalid"),
                )
            })?;
            let settings = op
                .args
                .get("settings")
                .cloned()
                .ok_or_else(|| replay_corrupt(op, "project.create args missing settings"))
                .and_then(|s| {
                    serde_json::from_value::<ProjectSettings>(s).map_err(|e| {
                        replay_corrupt(op, format!("project.create settings are invalid: {e}"))
                    })
                })?;
            *project = Project::new(name, settings);
            Ok(())
        }
        "project.sequence_create" => {
            let sequence = op
                .effects
                .iter()
                .find_map(|effect| effect.detail.get("sequence"))
                .cloned()
                .ok_or_else(|| replay_corrupt(op, "sequence_create effect missing sequence"))?;
            project.insert_and_activate_sequence(serde_json::from_value::<Sequence>(sequence)?);
            Ok(())
        }
        "project.sequence_switch" => {
            let id = op
                .args
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| replay_corrupt(op, "sequence_switch args missing id"))?;
            if !project.switch_sequence(id) {
                return Err(replay_corrupt(
                    op,
                    format!("sequence_switch target '{id}' does not exist"),
                ));
            }
            Ok(())
        }
        "project.sequence_rename" => {
            let id = op
                .args
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| replay_corrupt(op, "sequence_rename args missing id"))?;
            let name = op
                .args
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| replay_corrupt(op, "sequence_rename args missing name"))?;
            project.ensure_sequence_bank();
            let sequence = project
                .sequences
                .iter_mut()
                .find(|sequence| sequence.id == id)
                .ok_or_else(|| {
                    replay_corrupt(op, format!("sequence_rename target '{id}' does not exist"))
                })?;
            sequence.name = name.to_string();
            Ok(())
        }
        "project.sequence_delete" => {
            let id = op
                .args
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| replay_corrupt(op, "sequence_delete args missing id"))?;
            project.ensure_sequence_bank();
            let before = project.sequences.len();
            project.sequences.retain(|sequence| sequence.id != id);
            if project.sequences.len() == before {
                return Err(replay_corrupt(
                    op,
                    format!("sequence_delete target '{id}' does not exist"),
                ));
            }
            Ok(())
        }
        "project.checkpoint" => {
            let cp = op
                .effects
                .iter()
                .find_map(|e| e.detail.get("checkpoint"))
                .cloned()
                .ok_or_else(|| replay_corrupt(op, "checkpoint effect payload missing"))?;
            project
                .checkpoints
                .push(serde_json::from_value::<Checkpoint>(cp)?);
            Ok(())
        }
        "project.rename" => {
            // Replay the display-name change (args carry the new name).
            if let Some(name) = op.args.get("name").and_then(|v| v.as_str()) {
                if !name.trim().is_empty() {
                    project.name = name.trim().to_string();
                }
            }
            Ok(())
        }
        "project.format" => {
            // Metadata op (like rename): re-apply the recorded output settings so
            // a replay/rebuild reproduces the format change. set_format records the
            // FULL post-change snapshot {width, height, fps} in BOTH args and the
            // effect, so replay is a direct settings assignment. Timebased speed
            // ramps are then regridded to that stored output grid; legacy ramps
            // deliberately keep their old millisecond behavior. WITHOUT this arm
            // without this arm, recompute-by-replay undo, rebuild_from_log,
            // cold-reopen, and rebase cannot replay the format op. The snapshot
            // is authoritative — clamping already
            // happened at record time, so replay trusts the stored values.
            // `args` is a Value (.get → Option<&Value>); `detail` is a Map (also
            // .get → Option<&Value>) — read args first, fall back to the effect.
            let field = |key: &str| -> Option<&Value> {
                op.args
                    .get(key)
                    .or_else(|| op.effects.iter().find_map(|e| e.detail.get(key)))
            };
            if let Some(w) = field("width").and_then(|v| v.as_u64()) {
                let width = u32::try_from(w).map_err(|_| {
                    replay_corrupt(op, format!("project.format replay width {w} exceeds u32"))
                })?;
                if !(16..=7680).contains(&width) {
                    return Err(replay_corrupt(
                        op,
                        format!("project.format replay width {width} is outside 16..=7680"),
                    ));
                }
                project.settings.width = width;
            }
            if let Some(h) = field("height").and_then(|v| v.as_u64()) {
                let height = u32::try_from(h).map_err(|_| {
                    replay_corrupt(op, format!("project.format replay height {h} exceeds u32"))
                })?;
                if !(16..=4320).contains(&height) {
                    return Err(replay_corrupt(
                        op,
                        format!("project.format replay height {height} is outside 16..=4320"),
                    ));
                }
                project.settings.height = height;
            }
            if let Some(f) = field("fps").and_then(|v| v.as_f64()) {
                if !(f.is_finite() && f > 0.0 && f <= 240.0) {
                    return Err(replay_corrupt(
                        op,
                        format!("project.format replay fps {f} is outside (0, 240]"),
                    ));
                }
                project.settings.fps = f;
            }
            let (grid_fps, grid_audio_rate) = (project.settings.fps, project.settings.audio_rate);
            crate::speed_ramp_timing::regrid_timebased_speed_ramps(
                project,
                grid_fps,
                grid_audio_rate,
            );
            Ok(())
        }
        "project.color" => {
            // Metadata op (like project.format): re-apply the recorded working/output
            // color spaces so a replay/rebuild reproduces the color-management change.
            // set_color records the FULL post-change snapshot {working, output} in BOTH
            // args + the effect, so replay is a direct settings assignment — no inverse,
            // no timeline mutation. WITHOUT this arm a project that changed color would
            // be unreplayable (recompute-by-replay undo / cold-reopen rebuild / rebase
            // would hit the "not a core verb" escape) — mirrors the project.format fix.
            let field = |key: &str| -> Option<&Value> {
                op.args
                    .get(key)
                    .or_else(|| op.effects.iter().find_map(|e| e.detail.get(key)))
            };
            if let Some(w) = field("working").and_then(|v| v.as_str()) {
                project.settings.color.working =
                    crate::types::ColorSpace::parse(w).ok_or_else(|| {
                        replay_corrupt(
                            op,
                            format!("project.color replay working color space '{w}' is invalid"),
                        )
                    })?;
            }
            if let Some(o) = field("output").and_then(|v| v.as_str()) {
                project.settings.color.output =
                    crate::types::ColorSpace::parse(o).ok_or_else(|| {
                        replay_corrupt(
                            op,
                            format!("project.color replay output color space '{o}' is invalid"),
                        )
                    })?;
            }
            Ok(())
        }
        "project.brand" => {
            let value = op
                .args
                .get("brand")
                .or_else(|| op.effects.iter().find_map(|e| e.detail.get("brand")))
                .ok_or_else(|| replay_corrupt(op, "project.brand replay payload missing"))?;
            let brand = if value.is_null() {
                None
            } else {
                let parsed =
                    serde_json::from_value::<BrandKit>(value.clone()).map_err(|error| {
                        replay_corrupt(
                            op,
                            format!("project.brand replay payload is invalid: {error}"),
                        )
                    })?;
                Some(parsed.normalized().map_err(|cause| {
                    replay_corrupt(
                        op,
                        format!("project.brand replay validation failed: {cause}"),
                    )
                })?)
            };
            project.brand = brand;
            Ok(())
        }
        "grade.save" => {
            // Metadata op (like project.color): re-push the saved grade preset so a
            // replay/rebuild reproduces the gallery. save_grade_preset records the full
            // preset {name, grade} in the `preset` effect — name-keyed, so a re-save
            // replaces. WITHOUT this arm a project that saved a preset would be
            // unreplayable (cold-reopen rebuild / recompute-by-replay would hit the
            // "not a core verb" escape) — mirrors the project.color fix.
            let preset = op
                .effects
                .iter()
                .find_map(|e| e.detail.get("preset"))
                .cloned()
                .ok_or_else(|| replay_corrupt(op, "grade.save effect missing preset payload"))?;
            let preset: crate::types::GradePreset = serde_json::from_value(preset)?;
            if let Some(existing) = project
                .grade_presets
                .iter_mut()
                .find(|p| p.name == preset.name)
            {
                *existing = preset;
            } else {
                project.grade_presets.push(preset);
            }
            Ok(())
        }
        "media.bin_save" => {
            // Smart-bin metadata op (grade.save pattern): the full bin rides in
            // the effect; name-keyed re-save replaces. Idempotent on replay.
            let bin = op
                .effects
                .iter()
                .find_map(|e| e.detail.get("bin"))
                .cloned()
                .ok_or_else(|| replay_corrupt(op, "media.bin_save effect missing bin payload"))?;
            let bin: crate::types::SmartBin = serde_json::from_value(bin)?;
            if let Some(existing) = project.smart_bins.iter_mut().find(|b| b.name == bin.name) {
                *existing = bin;
            } else {
                project.smart_bins.push(bin);
            }
            Ok(())
        }
        "media.bin_delete" => {
            // Drop by name; a missing bin is a no-op so replays are idempotent.
            let name = op
                .effects
                .iter()
                .find_map(|e| e.detail.get("name").and_then(|v| v.as_str()))
                .ok_or_else(|| replay_corrupt(op, "media.bin_delete effect missing name"))?;
            project.smart_bins.retain(|b| b.name != name);
            Ok(())
        }
        "captions.save_style" => {
            // Caption style-preset metadata op (grade.save pattern): full preset
            // in the effect; name-keyed re-save replaces. Idempotent on replay.
            let preset = op
                .effects
                .iter()
                .find_map(|e| e.detail.get("preset"))
                .cloned()
                .ok_or_else(|| replay_corrupt(op, "captions.save_style effect missing preset"))?;
            let preset: crate::types::CaptionStylePreset = serde_json::from_value(preset)?;
            if let Some(existing) = project
                .caption_style_presets
                .iter_mut()
                .find(|p| p.name == preset.name)
            {
                *existing = preset;
            } else {
                project.caption_style_presets.push(preset);
            }
            Ok(())
        }
        "media.import" => {
            let (id, asset) = op
                .effects
                .iter()
                .find_map(|e| {
                    Some((
                        e.detail.get("asset_id")?.as_str()?.to_string(),
                        e.detail.get("asset")?.clone(),
                    ))
                })
                .ok_or_else(|| replay_corrupt(op, "import effect payload missing"))?;
            project
                .assets
                .insert(id, serde_json::from_value::<Asset>(asset)?);
            Ok(())
        }
        "import.otio" => {
            let detail = op
                .effects
                .iter()
                .map(|effect| &effect.detail)
                .find(|detail| detail.get("timeline").is_some())
                .ok_or_else(|| replay_corrupt(op, "OTIO import effect payload missing"))?;
            let assets = detail
                .get("assets")
                .and_then(Value::as_object)
                .ok_or_else(|| replay_corrupt(op, "OTIO import assets payload missing"))?;
            for (id, value) in assets {
                if project.assets.contains_key(id) {
                    return Err(replay_corrupt(
                        op,
                        format!("OTIO import asset id '{id}' already exists"),
                    ));
                }
                project
                    .assets
                    .insert(id.clone(), serde_json::from_value::<Asset>(value.clone())?);
            }
            let timeline = detail
                .get("timeline")
                .ok_or_else(|| replay_corrupt(op, "OTIO import timeline payload missing"))?;
            edit::apply_set_timeline(project, timeline)?;
            Ok(())
        }
        "motion.apply_import" => {
            let detail = motion_import_detail(op)?;
            let checkpoint: Checkpoint = serde_json::from_value(
                detail
                    .get("checkpoint")
                    .cloned()
                    .ok_or_else(|| replay_corrupt(op, "Motion checkpoint is missing"))?,
            )?;
            if detail.get("motion_editable_import").is_some() {
                // The preceding grouped native operations replay their own title,
                // shape, media, and keyframe edits. This op is their durable
                // Motion source identity binding and intentionally adds no pixels.
                // Its checkpoint was committed by the preceding project.checkpoint
                // op, unlike the rendered-media atomic path which owns its own.
                return Ok(());
            }
            project.checkpoints.push(checkpoint);
            let assets = detail
                .get("assets")
                .and_then(Value::as_object)
                .ok_or_else(|| replay_corrupt(op, "Motion import assets are missing"))?;
            for (id, value) in assets {
                if project.assets.contains_key(id) {
                    return Err(replay_corrupt(
                        op,
                        format!("Motion import asset id '{id}' already exists"),
                    ));
                }
                project
                    .assets
                    .insert(id.clone(), serde_json::from_value::<Asset>(value.clone())?);
            }
            let lowered: Vec<InverseOp> = serde_json::from_value(
                detail
                    .get("lowered")
                    .cloned()
                    .ok_or_else(|| replay_corrupt(op, "Motion insert steps are missing"))?,
            )?;
            let mut pinned = crate::rebase::PinnedIds::from_effects(&op.effects);
            for step in lowered {
                apply_edit_verb_pinned(project, &step.verb, &step.args, Some(&mut pinned))?;
            }
            Ok(())
        }
        "motion.link.refresh" => {
            let detail = op
                .effects
                .iter()
                .map(|effect| &effect.detail)
                .find(|detail| detail.get("motion_link_refresh").is_some())
                .ok_or_else(|| replay_corrupt(op, "Motion refresh effect is missing"))?;
            let assets = detail
                .get("assets")
                .and_then(Value::as_object)
                .ok_or_else(|| replay_corrupt(op, "Motion refresh assets are missing"))?;
            for (id, value) in assets {
                if project.assets.contains_key(id) {
                    return Err(replay_corrupt(
                        op,
                        format!("Motion refresh asset id '{id}' already exists"),
                    ));
                }
                project
                    .assets
                    .insert(id.clone(), serde_json::from_value::<Asset>(value.clone())?);
            }
            let lowered: Vec<InverseOp> = serde_json::from_value(
                detail
                    .get("lowered")
                    .cloned()
                    .ok_or_else(|| replay_corrupt(op, "Motion refresh lowering is missing"))?,
            )?;
            for step in lowered {
                apply_edit_verb(project, &step.verb, &step.args)?;
            }
            Ok(())
        }
        "motion.link.relink" => Ok(()),
        "media.remove" => {
            // Inverse of media.import on replay: drop the asset by id. Idempotent
            // (a missing id is a no-op) so a log that imports-then-removes rebuilds
            // to the correct end state on any build.
            let id = op
                .effects
                .iter()
                .find_map(|e| e.detail.get("asset_id").and_then(|v| v.as_str()))
                .ok_or_else(|| replay_corrupt(op, "media.remove effect missing asset_id"))?;
            project.assets.remove(id);
            Ok(())
        }
        "media.relink" => {
            // Apply the RECORDED relink outcome — path/hash from the effect, never
            // the filesystem, so replay is deterministic on any machine. Idempotent:
            // re-applying the same effect yields the same asset state.
            let detail = op
                .effects
                .iter()
                .map(|e| &e.detail)
                .find(|d| d.get("asset_id").is_some())
                .ok_or_else(|| replay_corrupt(op, "media.relink effect payload missing"))?;
            let id = detail
                .get("asset_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| replay_corrupt(op, "media.relink effect missing asset_id"))?;
            let path = detail
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| replay_corrupt(op, "media.relink effect missing path"))?;
            let hash = detail
                .get("hash")
                .and_then(|v| v.as_str())
                .ok_or_else(|| replay_corrupt(op, "media.relink effect missing hash"))?;
            let clear = detail
                .get("clear_derived")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if let Some(asset) = project.assets.get_mut(id) {
                asset.path = path.to_string();
                asset.hash = hash.to_string();
                if clear {
                    asset.probe = None;
                    asset.transcript = None;
                    asset.perception = None;
                    asset.proxy = None;
                    asset.filmstrip = None;
                }
            }
            Ok(())
        }
        // BACKWARD COMPAT: old builds accepted gain/mute/solo/pan on video or
        // caption targets even though the renderer has always consumed audio
        // from TrackKind::Audio only. Those recorded operations were no-ops in
        // delivered sound. Keep historic logs replayable while the live path now
        // rejects them; dropping a legacy video solo also prevents it from
        // silencing every real audio track under the corrected audibility rule.
        v @ ("edit.gain" | "edit.mute" | "edit.solo" | "edit.pan")
            if replay_targets_non_audio(project, v, &op.args) =>
        {
            Ok(())
        }
        // Review comments: the full Comment rides in the effects. `add`
        // inserts a new id; `resolve`/`draft` replace the same-id comment's
        // state. All idempotent on replay (find-by-id → replace, else push).
        "comment.add" | "comment.resolve" | "comment.draft" | "comment.import" => {
            let payload = op.effects.iter().find_map(|e| {
                e.detail
                    .get("comments")
                    .cloned()
                    .or_else(|| e.detail.get("comment").cloned().map(|cm| json!([cm])))
            });
            let comments: Vec<crate::types::Comment> = serde_json::from_value(
                payload.ok_or_else(|| replay_corrupt(op, "comment effect payload missing"))?,
            )?;
            for cm in comments {
                if let Some(existing) = project.comments.iter_mut().find(|c| c.id == cm.id) {
                    *existing = cm;
                } else {
                    project.comments.push(cm);
                }
            }
            Ok(())
        }
        "edit.restore" => {
            // Recompute-by-replay model: EVERY restore op (tip, rebase, and
            // revert steps) records its computed RESULT timeline in its effect —
            // `restored_timeline` (tip/revert) or `rebase_new_timeline` (rebase)
            // — and replay applies that directly. Deterministic, and no undo
            // recomputation runs on replay (only the recorded result is
            // reproduced). This keeps a log with restores/rebases replayable on
            // any build without re-deriving them.
            if let Some(snap) = op.effects.iter().find_map(|e| {
                e.detail
                    .get("restored_timeline")
                    .or_else(|| e.detail.get("rebase_new_timeline"))
            }) {
                edit::apply_set_timeline(project, snap)?;
                return Ok(());
            }
            // BACKWARD COMPAT — a pre-recompute (snapshot-era) tip restore has
            // no result timeline in its effect; it relied on the ORIGINAL op's
            // recorded snapshot inverse. Find that op and apply its inverse, as
            // the old model did, so historic logs still replay byte-identical.
            let target = op
                .args
                .get("op_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| replay_corrupt(op, "edit.restore args missing op_id"))?;
            let orig = prior.iter().find(|o| o.op_id == target).ok_or_else(|| {
                replay_corrupt(
                    op,
                    format!("restored op '{target}' not found in the log prefix"),
                )
            })?;
            edit::restore(project, orig)?;
            Ok(())
        }
        "project.revert" | "project.undo" | "project.redo" => {
            // History NAV ops — atomic revert plus the linear
            // undo/redo cursor — all ride their recomputed timeline in the
            // `restored_timeline` effect, computed when the op was applied;
            // replay applies it directly, exactly like edit.restore. Each is a
            // single append-only op (the log is never rewritten; the in-memory
            // cursor that drives undo/redo is NOT persisted and is rebuilt from
            // the log on open). Pre-Option-B logs that expanded revert into
            // edit.restore peels still replay via the edit.restore arm above —
            // unchanged.
            let snap = op
                .effects
                .iter()
                .find_map(|e| e.detail.get("restored_timeline"))
                .ok_or_else(|| {
                    replay_corrupt(op, "history nav op effect missing restored_timeline")
                })?;
            edit::apply_set_timeline(project, snap)?;
            if let Some(ids) = op
                .effects
                .iter()
                .find_map(|effect| effect.detail.get("removed_assets"))
            {
                for id in serde_json::from_value::<Vec<String>>(ids.clone())? {
                    project.assets.remove(&id);
                }
            }
            if let Some(assets) = op
                .effects
                .iter()
                .find_map(|effect| effect.detail.get("restored_assets"))
            {
                for (id, asset) in
                    serde_json::from_value::<BTreeMap<String, Asset>>(assets.clone())?
                {
                    project.assets.insert(id, asset);
                }
            }
            if let Some(ids) = op
                .effects
                .iter()
                .find_map(|effect| effect.detail.get("removed_checkpoints"))
            {
                let ids: Vec<String> = serde_json::from_value(ids.clone())?;
                project
                    .checkpoints
                    .retain(|checkpoint| !ids.contains(&checkpoint.id));
            }
            if let Some(checkpoints) = op
                .effects
                .iter()
                .find_map(|effect| effect.detail.get("restored_checkpoints"))
            {
                for checkpoint in serde_json::from_value::<Vec<Checkpoint>>(checkpoints.clone())? {
                    if !project
                        .checkpoints
                        .iter()
                        .any(|existing| existing.id == checkpoint.id)
                    {
                        project.checkpoints.push(checkpoint);
                    }
                }
            }
            Ok(())
        }
        // captions.set_range is a single direct timeline mutation
        // with no lowered steps — replay it through the edit-verb table (it is
        // registered there) before the lowered escape hatch below.
        //
        // EXCLUDE lowered-bearing ops: a verb committed via `apply_lowered`
        // records its authoritative replay as a `lowered` effects entry, even when
        // its LOGICAL name starts with `edit.` (e.g. `edit.add_shape`, which is a
        // DISPATCH verb that lowers to import + insert — NOT a core edit verb).
        // Without this guard such a verb misroutes here and replay/undo/rebase fail
        // with "'edit.add_shape' is not a core edit verb"; it must fall through to
        // the `lowered` escape hatch below instead. (Pre-existing gap — `title.add`
        // dodged it only because its name doesn't start with `edit.`; surfaced by
        // the shape.update replay-determinism test.) No genuine core
        // edit verb records a `lowered` entry, so this never diverts one.
        v if (v.starts_with("edit.") || v == "captions.set_range" || v == "captions.set_text")
            && !op.effects.iter().any(|e| e.detail.get("lowered").is_some()) =>
        {
            // Pin the ids this op recorded so replay is allocation-order
            // independent (rebase id-stability; in the no-skip case the pinned
            // id equals the positional one, so this is byte-identical).
            let mut pinned = crate::rebase::PinnedIds::from_effects(&op.effects);
            apply_edit_verb_pinned(project, v, &op.args, Some(&mut pinned))?;
            Ok(())
        }
        _ => {
            // Lowering escape hatch: higher-layer verbs record their core ops.
            // The lowered steps' allocations were flattened onto THIS op's
            // top-level effects (apply_lowered), so one PinnedIds drawn from the
            // op's effects pins every step. NOTE the limitation this implies:
            // pinning a multi-allocation lowered op is only id-stable when each
            // role appears once (a transcript.cut_words lowers to a single
            // ripple_delete — the common case). A lowered op that allocated
            // MULTIPLE clips of the same role is NOT yet rebase-safe; the
            // dependency gate + rebase_out's verify-replay catch that and refuse
            // (rebase.rs), so it can never silently corrupt.
            if let Some(lowered) = op.effects.iter().find_map(|e| e.detail.get("lowered")) {
                let steps: Vec<InverseOp> = serde_json::from_value(lowered.clone())?;
                // ONE shared PinnedIds across all steps: the queue roles
                // (markers, ripple splits) are consumed positionally as the
                // steps run, so a per-beat add_marker step pops m1, m2, … in
                // turn (the multi-allocation case the marker queue exists for).
                let mut pinned = crate::rebase::PinnedIds::from_effects(&op.effects);
                for s in steps {
                    apply_edit_verb_pinned(project, &s.verb, &s.args, Some(&mut pinned))?;
                }
                return Ok(());
            }
            Err(replay_corrupt(
                op,
                "verb is not a core verb and carries no `lowered` effects entry",
            ))
        }
    }
}

fn replay_targets_non_audio(project: &Project, verb: &str, args: &Value) -> bool {
    if verb == "edit.gain" {
        if let Some(track_id) = args.get("track").and_then(Value::as_str) {
            return project
                .track(track_id)
                .is_some_and(|track| track.kind != crate::types::TrackKind::Audio);
        }
        if let Some(clip_id) = args.get("clip").and_then(Value::as_str) {
            return project
                .find_clip(clip_id)
                .and_then(|(track_id, _)| project.track(track_id))
                .is_some_and(|track| track.kind != crate::types::TrackKind::Audio);
        }
        return false;
    }
    args.get("track")
        .and_then(Value::as_str)
        .and_then(|track_id| project.track(track_id))
        .is_some_and(|track| track.kind != crate::types::TrackKind::Audio)
}

/// Re-point each asset's DERIVED pointers from their deterministic on-disk files
/// after a log rebuild. Enrichment (probe/proxy/transcript/perception/filmstrip)
/// is a project.json cache write, not an op, so a pure replay leaves these fields
/// `None` — but the files persist under content-addressed names. This fills ONLY
/// missing fields (never overrides the log/cache), so it is a no-op on a normal
/// cache-present open and a recovery pass on a rebuild. Probe is inline JSON (no
/// pointer), persisted to `receipts/<id>.probe.json` at enrichment time precisely
/// so it is recoverable here too.
fn reconcile_derived_assets(project: &mut Project, dir: &Path) {
    let present = |rel: String| -> Option<String> {
        if dir.join(&rel).is_file() {
            Some(rel)
        } else {
            None
        }
    };
    for (id, asset) in project.assets.iter_mut() {
        if asset.proxy.is_none() {
            asset.proxy = present(format!("proxies/{id}.mp4"));
        }
        if asset.filmstrip.is_none() {
            asset.filmstrip = present(format!("filmstrip/{id}.jpg"));
        }
        if asset.transcript.is_none() {
            asset.transcript = present(format!("receipts/{id}.words.json"));
        }
        if asset.perception.is_none() {
            asset.perception = present(format!("receipts/{id}.perception.json"));
        }
        if asset.probe.is_none() {
            if let Ok(text) = std::fs::read_to_string(dir.join(format!("receipts/{id}.probe.json")))
            {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    asset.probe = Some(v);
                }
            }
        }
    }
}

/// Rebuild the materialized Project by replaying the full op log (timeline/op-log contract:
/// "project.json is rebuilt from the log on demand"). Pure + deterministic:
/// same log ⇒ identical Project (tested in tests/roundtrip.rs — the gate).
pub fn rebuild_from_log(ops: &[OpRecord]) -> Result<Project, CutError> {
    let mut project = Project::new("", ProjectSettings::default());
    for (i, op) in ops.iter().enumerate() {
        apply_record(&mut project, op, &ops[..i])?;
        project.sync_active_sequence();
    }
    Ok(project)
}

/// Rebuild the Project by replaying the log with op at `skip_idx` OMITTED — the
/// skip-replay primitive behind a rebase (rebase.rs). Every other op replays
/// with its RECORDED ids PINNED (apply_record), so the skipped op's absence
/// does NOT renumber later allocations: the timeline is reproduced exactly as
/// if the skipped op never happened, with every surviving id intact.
///
/// SAFETY: this is only ever called AFTER `rebase::can_rebase_out` has proven no
/// later op references an id the skipped op created — so the result has no
/// dangling references. It is also re-verified by [`ProjectStore::rebase_out`]
/// (a full clean re-replay must reproduce the same state) before anything is
/// committed. The skipped op MUST NOT have produced ids any later op consumes;
/// the dependency gate guarantees that.
///
/// The skipped op's `prior` slice passed to apply_record still uses the FULL
/// original prefix (so an `edit.restore` inside the surviving suffix can still
/// resolve the op it restored). Skipping is positional in the replay loop only.
pub fn rebuild_skipping(ops: &[OpRecord], skip_idx: usize) -> Result<Project, CutError> {
    let mut project = Project::new("", ProjectSettings::default());
    for (i, op) in ops.iter().enumerate() {
        if i == skip_idx {
            continue; // the rebased-out op: replay everything else as if absent
        }
        // `prior` is needed only for legacy edit.restore records that do not
        // carry their materialized restored_timeline/rebase_new_timeline effect.
        // Avoid cloning the whole prefix for every ordinary op during rebase.
        let needs_prior = op.verb == "edit.restore"
            && !op.effects.iter().any(|e| {
                e.detail.get("restored_timeline").is_some()
                    || e.detail.get("rebase_new_timeline").is_some()
            });
        if needs_prior {
            let prior: Vec<OpRecord> = ops[..i]
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != skip_idx)
                .map(|(_, o)| o.clone())
                .collect();
            apply_record(&mut project, op, &prior)?;
        } else {
            apply_record(&mut project, op, &[])?;
        }
        project.sync_active_sequence();
    }
    Ok(project)
}

/// Error constructor for replay-integrity violations (a log we can't replay
/// is corruption, not bad user input — surface it loudly and actionably).
fn replay_corrupt(op: &OpRecord, why: impl Into<String>) -> CutError {
    CutError::new(
        codes::CONFLICT,
        format!("cannot replay op '{}' ({})", op.op_id, op.verb),
        why,
    )
    .with_suggested_action("the op log is damaged or from a newer build; restore from a backup")
}

/// Content hash of a file — the Asset::hash format, used purely as a CACHE /
/// DEDUP KEY (proxy cache, transcribe/perception receipt cache, re-import
/// dedup), never for integrity. Lives here because import (server) and
/// perception caching both need the same string.
///
/// Two regimes by size (the old code `std::fs::read`-loaded the WHOLE file into
/// RAM then hashed — a ~30s stall AND a multi-GB RAM spike on raw 4K import,
/// the live-measured big-file block):
///   - ≤ 256 MB → exact full-file STREAMING sha256 ("sha256:<hex>"). Identical
///     identity to before for normal clips, render outputs, title .movs — so no
///     determinism / test change for the common case.
///   - > 256 MB → O(1) content-SAMPLED key over (size ++ 8 MB head ++ 8 MB
///     > tail), tagged "sha256s:<hex>". Enough to dedup real media (two distinct
///     > files colliding on size + first-and-last 8 MB is effectively impossible);
///     > import of a multi-GB file is now instant. The distinct prefix means a
///     > sampled hash never compares equal to a full hash.
pub fn hash_file(path: &Path) -> Result<String, CutError> {
    // 256 MB full-hash ceiling / 8 MB sample window — tuned so normal editing
    // files stay exact and only genuinely large raw footage is sampled.
    hash_file_impl(path, 256 * 1024 * 1024, 8 * 1024 * 1024)
}

/// Inner hashing with explicit thresholds, so a test can drive the sampled
/// branch on small fixtures (a real >256 MB fixture would be absurd in CI).
fn hash_file_impl(path: &Path, full_limit: u64, sample: u64) -> Result<String, CutError> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let size = f.metadata()?.len();
    let mut h = Sha256::new();
    if size <= full_limit {
        // Exact full hash, streamed in 1 MB chunks (no whole-file RAM load).
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        return Ok(format!("sha256:{:x}", h.finalize()));
    }
    // Large file: size + head + tail only — O(1) regardless of file length.
    h.update(size.to_le_bytes());
    let sample = sample.min(size);
    let mut head = vec![0u8; sample as usize];
    f.read_exact(&mut head)?;
    h.update(&head);
    f.seek(SeekFrom::Start(size.saturating_sub(sample)))?;
    let mut tail = vec![0u8; sample as usize];
    f.read_exact(&mut tail)?;
    h.update(&tail);
    Ok(format!("sha256s:{:x}", h.finalize()))
}

#[cfg(test)]
mod hash_tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(bytes).unwrap();
        p
    }

    /// Full branch (file ≤ limit): exact, deterministic, "sha256:" tag; distinct
    /// content → distinct hash. This is the unchanged identity for normal files.
    #[test]
    fn hash_full_branch_is_exact_and_deterministic() {
        let d = tempfile::tempdir().unwrap();
        let a = write_tmp(d.path(), "a.bin", b"hello shellx cut");
        let b = write_tmp(d.path(), "b.bin", b"hello shellx cut"); // identical content
        let c = write_tmp(d.path(), "c.bin", b"different content!"); // distinct
        let ha = hash_file(&a).unwrap();
        assert!(ha.starts_with("sha256:") && !ha.starts_with("sha256s:"));
        assert_eq!(ha, hash_file(&b).unwrap(), "same content → same hash");
        assert_ne!(
            ha,
            hash_file(&c).unwrap(),
            "distinct content → distinct hash"
        );
    }

    /// Sampled branch (file > limit): O(1) key, "sha256s:" tag, never equal to a
    /// full hash; differs on head/tail/size. Driven with a tiny limit so a small
    /// fixture exercises it (no absurd >256 MB CI file).
    #[test]
    fn hash_sampled_branch_keys_on_size_head_tail() {
        let d = tempfile::tempdir().unwrap();
        // 4 KB files; limit 1 KB, sample 256 B → sampled branch. Sampled regions
        // are bytes [0,256) (head) and [3840,4096) (tail); [256,3840) is the
        // unsampled middle. Build explicit head/middle/tail so each test varies
        // exactly one region.
        let mk = |name: &str, head_byte: u8, mid_byte: u8, tail_byte: u8| {
            let mut v = vec![mid_byte; 4096];
            for b in v[..256].iter_mut() {
                *b = head_byte;
            }
            for b in v[3840..].iter_mut() {
                *b = tail_byte;
            }
            write_tmp(d.path(), name, &v)
        };
        let base = mk("base.bin", 0xAA, 0x11, 0xBB);
        let same = mk("same.bin", 0xAA, 0x11, 0xBB); // identical
        let mid = mk("mid.bin", 0xAA, 0x22, 0xBB); // differs ONLY in the middle
        let head2 = mk("head2.bin", 0xCC, 0x11, 0xBB); // differs in head

        let hb = hash_file_impl(&base, 1024, 256).unwrap();
        assert!(hb.starts_with("sha256s:"), "big-file tag");
        assert_eq!(
            hb,
            hash_file_impl(&same, 1024, 256).unwrap(),
            "same sample → same key"
        );
        // Documented tradeoff: sampling can't see the middle — two files equal in
        // size+head+tail collide. Acceptable for a dedup/cache key on real media.
        assert_eq!(
            hb,
            hash_file_impl(&mid, 1024, 256).unwrap(),
            "middle-only diff collides (by design)"
        );
        assert_ne!(
            hb,
            hash_file_impl(&head2, 1024, 256).unwrap(),
            "head diff → distinct key"
        );
        // A sampled key never equals the full key of the same file.
        assert_ne!(hb, hash_file_impl(&base, u64::MAX, 256).unwrap());
    }

    #[test]
    fn replay_format_rejects_oversized_dimensions_instead_of_wrapping() {
        let mut project = Project::new("demo", ProjectSettings::default());
        let op = OpRecord {
            op_id: "op_000002".into(),
            ts: "2026-06-29T00:00:00.000Z".into(),
            actor: Actor::system(),
            verb: "project.format".into(),
            args: json!({
                "width": u64::from(u32::MAX) + 1,
                "height": 720,
                "fps": 30.0
            }),
            rationale: None,
            effects: vec![],
            inverse: None,
            status: OpStatus::Applied,
        };

        let err = apply_record(&mut project, &op, &[]).expect_err("oversized replay width");
        assert_eq!(err.code, codes::CONFLICT);
        assert_eq!(project.settings.width, ProjectSettings::default().width);
    }
}

#[cfg(test)]
mod audio_replay_tests {
    use super::*;
    use crate::edit::make_media_clip;
    use crate::types::Clip;

    fn legacy_op(verb: &str, args: Value) -> OpRecord {
        OpRecord {
            op_id: "op_legacy".into(),
            ts: "2026-06-28T00:00:00.000Z".into(),
            actor: Actor::system(),
            verb: verb.into(),
            args,
            rationale: None,
            effects: vec![],
            inverse: None,
            status: OpStatus::Applied,
        }
    }

    #[test]
    fn legacy_non_audio_mix_ops_replay_as_noops() {
        let mut project = Project::new("legacy", ProjectSettings::default());
        project
            .track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Media(make_media_clip("c1", "a1", 0, 1_000)));

        for op in [
            legacy_op("edit.gain", json!({"clip":"c1","db":-6.0})),
            legacy_op("edit.gain", json!({"track":"v1","db":-6.0})),
            legacy_op("edit.mute", json!({"track":"v1","on":true})),
            legacy_op("edit.solo", json!({"track":"v1","on":true})),
            legacy_op("edit.pan", json!({"track":"v1","pan":0.5})),
        ] {
            apply_record(&mut project, &op, &[]).expect("historic no-op stays replayable");
        }

        let video = project.track("v1").unwrap();
        let clip = match &video.clips[0] {
            Clip::Media(clip) => clip,
            _ => unreachable!(),
        };
        assert_eq!(clip.gain_db, 0.0);
        assert_eq!(video.gain_db, 0.0);
        assert!(!video.muted);
        assert!(!video.solo);
        assert_eq!(video.pan, 0.0);

        let audio_gain = legacy_op("edit.gain", json!({"track":"a1t","db":-3.0}));
        apply_record(&mut project, &audio_gain, &[]).expect("audio edit still replays");
        assert_eq!(project.track("a1t").unwrap().gain_db, -3.0);
    }
}

/// Linear undo/redo cursor — the core-engine gate.
///
/// Proves: undo/redo walk REAL multi-step history byte-identically; the
/// oscillation bug is gone (a 2nd undo lands on an OLDER state, never the redo);
/// a fresh edit after an undo clears the redo branch; the guardrails refuse at
/// the baseline / tip; and the in-memory cursor reconstructs correctly across a
/// reopen (replay reproduces the undone timeline and cursor).
#[cfg(test)]
mod undo_redo_tests {
    use super::*;
    use crate::ops::ActorKind;
    use crate::types::Asset;

    fn actor() -> Actor {
        Actor {
            kind: ActorKind::Agent,
            name: "claude".into(),
            via: "mcp".into(),
            request: None,
        }
    }

    /// A probed 10s asset (core never touches the bytes).
    fn asset() -> Asset {
        Asset {
            path: "/testdata/talking_head.mp4".into(),
            hash: "sha256:deadbeef".into(),
            probe: Some(json!({"duration_ms": 10_000})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        }
    }

    #[test]
    fn auto_asset_ids_are_monotonic_across_removed_assets() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        assert_eq!(
            s.record_import(None, asset(), actor(), None).unwrap().0,
            "a1"
        );
        assert_eq!(
            s.record_import(None, asset(), actor(), None).unwrap().0,
            "a2"
        );
        assert_eq!(
            s.record_import(None, asset(), actor(), None).unwrap().0,
            "a3"
        );
        s.record_remove_asset("a3", actor(), None).unwrap();

        let (next, _) = s.record_import(None, asset(), actor(), None).unwrap();
        assert_eq!(next, "a4", "removed historical ids must not be reused");
    }

    #[test]
    fn otio_import_is_one_replayable_undoable_timeline_op() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        s.apply(
            "edit.add_marker",
            json!({"at_ms":250,"label":"before"}),
            actor(),
            None,
        )
        .unwrap();
        let before_tracks = s.project.tracks.clone();
        let ids = s.next_asset_ids(1).unwrap();
        assert_eq!(ids, vec!["a1"]);
        let imported_track: Track = serde_json::from_value(json!({
            "id": "picture",
            "kind": "video",
            "clips": [
                {"id":"otio_c1","asset":"a1","src_in_ms":100,"src_out_ms":1100},
                {"kind":"gap","duration_ms":250}
            ]
        }))
        .unwrap();
        let imported_asset = Asset {
            path: "/media/imported.mov".into(),
            hash: "sha256:imported".into(),
            probe: Some(json!({"kind":"video","duration_ms":5000})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        };
        let op = s
            .replace_timeline_from_otio(
                vec![imported_track],
                BTreeMap::from([("a1".into(), imported_asset)]),
                "sha256:otio".into(),
                actor(),
                Some("import reviewed timeline".into()),
            )
            .unwrap();
        assert_eq!(op.verb, "import.otio");
        assert_eq!(s.project.tracks[0].id, "picture");
        assert!(
            s.project.markers.is_empty(),
            "foreign timeline clears old markers"
        );
        assert_eq!(
            s.log
                .read_all()
                .unwrap()
                .iter()
                .filter(|record| record.verb == "import.otio")
                .count(),
            1
        );

        let dir = s.dir.clone();
        drop(s);
        let mut reopened = ProjectStore::open(&dir).unwrap();
        assert_eq!(reopened.project.tracks[0].id, "picture");
        assert!(reopened.project.assets.contains_key("a1"));
        reopened.undo(actor()).unwrap();
        assert_eq!(reopened.project.tracks, before_tracks);
        assert_eq!(reopened.project.markers[0].label, "before");
    }

    #[test]
    fn invalid_otio_import_commits_nothing() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        let before_project = s.project.clone();
        let before_ops = s.log.read_all().unwrap().len();
        let bad_track: Track = serde_json::from_value(json!({
            "id":"v_import",
            "kind":"video",
            "clips":[{"id":"otio_c1","asset":"missing","src_in_ms":0,"src_out_ms":1000}]
        }))
        .unwrap();
        let error = s
            .replace_timeline_from_otio(
                vec![bad_track],
                BTreeMap::new(),
                "sha256:otio".into(),
                actor(),
                None,
            )
            .unwrap_err();
        assert_eq!(error.code, codes::INVALID_ARGS);
        assert_eq!(s.project, before_project);
        assert_eq!(s.log.read_all().unwrap().len(), before_ops);
    }

    #[test]
    fn atomic_motion_import_is_replay_safe_and_idempotent() {
        let d = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(d.path(), "demo", None).unwrap();
        let before_head = store.log.read_all().unwrap().last().unwrap().op_id.clone();
        let key = "a".repeat(64);
        let inserts = vec![
            json!({"track":"v1","at_ms":0,"src_range_ms":[0,1000]}),
            json!({"track":"v1","at_ms":1000,"src_range_ms":[0,1000]}),
        ];
        let result = store
            .apply_atomic_media_insert_plan_with_links(
                &key,
                vec![asset(), asset()],
                inserts.clone(),
                Some(vec![
                    json!({"schema":"shellx-cut/motion-link@1","packageId":"pkg","motionId":"motion","state":"linked-current"}),
                    json!({"schema":"shellx-cut/motion-link@1","packageId":"pkg","motionId":"motion","state":"linked-current"}),
                ]),
                actor(),
                Some("attested Motion plan".into()),
            )
            .unwrap();

        assert!(!result.already_applied);
        assert_eq!(result.asset_ids, vec!["a1", "a2"]);
        assert_eq!(result.clip_ids, vec!["c1", "c2"]);
        assert_eq!(result.checkpoint.at_op, before_head);
        assert_eq!(result.op.verb, "motion.apply_import");
        let links = result
            .op
            .effects
            .iter()
            .find_map(|effect| effect.detail.get("motion_links"))
            .and_then(Value::as_array)
            .expect("Motion links must be stored in the atomic op");
        assert_eq!(links[0]["clipId"], json!("c1"));
        assert_eq!(links[0]["assetId"], json!("a1"));
        assert_eq!(links[1]["clipId"], json!("c2"));
        assert_eq!(store.project.assets.len(), 2);
        assert_eq!(store.project.track("v1").unwrap().clips.len(), 2);
        let committed_ops = store.log.read_all().unwrap().len();

        let repeated = store
            .apply_atomic_media_insert_plan(&key, vec![asset(), asset()], inserts, actor(), None)
            .unwrap();
        assert!(repeated.already_applied);
        assert_eq!(repeated.op.op_id, result.op.op_id);
        assert_eq!(store.log.read_all().unwrap().len(), committed_ops);
        assert_eq!(store.project.assets.len(), 2);
        assert_eq!(store.project.track("v1").unwrap().clips.len(), 2);

        let dir = store.dir.clone();
        drop(store);
        let reopened = ProjectStore::open(&dir).unwrap();
        assert_eq!(reopened.project.assets.len(), 2);
        let reopened_clips = &reopened.project.track("v1").unwrap().clips;
        assert_eq!(reopened_clips.len(), 2);
        assert_eq!(reopened_clips[0].id(), Some("c1"));
        assert_eq!(reopened_clips[1].id(), Some("c2"));
        assert_eq!(reopened.project.checkpoints.len(), 1);
    }

    #[test]
    fn motion_link_refresh_is_atomic_replayable_and_undoable() {
        let d = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(d.path(), "demo", None).unwrap();
        let imported = store
            .apply_atomic_media_insert_plan_with_links(
                &"b".repeat(64),
                vec![asset()],
                vec![json!({"track":"v1","at_ms":0,"src_range_ms":[0,1000]})],
                Some(vec![json!({
                    "schema":"shellx-cut/motion-link@1",
                    "packageId":"pkg",
                    "motionId":"motion",
                    "sourceRevision":"old",
                    "state":"linked-current",
                    "render":{"path":"/old.mp4","sha256":"old","byteLength":1,"artifactHandleId":null}
                })]),
                actor(),
                None,
            )
            .unwrap();
        let link = imported
            .op
            .effects
            .iter()
            .find_map(|effect| effect.detail.get("motion_links"))
            .and_then(Value::as_array)
            .unwrap()[0]
            .clone();
        let mut refreshed_link = link;
        refreshed_link["sourceRevision"] = json!("new");
        refreshed_link["render"] = json!({
            "path":"/new.mp4","sha256":"new","byteLength":2,"artifactHandleId":null
        });
        let refreshed = store
            .apply_motion_link_refresh(
                "c1",
                Asset {
                    path: "/new.mp4".into(),
                    hash: "sha256:new".into(),
                    probe: Some(json!({"duration_ms": 10_000})),
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                refreshed_link,
                actor(),
                None,
            )
            .unwrap();
        assert_eq!(refreshed.asset_id, "a2");
        assert_eq!(refreshed.op.verb, "motion.link.refresh");
        let clip = &store.project.track("v1").unwrap().clips[0];
        assert_eq!(clip.id(), Some("c1"));
        assert!(matches!(clip, Clip::Media(media) if media.asset == "a2"));

        let dir = store.dir.clone();
        drop(store);
        let mut reopened = ProjectStore::open(&dir).unwrap();
        assert!(
            matches!(&reopened.project.track("v1").unwrap().clips[0], Clip::Media(media) if media.asset == "a2")
        );
        reopened.undo(actor()).unwrap();
        assert!(
            matches!(&reopened.project.track("v1").unwrap().clips[0], Clip::Media(media) if media.asset == "a1")
        );
        assert!(!reopened.project.assets.contains_key("a2"));
        reopened.redo(actor()).unwrap();
        assert!(
            matches!(&reopened.project.track("v1").unwrap().clips[0], Clip::Media(media) if media.asset == "a2")
        );
        assert!(reopened.project.assets.contains_key("a2"));
    }

    #[test]
    fn atomic_motion_import_undo_redo_and_checkpoint_revert_include_assets() {
        let d = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(d.path(), "demo", None).unwrap();
        let key = "c".repeat(64);
        let inserts = vec![json!({
            "track":"v1",
            "at_ms":0,
            "src_range_ms":[0,1000]
        })];
        let applied = store
            .apply_atomic_media_insert_plan(&key, vec![asset()], inserts.clone(), actor(), None)
            .unwrap();

        store.undo(actor()).unwrap();
        assert!(store.project.assets.is_empty());
        assert!(store.project.track("v1").unwrap().clips.is_empty());
        assert!(store.project.checkpoints.is_empty());
        let repeated = store
            .apply_atomic_media_insert_plan(&key, vec![asset()], inserts, actor(), None)
            .unwrap_err();
        assert_eq!(repeated.code, codes::CONFLICT);

        store.redo(actor()).unwrap();
        assert_eq!(store.project.assets.len(), 1);
        assert_eq!(store.project.track("v1").unwrap().clips.len(), 1);
        assert_eq!(store.project.checkpoints.len(), 1);

        store.revert(&applied.checkpoint.id, actor()).unwrap();
        assert!(store.project.assets.is_empty());
        assert!(store.project.track("v1").unwrap().clips.is_empty());

        let reopened = ProjectStore::open(&store.dir).unwrap();
        assert!(reopened.project.assets.is_empty());
        assert!(reopened.project.track("v1").unwrap().clips.is_empty());
    }

    #[test]
    fn atomic_motion_import_undo_refuses_shared_asset_breakage() {
        let d = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(d.path(), "demo", None).unwrap();
        store
            .apply_atomic_media_insert_plan(
                &"d".repeat(64),
                vec![asset()],
                vec![json!({"track":"v1", "at_ms":0, "src_range_ms":[0,1000]})],
                actor(),
                None,
            )
            .unwrap();
        store
            .sequence_create("Uses Motion asset", true, actor(), None)
            .unwrap();
        store.sequence_switch("seq1", actor(), None).unwrap();
        let before_cursor = store.undo_pos;

        let error = store.undo(actor()).unwrap_err();
        assert_eq!(error.code, codes::GUARDRAIL);
        assert!(error.message.contains("cannot remove Motion asset"));
        assert_eq!(store.undo_pos, before_cursor);
        assert_eq!(store.project.assets.len(), 1);
        assert_eq!(
            store
                .project
                .all_sequence_tracks()
                .flat_map(|track| track.clips.iter())
                .filter(|clip| clip.id().is_some())
                .count(),
            2
        );
    }

    #[test]
    fn atomic_motion_import_failures_at_each_insert_commit_nothing() {
        for invalid_index in 0..2 {
            let d = tempfile::tempdir().unwrap();
            let mut store = ProjectStore::create(d.path(), "demo", None).unwrap();
            let before_project = store.project.clone();
            let before_ops = store.log.read_all().unwrap();
            let before_cache = std::fs::read(store.dir.join("project.json")).unwrap();
            let mut inserts = vec![
                json!({"track":"v1","at_ms":0,"src_range_ms":[0,1000]}),
                json!({"track":"v1","at_ms":1000,"src_range_ms":[0,1000]}),
            ];
            inserts[invalid_index]["track"] = json!("missing-track");

            let error = store
                .apply_atomic_media_insert_plan(
                    &"b".repeat(64),
                    vec![asset(), asset()],
                    inserts,
                    actor(),
                    None,
                )
                .unwrap_err();
            assert_eq!(error.code, codes::NOT_FOUND);
            assert_eq!(store.project, before_project);
            assert_eq!(store.log.read_all().unwrap(), before_ops);
            assert_eq!(
                std::fs::read(store.dir.join("project.json")).unwrap(),
                before_cache
            );
        }
    }

    #[test]
    fn review_feedback_batch_is_atomic_and_replay_safe() {
        let d = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(d.path(), "demo", None).unwrap();
        let source = crate::types::CommentReviewSource {
            source_op_id: "op_000001".into(),
            render_id: "render_001".into(),
            render_hash: "sha256:reviewed".into(),
        };
        let notes = vec![
            crate::types::ReviewFeedbackNote {
                at_ms: 250,
                end_ms: None,
                text: " Tighten this opening. ".into(),
                author: " Client ".into(),
            },
            crate::types::ReviewFeedbackNote {
                at_ms: 1000,
                end_ms: Some(1500),
                text: "Hold this shot longer.".into(),
                author: "Client".into(),
            },
        ];
        let (comments, op) = store
            .import_review_comments(notes, source.clone(), actor(), None)
            .unwrap();
        assert_eq!(comments.len(), 2);
        assert_eq!(op.verb, "comment.import");
        assert!(!op.mutates_timeline().unwrap());
        assert_eq!(comments[0].text, "Tighten this opening.");
        assert_eq!(comments[0].review_source.as_ref(), Some(&source));

        let reopened = ProjectStore::open(&store.dir).unwrap();
        assert_eq!(reopened.project.comments, store.project.comments);
        assert_eq!(reopened.project.comments.len(), 2);
    }

    #[test]
    fn invalid_review_feedback_commits_nothing() {
        let d = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(d.path(), "demo", None).unwrap();
        let before_ops = store.log.read_all().unwrap().len();
        let err = store
            .import_review_comments(
                vec![crate::types::ReviewFeedbackNote {
                    at_ms: 1000,
                    end_ms: Some(900),
                    text: "invalid range".into(),
                    author: "Client".into(),
                }],
                crate::types::CommentReviewSource {
                    source_op_id: "op_000001".into(),
                    render_id: "render_001".into(),
                    render_hash: "sha256:reviewed".into(),
                },
                actor(),
                None,
            )
            .unwrap_err();
        assert_eq!(err.code, codes::INVALID_ARGS);
        assert!(store.project.comments.is_empty());
        assert_eq!(store.log.read_all().unwrap().len(), before_ops);
    }

    #[test]
    fn relink_repoints_asset_and_survives_replay() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        let (aid, _) = s.record_import(None, asset(), actor(), None).unwrap();
        // Simulate a fully-enriched asset (probe + derived pointers present).
        {
            let a = s.project.assets.get_mut(&aid).unwrap();
            a.probe = Some(json!({"kind": "video", "duration_ms": 5000}));
            a.proxy = Some(format!("proxies/{aid}.mp4"));
            a.transcript = Some(format!("receipts/{aid}.words.json"));
        }

        // Same-hash relink (file moved): path changes, derived pointers KEPT.
        let (old, _) = s
            .record_relink_asset(
                &aid,
                "/new/home/clip.mp4",
                "sha256:same",
                false,
                actor(),
                None,
            )
            .unwrap();
        assert_eq!(old, asset().path);
        let a = &s.project.assets[&aid];
        assert_eq!(a.path, "/new/home/clip.mp4");
        assert!(a.probe.is_some(), "repath-only relink keeps probe");
        assert!(a.proxy.is_some(), "repath-only relink keeps proxy");

        // Content-change relink: derived pointers CLEARED.
        s.record_relink_asset(
            &aid,
            "/new/other.mp4",
            "sha256:changed",
            true,
            actor(),
            None,
        )
        .unwrap();
        let a = &s.project.assets[&aid];
        assert_eq!(a.hash, "sha256:changed");
        assert!(a.probe.is_none() && a.proxy.is_none() && a.transcript.is_none());

        // FALSIFIER: a cold rebuild from the log alone must reproduce the final
        // state (replay arm applies recorded values, no filesystem access).
        let rebuilt = ProjectStore::open(&s.dir).unwrap();
        let ra = &rebuilt.project.assets[&aid];
        assert_eq!(ra.path, "/new/other.mp4");
        assert_eq!(ra.hash, "sha256:changed");
        assert!(ra.proxy.is_none(), "replay must reproduce cleared derived");
        // Relink is a metadata op: it must NOT enter the timeline undo stack.
        let (_, rec) = rebuilt
            .log
            .read_all()
            .unwrap()
            .iter()
            .filter(|o| o.verb == "media.relink")
            .map(|o| (o.op_id.clone(), o.clone()))
            .next_back()
            .unwrap();
        assert!(!rec.mutates_timeline().unwrap());
    }

    #[test]
    fn smart_bins_persist_replay_and_are_offcursor() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        let bin = crate::types::SmartBin {
            name: "broll".into(),
            kind: Some("video".into()),
            text: None,
            unused: Some(true),
            min_width: None,
            min_height: None,
            offline: None,
            modified_after_ms: None,
            modified_before_ms: None,
        };
        let (replaced, op) = s.save_smart_bin(bin.clone(), actor(), None).unwrap();
        assert!(!replaced);
        assert!(!op.mutates_timeline().unwrap(), "bin_save is a metadata op");
        // re-save under the same name REPLACES (name-keyed)
        let bin2 = crate::types::SmartBin {
            name: "broll".into(),
            kind: Some("video".into()),
            text: Some("drone".into()),
            unused: None,
            min_width: None,
            min_height: None,
            offline: None,
            modified_after_ms: None,
            modified_before_ms: None,
        };
        let (replaced2, _) = s.save_smart_bin(bin2.clone(), actor(), None).unwrap();
        assert!(replaced2);
        assert_eq!(s.project.smart_bins, vec![bin2.clone()]);
        // second bin + delete the first
        let other = crate::types::SmartBin {
            name: "audio only".into(),
            kind: Some("audio".into()),
            text: None,
            unused: None,
            min_width: None,
            min_height: None,
            offline: None,
            modified_after_ms: None,
            modified_before_ms: None,
        };
        s.save_smart_bin(other.clone(), actor(), None).unwrap();
        s.delete_smart_bin("broll", actor(), None).unwrap();
        assert_eq!(
            s.delete_smart_bin("broll", actor(), None).unwrap_err().code,
            codes::NOT_FOUND
        );
        // FALSIFIER: cold rebuild from the log reproduces the final bin list.
        let rebuilt = ProjectStore::open(&s.dir).unwrap();
        assert_eq!(rebuilt.project.smart_bins, vec![other]);
    }

    #[test]
    fn caption_style_presets_persist_and_replay() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        let style = crate::types::CaptionStyle {
            font: "Inter".into(),
            size: 80,
            color: "#ff0000".into(),
            bg: None,
            pos: Some("bottom".into()),
            extra: Default::default(),
        };
        let preset = crate::types::CaptionStylePreset {
            name: "my look".into(),
            style: style.clone(),
        };
        let (replaced, op) = s
            .save_caption_style_preset(preset.clone(), actor(), None)
            .unwrap();
        assert!(!replaced);
        assert!(
            !op.mutates_timeline().unwrap(),
            "save_style is a metadata op"
        );
        // name-keyed re-save replaces
        let mut p2 = preset.clone();
        p2.style.size = 40;
        let (replaced2, _) = s
            .save_caption_style_preset(p2.clone(), actor(), None)
            .unwrap();
        assert!(replaced2);
        assert_eq!(s.project.caption_style_presets, vec![p2.clone()]);
        // FALSIFIER: cold rebuild from the log reproduces the gallery.
        let rebuilt = ProjectStore::open(&s.dir).unwrap();
        assert_eq!(rebuilt.project.caption_style_presets, vec![p2]);
    }

    #[test]
    fn append_failure_does_not_mutate_live_project_or_cache() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        let log_path = s.log.path.clone();
        std::fs::remove_file(&log_path).unwrap();
        std::fs::create_dir(&log_path).unwrap();

        let err = s.rename("renamed", actor(), None).unwrap_err();
        assert_eq!(
            err.code,
            codes::CONFLICT,
            "replacing the open journal is an external-change conflict"
        );
        assert_eq!(
            s.project.name, "demo",
            "append failure must not mutate live project state"
        );

        let cached: Project =
            serde_json::from_str(&std::fs::read_to_string(s.dir.join("project.json")).unwrap())
                .unwrap();
        assert_eq!(
            cached.name, "demo",
            "append failure must not publish a cache ahead of ops.jsonl"
        );
    }

    #[test]
    fn open_rebuilds_when_project_cache_lags_log() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        let stale_cache = s.project.clone();
        s.apply(
            "edit.add_marker",
            json!({"at_ms": 1000, "label": "logged-only"}),
            actor(),
            None,
        )
        .unwrap();
        std::fs::write(
            s.dir.join("project.json"),
            serde_json::to_string_pretty(&stale_cache).unwrap(),
        )
        .unwrap();

        let reopened = ProjectStore::open(&s.dir).unwrap();
        assert_eq!(
            reopened.project.markers.len(),
            1,
            "open must trust ops.jsonl over a syntactically-valid stale cache"
        );
        assert_eq!(reopened.project.markers[0].label, "logged-only");
    }

    #[test]
    fn open_preserves_current_cache_only_asset_enrichment() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        let (aid, _) = s.record_import(None, asset(), actor(), None).unwrap();
        s.project.assets.get_mut(&aid).unwrap().proxy = Some("proxies/custom.mp4".into());
        s.project.assets.get_mut(&aid).unwrap().transcript =
            Some("receipts/custom.words.json".into());
        s.save().unwrap();

        let reopened = ProjectStore::open(&s.dir).unwrap();
        let reopened_asset = reopened.project.assets.get(&aid).unwrap();
        assert_eq!(reopened_asset.proxy.as_deref(), Some("proxies/custom.mp4"));
        assert_eq!(
            reopened_asset.transcript.as_deref(),
            Some("receipts/custom.words.json")
        );
    }

    /// project.color (set_color) stores the working/output config AND replays through
    /// the op log: a cold rebuild_from_log reproduces the color config exactly (the
    /// "project.color" apply_record arm), so a reopened project keeps its color
    /// management. Setting it back to the default leaves a default config.
    #[test]
    fn set_color_persists_and_replays() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        // Default is rec709/rec709.
        assert!(s.project.settings.color.is_default());
        // Set working rec709, output rec2020.
        s.set_color(
            Some(crate::types::ColorSpace::Rec709),
            Some(crate::types::ColorSpace::Rec2020),
            actor(),
            None,
        )
        .unwrap();
        assert_eq!(
            s.project.settings.color.output,
            crate::types::ColorSpace::Rec2020
        );
        assert!(!s.project.settings.color.is_default());
        // Replay the op log from scratch → the color config is reproduced.
        let ops = s.log.read_all().unwrap();
        let rebuilt = rebuild_from_log(&ops).unwrap();
        assert_eq!(
            rebuilt.settings.color.output,
            crate::types::ColorSpace::Rec2020
        );
        assert_eq!(
            rebuilt.settings.color.working,
            crate::types::ColorSpace::Rec709
        );
        // project.color is a settings-only op (off the undo cursor, like project.format).
        let last = ops.last().unwrap();
        assert_eq!(last.verb, "project.color");
        assert!(!ProjectStore::is_history_edit(last).unwrap());
    }

    #[test]
    fn set_brand_persists_replays_and_clears_off_cursor() {
        let d = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(d.path(), "demo", None).unwrap();
        let brand = crate::types::BrandKit {
            fonts: Some(vec![" Inter ".into(), "inter".into()]),
            colors: Some(vec!["#FFF".into()]),
            position: Some("bottom".into()),
            min_size: Some(24),
            max_size: Some(72),
            aspect: Some("1920:1080".into()),
        };
        let op = store
            .set_brand(Some(brand), actor(), Some("client delivery".into()))
            .unwrap();
        assert!(!ProjectStore::is_history_edit(&op).unwrap());
        assert_eq!(
            store.project.brand.as_ref().unwrap().aspect.as_deref(),
            Some("16:9")
        );
        assert_eq!(
            store
                .project
                .brand
                .as_ref()
                .unwrap()
                .fonts
                .as_ref()
                .unwrap(),
            &["Inter"]
        );

        let rebuilt = rebuild_from_log(&store.log.read_all().unwrap()).unwrap();
        assert_eq!(rebuilt.brand, store.project.brand);
        let reopened = ProjectStore::open(&store.dir).unwrap();
        assert_eq!(reopened.project.brand, store.project.brand);

        let clear = store.set_brand(None, actor(), None).unwrap();
        assert!(!ProjectStore::is_history_edit(&clear).unwrap());
        assert!(store.project.brand.is_none());
        assert!(ProjectStore::open(&store.dir)
            .unwrap()
            .project
            .brand
            .is_none());
    }

    /// grade.save persists a named grade preset AND replays through the op log: a cold
    /// rebuild_from_log reproduces the gallery exactly (the "grade.save" apply_record
    /// arm). A re-save under the same name REPLACES the preset (name-keyed). grade.save
    /// is a non-timeline metadata op (off the undo cursor, like comment.add).
    #[test]
    fn save_grade_preset_persists_replays_and_is_offcursor() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        assert!(s.project.grade_presets.is_empty());
        let g1 = crate::types::ClipGrade {
            contrast: 1.2,
            brightness: 0.0,
            saturation: 0.5,
            gamma: 1.0,
            temperature_k: Some(5200),
            lut: None,
        };
        let (preset, op) = s
            .save_grade_preset("look1", g1.clone(), actor(), None)
            .unwrap();
        assert_eq!(preset.name, "look1");
        assert_eq!(s.project.grade_presets.len(), 1);
        assert_eq!(s.project.grade_presets[0].grade, g1);
        // Off the undo cursor (project metadata, not a timeline edit).
        assert!(!ProjectStore::is_history_edit(&op).unwrap());

        // Re-save under the SAME name REPLACES (name-keyed gallery, no duplicate).
        let g2 = crate::types::ClipGrade {
            saturation: 0.0,
            ..g1.clone()
        };
        s.save_grade_preset("look1", g2.clone(), actor(), None)
            .unwrap();
        assert_eq!(s.project.grade_presets.len(), 1);
        assert_eq!(s.project.grade_presets[0].grade, g2);

        // A second distinct name appends.
        s.save_grade_preset("look2", g1.clone(), actor(), None)
            .unwrap();
        assert_eq!(s.project.grade_presets.len(), 2);

        // Cold replay from the log reproduces the gallery (both presets, look1 replaced).
        let ops = s.log.read_all().unwrap();
        let rebuilt = rebuild_from_log(&ops).unwrap();
        assert_eq!(rebuilt.grade_presets.len(), 2);
        let look1 = rebuilt
            .grade_presets
            .iter()
            .find(|p| p.name == "look1")
            .unwrap();
        assert_eq!(look1.grade, g2);
        let look2 = rebuilt
            .grade_presets
            .iter()
            .find(|p| p.name == "look2")
            .unwrap();
        assert_eq!(look2.grade, g1);
    }

    /// Serialize the full timeline state the undo cursor restores for byte-identical
    /// comparison.
    fn tl(s: &ProjectStore) -> String {
        serde_json::to_string(&json!({
            "tracks": s.project.tracks,
            "markers": s.project.markers,
            "caption_styles": s.project.caption_styles,
            "adjustments": s.project.adjustments,
            "nests": s.project.nests,
        }))
        .unwrap()
    }

    #[test]
    fn sequences_roundtrip_with_scoped_history_and_checkpoints() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        assert!(
            serde_json::to_value(&s.project)
                .unwrap()
                .get("sequences")
                .is_none(),
            "legacy single-sequence projects keep their serialized shape"
        );

        s.apply(
            "edit.add_marker",
            json!({"at_ms": 100, "label": "main"}),
            actor(),
            None,
        )
        .unwrap();
        let (main_cp, _) = s.checkpoint("main checkpoint", actor(), None).unwrap();
        assert_eq!(main_cp.sequence_id.as_deref(), Some("seq1"));

        let (review, _) = s.sequence_create("Review", false, actor(), None).unwrap();
        assert_eq!(review.id, "seq2");
        assert_eq!(s.project.active_sequence, "seq2");
        assert!(s.project.markers.is_empty(), "empty sequence starts clean");
        s.apply(
            "edit.add_marker",
            json!({"at_ms": 200, "label": "review"}),
            actor(),
            None,
        )
        .unwrap();
        let (review_cp, _) = s.checkpoint("review checkpoint", actor(), None).unwrap();

        s.undo(actor()).unwrap();
        assert!(
            s.project.markers.is_empty(),
            "review undo reaches review baseline"
        );
        s.redo(actor()).unwrap();
        assert_eq!(s.project.markers[0].label, "review");

        s.sequence_switch("seq1", actor(), None).unwrap();
        assert_eq!(s.project.markers[0].label, "main");
        s.undo(actor()).unwrap();
        assert!(
            s.project.markers.is_empty(),
            "main undo never consumes review edits"
        );
        s.redo(actor()).unwrap();
        assert_eq!(s.project.markers[0].label, "main");

        let err = s.revert(&review_cp.id, actor()).unwrap_err();
        assert_eq!(err.code, codes::GUARDRAIL);
        assert!(err.message.contains("seq2"));

        let ops = s.log.read_all().unwrap();
        assert_eq!(rebuild_from_log(&ops).unwrap(), s.project);
        let mut reopened = ProjectStore::open(&s.dir).unwrap();
        assert_eq!(reopened.project.active_sequence, "seq1");
        assert_eq!(reopened.project.markers[0].label, "main");
        reopened.sequence_switch("seq2", actor(), None).unwrap();
        assert_eq!(reopened.project.markers[0].label, "review");
        assert_eq!(
            reopened
                .project
                .checkpoints
                .iter()
                .find(|checkpoint| checkpoint.id == main_cp.id)
                .and_then(|checkpoint| checkpoint.sequence_id.as_deref()),
            Some("seq1")
        );
    }

    #[test]
    fn sequence_management_replays_and_protects_the_active_sequence() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        let (alternate, _) = s.sequence_create("Alternate", true, actor(), None).unwrap();
        let active_delete = s.sequence_delete(&alternate.id, actor(), None).unwrap_err();
        assert_eq!(active_delete.code, codes::GUARDRAIL);

        s.sequence_switch("seq1", actor(), None).unwrap();
        s.sequence_rename(&alternate.id, "Social", actor(), None)
            .unwrap();
        assert_eq!(
            s.project
                .sequences
                .iter()
                .find(|sequence| sequence.id == alternate.id)
                .unwrap()
                .name,
            "Social"
        );
        s.sequence_delete(&alternate.id, actor(), None).unwrap();
        assert_eq!(s.project.sequences.len(), 1);

        let (replacement, _) = s
            .sequence_create("Replacement", false, actor(), None)
            .unwrap();
        assert_eq!(
            replacement.id, "seq3",
            "deleted sequence ids must never be reused in append-only history"
        );

        let rebuilt = rebuild_from_log(&s.log.read_all().unwrap()).unwrap();
        assert_eq!(rebuilt, s.project);
    }

    #[test]
    fn shared_asset_usage_includes_inactive_sequences() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        let (asset_id, _) = s.record_import(None, asset(), actor(), None).unwrap();
        s.apply(
            "edit.insert",
            json!({
                "asset": asset_id,
                "track": "v1",
                "at_ms": 0,
                "src_range_ms": [0, 1000],
                "ripple": true,
            }),
            actor(),
            None,
        )
        .unwrap();
        s.sequence_create("Empty", false, actor(), None).unwrap();
        assert!(s.project.tracks.iter().all(|track| track.clips.is_empty()));
        let references = s
            .project
            .all_sequence_tracks()
            .flat_map(|track| &track.clips)
            .filter(
                |clip| matches!(clip, crate::types::Clip::Media(media) if media.asset == asset_id),
            )
            .count();
        assert_eq!(
            references, 1,
            "inactive sequence clips still own shared assets"
        );
    }

    #[test]
    fn grade_window_remove_replays_and_undoes_atomically() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        let (asset_id, _) = s.record_import(None, asset(), actor(), None).unwrap();
        let inserted = s
            .apply(
                "edit.insert",
                json!({
                    "asset": asset_id,
                    "track": "v1",
                    "at_ms": 0,
                    "src_range_ms": [0, 1000],
                    "ripple": true,
                }),
                actor(),
                None,
            )
            .unwrap();
        let clip = inserted.effects[0].detail["added_clip"]
            .as_str()
            .unwrap()
            .to_string();

        for points in [
            json!([[0.25, 0.25], [0.75, 0.75]]),
            json!([[0.0, 0.0], [0.5, 1.0]]),
        ] {
            s.apply(
                "edit.grade_window",
                json!({
                    "clip": clip,
                    "shape": "rect",
                    "points": points,
                    "brightness": 0.2,
                }),
                actor(),
                None,
            )
            .unwrap();
        }
        s.apply(
            "edit.grade_window",
            json!({"clip": clip, "remove_index": 0}),
            actor(),
            None,
        )
        .unwrap();

        let windows = |store: &ProjectStore| {
            store
                .project
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .find_map(|candidate| match candidate {
                    crate::types::Clip::Media(media) if media.id == clip => {
                        Some(media.grade_windows.clone())
                    }
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(windows(&s).len(), 1);
        assert_eq!(windows(&s)[0].window.points[0], [0.0, 0.0]);

        let mut reopened = ProjectStore::open(&s.dir).unwrap();
        assert_eq!(
            windows(&reopened),
            windows(&s),
            "cold replay preserves removal"
        );
        reopened.undo(actor()).unwrap();
        assert_eq!(
            windows(&reopened).len(),
            2,
            "undo restores the removed window"
        );
        reopened.redo(actor()).unwrap();
        assert_eq!(
            windows(&reopened).len(),
            1,
            "redo removes exactly one again"
        );
        assert_eq!(windows(&reopened)[0].window.points[0], [0.0, 0.0]);
    }

    #[test]
    fn undo_removes_adjustment_layers() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        s.apply(
            "edit.adjustment",
            json!({
                "range_ms": [0, 5000],
                "grade": {
                    "contrast": 1.0,
                    "brightness": 0.0,
                    "saturation": 0.0,
                    "gamma": 1.0
                }
            }),
            actor(),
            None,
        )
        .unwrap();
        assert_eq!(s.project.adjustments.len(), 1);

        s.undo(actor()).unwrap();

        assert!(
            s.project.adjustments.is_empty(),
            "undo must restore project-level adjustment layers too"
        );
    }

    /// e1·e2·e3 → undo×3 → redo×3 reproduces a byte-identical timeline at every
    /// step, undoing twice lands on a strictly OLDER state (no oscillation), and
    /// the baseline/tip guardrails fire.
    #[test]
    fn undo_redo_walks_history_byte_identical_no_oscillation() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        let (aid, _) = s.record_import(None, asset(), actor(), None).unwrap();
        assert_eq!(aid, "a1");

        let t0 = tl(&s); // baseline timeline (import adds an asset, not a clip)
        s.apply(
            "edit.insert",
            json!({"asset":"a1","track":"v1","at_ms":0}),
            actor(),
            None,
        )
        .unwrap();
        let t1 = tl(&s);
        s.apply(
            "edit.split",
            json!({"track":"v1","at_ms":4000}),
            actor(),
            None,
        )
        .unwrap();
        let t2 = tl(&s);
        s.apply(
            "edit.add_marker",
            json!({"at_ms":500,"label":"intro"}),
            actor(),
            None,
        )
        .unwrap();
        let t3 = tl(&s);
        assert_ne!(t0, t1);
        assert_ne!(t1, t2);
        assert_ne!(t2, t3);
        assert!(s.undo_available());
        assert!(!s.redo_available());

        // undo ×3 — each step lands on the OLDER state.
        s.undo(actor()).unwrap();
        assert_eq!(tl(&s), t2, "undo 1 → t2");
        s.undo(actor()).unwrap();
        // The oscillation bug would put us back on t3 here; the cursor lands t1.
        assert_eq!(tl(&s), t1, "undo 2 → t1 (NOT t3 — oscillation gone)");
        s.undo(actor()).unwrap();
        assert_eq!(tl(&s), t0, "undo 3 → baseline t0");
        assert!(!s.undo_available());
        assert!(s.redo_available());
        assert!(
            s.undo(actor()).is_err(),
            "undo at the baseline refuses (guardrail)"
        );

        // redo ×3 — forward, byte-identical to the originals.
        s.redo(actor()).unwrap();
        assert_eq!(tl(&s), t1, "redo 1 → t1");
        s.redo(actor()).unwrap();
        assert_eq!(tl(&s), t2, "redo 2 → t2");
        s.redo(actor()).unwrap();
        assert_eq!(tl(&s), t3, "redo 3 → t3");
        assert!(!s.redo_available());
        assert!(
            s.redo(actor()).is_err(),
            "redo at the tip refuses (guardrail)"
        );
    }

    /// Two ops sharing a group_id collapse to ONE history step, so a
    /// single undo removes BOTH (a linked A/V paste / linked delete is one
    /// Ctrl+Z), while an ungrouped edit beside them stays its own step.
    #[test]
    fn grouped_ops_undo_as_one_step() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        let mc = |s: &ProjectStore| s.project.markers.len();
        // A linked action: two marker ops sharing group "g1".
        s.apply(
            "edit.add_marker",
            json!({"at_ms":500,"label":"A","group_id":"g1"}),
            actor(),
            None,
        )
        .unwrap();
        s.apply(
            "edit.add_marker",
            json!({"at_ms":1000,"label":"B","group_id":"g1"}),
            actor(),
            None,
        )
        .unwrap();
        // A standalone edit (no group) beside it.
        s.apply(
            "edit.add_marker",
            json!({"at_ms":1500,"label":"C"}),
            actor(),
            None,
        )
        .unwrap();
        assert_eq!(mc(&s), 3);
        // The group of two collapsed to ONE entry → history is baseline + group + C.
        assert_eq!(s.undo_history.len(), 3, "group collapses to one step");

        s.undo(actor()).unwrap();
        assert_eq!(mc(&s), 2, "undo 1 removes the standalone C");
        s.undo(actor()).unwrap();
        assert_eq!(mc(&s), 0, "undo 2 removes BOTH grouped markers in one step");
        assert!(!s.undo_available());

        // Redo walks back forward, the group re-applying as a unit.
        s.redo(actor()).unwrap();
        assert_eq!(mc(&s), 2, "redo 1 re-applies the whole group");
        s.redo(actor()).unwrap();
        assert_eq!(mc(&s), 3, "redo 2 re-applies C");

        // Reopen: the group collapse is reconstructed from the persisted op tags.
        let dir = s.dir.clone();
        drop(s);
        let s2 = ProjectStore::open(&dir).unwrap();
        assert_eq!(
            s2.undo_history.len(),
            3,
            "reopen reconstructs the collapsed group from the log"
        );
    }

    /// A new edit made after an undo discards the redo future ("a fresh edit
    /// kills the redo branch").
    #[test]
    fn new_edit_after_undo_clears_redo() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
        s.record_import(None, asset(), actor(), None).unwrap();
        s.apply(
            "edit.insert",
            json!({"asset":"a1","track":"v1","at_ms":0}),
            actor(),
            None,
        )
        .unwrap();
        s.apply(
            "edit.split",
            json!({"track":"v1","at_ms":4000}),
            actor(),
            None,
        )
        .unwrap();
        s.undo(actor()).unwrap();
        assert!(s.redo_available(), "redo available after an undo");
        // A fresh edit truncates the redo branch.
        s.apply(
            "edit.add_marker",
            json!({"at_ms":100,"label":"x"}),
            actor(),
            None,
        )
        .unwrap();
        assert!(!s.redo_available(), "new edit clears the redo branch");
        assert!(s.redo(actor()).is_err());
    }

    /// Across a close+reopen, replay reproduces both the undone timeline and
    /// the logical cursor recorded by the navigation op.
    #[test]
    fn undo_survives_reopen_via_replay() {
        let d = tempfile::tempdir().unwrap();
        let dir = {
            let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
            s.record_import(None, asset(), actor(), None).unwrap();
            s.apply(
                "edit.insert",
                json!({"asset":"a1","track":"v1","at_ms":0}),
                actor(),
                None,
            )
            .unwrap();
            s.apply(
                "edit.split",
                json!({"track":"v1","at_ms":4000}),
                actor(),
                None,
            )
            .unwrap();
            s.undo(actor()).unwrap(); // live = after insert (split undone)
            s.dir.clone()
        };
        let s2 = ProjectStore::open(&dir).unwrap();
        // Replay reproduces the undone state: v1 holds ONE clip, not two.
        let v1 = s2.project.tracks.iter().find(|t| t.id == "v1").unwrap();
        assert_eq!(
            v1.clips.len(),
            1,
            "reopen replays the undone (single-clip) timeline"
        );
        // History reconstructed: baseline + 2 forward edits; cursor remains on
        // the insert, so split is immediately redoable.
        assert!(s2.redo_available(), "undone split is redoable after reopen");
        assert!(
            s2.undo_available(),
            "undo history is available after reopen"
        );
        let mut s2 = s2;
        s2.undo(actor()).unwrap();
        let v1 = s2.project.tracks.iter().find(|t| t.id == "v1").unwrap();
        assert!(
            v1.clips.is_empty(),
            "the first undo after reopen steps from insert to baseline immediately"
        );
        s2.redo(actor()).unwrap();
        let v1 = s2.project.tracks.iter().find(|t| t.id == "v1").unwrap();
        assert_eq!(v1.clips.len(), 1, "redo restores the insert after reopen");
    }
}
