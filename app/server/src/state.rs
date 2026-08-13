//! state.rs — cutd shared application state (server contract).
//!
//! Role: the one Arc<AppState> handed to every surface (REST handlers, WS,
//! MCP loop, CLI verb). Owns the open project, the job manager, the event
//! bus, the verb registry, and the UI-client channel.
//! Dependencies: cut-core, registry/jobs/events. Primary callers: main.rs
//! (construction), http.rs, dispatch.rs, mcp.rs.

use crate::doctor::DoctorReport;
use crate::events::{Event, EventBus};
use crate::framecache::FrameCache;
use crate::jobs::JobManager;
use crate::registry::VerbRegistry;
use crate::ui_bridge::UiBridge;
use cut_core::ProjectStore;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, Semaphore};

/// How many recent scrub frames the in-memory LRU holds. 64 JPEGs
/// at ≈10–200 KB each is a few MB — cheap insurance against re-encoding a frame
/// the human just scrubbed past or the agent just looked at.
const FRAME_CACHE_CAP: usize = 64;
/// JPEG bytes retained across the frame LRU. A count-only cap is not enough
/// once callers may request 4K previews.
const FRAME_CACHE_BYTE_CAP: usize = 32 * 1024 * 1024;
/// Uncached frames each own an ffmpeg process and decoded image buffers. Keep
/// interactive scrubbing responsive without admitting an unbounded queue.
const FRAME_RENDER_CONCURRENCY: usize = 2;

/// Shared server state. Clone = cheap (Arc fields).
#[derive(Clone)]
pub struct AppState {
    /// The open project, if any. RwLock: verbs mutate, readers snapshot.
    pub project: Arc<RwLock<Option<ProjectStore>>>,
    /// Serializes create/open/close/delete ownership changes. A project switch
    /// temporarily removes the current store while its background jobs drain;
    /// only one such transition may run, and deletion must not race that gap.
    pub project_transition: Arc<Mutex<()>>,
    /// Serializes caller-controlled idempotency preflight through durable
    /// response-receipt publication. Legacy calls keep their existing locks.
    pub request_gate: Arc<Mutex<()>>,
    /// Background jobs (transcribe/perception/render).
    pub jobs: JobManager,
    /// WS event fan-out.
    pub events: EventBus,
    /// Parsed schema/verbs.json — the verb contract.
    pub registry: Arc<VerbRegistry>,
    /// Last known UI-client state JSON (panels/playhead/selection), pushed by
    /// the UI over WS; None when no UI has ever connected this run.
    pub ui_state: Arc<RwLock<Option<serde_json::Value>>>,
    /// Server→UI command channel + screenshot request correlation.
    pub ui_bridge: UiBridge,
    /// Cached environment doctor report (the `system.doctor` source of
    /// truth). None until the first scan (startup, or the first verb call).
    /// `system.doctor{refresh:true}` and a completed `system.fetch_tool`
    /// recompute it and publish `doctor_updated` on a capability change.
    pub doctor: Arc<RwLock<Option<DoctorReport>>>,
    /// The server's bind address, stamped onto every doctor report so an agent
    /// reading the card knows which cutd it is talking to. None for non-serve
    /// surfaces (CLI/MCP-standalone) — the report then omits `addr`.
    pub addr: Arc<RwLock<Option<String>>>,
    /// Bounded LRU of recently served scrub frames. Keyed on the
    /// timeline revision, so any edit invalidates the whole timeline's frames
    /// by key change — never serves a stale frame. Derived state.
    pub frame_cache: Arc<FrameCache>,
    /// Admission slots for uncached frame rendering. Cache hits do not consume
    /// a slot, while excess misses fail promptly instead of queuing ffmpeg work.
    pub frame_render_limiter: Arc<Semaphore>,
}

impl AppState {
    /// Fresh state with no project open.
    pub fn new() -> Self {
        align_sidecar_ffmpeg_env();
        let events = EventBus::new();
        Self {
            project: Arc::new(RwLock::new(None)),
            project_transition: Arc::new(Mutex::new(())),
            request_gate: Arc::new(Mutex::new(())),
            jobs: JobManager::new(events.clone()),
            events: events.clone(),
            registry: VerbRegistry::shared(),
            ui_state: Arc::new(RwLock::new(None)),
            ui_bridge: UiBridge::default(),
            doctor: Arc::new(RwLock::new(None)),
            addr: Arc::new(RwLock::new(None)),
            frame_cache: Arc::new(FrameCache::new(FRAME_CACHE_CAP, FRAME_CACHE_BYTE_CAP)),
            frame_render_limiter: Arc::new(Semaphore::new(FRAME_RENDER_CONCURRENCY)),
        }
    }

    /// Record the server bind address (called by `cutd serve` at startup) so
    /// doctor reports can name the endpoint.
    pub async fn set_addr(&self, addr: impl Into<String>) {
        *self.addr.write().await = Some(addr.into());
    }

    /// Return the cached doctor report, scanning ONCE if it has never run.
    /// Cheap on the hot path: subsequent reads return the cache. The scan
    /// itself runs on a blocking thread (it spawns `-version` probes).
    pub async fn doctor_cached(&self) -> DoctorReport {
        if let Some(r) = self.doctor.read().await.clone() {
            return r;
        }
        self.doctor_rescan().await
    }

    /// Re-run the environment scan, update the cache, and publish
    /// `doctor_updated` IFF the capabilities changed (status/source/version of
    /// any card) — a pure timestamp bump never spams the event stream.
    /// Returns the fresh report. Used by `system.doctor{refresh:true}`, the
    /// startup scan, and `system.fetch_tool` on completion.
    pub async fn doctor_rescan(&self) -> DoctorReport {
        let addr = self.addr.read().await.clone();
        // The scan blocks (subprocess version probes) — run it off the async
        // executor so the verb loop never stalls on a wedged binary.
        let report = tokio::task::spawn_blocking(move || crate::doctor::scan(addr))
            .await
            .unwrap_or_else(|_| crate::doctor::scan_minimal());
        let changed = {
            let prev = self.doctor.read().await;
            match prev.as_ref() {
                Some(p) => !p.same_capabilities(&report),
                None => true,
            }
        };
        *self.doctor.write().await = Some(report.clone());
        align_sidecar_ffmpeg_env();
        if changed {
            self.events.publish(Event::DoctorUpdated {
                report: report.clone(),
            });
        }
        report
    }
}

/// Keep `SHELLX_CUT_FFMPEG_DIR` pointed at the resolved ffmpeg so the python
/// perception sidecar finds the same ffmpeg/ffprobe binaries the engine uses.
///
/// This must run before the first import/enrich job, not only after the async
/// startup doctor scan. A cold desktop app can launch media work while that scan
/// is still running, and macOS GUI apps often have a stripped PATH even though
/// the engine resolver can find Homebrew or the user's selected ffmpeg.
fn align_sidecar_ffmpeg_env() {
    if let Some(dir) = cut_media::toolpath::resolved_ffmpeg_dir() {
        std::env::set_var(cut_media::toolpath::ENV_FFMPEG_DIR, dir);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
