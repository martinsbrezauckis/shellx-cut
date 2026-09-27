---
name: shellx-cut
description: Use when editing video with ShellX Cut or its cutd server — video edit, cut video, trim footage, remove silences, remove filler words, transcript editing, text-based video editing, captions/SRT, talking-head or screen-demo cleanup, render receipt, verify a render, export FCPXML, shellx cut, cutd. Covers driving the verb API (REST + MCP) headless and verifying results with receipts.
---

# ShellX Cut — agent-first video editing

<!-- shellx-cut-release-truth: candidate; version=0.6.114; published=0.6.113 -->
> **Engine v0.6.114 candidate.** v0.6.113 is the latest published release.
> Synced to the contract (`schema/verbs.json` — the single
> machine-readable source of truth; if this guide and that file disagree, trust
> the file): **308 verbs across 34 domains** under the public verb contract.
> **`reference.md` is the full 308-verb table —
> consult it for any verb not detailed below.** A
> capability-grouped public-safe feature inventory lives in
> `docs/public/FEATURES.md`.
>
> **This guide walks the core workflow end-to-end** — talking-head / podcast /
> screen-demo cuts driven by the transcript (the workflow below). The contract has
> grown well past that wedge; the capabilities below are NOT detailed in the
> workflow but are driven exactly the same way (one verb → one op → a receipt) —
> listed so you know they exist. Consult `reference.md` for args.
>
> - **Speaker diarization** — `media.diarize` ("who spoke when": a job that POSTs
>   the asset's audio to the Sortformer-v2 service → arrival-order speaker turns +
>   per-word `speaker` labels, refreshing the transcript so captions/multicam can
>   key by speaker). For a human, use **Transcript Tools > Label speakers** only
>   after the Doctor diarize card is ready; it binds the selected current-project
>   asset and accepts only its returned job and `receipts/<asset>.diarize.json`.
> - **AI dubbing + subtitle translation** — `audio.dub` (re-voice an asset's
>   speech into another language in a cloned voice, time-fit to the original,
>   added as a NEW audio track; reuses `transcript.translate` + the OmniVoice TTS
>   service). For a human, use **Transcript Tools > Dub audio** after the Doctor
>   dub card is ready and choose the target language there; the control verifies
>   the selected asset, target language, resulting track, and matching receipt.
>   `transcript.translate` / `captions.translate` are TEXT-only
>   translation (CLI-primary, local Opus-MT/MADLAD fallback — no dubbing).
> - **Kinetic captions** — `captions.kinetic` defaults to animated caption
>   lines and requires caption cues from `captions.generate`; pass
>   `per_word:true` to animate EDL-mapped transcript words instead, without a
>   caption track. Both forms return the placed overlay and cue count.
> - **Agent chat (natural-language editing)** — `agent.chat` launches the user's
>   installed Claude Code, Codex, Grok, or Antigravity CLI. Claude uses Cut's
>   version-independent contained capability contract. Provider version text is
>   informational only and each route's required containment flags are verified before every turn. Codex keeps the user's normal configuration, native
>   sandbox, and permissions; Cut adds its filtered MCP server without copying or
>   rewriting Codex login files. Grok receives a disposable config/home with
>   native tools disabled and only Cut's MCP route, while retaining its existing
>   login file in place. Cut trusts only that newly created empty workspace for
>   the turn so Grok can start the project-scoped Cut MCP server. Antigravity
>   keeps its normal settings, sandbox, and
>   login with a new disposable project containing one Cut-only MCP plugin.
>   Its headless approval mode is bounded by that empty sandbox and Cut's filtered
>   server-side verb policy; the resolved CLI's complete launch contract is
>   verified before each turn on every supported platform. Every Cut verb applied by these routes is a normal
>   reversible op.
>   `attachments` can carry up to eight registered project asset IDs as references;
>   the server validates them against the open project and exposes no arbitrary
>   source-path input. An optional `shellx-cut/chat-timeline-target/1` names an
>   immutable project identity/revision and exact clips/range or point; Cut
>   revalidates it before launch and returns it with the turn rather than using a
>   later UI selection. Each launched turn applies its validated result directly
>   and returns `plan` plus a `review` artifact with its pre-turn baseline,
>   post-turn tip, computed diff, and a `revert_safe` verdict. Agent ops carry a
>   unique per-turn actor. Concurrent human/system ops are listed separately and
>   disable whole-turn revert. The UI uses those refs for Preview, Diff,
>   tip-guarded **Step back**, and **Ask replacement**; there is no Draft, Accept,
>   or editor approval stage. A stale or deleted retained target refuses visibly
>   instead of applying to a replacement selection.
>   The Agent Chat prompt library offers eight curated Polish, Repurpose, Speech,
>   and Review requests mapped only to inspect/edit verbs admitted by the contained
>   Chat policy. Choosing one merely pre-fills the editable composer; it never
>   launches a CLI turn by itself. A speech-service, render, delivery, or
>   verification request must navigate the human to Transcript Tools, Clips, or
>   Review > QC rather than claiming Chat can perform that denied work.
> - **Scoped plugins (agent-only)** — `plugins.list`, `plugins.enable`, and
>   `plugins.call` expose the built-in Openverse-assets and matte-runtime scopes
>   as a permission fence over the SAME verb registry. A disabled, out-of-scope,
>   or corrupt/unavailable-state call fails closed; this is not a second
>   extension API. Inspect `plugins.list` for recovery guidance; an explicit
>   `plugins.enable {name,enabled:true}` repairs only that named grant.
> - **Agent motion measurement** — `edit.track {clip, bbox|point, ...}` is a
>   direct REST/MCP measurement call, not an editor control or Agent Chat action.
>   It returns sampled `points` and `pos_x`/`pos_y` keyframe arrays for a seeded
>   region; bind them explicitly through `edit.keyframe` or a tracked
>   `edit.redact` mask. It does not mutate the project, and reports the required
>   perception-sidecar setup when that dependency is unavailable.
> - **Native editable Generate** — `generate.list` / `generate.describe` /
>   `generate.preview` / `generate.insert` / `generate.from_prompt` /
>   `generate.storyboard` power the editor-side Generate tab: reusable
>   templates, prompt-planned editable visuals, non-mutating PNG previews,
>   undoable timeline inserts, and multi-scene storyboard IR. The
>   prompt/storyboard PLANNER ships as bundled adapters that route to the
>   user's LOCAL CLI subscription agent (claude/codex/grok, `agent:"auto"` =
>   first installed; no CLI → honest `not_run`) — an agent driving the verbs
>   can either call them directly (the engine plans) or produce the plan/IR
>   itself per `craft/generate-storyboard-planning.md`. For v0.6.114, Motion
>   qualification is limited to read-only discovery. The existing connection
>   routes below remain unqualified and are outside this release's testing;
>   do not report them as a completed integration. Motion-backed
>   templates lower through `motion.template_to_cut` or `motion.script_to_cut`,
>   which call ShellX Motion and import rendered media when the user chooses
>   Insert. Cut supplies a stable path-private workspace caller id on every
>   Motion CLI entry point. For observable progress, supply a unique `job_id`
>   on `motion.template_to_cut`, `motion.script_to_cut`, or
>   `motion.link.refresh`, then poll `motion.job.get` from another request (or
>   use `motion.job.list`). Treat `pending` as waiting for a machine slot,
>   `running` as active work, and the other four Motion states as terminal.
>   Poll no faster than `pollAfterMs` and stop when it is absent. Cut derives
>   the active-project caller scope and exposes no all-callers option. Treat
>   `render_cancelled` as a deliberate stop and
>   never retry it; `job_queue_timeout` means the shared Motion machine is busy
>   and may be retried after capacity frees. Connector handoffs from ShellX Motion use `motion.map_import` for
>   non-mutating import-plan preflight and `motion.apply_import` to dry-run or
>   commit a receipt-bound plan. A Motion receipt with `status:"warning"` is a
>   successful advisory, not a failed render: continue the workflow and surface
>   its deduplicated `warnings`; only `passed` and `warning` are accepted. Always map first and inspect
>   `lineageProofs[].status`: `verified` means the Motion package's two base
>   hashes (plus all three glTF provenance hashes for `adapter.gltf`) were bound
>   through the artifact identity, exact render receipt, and Cut-plan receipt;
>   `legacy-unverified` keeps older template/script connectors usable but is not
>   package-lineage proof. When `packageDir` is supplied, also inspect
>   `currentPackage.status`: `exact` means the independently read package bytes
>   match, `changed` names the differing hash fields, and `unavailable` is not a
>   match claim. After a real rendered apply, confirm the same path-free
>   proof at `project.state … motion_link.originAttestation`; do not describe a
>   legacy result as verified. `editable_lowering` currently maps exact
>   document backgrounds, text, and basic vector shapes to native Cut title/shape objects with stable
>   source-layer bindings and grouped undo; uniform opacity and x/y position
>   tracks lower to native `edit.keyframe` automation, including off-screen
>   position values and non-overlapping fade-in/out transitions that use one
>   Cut-compatible easing. Scale/rotation keyframes remain rendered-media
>   fallback because Cut cannot preserve their Motion transform semantics exactly.
>   One Cut-origin video may use
>   `cut-asset:<id>` to stay a normal native media clip; portable paths still
>   require rendered media. Unprocessed Cut-origin audio can use the same
>   reference to return to the native audio track. Changed plans for the same Motion
>   identity update those bound objects in place when layer kinds and timing
>   still match; dynamic/unknown fields fail closed
>   to the rendered-media path. `rendered_media` remains the universal fallback.
>   The promoted footage-rich Generate families are
>   `builtin.motion.cinematic-fog-title`,
>   `builtin.motion.editorial-liquid-surface`,
>   `builtin.motion.keyed-subject-promo`, and
>   `builtin.motion.tracked-callout-overlay`. Discover them with
>   `generate.list{kind:"motion"}`, inspect bounded decimal/text/color controls
>   with `generate.describe`, prove a non-mutating frame with `generate.preview`,
>   then call `generate.insert` only after review. The default package-local
>   sample media is safe for preview; replace production scene/subject media in
>   ShellX Motion through **Edit in Motion**, then refresh the same linked
>   Cut clip. Fog, water, keying, matte cleanup, and tracked-callout motion remain
>   Motion-owned rendered effects rather than fake native Cut controls.
>   Rendered Motion imports retain a stable `motion_link` on the live Cut clip:
>   `clipId` is the editorial identity, `packageId`/`motionId` are the source
>   identity, and the attested render digest is replaceable derived media. The
>   timeline `M` badge and Inspector link section expose this state; do not imply
>   rain, water, snow, shaders, 3D, particles, Motion blur, or film controls are
>   native Cut edits. Open those controls and their curves through
>   `motion.link.edit`, render the edited copy-on-write revision in ShellX Motion, then use
>   `motion.link.refresh` to update the same Cut clip. Cut creates a path-private
>   return request for the launch; Canvas publishes an immutable ready descriptor
>   only after a verified render, and refresh rechecks identity plus source revision
>   before adopting it.
>   Use `motion.link.relink` to repair a missing local package only after its
>   package/motion identity matches. Use `motion.link.refresh` to render a new
>   immutable project-owned artifact and atomically replace the same Cut clip;
>   receipt/digest/source races fail without disturbing the last good render.
>   `motion.link.edit` returns that current verified source revision and opens
>   the same identity in ShellX Motion through its `--motion-package` host intake
>   and trusted `--motion-cut-return-request`
>   handback; `SHELLX_CANVAS_BIN` is a backward-compatible executable override if
>   needed. Never invent or expose either
>   filesystem path in an agent response.
>   Inspect `project.state` first: `motion_link.effects` reports bounded keyed,
>   animated-roto, and tracked-roto counts plus safe layer summaries without
>   paths, vertices, tracking ids, or unknown package fields. These are visible
>   source facts, not native Cut controls; edit in Motion and refresh the render.
>   For a linked package with footage, use `motion.link.tracking.inventory` to
>   choose a package-local video asset and visual target layer. Run
>   `motion.link.tracking.request` with a normalized seed region, inspect source
>   freshness, then apply stabilization keyframes with
>   `motion.link.tracking.apply`. Verify with `motion.link.tracking.verify` and
>   detach exactly with `motion.link.tracking.detach`. Request/apply/detach are
>   copy-on-write local package revisions guarded by package identity, receipts,
>   fixed argv, and link-race checks; apply/detach leave the last good Cut render
>   intact until an explicit `motion.link.refresh` succeeds.
>   Dry plans carry `plannedPath`; real plans are accepted only through verified
>   `shellx-motion/artifact-handle-ref@1` descriptors bound to unchanged media
>   bytes and successful render/connector receipts. Use `background:true` for a
>   progress-reporting apply that can be stopped with `jobs.cancel`; the exact
>   plan hash makes retries idempotent, and undo/revert removes the plan-owned
>   clips and assets together. Keep this distinct
>   from `assets.generate`, which imports provider-backed media from the user's
>   own generation CLI. `assets.generate` supports Codex images, Grok Imagine
>   images/video, and Antigravity (`agy`) images only; Antigravity uses its
>   native sandboxed non-interactive contract with its existing login/settings
>   left in place. Omitted generation deadlines are 4m for Codex, 10m for Grok,
>   and 11m for Antigravity; explicit values remain bounded to 10s–30m.
>   Generation accepts up to four registered image/video
>   references and an explicit variation label; `assets.generated_list` exposes
>   a path-light, integrity-checked project history for reference and retry.
>   `system.motion_status {}` is the separate bounded read-only Motion
>   management check. It runs only the fixed runtime probe, catalog, and
>   Template-to-Cut descriptor route without a caller id, provider login,
>   connector execution, download, or mutation. A discovered source/PATH/npm
>   runtime remains unmanaged and execution-unqualified. Install, Repair,
>   Update, and Remove stay unavailable behind `MOTION-DIST-01` until Motion
>   publishes a verified immutable platform manifest and artifact; never
>   relabel an existing checkout or CLI as a managed installation.
> - **Recipe layer** — `recipe.list` / `recipe.describe` / `recipe.run`:
>   declarative, gated pipeline MANIFESTS over the existing verbs (built-ins
>   in `schema/recipes.json`: `first-project`, `edit-for-clarity`,
>   `podcast-repurpose`, `talking-head-cleanup`,
>   `screen-demo-polish`, `phone-clip-cleanup`, `social-short-bundle`,
>   `area-privacy-mask`, `add-captions`, `youtube-export`, `tiktok-export`).
>   `run` is a pure orchestrator — one auto-checkpoint,
>   per-stage gates, stops + reports on the first failed verb or gate.
> - **Recording Studio** — the `screen_record.*` domain wires the integrated
>   Cut recorder crates in process: `doctor` / `system_audio_probe` /
>   `preview_capability` / `preview_start` / `preview_status` / `preview_frame` /
>   `preview_pause` / `preview_resume` / `preview_hide` / `preview_stop` /
>   `rehearsal_start` / `rehearsal_discard` / `start` /
>   `screen_record.status` / `screen_record.pause` / `screen_record.resume` / `stop` /
>   `recovery_status` / `studio_event` / `autoedit` / `polish` / `export` (live screen/audio
>   capture, raw streams, auto-edit plan, content-addressed bake). On Windows and
>   macOS, Doctor can advertise opaque camera choices for an explicit Auto-edit
>   recording. The admitted camera is finalized as a separate editable take with
>   shared-clock evidence; permission, busy-device, and no-frame failures never
>   fall back to another device. Preview Pause, Resume, Hide, and Stop each take
>   the latest positive `generation` and matching opaque `lease_nonce` returned
>   with that active lease by Start, Status, Frame, or a nonterminal Pause/Resume
>   result. A stale generation or nonce
>   is rejected without changing a newer or restarted-server preview; the nonce
>   changes on Cut-server restart and is neither auth nor a native target/ticket.
>   Recording Scenes freezes named Screen and
>   Presenter PiP presets at Start; `screen_record.scene_activate` and
>   `screen_record.scene_timer` durably
>   save shared-clock transitions before acknowledging them.
>   `unavailable_reason` appears only when Cut cannot prove it released a prior
>   native preview; it requires restarting that process, not merely reopening
>   Record.
>   Doctor health stays strict: on Linux `start_allowed:true` means only the
>   deliberate prompt-deferred XDG ScreenCast portal card may enter the
>   user-initiated source picker; it does not make `ready` true, and missing,
>   degraded, or other unknown required cards still refuse `start`.
>   Doctor reports `system_audio` separately as an optional passive card. A
>   compiled backend remains `unknown` until a real user-started recording
>   proves packets; Doctor opens no loopback/tap stream and cannot trigger the
>   separate macOS Audio Capture prompt. This card does not gate screen-only
>   recording.
>   Windows/macOS Doctor monitor rows include an opaque, versioned native `id`
>   only when an exact identity is available. Pass it unchanged as
>   `screen_record.start{monitor_id}`; Cut re-enumerates and matches it exactly
>   before capture. It is not a display name, ordinal, primary flag, geometry,
>   or device path; never invent a replacement. The legacy full-display picker
>   still accepts its documented `index` when `id` is absent, and Linux
>   intentionally returns no in-app monitor rows. The human Record UI separates
>   Display and Window targets at the first level and does not expose Region
>   until a real native picker can succeed; agents must not infer a crop or
>   unavailable Region capability from that UI.
>   **Visible Record workspace.** Enter through the shared
>   `[data-cut-mode="record"]` tab. The same-position Edit tab is the one
>   `[data-cut-action="record-back-edit"]` control; when it has
>   `data-cut-record-back-blocked="true"`, do not force a workspace change—read
>   `[data-cut-record-back-reason]` and resolve the active capture first. Start
>   with `[data-cut-rec-readiness]`; expand
>   `[data-cut-action="record-readiness-details-toggle"]` only when the
>   individual checks matter. Use `[data-cut-action="record-source-refresh"]`
>   after changing a source or capture setup, then wait for the fresh result:
>   Refresh makes Start unavailable while its result is unknown, and a prior
>   positive outcome is not reusable. `[data-cut-studio-preview]` is the one
>   source-image plane. Use `[data-cut-rec-settings-tab="camera"]` for camera
>   position, shape, size and Reset; use
>   `[data-cut-rec-settings-tab="background"]` for the background. The
>   composition context menu remains an alternate route on the preview.
>   Do not infer a native
>   source, permission, or first frame from a fixture or a visible control.
>   F9 is one Cut-wide Start/Stop action, including from Edit or while another
>   app has focus when the OS admits a global callback. Reuse only the last
>   validated setup intent, then recheck the current project, exact source,
>   devices, and permissions on every Start. If stale, open Record with a
>   reason and do not claim capture. A passive recording indicator follows the
>   exact capture state outside Record; native icon/badge presentation needs
>   installed-host proof.
>   Read the desktop hotkey capability and its observed callback state before
>   claiming global behavior. On GNOME Wayland, Cut attempts its owned custom
>   shortcut on first launch. The Record view has no global F9 enable checkbox;
>   its transport reports observed scope or failure passively.
>   `configured` remains focused-only until Cut
>   observes its forwarded callback as `observed`/`global`. Disabled or
>   unavailable capability keeps focused F9 and exposes its reason. Never infer
>   global operation from the control being present.
>   Output quality is capability-gated in the same way. Read
>   `screen_record.doctor.quality`; only when `supported:true` may an agent pass
>   one advertised Source/1080p/720p plus Standard/High pair in
>   `screen_record.start{quality}`. Current Windows/macOS backends return empty
>   choices and reject direct requests. Stop may return final quality facts only
>   after the Linux final-source verifier confirms the requested height limit,
>   H.264 container, and named libx264 encoder.
>   After Start, `screen_record.status{capture_id}` is the short-lived native
>   admission check for automation: continue only at `ready:true` with
>   `terminal:false`. Process start, elapsed time, or an output file is not a
>   substitute for the first real screen frame, and a terminal capture is never
>   admitted even when it delivered frames earlier. Its `source_lifecycle` is
>   narrow evidence: `source_lost` means an armed exact native selected-source
>   close won before Cut's own close, not that Stop, duration expiry, permission,
>   encoder, or disk failure was relabelled. `controller_placement` reports the
>   observed presence or coverage of Cut on the selected display; inspect real
>   frames and output before claiming what appeared in the recording. Its
>   `audio_meters` read only the already-admitted mic/system
>   streams; live, stale, device-lost, stopped, and unavailable are distinct, and
>   status never opens a device or starts monitoring playback.
>   After `screen_record.start` returns a `capture_id`, retain it. A
>   non-successful `screen_record.stop` leaves that capture unresolved: do not
>   start a replacement. Check `screen_record.status{capture_id}`. Offer Retry
>   Stop only when the same capture is still live (`terminal:false`); classify
>   terminal or `not_found` as ended and uncertain ownership as unknown.
>   `screen_record.recovery_status` is the separate durable inventory. The
>   Record UI saves the original MP4 to the default export folder for both Raw
>   and Polished; optional copy or Export destination selection follows Stop.
>   Explicit API callers may still use authorized `raw_path`. A later
>   `screen_record.polish` error is processing failure, not a live capture.
>   The Record outcome choice has only Raw MP4 and Polished clip in Edit. Both
>   call Stop with `mux_raw:true`; Polished also requests `autoedit:true` and
>   inserts an editable polished clip with zoom-to-cursor, cursor smoothing,
>   and framing. Show keystrokes sits beneath Polished and starts off. The
>   selected-source setup preview is real native pixels; during capture read
>   `screen_record.live_frame{capture_id}` only for the active owner. Show
>   unavailable or stale state when frames stop, while keeping Stop available.
>   Camera exposes four corners, shape, size, and explicit Reset camera layout;
>   preserve manual placement across internal scenes. On-video Countdown accepts
>   an applied 00:00:01–23:59:59 duration and never ends capture at zero.
>   Native preview is separately opt-in and process-local. First read
>   `preview_capability`: `source_selection:"exact"` admits only an unchanged
>   current Doctor monitor/window id, while `"portal"` admits only
>   `{source:{kind:"portal"}}` and lets the Linux system picker choose. Poll
>   `preview_status`/`preview_frame` slowly enough for the 10 FPS cap and bind
>   pixels to `generation`; `ready` requires a real bounded memory-only BMP.
>   Pause releases the native owner but remembers the exact opaque source;
>   resume advances generation. Hide/Stop and ordinary recording Start release
>   native preview ownership and pixels. Never infer a source from labels,
>   ordinals, coordinates, browser OS, or `getDisplayMedia`.
>   The human Record HUD projects rolling mic/system peak and RMS from the
>   already admitted capture streams. It never opens monitoring playback or a
>   second input. Silence, stale packets, loss, clipping, and unsupported inputs
>   remain explicit; current macOS system-audio metering is unavailable even
>   though finalized tap audio can still be reported at Stop.
>   `rehearsal_start` creates one video-only 3–5 second native test take in a
>   Cut-owned temporary root and returns only an opaque same-origin playback
>   handle. It creates no project, recovery record, journal, timeline asset, or
>   promotion path. Always call `rehearsal_discard` when playback closes; a new
>   rehearsal or ordinary Start also discards it. During a normal take, use the
>   acknowledged recorder verbs behind the compact live controller—never fake
>   Pause, markers, scenes, timers, or input state locally.
>   The visible **Test microphone** control tests System Default or the current
>   selected source and reports one real peak. Windows/macOS selection consumes
>   one current opaque Doctor token into private endpoint storage; public labels
>   are generic `Microphone N`, Linux stays System Default only, and an absent
>   saved choice refuses mic-enabled start rather than changing source.
>   When the user deliberately chooses **Test system audio**, call
>   `system_audio_probe{max_ms?:500..5000}` while a short sound is playing. This
>   is the bounded consenting path: it may open the macOS Audio Capture prompt,
>   reports real packet delivery and separately whether the samples contain a
>   signal, retains no audio, and does not start a screen recording or require a
>   project. Do not call an all-silent delivered stream ready.
>   `recovery_status{after?,limit?}` is read-only and process-free: page only
>   with an emitted `next_cursor`, treat an unknown cursor as rejected, and use
>   its path-safe receipt/loss facts for Settings → Health & Recovery rather than
>   trying to inspect cache files or trigger repair from a read. Settings gathers
>   sequential 100-row lexical pages (at most 4,096 rows) and labels a completed
>   result as reported in that check; malformed or partial inventory is attention.
>   On Windows 10 build 20348 or newer, system audio uses native,
>   endpoint-independent process loopback instead of opening the physical render
>   driver. Security software may ask to allow audio capture for a new Cut binary;
>   if access is blocked, screen and microphone recording continue and the missing
>   `system.wav` is reported in the capture log/raw-stream result.
>   On macOS 14.2 or newer, the installed signed app captures system audio through
>   a Core Audio process tap alongside ScreenCaptureKit. Approve the separate
>   Screen Recording and Audio Capture prompts on first use, restart Cut if macOS
>   asks, and verify `raw_streams.system` after `screen_record.stop`.
>   `raw_has_system` is true only when `mux_raw:true` also included that stream
>   in the optional combined raw output. Stop ends
>   that tap at the video boundary before checkpoint stitching, and its finite
>   wait scales from capture work rather than assuming every native finalize fits
>   a fixed short timeout.
>   Studio background and marker changes are ordered by the server and appended
>   to a bounded, crash-recoverable `studio-events.jsonl` journal. Legacy
>   `studio-events.json` captures remain readable. In the human UI, **None** is
>   a truthful full-bleed recording with no backdrop or frame, and ordinary
>   completion copy remains path-light until the user deliberately chooses
>   Reveal or Copy path.
>   `jobs.retry {job_id}` can create exactly one linked attempt for either an
>   eligible failed default-output `screen_record.export` or an eligible failed
>   `verify.rerun`. The recorder route revalidates the project revision, capture
>   media and audio, EditPlan, and output lease, then gives the queued renderer
>   private no-follow staged copies of those exact bytes; the verification route
>   revalidates the active project, immutable RenderReceipt, and exact rendered
>   bytes. Never retry a cancellation, explicit Save As, success, legacy record,
>   changed input, unowned job kind, or already retried job.
> - **Timeline voiceover** — `voiceover.start` is human-UI-only and only becomes
>   available after the current `screen_record.doctor` explicitly admits native
>   capture. It binds a current unlocked audio track plus playhead or In/Out and
>   a required `request_id`/`expected_revision` before reserving the private
>   source. It returns a server-issued memory-only owner claim bound to that
>   caller and request; every tick, Stop, Cancel, and Preview Out must present
>   the claim, while a response-loss retry keeps the original request id and
>   non-secret reattach session id. The server owns real readiness/count-in,
>   exact correlated Preview seek/playback acknowledgement, sealed WAV
>   verification, and one atomic asset-plus-clip placement (one Undo). Poll
>   `voiceover.tick`; call `voiceover.stop`, `voiceover.cancel`, or
>   `voiceover.observe_playhead` only from the visible Timeline flow. Direct
>   microphone monitoring is always off.
>   Reattach A before comparing its later UI revision/range: an exact live A
>   takes precedence over ordinary project edits, while B remains refused until
>   A resolves. Only the explicit `voiceover_start_retry_rejected` start code
>   permits clearing the browser's non-secret retry identity; ownership or
>   transport errors remain retry-ambiguous.
>   A disabled control or refusal is not evidence that a host has native
>   microphone readiness; no installed/native qualification is claimed here.
> - **Director / reframe / delivery** — `render.reframe` (subject-tracked moving
>   crop to a platform aspect — the HONEST alternative to a static centre-crop),
>   `render.direct` (director-model pass: a per-scene contact sheet the foundation
>   model reads to choose WHICH subject each shot is about, fed back into
>   `render.reframe{direction}`), `render.queue` (batch delivery), `render.bundle`
>   (social repurposing pack), `verify.pregate` (PRE-render predictive quality
>   gate, no render spent).
> - **Repurposing / assembly** — `assemble.repurpose`, `assemble.shorts`, and
>   `assemble.from_script` return a reviewed, revision- and transcript-bound
>   plan by default.
>   Resend the unchanged request with its `apply:plan_binding`, `request_id`,
>   and matching `expected_revision` to make one editable, normal-Undo timeline
>   operation. Shorts apply their planned source crop and transcript captions
>   only when the current project aspect already matches the requested aspect;
>   the plan reports a disabled reason otherwise. `assemble.broll` remains the
>   direct slot→retrieve→place path. If its delayed provider search sees a project or
>   revision change, its partial result names the path-free origin identity, last
>   accepted revision, and checkpoint; verify the identity, compare the current
>   revision, and review intervening changes before restore. It never reverts the
>   now-current project. `clip.candidates` and `score.clip` remain model-free
>   selection tools.
> - **AI matte (no green screen)** — `edit.matte` (+ `system.setup_matte`):
>   background removal/replace, RVM auto default or premium target-assigned
>   MatAnyone2 (SAM2 click-to-pick subject).
>   Subject seeds use integer source pixels and source time in milliseconds.
>   Read applied/cleared intent from the committed `op.effects[]` fields;
>   applying also returns the immediate alpha-bake quality receipt as `matte`.
> - **Advanced color** — the `grade` gallery (`grade.save`/`apply`/`list`),
>   `edit.grade_stack` (layered grades), `edit.grade_window` (power window),
>   `project.color` / `edit.color_space` (Rec.709, Rec.2020, sRGB, or
>   scene-linear working/output/input conversion). This is a lightweight
>   four-space path, not camera Log interpretation or HDR mastering/delivery.
> - **Multicam** — `edit.multicam_sync` (audio-align angles) + `edit.multicam_switch`
>   (auto-cut the program to the active-speaker angle).

