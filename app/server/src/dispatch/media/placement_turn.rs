//! Request-order turns for first-import auto-placement.
//!
//! The first import into an empty timeline becomes the timeline. Each import runs
//! as its own background chain and decides when its own probe finishes, so with
//! several imports in flight the one whose probe happened to finish first used to
//! win, not the one that was asked for first. A turn orders only that decision:
//! probing stays concurrent, and an import takes its placement decision after
//! every import requested before it has decided (placed, declined, or failed).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[derive(Default)]
struct Turns {
    /// The next ticket to hand out.
    next_ticket: u64,
    /// Every ticket below this has finished.
    next_to_run: u64,
    /// Finished tickets above `next_to_run` (finished out of order).
    finished: BTreeSet<u64>,
}

/// The ordering for one server. Cheap to clone.
#[derive(Clone, Default)]
pub(crate) struct PlacementTurns {
    state: Arc<Mutex<Turns>>,
    changed: Arc<Notify>,
}

impl PlacementTurns {
    /// Take the next turn. Call in the order imports are requested.
    pub(crate) fn take(&self) -> PlacementTurn {
        let ticket = {
            let mut turns = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let ticket = turns.next_ticket;
            turns.next_ticket += 1;
            ticket
        };
        PlacementTurn {
            turns: self.clone(),
            ticket,
            finished: false,
        }
    }

    fn finish(&self, ticket: u64) {
        let mut turns = self.state.lock().unwrap_or_else(|e| e.into_inner());
        turns.finished.insert(ticket);
        loop {
            let next = turns.next_to_run;
            if !turns.finished.remove(&next) {
                break;
            }
            turns.next_to_run += 1;
        }
        drop(turns);
        self.changed.notify_waiters();
    }

    fn reached(&self, ticket: u64) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .next_to_run
            >= ticket
    }
}

/// One import's place in line. Dropping it, however the import ends, lets the
/// next import decide: a failed or cancelled import cannot block the others.
pub(crate) struct PlacementTurn {
    turns: PlacementTurns,
    ticket: u64,
    finished: bool,
}

impl PlacementTurn {
    /// Wait until every import requested before this one has decided.
    pub(crate) async fn wait(&self) {
        loop {
            let notified = self.turns.changed.notified();
            tokio::pin!(notified);
            // Register before checking, so a finish between the check and the
            // await cannot be missed.
            notified.as_mut().enable();
            if self.turns.reached(self.ticket) {
                return;
            }
            notified.await;
        }
    }

    /// Declare this import's placement decision made.
    pub(crate) fn finish(&mut self) {
        if !self.finished {
            self.finished = true;
            self.turns.finish(self.ticket);
        }
    }
}

impl Drop for PlacementTurn {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Test hook: hold one source's probe back, to force the order in which probes
/// finish. Keyed by the exact source path, so parallel tests do not interact.
#[cfg(test)]
pub(super) fn delay_probe_of(source: std::path::PathBuf, delay: std::time::Duration) {
    probe_delays()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(source, delay);
}

#[cfg(test)]
fn probe_delays(
) -> &'static Mutex<std::collections::HashMap<std::path::PathBuf, std::time::Duration>> {
    static DELAYS: std::sync::OnceLock<
        Mutex<std::collections::HashMap<std::path::PathBuf, std::time::Duration>>,
    > = std::sync::OnceLock::new();
    DELAYS.get_or_init(Default::default)
}