## Current receipt, fix-loop, and repurposing capabilities

- **Verify before you ship — the receipt family.** Beyond `verify.checks` (the
  render battery) and `verify.judge` (perceptual visual review), seven
  render-free measurement/QC receipts let you inspect the cut before a final
  encode:
  `verify.pacing` (visual shot rhythm), `verify.delivery` (verbal: WPM + filler
  density over the transcript), `verify.captions` (caption QC vs BBC/Netflix
  timed-text standards), `verify.brand` (proves caption styles + output aspect
  conform to the durable `project.brand` kit, or an explicit one-call override;
  the agent resolves AGAINST brand, never overwrites it). `render.bundle`
  automatically enforces the saved kit and records its source in the manifest.
  `verify.loudness` measures an asset's LUFS/peak/range, `verify.scopes` measures
  the composed picture and can render scope images, and `verify.pregate`
  predicts timeline risks before rendering. Report their numbers like any
  receipt.
- **Recheck historic rendered bytes without changing their evidence.** In
  Review → Receipts, `verify.rerun {render_id}` queues one cancellable,
  output-only job for the selected immutable render. Cut validates the stored
  receipt identity, re-fences and fully re-hashes the output before work,
  before the sidecar/probe, and before publishing a separate
  `receipts/verify_rerun_<job_id>.json`. Read `jobs.status.result`; do not claim
  that this rerun checked source words, captions, edit boundaries, or the
  current timeline, and do not replace the original RenderReceipt with it.
- **Measure → fix loops.** Each actionable check now has a dedicated fix verb,
  so you can close the loop instead of hand-editing: `lufs` ← measure with
  `verify.loudness {asset}` (integrated LUFS / true-peak / LRA + the exact
  normalize recommendation), fix with `render.final {normalize_loudness:-14}` ·
  `verify.captions` ← `captions.reflow` (split
  over-length cues + extend too-fast cues into gaps) · `silence_at_edges` ←
  `edit.trim_edges` (top-and-tail dead air, speech-anchored, preserves internal
  pacing). Pattern: run the verify, apply the fix, re-run the verify.
- **One recording → many deliverables (repurposing).** Two paths, both leave the
  project untouched so one cut publishes to many formats:
  - **`render.reframe{aspect:"9:16"}`** (preset `talking_head`/`sports`/`pets`/
    `cars`/`general`) is the HONEST default for vertical/square: it renders the
    finished edit, runs the local-CV `subject` instrument, then a subject-tracked
    moving-crop post-pass that FOLLOWS the subject with a smoothed pan. The receipt
    reports subject-in-frame %, the device it analyzed on, and that reframe is a
    LOSSY crop. Prefer this when the framing should track a person/subject.
  - **`render.final{aspect:"9:16"}`** (or `"1:1"`/`"4:5"`/explicit `width`+`height`,
    defaults `fit:cover`) is the STATIC centre-crop — deterministic, no analysis,
    byte-identical replay. Use it when you want an exact fixed geometry and don't
    need subject tracking. `render.final{dry_run:true}` returns the plan (geometry,
    duration, checks) before a slow encode. Text deliverables: `export.vtt` (web
  captions), `export.chapters` (YouTube/podcast markers — pair with
  `edit.mark_scenes`), `export.transcript` (readable script of the final cut,
  txt/md, for show notes).
- **Convenience cuts.** `edit.split_at_scenes` / `edit.mark_scenes` (auto shot
  detection → cuts or markers), `transcript.search` (phrase → word ranges for
  `cut_words`/`assemble`), `transcript.ignore_words` (non-destructive source-word
  ignore: captions/reels skip it, audio/timing stay intact), `transcript.assemble`
  (non-contiguous highlight reel), `render.storyboard` (contact-sheet
  overview), `media.waveform`.
- **Reviewed caption replacement.** Use `captions.bulk_preview` with one
  caption track, literal `find`/`replace_with`, `match_mode:"contains"` or
  `"whole_word"`, explicit case choice, and optional `range_ms`; inspect its
  opaque hash and rows before `captions.bulk_apply`. Apply supplies only that
  hash plus `request_id` and `expected_revision`: the server keeps the complete
  reviewed target set privately (the UI shows only the first 100 rows),
  revalidates cue id/range/source text, and commits one Undoable update. Do not
  supply targets yourself. Timing stays unchanged unless every affected cue has
  exact transcript-word evidence and `refresh_timing:true` is chosen.

## Overview

ShellX Cut is an agent-first NLE: the **verb API is the primary surface**, the
UI is just another client. Every mutation goes through a verb; every verb
appends an immutable operation record (`ops.jsonl`); renders end in a
**RenderReceipt** with measured checks. Your job as the editing agent:
make verifiable cuts, justify each one, and never claim "done" without a receipt.