#[cfg(test)]
pub(super) async fn probe_delay(source: &std::path::Path) {
    let delay = probe_delays()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(source)
        .copied();
    if let Some(delay) = delay {
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test(flavor = "current_thread")]
    async fn a_later_import_waits_for_every_earlier_one() {
        let turns = PlacementTurns::default();
        let mut first = turns.take();
        let second = turns.take();
        let mut third = turns.take();

        // The first import is the slowest to decide, the others are ready at once.
        let mut second = second;
        let later = tokio::spawn(async move {
            second.wait().await;
            second.finish();
            third.wait().await;
            third.finish();
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!later.is_finished(), "must wait for the first import");

        first.wait().await;
        first.finish();
        tokio::time::timeout(Duration::from_secs(2), later)
            .await
            .expect("released once the first import decided")
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_failed_import_cannot_block_the_ones_after_it() {
        let turns = PlacementTurns::default();
        let first = turns.take();
        let second = turns.take();
        let waiting = tokio::spawn(async move { second.wait().await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished());
        drop(first); // the chain ended without deciding (probe failed, cancelled)
        tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .expect("a dropped turn releases the next one")
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn finishing_out_of_order_still_releases_in_order() {
        let turns = PlacementTurns::default();
        let mut a = turns.take();
        let mut b = turns.take();
        let c = turns.take();
        b.finish(); // decided early (for example a still image)
        assert!(!turns.reached(2), "a has not finished yet");
        a.finish();
        assert!(turns.reached(2));
        c.wait().await;
    }

    fn synth_clip(path: &std::path::Path, seconds: &str) {
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=160x90:r=30",
                "-t",
                seconds,
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(path)
            .status()
            .expect("ffmpeg is required for this test");
        assert!(status.success());
    }

    /// Three imports requested back to back into an empty timeline: the FIRST
    /// requested becomes the timeline even though its probe finishes last, and
    /// every import's job result says whether it was placed and, if not, why.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_first_import_requested_becomes_the_timeline() {
        use crate::dispatch::dispatch;
        let actor = || cut_core::Actor {
            kind: cut_core::ActorKind::Agent,
            name: "placement-order-test".into(),
            via: "test".into(),
            request: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let state = crate::state::AppState::new();
        let created = dispatch(
            &state,
            "project.create",
            serde_json::json!({"name":"t","dir": dir.path().join("t.cutproj")}),
            actor(),
        )
        .await;
        assert!(created.ok, "{:?}", created.error);

        let mut files = Vec::new();
        for (name, seconds) in [
            ("first.mp4", "3"),
            ("second.mp4", "1.5"),
            ("third.mp4", "1"),
        ] {
            let path = dir.path().join(name);
            synth_clip(&path, seconds);
            files.push(path.canonicalize().unwrap());
        }
        // The first file is the slowest to probe: left to race, another import
        // would reach the placement decision before it.
        delay_probe_of(files[0].clone(), std::time::Duration::from_millis(1500));

        let mut jobs = Vec::new();
        let mut assets = Vec::new();
        for file in &files {
            let imported = dispatch(
                &state,
                "media.import",
                serde_json::json!({"path": file}),
                actor(),
            )
            .await;
            assert!(imported.ok, "{:?}", imported.error);
            let result = imported.result.unwrap();
            assets.push(result["asset_id"].as_str().unwrap().to_string());
            jobs.push(result["job_id"].as_str().unwrap().to_string());
        }

        let mut results = Vec::new();
        for job_id in &jobs {
            let mut record = None;
            for _ in 0..400 {
                if let Some(rec) = state.jobs.get(job_id) {
                    if matches!(
                        rec.state,
                        crate::jobs::JobState::Done | crate::jobs::JobState::Failed
                    ) {
                        record = Some(rec);
                        break;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            let record = record.expect("import chain reached a terminal state");
            assert!(
                matches!(record.state, crate::jobs::JobState::Done),
                "{record:?}"
            );
            results.push(record.result.expect("import result"));
        }

        let on_timeline: Vec<String> = {
            let guard = state.project.read().await;
            let store = guard.as_ref().unwrap();
            store
                .project
                .tracks
                .iter()
                .filter(|t| t.kind == cut_core::TrackKind::Video)
                .flat_map(|t| t.clips.iter())
                .filter_map(|c| match c {
                    cut_core::Clip::Media(m) => Some(m.asset.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            on_timeline,
            vec![assets[0].clone()],
            "the first import requested must be the one on the timeline"
        );
        assert_eq!(
            results[0]["placed"],
            serde_json::json!(true),
            "{}",
            results[0]
        );
        assert!(results[0]["placement"]["tracks"]
            .as_array()
            .is_some_and(|t| !t.is_empty()));
        for later in &results[1..] {
            assert_eq!(later["placed"], serde_json::json!(false), "{later}");
            assert!(
                later["placement"]["reason"]
                    .as_str()
                    .is_some_and(|r| r.contains("already has media")),
                "a declined import says why: {later}"
            );
        }
    }
}