Wedge: talking-head / podcast / screen-demo cuts driven by the transcript —
not unrestricted freeform compositing.

## Connect

```bash
cutd serve --project ~/edits/demo.cutproj --headless   # background server, default 127.0.0.1:6161
# port taken / running several instances: --addr 127.0.0.1:<port> (loopback only)
```

Three equivalent channels (same verbs, same JSON args — see reference.md):

| Channel | How |
|---|---|
| REST | `POST http://127.0.0.1:6161/api/verb/{name}` body = args JSON; if the installed app had to use another loopback port, use the URL reported by `engine_status` / `/api/agent` |
| MCP | `cutd mcp` (stdio) — PROXIES the running serve via live discovery when the app is not on 6161; tools generated from `schema/verbs.json`, dots→underscores |
| CLI | `cutd verb <name> '<json>'` — quick tests; uses the same live discovery as MCP |

**Local trust boundary.** The supported default is one personal workstation /
one trusted interactive environment. `cutd` has no API token: loopback is a
machine-wide reachability boundary, not same-user authentication, so any local
process or OS account that can connect can operate the editor. Origin/Host
checks mitigate browser cross-origin and DNS-rebinding requests only; native
callers can omit or forge those headers. MCP is a stdio proxy and inherits the
same boundary. Native LAN/public listening is unsupported and refused by
default; a debug build with `SHELLX_CUT_ALLOW_NON_LOCAL=1` permits non-loopback
binding and skips the browser Origin/Host/Fetch-Metadata guard. Packaged builds
ignore the flag, and Cut does not authenticate a remote caller. Remote use is supported only through an
independently authenticated and authorized SSH/VPN/ShellX broker or equivalent
transport; without it, refuse remote access. Do not use shared/multi-user
machines, untrusted local services, host-network containers, or exposed ports
for this mode. The brokered `agent.chat` routes narrow their Cut tool surface;
they do not change local REST/MCP authentication. Read
`docs/public/shellx-cut-threat-model.md` before altering deployment.

Register that same proxy with the exact packaged executable reported by
`/api/agent` or Settings > Agent control:

- Claude Code: `claude mcp add --scope user shellx-cut -- "/absolute/path/to/cutd" mcp`.
  Claude defaults to local scope; the shown user scope works across projects.
  `claude mcp get shellx-cut` or `claude mcp list` health-checks approved entries.
- Codex: `codex mcp add shellx-cut -- "/absolute/path/to/cutd" mcp`. Codex stores
  it in `~/.codex/config.toml`; `codex mcp get shellx-cut --json` confirms the
  entry but is not by itself a live-handshake claim.
- Grok Build: `grok mcp add --scope user shellx-cut -- "/absolute/path/to/cutd" mcp`.
  Grok defaults to user scope; `grok mcp doctor shellx-cut` checks the command,
  handshake, and tool discovery.
- Antigravity CLI: `agy mcp add shellx-cut /absolute/path/to/cutd mcp`. This is
  a user-level entry in `~/.gemini/config/mcp_config.json`; `agy mcp list`
  confirms configuration and `/mcp` exposes live status. Headless `agy --print` auto-denies permission
  prompts; put only exact unattended grants such as
  `mcp(shellx-cut/system_mcp_test)` under `permissions.allow` in
  `~/.gemini/antigravity-cli/settings.json`. Do not use a global MCP wildcard or
  `--dangerously-skip-permissions` just to test Cut.

For every client, call `system.mcp_test {}` through the configured MCP server as
the final proof of protocol negotiation, ping, all 308 tools, and same-engine
resolution. Client-specific configuration commands never change Cut's verb or
argument contract.

`6161` is the normal fixed port. When another local process already owns it,
`cutd serve` writes the actual loopback address to the engine discovery file;
`cutd mcp` and `cutd verb` use that live address and fall back to 6161 only when
the discovery file is missing or stale.

Fresh installed apps expose their bundled operator docs at `GET /api/agent`.
That discovery response also carries the exact packaged executable and a
copyable MCP client config. `system.mcp_test {}` is a bounded read-only check
of initialize, ping, tools/list, structured output, and same-engine proxy
resolution; Settings > Agent control exposes it without editing client config.
Use its `read_first` links, including
`/api/agent-doc/docs/public/DEBUG_API.md`, for the endpoint and security contract
shipped with that build.

Every verb returns the envelope
`{ok, result?, op_ids?, project_revision?, warnings?[], error?{code, message, clip_id?, at_ms?, cause, suggested_action?}}`.
Errors are actionable — they carry the clip, timecode, cause, and a suggested
next move; read them, don't retry blind. `warnings[]` carries non-fatal
guardrail findings in-band.

For any externally retryable project mutation, generate a unique `request_id`
and pass the latest `project_revision` from `project.state`, `project.ops`, or a
mutating response as `expected_revision`. An identical lost-response retry
returns the original durable response/op IDs; a changed payload or stale
revision conflicts. Never change request IDs merely to get past an ambiguous
commit—inspect the reported durable op IDs first.

The `args` object for every verb is executable JSON Schema Draft 7. It is
compiled once by the live engine and enforced at the shared dispatch boundary,
so REST, CLI proxy, MCP, and nested recipe/plugin calls reject the same invalid
payload. `invalid_args` identifies the exact JSON Pointer and failed keyword;
read `GET /api/verbs` and correct the JSON value/type rather than retrying with
string coercions. Handler semantic checks still run after structural validation.

Live events: WebSocket `GET /api/events` →
`{type: op_applied | job_progress | render_done | receipt_ready | project_changed | ui_state | doctor_updated, ...}`. `op_applied` additionally carries `{revision, from_revision, delta:{kind:"op",count:1}}`; treat events as best effort and pull `project.state{since_revision:last_applied}`. The server returns a bounded applicable delta or an explicit snapshot fallback, so reconnects and missed frames do not require replaying an unbounded log.
`project_changed` means a REST, CLI, MCP, or UI client created, opened, or
closed the active project; visible clients refresh their workspace from it.
After a reconnect, `project.state` returning `no_project` is authoritative
confirmation of that close: discard every saved cursor and in-flight history
page before resetting the workspace. A transport or other transient error is
not a close signal; keep the cached workspace and retry.
`doctor_updated` means the environment capability report changed; refresh
setup guidance instead of polling stale tool/runtime state.
Prefer subscribing to events over tight polling.

## Operational environment

Runtime knobs (env vars on the `cutd` process — not verb args). Sensible
defaults; you rarely set these, but know they exist when a render is slow, a
box is small, or you need byte-reproducible output. Inspect resolved tooling
(ffmpeg path, perception tier, HW-encode, disk) any time with `system.doctor`.
For a `judge.*` card, `details.found` only means its CLI resolved:
`details.judge_ready` is the separate no-model render-review admission result,
and `availability_reason` explains a false result. A missing or malformed
admission stays `status:"unknown"`; do not infer it from `found` or version.
`details.chat` remains the independent Agent Chat capability/session state.

| Env var | Default | Effect |
|---|---|---|
| `SHELLX_CUT_RENDER_MEM_HIGH_PCT` | `75` (clamp 10–95) | Render memory **soft ceiling** as a % of total RAM (Linux). At the ceiling the kernel throttles + spills to swap — the render keeps going, it does NOT die. Lower it on a shared/small box. |
| `SHELLX_CUT_RENDER_NICE` | `10` | CPU niceness applied to every render's ffmpeg (keeps the box responsive during a long encode). |
| `SHELLX_CUT_RENDER_THREADS` | unset (ffmpeg auto = 1/core) | Cap render `-threads` + `-filter_complex_threads` at N. Each thread carries its own frame buffers, so a lower cap shrinks the footprint on a constrained box (trades speed). Render path only — never caps probe/scrub. Default-off keeps per-machine reproducibility. |
| `SHELLX_CUT_RENDER_SEGMENT_SEC` | unset (adaptive) | Override the segmentation gate: a `render.final` longer than this many seconds renders SEGMENTED. Unset = the adaptive gate (segment when overlays exist AND the timeline exceeds one adaptive window, or past a 10-min base-only ceiling). `0` forces segmentation; a large value disables it. |
| `SHELLX_CUT_RENDER_WINDOW_SEC` | unset (adaptive) | Override the segment window size (clamp 2–300 s). Unset = ADAPTIVE: window shrinks with resolution × overlay count so each window's peak RSS stays near the budget (4K with 2 overlays → ~2 s windows ≈ 1.2 GB, instead of 30 s ≈ 17 GB). Also bounds `render.frame{compose}` (composes only the window holding `at_ms`). |
| `SHELLX_CUT_RENDER_WINDOW_BUDGET_MB` | `1500` (clamp 256–8192) | Per-window peak-RSS budget the adaptive window targets. Lower it on a small box; raise it on a big one for fewer, faster passes. |
| `SHELLX_CUT_RENDER_PARALLEL` | `1` (serial) | OPT-IN: render N segment windows concurrently (Linux only; each capped at budget/N via its own cgroup so the box stays bounded). Parallel windows trade substantially more memory for a load- and hardware-dependent speed gain, so serial rendering stays the default. |
| `SHELLX_CUT_RENDER_RAM_PCT` | unset | OPT-IN alternative to `_PARALLEL`: auto-size the concurrent-window count to about this percentage of RAM (Linux + cgroup only). For example, `60` on a 64 GB machine permits about nine windows, with the same memory and load tradeoffs as `_PARALLEL`. |
| `SHELLX_CUT_NO_HWENC` | unset | Set `=1` to force the **software encode** tier (skip NVENC/QSV/AMF/VideoToolbox) — for CI / byte-reproducibility / a flaky GPU encoder. Also disables the GPU render fast-track (it needs NVENC). |
| `SHELLX_CUT_RENDER_GPU` | unset (software) | **EXPERIMENTAL opt-in GPU render fast-track** (`=1`/`true`/`yes`/`on`). NVDEC + `scale_cuda` + `nvenc` keep eligible frames in VRAM, reducing CPU load; speed varies with hardware and system load. GPU output can vary by driver/hardware, so it is OFF by default and the software path stays the receipt-exact baseline. The probe-gated path is limited to a single base video track of hard cuts at matching source aspect on NVIDIA hardware. Overlays/PiP, grade, titles, captions, fades, crop, xfade, or mismatched aspect transparently fall back to software. `RenderOutput.pipeline` records `"gpu"` when used. |
| `SHELLX_CUT_DETECTOR` | CPU-floor | `=high` selects the heavier Faster R-CNN (GPU) for `render.reframe` subject tracking; default is the BSD torchvision SSDlite CPU floor. |

**Render resource governance (Linux).** A heavy/long render cannot wedge the
machine. On Linux+systemd every render's ffmpeg runs inside a transient cgroup-v2
scope with `MemoryHigh` (~75% RAM, throttle+spill under memory pressure)
and a `MemoryMax` backstop (total − 1 GB) that confines any worst-case kill to
the render's OWN cgroup, never `sshd`/the desktop. The philosophy is **work
within resources, finish the job** — soft-limit, never a hard kill that abandons
the render. macOS/Windows have no cgroups (they auto-compress/page), so there the
render is a plain `nice`d spawn. The probe for systemd availability is cached
once per process. This is the safety net; the real memory *bound* is **segmented
rendering**: a heavy `render.final` (overlays + length, or 4K) renders the video
in adaptive time-windows (each window's filtergraph holds only that window's
clips → peak RSS bounded near `WINDOW_BUDGET_MB`, not growing with timeline
length), with a single cheap audio pass muxed on; `render.frame{compose}`
likewise composes only the window holding the frame. Frame-identical to the
whole-graph render (verified by PSNR + receipt parity). HW encode auto-selects the
best working GPU encoder (probe-gated, so a listed-but-broken encoder can never
produce a bad render) and falls back to software.

## Workflow

### 1. Project

`project.create {name}` or `project.open {path}`. For the self-contained guided
sample, use `project.create {name, starter:"first-edit"}` and pass the returned
`starter_asset_path` through the normal `media.import` path. `project.state {}`
returns the full materialized timeline (assets, tracks, markers, checkpoints) plus
path-free `project_identity:{schema:"shellx-cut/project-identity/1",origin_path_sha256,project_name}`;
the canonical project path remains server-local. Drop a
checkpoint before any editing pass: `project.checkpoint {name:"pre-edit"}`.

An active native capture pins its owner project until native teardown releases its
reservation. While the currently open project is that capture's owner,
`project.create`, `project.open`, and `project.close` return `conflict`;
`project.delete` may remove only a different closed project. An unbound native
reservation fails closed for every project transition. Do not use a UI state change
as proof that the capture has released ownership.

In the desktop UI, Projects is the initial workspace. Dropping video, audio, or
an image while no project is open creates a sensibly named project, imports the
media, and places it on the first timeline. A fresh project keeps an internal
1920x1080@30 fallback, but its first video adopts the source geometry and frame
rate while that format is still untouched. Treat `project.format` as timeline
composition timing/canvas, not an export-quality picker; use per-render
geometry/aspect/codec/bitrate for delivery variants.

When a project has a speed curve, changing `project.format` FPS deliberately
regrids that curve to the new frame/sample grid. A lower FPS may use fewer safe
render slices, but its bounded requested detail is retained and returns when a
later format permits it; old projects with no frame-aware ramp timebase retain
their historical millisecond behavior.

For multi-sequence projects, use `project.sequence_index {query?, asset?, kind?,
sequence?, track_kind?, status?, limit?}` to search clips and markers across
active and inactive timelines without switching through them. `status` can
isolate `issues`, `offline`, `gaps`, `effects`, `hidden`, `locked`, or `muted`;
offline is checked live, and issue rows never reveal source paths. Results carry
stable sequence/track/item ids, laid timeline ranges, effect names,
and track state. `at_ms` uses the same post-crossfade layout as the renderer and
`ui.playhead`; `asset` is an exact media-id filter applied before `limit` for
complete bounded per-asset occurrence navigation.
In the app, Find → Sequence exposes the same filters, copies the currently shown
bounded rows as spreadsheet-safe CSV, and opens a result by switching sequence
when needed and moving the playhead to `at_ms`.

**Deleting things:** `project.forget {id|path}` only drops the recent-index
entry; `project.delete {id|path}` PERMANENTLY removes the `.cutproj` directory on disk +
forgets it (guardrailed: only a `*.cutproj` dir, never the open project). To remove a
single imported file from the open project, `media.remove {asset}` — the inverse of
`media.import`: drops the asset + its regenerable proxy/thumbnails (the SOURCE file is
kept), replay-safe, and refuses while any timeline clip still uses it (delete those clips
first — that delete is undoable; the asset removal is not).

### Offline-media bulk recovery (B5)

For several missing sources, use `media.relink_preview {root}` first and treat
its opaque `plan_hash` as a fresh, bounded evidence set. The preview refuses
symlink traversal and only makes a row selectable when one candidate's **full
SHA-256** exactly equals the asset's stored full identity. A unique candidate
with exact basename, kind, stored byte size, and duration (audio/video) or
dimensions (still) may be marked `metadata_review`; it is a disabled “Possible
replacement — review individually” hint that directs the person to the normal
one-file Relink flow. Available dimensions, container, or codec disagreement,
missing stored probe/size, and tied private rankings refuse that hint. Duplicate
exact candidates and sampled/missing source hashes remain diagnostics, never
authority to relink. Preview diagnostics contain safe matched-fact labels only,
not candidate paths, roots, raw probes, timestamps, ranks, or metadata values.

Apply only selected eligible rows with `media.relink_apply {root, plan_hash,
accept, request_id, expected_revision}`. It re-scans/re-hashes and writes one
replayable project metadata op plus an immutable request receipt. This has no
Library transaction, import/proxy/enrichment job, derived-state cleanup, or
Ctrl-Z promise. Preserve the `shellx-cut/media-relink-receipt/1` result for B6:
it carries the project identity, revisions, plan hash, grouped op id, and each
asset's expected hash/chosen path/disposition.

### Portable package (B6)

Do not use Library “Keep a copy” or loop `media.relink` to make a package. A
person can use Projects → Make a copy: it uses a native destination-folder
picker, shows the full-SHA-256 preview, and requires an explicit create
confirmation. For direct API work, first call
`project.package_plan {destination,name,b5_receipt?}`. The normal Projects UI
reconstructs the current receipt from the durable journal after reopen and
passes it automatically when the current revision is the grouped B5 operation;
any later revision clears it. Direct clients must preserve the exact current
receipt themselves. The plan returns the exact
`plan_hash`, byte totals, dedupe plan, current `target_status`, and B5 receipt
digest. Pass the B5 receipt only when recovery repaired media; it must be the
immutable result from the matching source revision. A plan refuses
offline/symlinked sources, unsafe destinations, stale B5 evidence, and
unsupported Motion-linked provenance.

Then call `project.package_create {destination,name,plan_hash,b5_receipt?}` and
wait on its `portable_package` job. It creates a new Cut-native `.cutproj` in a
private same-parent stage, copies only referenced bytes, clears cache pointers,
verifies a `package.manifest.json`, and publishes without replacement through
the native Linux, macOS, or Windows no-clobber primitive. It does not append a
source project op or change source paths/assets/media; unknown targets fail
closed rather than substituting a weaker publish.

### Project cache lifecycle

Use `project.cache_rebuild {asset_ids?,estimate_only?}` only to backfill missing or stale
Cut-owned base proxies/filmstrips for registered current-source assets. Omit
`asset_ids` only when the project has at most 64 assets; otherwise select at
most 64 unique ids. Its uniform path-free result reports queued asset/output,
up-to-date, and items-needing-attention counts for `estimated`, `queued`,
`already_queued`, and `not_needed`. Use `estimate_only:true` for the same
source-identity admission and verified work-unit/byte count without reserving
outputs or creating a job; it is not a guessed duration.
Poll a queued `cache_rebuild` through `jobs.status` and use `jobs.cancel` when
requested. It verifies source identity before publication, reserves ownership
durably before output, never adopts legacy/unowned files, and leaves unfinished
pending reservations resumable after cancellation or restart. It never changes
source media, exports, recordings, captures, receipts, or foreign files.

Use `project.cache_preview {}` before any cleanup claim. It returns a path-free,
one-use plan only for aged, unreferenced proxy and filmstrip files that still
match Cut's durable ownership ledger. A legacy, unowned, foreign, symlinked,
partial, changed, or actively produced cache blocks the plan; source media,
exports, recordings, receipts, and arbitrary files are never candidates.

Only after the user confirms that exact preview, call
`project.cache_purge {plan_id,confirm:true}`. Poll its cancellable job through
`jobs.status` and use `jobs.cancel` when requested. Its terminal result reports
path-free `before`, `planned`, `removed`, and `after` file/byte totals plus
whether they balance, whether partial progress occurred, whether `after` came
from a strict scan or the exclusive-lease delta, and whether ledger recovery is
required. Run a new preview after it finishes before another cleanup. Never
infer deletion authority from directory size, file age alone, or
`project.health`.

### 2. Import and wait for perception

`media.import {path}` returns `{asset_id, job_id, enrich_job}` — it registers the
asset (one op) and kicks the import job **probe → proxy → filmstrip →
ready-to-edit** (fast). Transcribe + perception then run as a SEPARATE background
**enrich** job (`enrich_job`) so slow transcription on long footage NEVER blocks
editing (and a missing/failed sidecar degrades to a warning, not a failed
import). Transcript-driven verbs (captions/cut_words/remove_fillers) need the
ENRICH job finished — wait on `enrich_job`. Wait by either:

- polling `jobs.status {job_id}` (every 2–5 s, not a hot loop) until
  `state:"done"`, or
- watching WS `job_progress` events for that job_id.

`media.import` is intentionally project-local. Do not assume it populates the
cross-project Library: generated and pipeline-internal imports use this verb too.
When the user wants the source reusable across projects, follow the successful
import with `library.add {asset:<asset_id>, source:"agent"}`. The human Assets
Import action performs that mirror explicitly and reports if the Library step
fails.

`library.list` is paged (100 by default; follow `next_offset`). For a linked
Library item whose original moved, call `library.relink {id,path}` only with the
same media bytes at the new path. A `conflict` means the file is different:
preserve the old item and use `library.add` to create a new identity.

**First import auto-places**: on an empty timeline the chain places a probed
muxed video as one system-actor `edit.insert_linked` op on v1/a1t; audio-only,
video-without-audio, and still placement retain `edit.insert`. Later imports
(b-roll) are NOT placed; add them with the matching explicit insert verb.
For human UI placement, Assets **Insert** and normal timeline drops target the
base story timeline with ripple. Use an explicit overlay path only when the clip
should sit above the base picture: Alt-drop/new overlay lane, drop on an
existing overlay lane, or `edit.add_track {kind:"video"}` followed by
`edit.insert {track:<overlay>, ripple:false}`. For a probed muxed video,
`edit.insert_linked` needs exactly one video strategy (`video_track` or
`create_video_track:true`) and one audio strategy (`audio_track` or
`create_audio_track:true`). It validates both legs and commits both clips and
any requested tracks together; an error leaves no partial placement. Its shared
`src_range_ms` keeps the source clocks aligned, and its single ripple opens time
only once.

**Overwrite is its own edit, never `edit.insert {ripple:false}`.** Use
`edit.overwrite {asset, at_ms, video_track?, audio_track?, src_range_ms?}` to
replace the fixed source-duration interval without moving downstream material.
Choose both destinations for one atomic linked A/V overwrite, or only one for
an intentional video-only/audio-only edit. Source Monitor In/Out may be sent
as `source_in_ms` + `source_out_ms` instead of `src_range_ms`; every overlapping
clip/gap on the chosen tracks is consumed, partial clips are boundary-trimmed,
and a short tail is gap-padded/extended. Captions, markers, duck windows, and
unselected tracks stay at their existing timeline times. For stills, use
`duration_ms` on a video destination. The receipt names each overwritten track,
its new clip, consumed clips/gaps, and tail extension.
`at_ms` is the cumulative **editorial** coordinate, not the shorter rendered
playhead after a crossfade. Convert a visible position through the selected
track layout first. A linked V+A overwrite is valid only when both targets map
that visible point to the same editorial position; otherwise target one track
or align their transitions.
An overwrite that would shorten or remove a live right-owned crossfade is
refused before any target changes, because that overlap fixes downstream
rendered positions. Place the edge outside the transition owner, or rebuild
the transition deliberately in a separate edit.
A visible playhead inside the rendered dissolve overlap is multiply covered,
so Source Monitor disables overwrite rather than choosing the left clip's
editorial time and potentially changing pixels before the playhead. Move to a
non-overlapped point first.
To preview an unused timed asset before placement, open its **Source monitor**
from Assets, seek and mark In/Out, then choose **Insert range**. The monitor
prefers a ready editing proxy so large or host-unsupported source codecs stay
auditionable, with an explicit keyboard-accessible Play/Pause control. This
uses one source range for both picture and linked audio at
the current timeline playhead; it does not change Program playback while
auditioning the source. Audio-bearing sources also expose the same cached
`media.waveform` projection used by the timeline, with In/Out/current markers
and pointer or keyboard seeking bound to the Source transport.
An online still image opens in that same monitor as an image preview. Choose a
bounded `duration_ms` on one unlocked video destination for **Overwrite still**
at the live playhead; the still surface has no source In/Out, transport, range
insert, or audio destination.
For UI-only source navigation, choose **Match Frame** from a selected footage
clip or a video track-header menu at the live playhead. It opens Source Monitor
on the exact normal/reverse/freeze source frame. Do not represent a speed-ramp
frame as exact: the action stays disabled until the UI has authoritative ramp
segments; missing/offline sources likewise disclose a relink reason. A
track-header target also stays disabled when multiple video clips cover the
playhead; select one clip directly to disambiguate. Source
Monitor **All uses** queries the existing `project.sequence_index` with the
stable asset id, then uses `project.sequence_switch` and `ui.playhead` for the
human-selected occurrence. It is not a new verb, makes no ramp-exact claim,
and visibly labels a 500-row bound.
The human-only Source Monitor and footage clip menu can also **Reveal in
Project** or **Reveal in Library** for the exact registered asset. They clear
local filters and select that existing UI row/card; no new agent verb exists.
**Reveal Source File** is likewise a desktop UI affordance, not an agent path:
the native shell resolves the live registered asset itself and refuses browser,
missing, offline, or non-file cases without exposing a raw source path.
For unified cited retrieval, inspect `media.intelligence_status` first. If no
index exists or current evidence is stale, `media.intelligence_rebuild` derives
one in a cancellable job from transcript, existing visual indexes, perception,
markers, and metadata already stored by Cut. It never silently starts missing
transcription, perception, or visual indexing. Then call
`media.intelligence_search` with an evidence-kind and sequence scope. Treat each
`EvidenceHit@1` as source-relative cited evidence: keep its source range,
provenance, and live timeline occurrences distinct; stale evidence is excluded.
Creating visual embeddings is separate: `media.index` is an advanced explicit
API operation, not a Find moment control and not an Agent Chat route. It needs
the local perception runtime and SigLIP2 model; ordinary installed mode uses
the optional `torch` and `transformers` extras with `AutoModel`/`AutoProcessor`
resolution for the default model. Cut has no separate SigLIP fetch/install
flow. Do not represent **Prepare search** as triggering that work.
In Find → Moment, **Preview** opens the registered source at its anchor and
**Timeline** jumps only to the nearest real occurrence in the active sequence.
An offline or unused source remains an honest citation without a fabricated
preview or timeline jump. Use `inspect.media` for a bounded, path-light asset
view and `inspect.range` to resolve selected opaque evidence ids against the
current index before reasoning about their ranges or provenance. Selecting hits
and choosing **Ask Agent** adds index-bound citation chips and prefills Agent
Chat; it does not send a turn or apply an edit. When the user sends, Cut
revalidates every citation before provider launch and the agent must call
`inspect.range`. If the index or an authority changed, stop and ask the user to
select current evidence instead of relying on copied prompt text.

For human setup guidance, prefer the visible surfaces over raw diagnostics:
Assets **Media Health** summarizes missing source files, proxy/source playback
state, and large camera/phone clips, with per-asset readiness badges
(`[data-cut-asset-readiness]`), a `[data-cut-asset-attention-filter]` view, and
a direct Relink action. Command search can open and highlight Media Health,
Proxy imports, Video tools setup, and CLI agent setup. If FFmpeg is confirmed
missing, Preview and the Render/Export area show direct setup actions that open
Settings > Video processing, open the manual guide, or re-check after
installation.
Perception/transcription sidecars inherit the same resolved FFmpeg/ffprobe
directory at `AppState` startup and after doctor re-scans, so captions and
analysis jobs reuse the engine's selected/Homebrew/app-data video tools instead
of depending on the sidecar process PATH.
The human Render button and FFmpeg-backed export choices also run
`verify.pregate {}` before starting output. High-risk preflight findings block
the action; lower-risk warnings show a concise warning with collapsible details
and a deliberate Continue button. The warning's Guide action opens the bundled
manual article `cut.export.preflight`; it does not open the preflight UI or
start output. The stable online manual remains available while the compiled
real-frontend manual is staged separately for interactive review.

Results land in the project: word-level timestamps (`receipts/<asset>.words.json`)
and instrument facts — silence, scenes, beats, loudness, and `content_bbox`
(`receipts/<asset>.perception.json`).

**Framing check:** read `content_bbox` from the asset's perception report.
When `uniform_border:true`, the source has a baked-in letterbox/pillarbox (the
capture canvas/window mismatch — black bands in the source pixels). Fix it
once on the clip with `edit.crop {clip, x, y, w, h}` using the bbox's
`{x, y, width, height}` — crop runs in source space before the conform, so the
render fills the frame. The `uniform_border` receipt check fails the render if a
margin survives (it is NOT waived even on the `silent_screen_demo` profile), so
an uncropped screen demo no longer passes silently. Alternative for one render:
`render.final {fit:"cover"}` crop-to-fills instead of editing the source.

### 3. Edit through the transcript

This is the core loop. Read words first: `transcript.get {asset}` → words with
indices (`idx`) and ms spans.

The human Transcript panel groups those same authoritative words into bounded
phrase rows with visible start-end ranges. Activating a row seeks its exact
Clip, reused Program occurrence, or Source position; expanding it reveals the
existing word actions. This is presentation over the verb timestamps, not a
second timing model.

- `transcript.cut_words {asset, word_range, rationale}` — ripple-cuts audio+video
  at word boundaries. The engine never cuts inside a word (pads to word edges
  ±40 ms) — so think in **word indices**, not raw milliseconds.
- `transcript.remove_silences {aggressiveness, min_ms?, padding_ms?}` —
  **`aggressiveness` is REQUIRED** (the API enforces it): `"calm"` (long pauses
  only), `"natural"` (default feel), or `"jumpy"` (tight social-media pacing).
  The preset is part of your editorial intent and belongs in the record.
- `transcript.remove_fillers {lexicon?}` — um/uh runs (default lexicon:
  um, uh, erm, ah, hmm, mhm).

Both removal verbs are timeline-wide by default; `asset`/`track` narrows
*detection* (which spans qualify) — the cut itself still ripples all tracks
so AV stays in sync.
Each removed span = **one operation** in the log, so a human can skim
accept/reject them individually in the Review rail. Raw-timeline verbs
(`edit.split`, `edit.ripple_delete`, `edit.trim`, `edit.move`, `edit.insert`, `edit.overwrite`,
`edit.gain`, `edit.speed` (per-clip retime / slow-mo, pitch-preserved, 0.25–4×),
`edit.grade` (color), `edit.add_marker`, `edit.remove_marker`,
`edit.move_marker`) exist for non-speech work — but if the cut is about *what
was said*, use a transcript verb so the word-boundary guarantee holds.
For `edit.add_marker`, `at_ms` is an absolute project-timeline position, not
an offset from a selected clip or range; honor an exact time in the request.

### 3b. Audio finish (music bed, ducking, crossfades)

- **Music bed:** `media.import {path}` the music, wait for the import chain
  (`jobs.status`), then `audio.add_music {asset}`. By default it drops the bed on
  a dedicated `music1` track at -18 dB and **auto-ducks** it under the speech
  track (-15 dB inside detected speech, computed from perception silences and
  **recorded on the op** — deterministic, auditable; the same windowed-gain model
  as `edit.duck`, NOT a render-time sidechain). It also drops `beat:N` markers
  from the music's beat grid (useful for cut-on-beat later). Tune with
  `bed_gain_db` / `duck:{db, attack_ms, against_track}` / `beat_markers:false`,
  or `duck:false` to skip ducking. Re-run after adding/moving speech (ripples
  remap existing duck windows; new speech needs a fresh pass).
- **Crossfade:** `edit.crossfade {track, at_ms, duration_ms}` dissolves the cut
  between two adjacent clips (video `xfade` / audio `acrossfade`). It is a *seam*
  operation — distinct from `edit.fade` (the at-the-ends ramp). The timeline
  **shortens by `duration_ms`** (the overlap is taken from both clips), and a
  crossfade **clears the boundary's per-clip fades** (it owns the cut). Split the
  clip first if `at_ms` is not already an exact clip boundary.
- **Lift vs ripple:** `edit.ripple_delete {ripple:false}` LEAVES a gap (lift);
  the default closes it (extract). Caption clips reposition via
  `captions.set_range` (not `edit.move`/`edit.trim`).
- **Keep imported A/V aligned:** live `edit.move` and `edit.trim` calls infer one
  exact opposite-kind counterpart and mutate the linked pair atomically by
  default. Pass `linked:false` only for a deliberate independent move/trim;
  ambiguity or a locked counterpart is an error rather than a silent desync.
  In the human UI, Q trims from the playhead to the selected clip's start and W
  trims from the playhead to its end, closing the removed span for both halves.
- **EQ the voice:** `edit.eq {clip, preset:"voice"}` cleans up a talking-head /
  podcast audio clip (low-cut rumble + de-mud + presence lift) — the audio analog
  of `edit.grade`. Presets: `voice` / `warmth` / `de_rumble` / `phone` (telephone
  band-limit) / `de_ess` (tame sibilance) / `brighten` (high-end air); or raw
  `high_pass_hz` / `low_pass_hz` / `bands:[{freq_hz, gain_db, q?}]`. AUDIO-track
  clips only (a video's audio is its own clip on the audio track). `enabled:false`
  clears. Pairs with `edit.effect {effects:[{type:"gate"},{type:"compressor"}]}`
  (noise gate kills room tone between phrases → compressor evens dynamics) for the
  full talking-head/podcast voice chain.

### 3c. Layers / compositing (video-on-video, PiP)

- **Stack video tracks:** the FIRST video track with clips is the base canvas;
  every later video track composites ABOVE it in track order (full-frame by
  default, gaps transparent). `edit.add_track {kind:"video"}` adds an overlay
  layer; `edit.insert` clips onto it with `ripple:false` for normal overlay/B-roll
  placement. Do not create a new video track for every ordinary edit; extra
  video tracks are compositing layers, not sequential story lanes.
- **Place + blend the overlay:** `edit.transform {clip, x, y, scale, opacity}` —
  normalized PiP geometry (x/y top-left fraction, scale = width fraction) plus
  `opacity` 0..1 (blend/ghost; 1 = opaque). Identity `(0,0,1,1)` clears. On the
  base track, the same transform places/scales the picture over black and opacity
  blends against black; a hidden or gapped base remains black rather than
  promoting an overlay.
- **Z-order:** `edit.reorder_track {track, index}` brings a layer forward (higher
  same-kind index) or sends it back. `index` is relative to tracks of that kind,
  never an absolute `project.tracks` index. Audio/caption reorders are render
  no-ops but allowed.
  Human UI exposes this on video track headers through
  `[data-cut-action="track-send-back"]` and
  `[data-cut-action="track-bring-forward"]`, plus the Layer/PiP drawer controls.
- **Animate the overlay (motion):** keyframe the PiP — `edit.keyframe {clip,
  param:"opacity"|"pos_x"|"pos_y"|"scale", points:[{t_ms, value}], interp?}` animates a
  webcam/logo to fade, SLIDE, or ZOOM over time (`pos_x`/`pos_y` are frame fractions,
  unclamped → slide in from / out to off-screen; `scale` = animated zoom, multiplier
  1=native, the multi-point eased generalization of `edit.animate`, mutually exclusive
  with it). `interp` defaults to `linear` but accepts any **Penner `ease_*` curve**
  (`ease_in_out_cubic`, `ease_out_back`, `ease_out_elastic`, `ease_out_bounce`, …) so
  motion reads professional, not mechanical. Un-animated overlays render byte-identically
  to before. The easy
  path: `edit.slide {clip, edge:"left"|"right"|"top"|"bottom", mode:"in"|"out"}` reads
  the resting transform and lowers to the right position keyframes for you.
- UI: the **Layer/PiP drawer** drives all three (position·scale·opacity sliders
  + bring-forward / send-back). Track visibility applies consistently to live
  preview and final output. Locking a track disables its timeline gestures plus
  Layer and Inspector editing until the track is unlocked.

### 3d. Clip volume automation (audio)

- **Shape a selected audio clip, not the track:** its waveform carries a
  **Clip volume** curve. The visible **Add point** action uses the playhead;
  Ctrl/Cmd-click is the accelerator. The curve displays −60…+12 dB while the
  existing `edit.keyframe {clip, param:"volume", points, interp}` contract
  retains its linear multipliers and complete SET semantics. Do not substitute
  `edit.gain` (static clip/track gain) or `edit.duck` (track duck windows).
- **Preserve playback and concurrent edits:** first automation points seed the
  current static clip gain; Clear leaves that static gain untouched. A drag is
  local preview only until pointer-up, which emits one full sorted keyframe
  track/one undoable op. Escape or pointer cancellation emits nothing. Keep
  points between adjacent timestamps, fail closed without a current revision,
  and explain locked-track, no-duration, and speed-ramp refusal. Inspector is
  the accessible exact-value/interpolation alternative. This surface has no
  Record/Arm/Stop or `screen_record` behaviour.

### 4. Review discipline

- **Every op gets a rationale.** Pass `rationale` where the verb accepts it
  (every mutating verb does); the op record's rationale field is what makes
  the edit auditable. "Cut words 114–131" is not a rationale; "filler run
  'um, so, like' breaks the sentence" is.
- Ops are immutable. Ctrl+Z/Ctrl+Shift+Z use `project.undo`/`project.redo`; to
  reject a reviewed operation, append `edit.restore {op_id}`. Never try to
  delete or rewrite history. **Default restore is tip-only** (`mode:"tip"`): it
  recomputes the pre-target journal prefix, so it only undoes the LATEST
  timeline op — reject ops as you review (newest first). To selectively
  undo an OLDER op while keeping the later ones, use
  `edit.restore {op_id, mode:"rebase"}` — it reproduces the timeline as if that
  op never happened (id-pinned skip-replay) and is **refused with a guardrail
  error naming the dependents** if any later op references an id the target
  created (e.g. a `split` whose right-half a later `gain` addresses). Both modes
  APPEND, never rewrite. For a full rollback to a point, use
  `project.revert {to: checkpoint-or-op}`.
- For one existing linked/compound action, first call
  `project.group_preview {op_id}`. Only a `reject.status:"ready"` preview may
  be sent unchanged to `project.group_reject` with a fresh `request_id` and
  that exact `project_revision`. It is tip-only and appends one normal restore;
  one `project.undo` restores the entire group. Do not turn this into a generic
  selected-op replay: newer history is deliberately refused.
- Checkpoint between passes (`project.checkpoint`), and **always run
  `project.diff {from: last_checkpoint, to: "now"}` before rendering** — read the summary (clips
  added/removed/moved, `duration_delta_ms`, `tracks_touched`) and confirm it
  matches what you intended to do. A diff you can't explain means stop and
  inspect `project.ops {since}`.
- **Acting on human review notes.** A reviewer leaves timecoded notes with
  `comment.add {at_ms, text, end_ms?}`; list open ones with
  `comment.list {status:"open"}`. New comments may carry
  `anchor:{track_id,clip_id,offset_ms}`; resolve that against current
  `project.state` if content was rippled, and treat a missing clip as stale
  rather than seeking blindly. In the editor, **Make changes** routes the note
  and `shellx-cut/chat-timeline-target/1` through ordinary `agent.chat`; the
  result applies directly as reversible project operations, with guarded
  **Step back** and **Ask replacement** for recovery. Do not use
  `comment.draft` or `comment.apply` to emulate an editor Draft/Accept stage:
  they are agent-only compatibility and diagnostic endpoints. `comment.resolve
  {comment_id, status}` closes the note (`addressed`/`dismissed`). Comment ops
  are review metadata, not timeline edits — outside the undo stack. For an
  external handoff, render the current cut and run `comment.export {}` to create
  an offline HTML reviewer beside a verified render copy. Import the reviewer's
  downloaded JSON with `comment.import {path}`; Cut verifies its render
  hash/source op and appends the entire feedback batch atomically. Later
  comment/preset/name metadata does not stale unchanged rendered bytes; a later
  render-affecting edit is rejected by default and requires `allow_stale:true`
  plus a recorded `rationale`.

### 5. Render + receipts (the doctrine)

`render.final {path?, preset?}` returns `{job_id, render_id}`. On completion
the server **auto-runs `verify.checks` and emits a RenderReceipt**
(`receipt_ready` event — always after `render_done`).

**You are not done until you have read the receipt.** Job completion is not
success; a green receipt is. `verify.checks {render_id}` returns the receipt:
`{render_id, output_path, output_hash, duration_ms, checks, pass}`, each check
`{name, pass, details, evidence}`:

`jobs.status.completion:"done_with_warnings"` is terminal and may leave optional
post-render instrumentation unmeasured. Inspect `result.verification_status`,
`result.verification_error`, and the actual receipt before judging the media.
For a terminal `state:"failed"`, inspect `outcome` and `outcome_reason` before
calling it a failure: cancellation, a project switch, a restart interruption,
and supersession are distinct from `true_failure`.
A missing/unmeasured check is **not a pass and not a content failure**; affected
rows carry `details.status:"unmeasured"` and `details.measured:false`, preserve
the runtime cause, and never produce `fix_actions`. Verify independently or
repair the instrumentation.

Use `verify.rerun {render_id}` only when you need fresh output-only evidence for
the exact persisted artifact. It returns `{job_id, render_id, output_hash}`;
poll `jobs.status` or cancel with `jobs.cancel`. The terminal result is bound to
the source receipt id, full output hash, footage profile, exact five-check set,
and its own verification-receipt path. It does **not** run `render.final`, alter
the source RenderReceipt, or infer source/caption/word-cut/current-timeline facts.

| Check | What it proves | How to read it |
|---|---|---|
| `cut_on_word` | No EDL boundary lands inside a spoken word (boundaries vs STT word spans) | Any fail names the clip + at_ms — undo the bad op (`edit.restore` if latest, else `project.revert{to}`) + re-cut via transcript verb |
| `lufs` | Integrated loudness + true peak vs target | The measured number is a fact — report it even on pass (e.g. −16.2 LUFS, TP −1.8 dB) |
| `caption_presence` | Captions exist where speech exists, and cue text is sane (`repeated_word_ratio` catches doubled-word generation) | Fail → re-run `captions.generate` or inspect the caption track |
| `black_or_frozen_frames` | No dead video in the output | Evidence points at timecodes — eyeball with `render.frame {at_ms}` |
| `uniform_border` | No baked-in letterbox/pillarbox beyond tolerance | Fix the source with `edit.crop` to its perception `content_bbox`, or use deliberate `fit:"cover"` |
| `silence_at_edges` | Output doesn't start/end on dead air | Usually a missed trim at timeline edges |
| `duration_matches_edl` | Output duration == EDL math | Mismatch = engine/render bug, report it, don't ship |

`verify.judge {render_id?, backend?}` is the perceptual visual review — a JOB
(returns `{job_id}`). Its backend union remains Claude → Codex → Antigravity →
Grok, but detection is not admission. In v0.6.114, only Claude with restricted
Read capability at version 2.1.248 or later plus a safe copied-frame workspace,
and Grok at version 1.0.21 or later with its no-model-tools policy, are
`judge_ready`. Codex and Antigravity may be found but an explicit render-judge
request returns `{status:"not_run", reason:"render judge unavailable until
restricted tool/file access is verified"}` before review work; this does not
alter Agent Chat or generation availability. Auto keeps that configured order,
skips unready rungs, and may continue after an attempted ready-rung
infrastructure error. A named backend never falls back. `not_run` means *not
reviewed*, never *passed*; report its recorded reason and never fabricate
evidence. Admission does not prove a live provider turn or native behavior.

### 6. Export

- Default file-writing exports use the configured export folder
  (`project.set_output_dir`) when present, otherwise `<project>/exports/`.
  When a default target already exists, Cut writes the next available sibling
  name such as `recording-2.mp4`; explicit `path` / Save As values stay exact,
  remain fenced, and may overwrite existing export media/sidecar files.
- In the UI, the status-bar `export folder:` chip shows the current destination
  and opens Settings at the folder row. The topbar Settings button and the
  Record tab's Default folder button open the same setting; per-export Save As
  stays in export controls.
- **Captions / interchange:** `export.srt {path?}` (the ONLY SRT exporter),
  `export.vtt {path?}` (WebVTT for HTML5 `<track>`), `export.xml
  {format:"fcpxml"|"premiere"|"resolve", path?}` for handoff to a traditional
  NLE, and `export.otio {path?}` for OpenTimelineIO. Before replacing a timeline,
  call `import.otio {path, mode:"preview"}` and present its track/media summary;
  pass the returned `source_hash` to `mode:"replace"` so changed bytes conflict.
  Replacement is one undoable op, preserves project format, and represents
  unavailable media as timed gaps. `export.chapters {path?}` (markers → YouTube/podcast chapter list — pair
  with `edit.mark_scenes`), `export.transcript {format?, timestamps?, path?}`
  (readable script of the final cut for show notes).
- **Extract to Assets (reusable media out of THIS project):**
  `export.frame {at_ms, to_asset? = true, path?}` saves the composed full-res frame at
  `at_ms` as a JPEG AND (default) imports it as a new image asset — the
  "save one specific frame as an image" path (`render.frame` is the fast,
  non-saving scrub view). `export.range {range_ms:[start,end), to_asset? = true}`
  renders a timeline window (all effects baked) to an MP4 and imports it as a new
  asset — the "cut out / save a section as its own clip" path; the project
  timeline is untouched. It renders to a hidden sibling temp file first and only
  publishes the final MP4 after ffmpeg and probe succeed, so failed attempts do
  not leave a broken final export path. `export.audio {format?, to_asset?}` exports the mixed
  audio only (mp3/m4a/aac/wav/flac/opus — same audio graph as `render.final`, no
  video cost). **The Preview's live audio monitor (🔊 toggle) reuses
  `export.audio` under the hood** — it renders the mix lazily (keyed by the head
  op id) and plays it through a hidden `<audio>` synced to the playhead, so "press
  play → hear the mix" is WYSIWYG with the export and `export.gif {range_ms?, fps?, width?, to_asset?}` exports a
  short window as a looping GIF (palettegen/paletteuse, 30s hard-cap). All land
  the result in the Assets tray (draggable / insertable) and on disk
  (importable into another project).

- **Choose the destination folder:** `project.set_output_dir {dir?}` picks the
  folder where default-named exports + renders land when a verb has no explicit
  `path` (UI: Export ▾ → **Choose folder…**, native OS picker). The folder must
  already exist; it is canonicalized and becomes an allowed output-fence root.
  Empty `dir` clears it (back to `<project>/exports`). A SESSION preference —
  NOT a timeline op, never logged/replayed.

Exports are derived artifacts — render receipts stay the proof of the edit
itself. Explicit `path` args are fenced to the project / outputs dir.

### 7. Seeing the app (UI verification)

**The monitor preview is a LIVE composite.** The human's monitor
composites the timeline in real time: overlay video tracks render as stacked
PiP layers at their `edit.transform` geometry + opacity, caption clips as live
text, and per-clip `edit.grade` as an approximate CSS filter — all over the
stable first non-empty base track, played smoothly (the base `<video>` is the
master clock; overlays sync to it). A hidden or gapped base stays black; hidden
overlay and caption tracks are omitted. It is deliberately APPROXIMATE (grade
is CSS-approximated; gamma + 3D LUT
are not shown live) — exact verification stays `render.frame {compose:true}` and
`render.final`. The agent's frame-exact eyes are still `render.frame`; the live
composite is the human's editing feedback. A **"◆ Section"** monitor button (or
`export.range {range_ms, to_asset:false}`) renders the EXACT composite over the
selected span (or a playhead window) and plays it back with full audio — the
"check exactly how the final looks for this part" path, and the SHORTS/HIGHLIGHT
exporter (add a layer over a span → render it → save the clip).

- `render.frame {at_ms}` — JPEG of the timeline frame: your eyes on the
  *content*, no UI needed (add `inline: true` for base64 over MCP). **Fast
  scrub:** by default this serves a low-latency proxy-seek frame, scaled to height `h`
  (default 540; maximum 2160 and a 4K-UHD derived-pixel budget). It omits captions/overlays. When you need the EXACT composed
  frame for verification (captions burned in, PiP composited, project geometry),
  pass `compose: true`. Raw bytes also at `GET /api/frame?at_ms=[&h=][&compose=1]`
  (the `X-Cut-Frame-Fast` header says which path served it).
- **Preview → Compare** — a human-only, read-only review control that pauses
  playback and asks `render.compare {at_ms, revision}` for the exact composed
  current durable-head frame beside the state before its latest timeline-mutating edit.
  It never calls Undo or adds an operation. It refuses instead of guessing when
  no earlier timeline state, compatible sequence format, or frame exists; the pair
  also closes if the playhead or durable head changes while it is open.
- `render.preview {draft: true}` — **Incremental draft preview:** a fast
  proxy-grade preview of the WHOLE timeline that re-renders only the segments
  whose inputs changed since the last preview and reuses the rest. The
  result names `rendered[]` / `reused[]`. Works on timelines with intro/outro
  **still-image cards** (regression behavior): a still has no proxy (its import stops after
  probe), so it is conformed straight from the source image (looped for the clip
  duration) instead of requiring one — a card no longer blocks the whole draft
  preview. DERIVED state — never a receipt; the render receipt still comes only
  from `render.final`. Window mode remains available as
  `render.preview {at_ms, duration_ms?}` for a fast low-res clip.)
- `ui.screenshot {}` — the connected UI client captures its own DOM and returns
  a PNG: your eyes on the *app*. Errors `no_ui_client` if no UI is connected
  (headless is fine — call `system.doctor {}` for the live loopback address and
  open that URL when you need the UI).
- `ui.state {}`, `ui.open {panel}`, `ui.playhead {at_ms}`, `ui.select {clip_ids}`
  let you drive the human's view — e.g. park the playhead on the cut you want
  them to review. `ui.open` includes the editor grid, Record and Library
  workspaces, left/right/Review tabs, Comments, every Settings destination,
  Find/Generate subtabs, and editing drawers. Read its exact enum from
  `GET /api/verbs`. These commands are CONFIRMED: `ok:true` means the exact UI
  client committed a later observable state revision and, for `ui.open`, the
  registered selector exists. Unknown/unavailable targets, missing selections,
  already-current no-ops, disconnects, and timeouts fail explicitly.
  `ui.state` returns `shellx-cut/ui-state/2` with active workspace/tabs,
  overlays/dialogs, available surface ids, selection, playhead, and path-safe
  project identity.
- `ui.highlight {selector|clip|panel|clear:true}` confirms that a registered
  target is visibly highlighted. Give the human a stable selector, clip, or
  panel; `duration_ms:0` leaves the dismissible overlay open for guidance.
- `debug.screenshot {inline?, monitor?, window?}` is the server-side visual
  verification path when no connected UI client is available. It captures the
  actual display/window through the compiled native recorder; use the opaque
  window id from `screen_record.doctor`, never a window title.

## Worked example — clean a talking-head take

REST shown; MCP tool calls take the same args. `$V` below uses the default
server URL; substitute the installed app's actual loopback URL if it reports a
fallback port.
Result excerpts below match the documented server response shapes.

```bash
cutd serve --headless &
V=http://127.0.0.1:6161/api/verb

curl -s $V/project.create -d '{"name":"launch"}'
# {"ok":true,"result":{"path":"…/launch.cutproj","project":{…}}}

curl -s $V/media.import -d '{"path":"/home/example/footage/take3.mp4","rationale":"raw take"}'
# {"ok":true,"result":{"asset_id":"a1","job_id":"job_001","op":{…}},"op_ids":["op_000002"]}

curl -s $V/jobs.status -d '{"job_id":"job_001"}'   # poll 2-5s until state=done
# {"ok":true,"result":{"job_id":"job_001","kind":"import_chain","state":"done","progress":1.0}}

curl -s $V/project.checkpoint -d '{"name":"pre-edit"}'
# {"ok":true,"result":{"checkpoint":{"id":"cp_001","name":"pre-edit","at_op":"op_000004","ts":"…"}},"op_ids":["op_000005"]}

curl -s $V/transcript.get -d '{"asset":"a1"}'
# {"ok":true,"result":{"asset":"a1","model":"parakeet-tdt/nemo-parakeet-tdt-0.6b-v3@onnx",
#   "words":[{"idx":0,"word":"Hey","start_ms":120,"end_ms":310}, …]}}

curl -s $V/transcript.remove_silences -d '{"aggressiveness":"natural","rationale":"tighten pauses for YT pacing"}'
# {"ok":true,"result":{"spans_removed":3,"total_removed_ms":9400},
#  "op_ids":["op_000006","op_000007","op_000008"]}   # one op per removed span

curl -s $V/transcript.remove_fillers -d '{"rationale":"um/uh cleanup"}'
# {"ok":true,"result":{"fillers_removed":2,"total_removed_ms":1100},"op_ids":["op_000009","op_000010"]}

curl -s $V/transcript.cut_words -d '{"asset":"a1","word_range":[114,131],
  "rationale":"false start — speaker restarts the pricing sentence at word 132"}'
# {"ok":true,"result":{"removed_ms":3800,"word_range":[114,131],"text":"so the price …"},
#  "op_ids":["op_000011"]}

curl -s $V/project.checkpoint -d '{"name":"rough-cut"}'
curl -s $V/project.diff -d '{"from":"pre-edit","to":"rough-cut"}'
# {"ok":true,"result":{"from_op":"op_000005","to_op":"op_000012","ops":[…],
#   "duration_delta_ms":-14300,"tracks_touched":[…]}}
# ← read this. 14.3s removed across 6 ops matches intent → proceed.

curl -s $V/captions.generate -d '{"style_ref":"brand1","rationale":"YT captions"}'
# {"ok":true,"result":{"track_id":"cap1","caption_count":42,"op":{…}},"op_ids":["op_000013"]}

curl -s $V/render.final -d '{"rationale":"v1 render for review"}'
# {"ok":true,"result":{"job_id":"job_002","render_id":"render_001"}}
# wait for receipt_ready on WS (or poll jobs.status job_002, then verify.checks)

curl -s $V/verify.checks -d '{"render_id":"render_001"}'
# {"ok":true,"result":{"render_id":"render_001","output_path":"exports/render_001.mp4",
#   "output_hash":"sha256:…","duration_ms":46800,"pass":true,"checks":[
#   {"name":"cut_on_word","pass":true,"details":"31 boundaries, min word-edge distance 46ms","evidence":"…"},
#   {"name":"lufs","pass":true,"details":"integrated -16.2 LUFS (target -16 ±2 LU), true peak -1.8 dBTP","evidence":"…"},
#   {"name":"caption_presence","pass":true,"details":"speech 0–61s, captions cover 98.7%","evidence":"…"},
#   {"name":"black_or_frozen_frames","pass":true,"details":"none","evidence":"…"},
#   {"name":"uniform_border","pass":true,"details":"max inset 0px (<=8px tol)","evidence":"…"},  # no baked-in letterbox
#   {"name":"silence_at_edges","pass":true,"details":"lead-in 180ms, tail 240ms","evidence":"…"},
#   {"name":"duration_matches_edl","pass":true,"details":"46.80s == 46.80s","evidence":"…"}]}}

curl -s $V/verify.judge -d '{"render_id":"render_001"}'
# {"ok":true,"result":{"job_id":"job_003"}}   → jobs.status until done, then read result:
# {"status":"not_run","reason":"no supported judge CLI/runtime available — …"}  ← NOT a pass

curl -s $V/export.srt -d '{}'                       # {"path":"…/exports/captions.srt","caption_count":42}
curl -s $V/export.xml -d '{"format":"fcpxml"}'      # {"path":"…/exports/timeline.fcpxml","format":"fcpxml"}
```

Honest completion report: *"Rendered render_001.mp4 — all 7 receipt checks PASS
(−16.2 LUFS, 14.3 s removed across 6 ops, diff reviewed); judge review not_run
(no supported CLI/runtime available). SRT + FCPXML exported."*

## Anti-patterns

- **Bypassing verbs.** Never edit `project.json`/`ops.jsonl` by hand, never run
  ffmpeg on project media yourself. Out-of-band changes break the op log, which
  breaks diff, restore, and every receipt check. If a verb is missing, that's
  API feedback — file it, don't work around it.
- **Claiming success without a RenderReceipt.** "Render job finished" or
  "command returned ok" is not done. Done = receipt read, checks reported with
  their measured values, failures either fixed or explicitly surfaced.
- **Treating the judge stub as a pass.** `status:"not_run"` means unreviewed.
  Say so.
- **Skipping rationale.** An op without a why is unreviewable; the human's
  accept/reject rail runs on your rationales.
- **Cutting speech with raw-ms verbs.** `edit.ripple_delete` at hand-picked
  milliseconds bypasses the word-boundary guarantee and is what `cut_on_word`
  failures are made of. Speech cuts go through `transcript.*`.
- **Rendering without reading `project.diff`.** The diff is your pre-flight; an
  unexplained delta means a wrong op is in the log.
- **Undo by rewriting.** Ops are immutable — use `edit.restore {op_id}` for
  the latest op (default `mode:"tip"`), `edit.restore {op_id, mode:"rebase"}`
  to selectively undo an OLDER independent op while keeping the later ones
  (refused, naming the dependents, if a later op depends on it), or
  `project.revert` to a checkpoint. Never attempt to remove log entries.
- **Hot-loop polling.** Poll `jobs.status` at 2–5 s or subscribe to
  `/api/events`; don't hammer the API.

## Reference

Full verb table (args, returns, op emission, job behavior): see `reference.md`.
