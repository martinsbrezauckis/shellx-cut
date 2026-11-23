---
name: shellx-cut
description: Use when editing video with ShellX Cut or its cutd server — video edit, cut video, trim footage, remove pauses, remove filler words, transcript editing, text-based video editing, captions/SRT, talking-head or screen-demo cleanup, render receipt, verify a render, export FCPXML, shellx cut, cutd. Covers driving the verb API (REST + MCP) headless and verifying results with receipts.
---

# ShellX Cut — agent-first video editing

When speaking to a person, use the person-facing names below. Keep verb names,
argument names and stored values exact in calls and code.

Still-picture admission (`media.import`, Library import, `assets.fetch`, generated
media import, `import.otio` replace, `media.relink`, `media.relink_apply`) includes
one non-looped decode under a ten-second limit. A damaged picture is refused
before registration/repath with `unreadable_media`, its filename, `cause` and
`suggested_action`: obtain a fresh copy and import it again. `media.probe` also
checks a still's decode before storing its probe. Existing projects
still open. `media.check` preserves `exists` as file existence and adds
`readable`; false also covers undecodable stills, included in `offline_count`.
The preview uses its existing offline-source treatment for those pictures.
When `readable` is absent, the picture is not checked yet; ask again later.

| What a person calls it | What the verb is called |
|---|---|
| Timeline contact sheet | `render.storyboard`; shot planning remains `generate.storyboard` |
| Platform exports | `export.publish`; saves a local file without uploading |
| Freeze at first frame | `edit.freeze` with omitted `at_ms` (defaults to `0`); audio continues |
| Make a copy | `project.package_plan`, then `project.package_create`; includes used media |
| Remove pauses | `transcript.remove_silences` |
| Remove filler words | `transcript.remove_fillers` |
| Pause removal: Light / Standard / Strong | `aggressiveness`: `calm` / `natural` / `jumpy`; minimum pause 1200 / 700 / 350 ms, keeping 200 / 120 / 60 ms at each end |
| Content type for quality checks | `render.final` / `export.publish` argument `profile`: `talking_head` or `silent_screen_demo` |
| Trim silent start and end | `edit.trim_edges` |
| Find highlights | `clip.candidates`; UI panel `clips` |
| Render history & checks | UI panel `receipts`; `verify.checks` reads render checks |
| Original transcript for each imported file | `transcript.get`; Source view |
| Speakers and dubbing | `media.diarize`, `audio.dub` |
| Start, length & pause | `screen_record.start`, `screen_record.pause`, `screen_record.resume` |
| Live scene controls / live timer controls | `screen_record.scene_activate`, `screen_record.scene_timer` |
| Export review availability | `system.doctor` field `details.judge_ready`; review uses `verify.judge`; Agent Chat remains separate |
| Check setup | `system.doctor`; Settings (Re-scan button) |
| Captions and transcription | `system.setup_perception`, `media.transcribe`; broader media analysis remains `media.perception` |
| Edit history | `project.ops`; UI panel `review` |
| Compare edits | `project.diff`; UI panel `diff` |
| Quality checks | `verify.checks`; UI panel `qc` |
| Current edit / Earlier edit | `project.diff` arguments `from` / `to` use operation IDs or named checkpoints |
| Undo edit | `edit.restore`, `project.undo`; do not change recorded operation names |
| Color / Transform / Remove background | `edit.grade` / `edit.transform` / `edit.matte`; UI panels `grade` / `layer` / `matte` |
| Highlight reel mode | `transcript.assemble`; collect moments before assembling |
| Apply fixes | `autopilot.run` policy `auto_low_risk`; existing review restrictions still apply |
| Prompt | `generate.from_prompt`; UI panel `generate-prompt` |
| Copy / Export recording | `screen_record.copy_raw` / `screen_record.export`; check Jobs before retrying an unconfirmed start |
| Edit action / No note | Operation `verb` / `rationale`; technical identifiers remain available |
| Soft Light / Hard Light | `edit.blend` mode values `softlight` / `hardlight` |
| Quick prompt / Director brief / Existing media | `generate.storyboard` mode values `quick_prompt` / `director_brief` / `existing_media` |
| Slide up / Slide down / Slide left / Slide right | `shape.update` animation values `slide_up` / `slide_down` / `slide_left` / `slide_right` |

**Downloading or installing a tool.** Before you download or install a tool,
tell the person what it is and under which licence, and ask whether you may
download it and accept the licence; if the person asked for the download
themselves ("download FFmpeg", "set up captions"), that is the go-ahead: do it,
and say in your reply which licence applied. Four entries of `system.doctor`'s
`requirements.tools` name a licence (`license`): FFmpeg and ffprobe (GNU GPL
version 3, with a link), Microsoft's Visual C++ runtime on Windows (Microsoft's
own licence terms, linked on its card; its installer runs with progress only and
Windows asks for administrator permission), and premium background removal
(non-commercial use only; `accept_noncommercial:true` states the person's yes).
Example: "FFmpeg is missing. It is free software under the GNU GPL version 3
(https://www.gnu.org/licenses/gpl-3.0.html). May I download it and accept that
licence?"; on yes, `system.fetch_tool {"tool":"ffmpeg","accept_license":true}`.

<!-- shellx-cut-release-truth: published; version=0.6.121; published=0.6.121 -->
> **Engine v0.6.121 published.** v0.6.121 is the latest published release.
> Synced to the contract (`schema/verbs.json` — the single
> machine-readable source of truth; if this guide and that file disagree, trust
> the file): **326 verbs across 34 domains** under the public verb contract.
> **`reference.md` is the full 326-verb table —
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
>   One run takes up to 2 hours of audio: a longer asset (or one with no
>   readable audio) is refused `invalid_args` with the reason; tell the person to
>   trim or split it.
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
>   informational only and each route's required containment flags are verified before every turn. Codex uses the selected account's configuration, native
>   sandbox, and permissions; an enrolled alternate account home reaches only the
>   Codex child through `CODEX_HOME`. Cut adds its filtered MCP server without copying or
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
>   Review > Quality checks rather than claiming Chat can perform that denied work.
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
>   `recovery_status` / `active` / `pending` / `pending_dismiss` / `recover` /
>   `studio_event` / `autoedit` / `polish` / `export` (live screen/audio
>   capture, raw streams, auto-edit plan, content-addressed bake). Use the
>   returned `plan` path for polish/export. Each take or changed auto-edit result
>   keeps its own plan; do not predict a shared cache filename. On Windows and
>   macOS native-camera builds, Doctor can advertise opaque camera
>   choices for an explicit Auto-edit recording. The admitted camera is
>   finalized as a separate retained take with shared-clock evidence. Permission,
>   busy-device, and no-frame failures never
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
>   prompt-deferred XDG ScreenCast portal card may enter the user-initiated
>   source picker; optional unavailable `wayland_input` permits a plain take.
>   `keys:true` still requires readable input devices; turn keys off or follow
>   the reported command and log out and back in. Other required failures
>   still refuse, and `ready` retains the missing-input fact.
>   Doctor reports `system_audio` separately as an optional passive card. A
>   compiled backend remains `unknown` until a real user-started recording
>   proves packets; Doctor opens no loopback/tap stream and cannot trigger the
>   separate macOS Audio Capture prompt. This card does not gate screen-only
>   recording.
>   `input_hook` on Status/Stop reports per-take startup registration only.
>   An observed optional startup failure warns that input-driven effects may
>   be absent, while saved video remains usable (including its baked native
>   Window cursor). A Wayland `unavailable` observation means no evdev devices
>   opened: click, scroll and key data are absent, while PipeWire cursor
>   metadata can remain available. Unknown/legacy observations are not failures.
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
>   app has focus when the OS admits a global callback. Use the current Record
>   choices even after switching to Edit; use saved setup only before Record
>   has supplied a draft. Invalid current choices never fall back to an older
>   take. With no saved setup, first-use F9 checks the
>   current primary display or Linux portal and starts a screen-only polished
>   take with audio, camera, and keystrokes off. Start uses the open project when one exists, or
>   creates a new recording project when none is open; it then rechecks the
>   exact source, devices, and permissions before capture. Global F9 does not
>   restore a minimized Cut window. If setup is stale, report the visible reason
>   without changing workspace and do not claim capture. Live Pause/Resume
>   acknowledgements survive Edit/Record navigation. A passive recording indicator follows the
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
>   When `screen_record.start{audio:true}` asks for microphone sound, a
>   passive microphone readiness problem refuses Start before capture begins.
>   This read does not open or test the microphone. Linux blocks only when a
>   completed inventory on an identifiable PipeWire-Pulse route has no capture
>   source; a missing, stalled or ambiguous audio route leaves the answer unknown and permits Start. Output monitors
>   are not microphone sources. Relay
>   the stable reason from error.cause, plus the existing message and
>   suggested_action, to the person; retry
>   only after the stated problem is fixed. A start without microphone sound
>   does not need microphone readiness.
>   After `screen_record.start` returns a `capture_id`, retain it.
>   For an ordinary Windows take after its first frame, Stop answers at once
>   with `status:"finalizing"` and a `job_id` while saving continues, or returns
>   the completed Stop result if saving has already finished. Poll
>   `jobs.status{job_id}` with a deadline of at most 15 minutes. Use saved paths
>   only after `state:"done"` and a full Stop `VerbResult` envelope in the job's
>   `result` with `ok:true` and the same `capture_id`; when `mux_raw:true`, require
>   a nonempty `raw_path`. Failed, cancelled, missing or timed-out jobs do not
>   confirm a saved file. Saving cannot be cancelled or abandoned by Quit.
>   Linux, macOS and `reason:"app_exit"` wait for the completed Stop result.
>   A non-successful `screen_record.stop` leaves that capture unresolved: do not
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
>   inserts an editable polished clip with framing and, when source geometry is
>   valid, zoom-to-cursor and cursor smoothing. New Windows/macOS window captures
>   retain the native cursor in Raw and Polished pixels; unavailable window input
>   geometry does not support synthetic cursor styling or click-driven zoom.
>   Monitor captures retain the synthetic polished cursor path.
>   Show keystrokes sits beneath Polished and starts off. The
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
>   its path-safe receipt/loss facts for Settings → Health & recovery rather than
>   trying to inspect cache files or trigger repair from a read. Settings gathers
>   sequential 100-row lexical pages (at most 4,096 rows) and labels a completed
>   result as reported in that check; malformed or partial inventory is attention.
>   A recording is never lost silently. `screen_record.active{}` shows the running
>   recording (no id needed; the result carries `capture_id`); the app shows a recording an agent started, with
>   Stop. Closing or quitting the desktop app during a recording stops and saves it
>   first (`stop{reason:"app_exit"}`). At the next start `screen_record.pending{}`
>   lists interrupted or close-saved recordings with a `message` for the person and
>   a `suggested_action`; with the recording's project open,
>   `screen_record.recover{capture_id}` adds what was saved to the end of the
>   timeline, and `pending_dismiss{capture_id}` forgets a notice and keeps files.
>   If the window does not answer a close within 6 seconds the desktop shell
>   stops the recording itself; an engine that shuts down finishes a running
>   recording first. Save points come every 15 seconds, so a crash loses the
>   video after the last one; the sound is kept.
>   On Windows and macOS, the screen keeps recording across the 15 s save points.
>   Known Windows limits (tell the person when they ask about a hitch): about
>   half a second at the start of a take can repeat the first picture.
>   Short, separate system sounds can drift out of sync with the picture.
>   Opening a project with an interrupted recording starts a visible
>   `recording_recovery` job ("Preparing interrupted recording"); `recover`
>   refuses with "this recording is still being prepared" until it ends.
>   `screen_record.polish` is a `screen_record_polish` job with frame progress;
>   its clip goes only into the recording's own project (`placed:false` with
>   `waiting_in_project` when another project is open).
>   A Stop before the first frame returns `nothing was recorded` and leaves no
>   stuck capture. Start refuses a window absent from the Doctor window list
>   (`not_found`). While a microphone or system-audio test runs, every start is
>   refused with `conflict` ("An audio test is still running, so the recording
>   cannot start yet."): wait a few seconds and start again. A missing folder
>   says "<label> not found: <path>", an unwritable one "<label> cannot be
>   written: <path> (access is denied)"; `project.set_output_dir` refuses a
>   folder it cannot write. Count
>   a recording timer from `first_screen_frame_unix_ms` (status or active), not
>   from the Start reply.
>   On Windows 10 build 20348 or newer, system audio uses native,
>   endpoint-independent process loopback instead of opening the physical render
>   driver. Security software may ask to allow audio capture for a new Cut binary;
>   if access is blocked, screen and microphone recording continue and the missing
>   `system.wav` is reported in the capture log/raw-stream result.
>   On macOS 14.2 or newer, the installed signed app captures system audio through
>   a Core Audio process tap alongside ScreenCaptureKit. Approve the separate
>   Screen Recording and Audio Capture prompts on first use (they come with the
>   first `screen_record.doctor{mode:"check"}` or recording start, not when Record
>   or a project opens), restart Cut if macOS asks, and verify `raw_streams.system` after `screen_record.stop`.
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
>   available when the microphone is ready (`screen_record.doctor`
>   `{mode:"microphone"}`); it never checks screen capture. It binds a current unlocked audio track plus playhead or In/Out and
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
>   A refusal names a missing prerequisite (microphone permission, a saved
>   microphone, or the recorder), not a prohibition: tell the person what is
>   needed and where to provide it, offer to try again, and never work around
>   it quietly. `microphone_blocked.reason` tells the cases apart: on Windows
>   `windows_microphone_access_off`, `windows_apps_microphone_off` or
>   `windows_desktop_apps_microphone_off` (a Windows privacy switch is off; the
>   action names the switch under Settings > Privacy & security > Microphone),
>   on macOS `macos_denied` or `macos_restricted`; on Windows, macOS and Linux
>   `no_microphone_device` when native inventory proves no input exists. If
>   Linux cannot check the sound server, it leaves the blocker null and allows
>   the existing Start path. A take
>   that fails after it started returns `failure:{reason,message,suggested_action,detail?}`
>   in its status (on Windows `microphone_in_use`, `microphone_unavailable`,
>   `microphone_open_failed` or `windows_microphone_access_denied`); relay the
>   message and action. The same applies to `screen_record.doctor` cards: use
>   `{mode:"status"}` to show readiness without a capture or prompt, and run the
>   full check only when the person asks to check or record. Pass the chosen
>   `fps` and `monitor_id` or `window` to check video recording on Windows.
>   The `video_encoder` answer includes `checked`, `ready`, `reason`, `message`,
>   dimensions and frame rate. Its `video_encoder_unavailable` reason means
>   the selected source cannot initialize video recording; show the plain
>   message and keep native details in diagnostics. `window_minimized` returns
>   `checked:false` and `ready:false`: Doctor leaves the selected window
>   minimized without checking its encoder. Ask the person to restore it and
>   check again. Only explicit recording Start may restore that window.
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
>   Premium UI readiness requires confirmed NVIDIA/CUDA hardware and consent
>   to its non-commercial model licence. Setup uses platform-selected PyTorch;
>   installation or a slow CPU API attempt does not qualify usable Mac Premium.
>   Use Standard RVM on Mac.
>   Subject seeds use integer source pixels and source time in milliseconds.
>   Read applied/cleared intent from the committed `op.effects[]` fields;
>   applying also returns the immediate alpha-bake quality receipt as `matte`.
>   Derived files kept in the project (matte, mask and power-window alphas,
>   stabilization data, nest bakes, segment windows, analysis reports) are
>   reused only when this computer made them; anything that came with the
>   project is made again.
>   Proxies too: at open a `proxy_check` job gives back the proxies this
>   computer made, encodes once more a proxy it has no record of (made before the
>   upgrade or on another computer), and makes the proxies its footage lacks;
>   read `project.state.proxy_check` while it runs. A missing title movie is
>   drawn again (`title_remake` job) when it is next needed; at open every
>   present title movie and thumbnail strip this computer did not make for
>   exactly its asset and settings is drawn again, a proxy counts only for its
>   own asset and source, and each asset's probe is read again from its file
>   (`proxy_check.result.proxy_check.probes_unknown` lists the assets whose
>   file is missing or changed: their probe is shown but is not a fact). A copied project's first render is therefore
>   slower; `render.final` bakes mattes again itself, while other render verbs
>   that meet a matte this computer did not bake stop with `not_found`
>   ("was not made on this computer") and that way out.
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
  render-free quality check results let you inspect the cut before a final
  encode:
  `verify.pacing` (visual shot rhythm), `verify.delivery` (verbal: WPM + filler
  density over the transcript), `verify.captions` (caption quality checks against BBC/Netflix
  timed-text standards), `verify.brand` (proves caption styles + output aspect
  conform to the durable `project.brand` kit, or an explicit one-call override;
  the agent resolves AGAINST brand, never overwrites it). `render.bundle`
  automatically enforces the saved kit and records its source in the manifest.
  `verify.loudness` measures an asset's LUFS/peak/range, `verify.scopes` measures
  the composed picture and can render scope images, and `verify.pregate`
  predicts timeline risks before rendering. Report their numbers like any
  receipt.
- **Recheck historic rendered bytes without changing their evidence.** In
  Review → Render history & checks, `verify.rerun {render_id}` queues one cancellable,
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
  (non-contiguous highlight reel), `render.storyboard` (Timeline contact sheet; contact-sheet
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
- Codex: `codex mcp add shellx-cut -- "/absolute/path/to/cutd" mcp`. With the
  default account, Codex stores it in `~/.codex/config.toml`; for an enrolled alternate
  account, run the check in that account's `CODEX_HOME` environment.
  `codex mcp get shellx-cut --json` confirms the entry but is not by itself a
  live-handshake claim.
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
the final proof of protocol negotiation, ping, all 326 tools, and same-engine
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
It allows 120 seconds for the full doctor's cold tool/hardware checks and reports
total proxy-check time in `elapsed_ms`; it does not skip doctor checks.
Use its `read_first` links, including
`/api/agent-doc/docs/public/DEBUG_API.md`, for the endpoint and security contract
shipped with that build.

Every verb returns the envelope
`{ok, result?, op_ids?, project_revision?, warnings?[], error?{code, message, clip_id?, at_ms?, cause, suggested_action?, requirement?}}`.
When a refusal is about a missing tool or condition, `error.requirement` is
`{id, name, get}` from the requirements registry: `get` names the Settings
entry, the button, the verb that sets it up and what only a person can do
(`system.doctor {feature}` gives the same answer before you call a verb).
Errors are actionable — they carry the clip, timecode, cause, and a suggested
next move; read them, don't retry blind. A refusal names something the app is missing (a tool, a permission, a shorter file, a running test to finish), not a ban: tell the person what is needed and offer to do it, and do not route around the refusal quietly. `warnings[]` carries non-fatal
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
`{type: op_applied | job_progress | render_done | receipt_ready | project_changed | ui_state | doctor_updated | output_folder_changed, ...}`. `job_progress` carries `{job_id, kind, progress, message?, terminal?}`: `terminal:true` marks the event that ENDS the job (done, failed or cancelled), and an ended job keeps the progress it really reached, because progress 1.0 means finished work. Decide that a job ended from `terminal` or the job's `state`, never from the progress value. `op_applied` additionally carries `{revision, from_revision, delta:{kind:"op",count:1}}`; treat events as best effort and pull `project.state{since_revision:last_applied}`. The server returns a bounded applicable delta or an explicit snapshot fallback, so reconnects and missed frames do not require replaying an unbounded log.
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
Export review availability is checked without running a model and reported
in `details.judge_ready`; `availability_reason` explains a false result.
A missing or malformed availability response stays `status:"unknown"`;
do not infer it from `found` or version.
`details.chat` remains the independent Agent Chat capability/session state.

| Env var | Default | Effect |
|---|---|---|
| `SHELLX_CUT_RENDER_MEM_HIGH_PCT` | `75` (clamp 10–95) | Render memory **soft ceiling** as a % of total RAM (Linux). At the ceiling the kernel throttles + spills to swap and the render keeps going; the hard stop is RAM minus 1 GB. A segment window gets the same budget (N parallel windows share it). A render the system stops for memory fails with code `out_of_memory`, a full output drive with `not_enough_space`; both say what to do. Lower it on a shared/small box. |
| `SHELLX_CUT_RENDER_NICE` | `10` | CPU niceness applied to every render's ffmpeg (keeps the box responsive during a long encode). |
| `SHELLX_CUT_RENDER_MEMORY_MB` | unset (memory free now) | Plan renders with at most this many MB: the preflight lowers libx264 threads (and warns) as if only this much were free, and says so with the variable's name. A limit of your own for a shared box, and the way to try the small-machine path on a big one. |
| `SHELLX_CUT_RENDER_THREADS` | unset (libx264 auto, about 1.5 per core; fewer only when the render would not fit the free memory) | Cap render `-threads` + `-filter_complex_threads` at N. Each thread carries its own frame buffers, so a lower cap shrinks the footprint on a constrained box (trades speed). Render path only — never caps probe/scrub. Default-off keeps per-machine reproducibility. |
| `SHELLX_CUT_RENDER_SEGMENT_SEC` | unset (adaptive) | Override the segmentation gate: a `render.final` longer than this many seconds renders SEGMENTED. Unset = the adaptive gate (segment when overlays exist AND the timeline exceeds one adaptive window, or past a 10-min base-only ceiling). `0` forces segmentation; a large value disables it. |
| `SHELLX_CUT_RENDER_WINDOW_SEC` | unset (adaptive) | Override the segment window size (clamp 2–300 s). Unset = ADAPTIVE: window shrinks with resolution × overlay count so each window's peak RSS stays near the budget (4K with 2 overlays → ~2 s windows ≈ 1.2 GB, instead of 30 s ≈ 17 GB). Also bounds `render.frame{compose}` (composes only the window holding `at_ms`). |
| `SHELLX_CUT_RENDER_WINDOW_BUDGET_MB` | `1500` (clamp 256–8192) | Per-window peak-RSS budget the adaptive window targets. Lower it on a small box; raise it on a big one for fewer, faster passes. |
| `SHELLX_CUT_RENDER_PARALLEL` | `1` (serial) | OPT-IN: render N segment windows concurrently (Linux only; each capped at budget/N via its own cgroup so the box stays bounded). Parallel windows trade substantially more memory for a load- and hardware-dependent speed gain, so serial rendering stays the default. |
| `SHELLX_CUT_RENDER_RAM_PCT` | unset | OPT-IN alternative to `_PARALLEL`: auto-size the concurrent-window count to about this percentage of RAM (Linux + cgroup only). For example, `60` on a 64 GB machine permits about nine windows, with the same memory and load tradeoffs as `_PARALLEL`. |
| `SHELLX_CUT_NO_HWENC` | unset | Set `=1` to force the **software encode** tier (skip NVENC/QSV/AMF/VideoToolbox) — for CI / byte-reproducibility / a flaky GPU encoder. Also disables the GPU render fast-track (it needs NVENC). |
| `SHELLX_CUT_RENDER_GPU` | unset (software) | **EXPERIMENTAL opt-in GPU render fast-track** (`=1`/`true`/`yes`/`on`). NVDEC + `scale_cuda` + `nvenc` keep eligible frames in VRAM, reducing CPU load; speed varies with hardware and system load. GPU output can vary by driver/hardware, so it is OFF by default and the software path stays the receipt-exact baseline. The probe-gated path is limited to a single base video track of hard cuts at matching source aspect on NVIDIA hardware. Overlays/PiP, grade, titles, captions, fades, crop, xfade, reversed or frozen clips, or mismatched aspect transparently fall back to software. The pixels differ from the software encode, but every output frame shows the same source frame as the software render (the same frame count per clip, cut frames and rate conversion). When the GPU path is asked for, `encode.pipeline` in the job result, receipt and dry run is `gpu` or `standard`; with `standard`, `encode.pipeline_reason` says why in plain words (the Render history & checks tab shows it). |
| `SHELLX_CUT_DETECTOR` | CPU-floor | `=high` selects the heavier Faster R-CNN (GPU) for `render.reframe` subject tracking; default is the BSD torchvision SSDlite CPU floor. |

**Render resource governance (Linux).** A heavy/long render cannot wedge the
machine. On Linux+systemd every render's ffmpeg runs inside a transient cgroup-v2
scope with `MemoryHigh` (~75% RAM, throttle+spill under memory pressure)
and a `MemoryMax` backstop (total − 1 GB) that confines any worst-case kill to
the render's OWN cgroup, never `sshd`/the desktop. `MemoryHigh` slows the
render first; at `MemoryMax` the kernel stops ffmpeg inside its own scope and the
render fails with `out_of_memory` and the way out. A render whose ffmpeg writes no new output and uses
under 2% of one core for 120 s (`SHELLX_CUT_RENDER_STALL_SEC`, 0 = off) fails with `render_stalled`, naming the
timeline part and the clip (`clip_id`, `at_ms`) and, when the kernel held it at `MemoryHigh`, saying so. A reversed
clip is held in memory whole (source size, about 12 MB a 4K frame), so windows that hold more reversed video than
`WINDOW_BUDGET_MB` are split and a timeline holding more than 4 GB of it renders in windows. macOS/Windows have no cgroups (they auto-compress/page), so there the
render is a plain `nice`d spawn. The probe for systemd availability is cached
once per process. This is the safety net; the real memory *bound* is **segmented
rendering**: a `render.final` with an overlay track that is longer than one
adaptive window, or any timeline longer than 10 minutes, renders the video
in adaptive time-windows (each window's filtergraph holds only that window's
clips → peak RSS bounded near `WINDOW_BUDGET_MB`, not growing with timeline
length), with a single cheap audio pass muxed on; `render.frame{compose}`
likewise composes only the window holding the frame. The result matches the
whole-graph render frame for frame: the same frame count, and every output
frame shows the same source frame (each clip gets exactly its own frame count;
a clip whose source ends early holds its last frame); only the bytes differ,
because the parts are encoded separately.
Before a render starts, `render.final` (and its `dry_run`) estimates memory and
output size against free memory and disk. libx264 keeps its automatic thread
count (1.5 per core) when the estimate fits; otherwise the render uses the
largest of 32, 24, 16, 12, 8, 6, 4 threads that fits, and
`resources.memory.note`, the job's progress message and the receipt's
`encode.memory_note` (with `encode.encoder_threads`) say so: relay it, it
explains a slower export. Still short even at 4 threads: `memory_short`
warning; larger than memory and swap: refused with `out_of_memory`; disk:
`disk_short` / `not_enough_space`. The estimate counts a source used several
times by what ffmpeg really holds (one shared copy when the frames pass
through unchanged, a copy per later cut when they are scaled or converted),
and a segmented render by its largest window, including the shorter windows
needed for reverse. Reversed frames count at the source size and rate before
scaling or frame-rate conversion. Sources larger than DCI 4K
decode with 8 threads (no measurable speed cost). HW encode auto-selects the best working GPU encoder (probe-gated, so a
listed-but-broken encoder can never produce a bad render) and falls back to
software; a frame larger than the hardware encoder takes is named with that
encoder's limit, and a hardware encoder that fails DURING the render makes the
whole render run again in software. The receipt, job result and dry run carry
`encode {encoder, hardware_requested, hardware_used, fallback_reason?,
fallback_detail?, memory_note?, encoder_threads?}`: relay `fallback_reason`
and `memory_note` to the person.

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

A project change (`project.create`, `project.open`, `project.close`) that would
cancel running work someone asked for (any job except the automatic import
follow-ups `import_chain`, `proxy`, `enrich`, the open-time `proxy_check` and
`title_remake`, and a `cache_purge`) is refused
with `jobs_running` and changes
nothing; the cause lists the jobs. Ask the person, then wait, cancel them with
`jobs.cancel`, or repeat the call with `cancel_running_jobs:true`. Running
import follow-ups and cache purges are cancelled by a project change and listed
in the result's `cancelled_jobs`. `project.open` refuses a folder that is not a
project (`invalid_args`, nothing written); use `project.create` for a new one.

Closing or quitting the desktop app follows the same rule. There is no verb
that closes the app; `jobs.before_quit{}` tells what a quit would cancel (the
same `jobs_running` refusal, naming each job). The app asks the person ("An
export is still running", **Keep working** is the default, **Quit and stop it**
cancels the work cleanly and then quits); while that question is open,
`ui.state` shows it as `overlays.quit_question`. A recording is saved first
and its question comes before this one. When the engine does not answer, the app
does not take that as nothing running: it asks whether to quit anyway (Keep
working is the default). Installing an update asks the same way before it
stops the engine, and asks to stop a running recording first. Work cut off by a kill, crash or power
cut is listed by `jobs.interrupted{}` (no project needed; a helper of the dead run
that is still running is stopped first, then the export's unfinished files are
removed from the project's `exports` folder and from the app-wide export
folder) and
forgotten with `jobs.interrupted_dismiss{key}`; the app shows it in the status
bar at the next start. A stop that arrives after a render has published its
file ends that job done with its receipt.

Only one desktop window runs per user: opening the app again brings it forward,
and a `.cutproj` path given on the command line opens in it (a launch with its
own `SHELLX_CUT_HOME` is separate). When an engine the desktop app started exits
on its own, the app shows "ShellX Cut engine stopped" with **Restart the
engine** / **Quit ShellX Cut**; a restarted engine has no project open, so call
`project.open` again and read `jobs.interrupted{}`.

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
person can use Projects → Make a copy to include the used media. A copy of the
project folder alone may still need the original linked media. Make a copy uses
a native destination-folder picker, shows the full-SHA-256 preview, and
requires an explicit create confirmation. For direct API work, first call
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

Use `project.cache_rebuild {asset_ids?,estimate_only?}` only to backfill missing
or stale Cut-owned base proxies/filmstrips for registered current-source assets.
Omit `asset_ids` only when the project has at most 64 assets; otherwise select
at most 64 unique ids. Its uniform path-free result reports queued asset/output,
up-to-date, and per-reason counts for `estimated`, `queued`, `already_queued`,
and `not_needed`. Fresh outputs retain the verified source identity; the project
ownership record alone cannot establish engine origin. Restore or relink a
changed source before asking to rebuild its stale previews. Read
`skipped_assets` for the exact asset, reason, affected cache kind, and next
action; do not treat a count as a full answer about which assets were skipped.
An entry with reason `already_current` is an up-to-date asset (counted in
`counts.fresh_assets`), not a skip. `counts.unowned_outputs` and
`counts.legacy_outputs` count outputs (a proxy and a filmstrip of one asset make
two); the number of assets needing attention is the number of distinct
`asset_id` values in the other `skipped_assets` entries. Use
`estimate_only:true` for the same source-identity admission and verified
work-unit/byte count without reserving outputs or creating a job; it is not a
guessed duration.
Poll a queued `cache_rebuild` through `jobs.status` and use `jobs.cancel` when
requested. It verifies source identity before publication, reserves ownership
durably before output, never adopts legacy/unowned files, and leaves unfinished
pending reservations resumable after cancellation or restart. It never changes
source media, exports, recordings, captures, receipts, or foreign files.
If scheduling fails after an earlier owned cache output was retired, inspect
the failed `cache_rebuild` job through `jobs.status`: its partial result names
only verified pending asset/output identities and retired counts, or marks
ownership verification unavailable. Resolve the cause, restore or relink a
changed source if needed, then retry the same rebuild. Do not claim that an old
derived cache file was restored.

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
asset (one op) and kicks the import job **probe → auto-place →
ready-to-edit** (fast). Proxy and filmstrip run in a separate `proxy_job`.
Transcribe + perception run as a SEPARATE background
**enrich** job (`enrich_job`) so slow transcription on long footage NEVER blocks
editing (and a missing/failed sidecar degrades to a warning, not a failed
import). Transcript-driven verbs (captions/cut_words/remove_fillers) need the
ENRICH job finished — wait on `enrich_job`. Wait by either:

- polling `jobs.status {job_id}` (every 2–5 s, not a hot loop) until
  `state` is `done` or `failed`, or
- watching WS `job_progress` events for that job_id (`terminal:true` = ended).

An `ok` import answer confirms asset registration; read its job before editing.
If the job says "Media analysis did not start" because it could not be saved,
free space or restore write access to the project folder, then reopen the project.
Cut retries analysis for the same saved asset; do not import a duplicate.

**Did the job do what was asked? Read `outcome`.** `succeeded` = yes (with
`completion:"done_with_warnings"`, read `result.warnings`); `partial` = it ran
to its end but did NOT fully deliver (`outcome_reason:"checks_failed"`: a
written file failed its own checks, `result.checks`; `"items_failed"`: some
batch items failed, `result.failures`); `failed`, `cancelled`, `interrupted`,
`superseded` = no. The trap: `state:"done"` alone is not success (a render
queue with 1 of 3 exports failed is `state:"done"`, `outcome:"partial"`).

A job you start that fails is also shown to the person in the status bar (what
failed, the reason and the suggested action from `jobs.status.error`), so say in
your own words what is missing and what the person can do. A DOM check reads
`[data-cut-job-failed-row]` (children `data-cut-job-failed-title`,
`data-cut-job-failed-text`; controls `data-cut-job-failed-details` and
`data-cut-job-failed-dismiss`; `data-cut-job-failed-more` counts older ones).
To see failures you did not start (a person's import, another agent's render),
read `jobs.list {}`: it is the status bar's own source, every job of this
server run, newest first, each failed one with its `error`. `ui.state.notices`
lists the failed-job rows the person still sees (`failed_job {job_id,
job_kind}`, `failed_jobs_more {count}`).

For a project-owned UI job, include the immutable
`expected_origin_path_sha256` from `project.state.project_identity` in
`jobs.status` and `jobs.cancel`; a changed project returns `conflict` before a
possibly reused job ID is read or cancelled. Existing unscoped calls remain
supported.
The app also passes this digest when admitting `render.queue`,
`screen_record.export`, or `screen_record.copy_raw` so a project switch cannot
retarget the new job. Legacy callers may omit it.

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
muxed video with separate system-actor `edit.insert` ops on the first video
and audio tracks. Stills are not auto-placed. Later imports
(b-roll) are NOT placed; add them with the matching explicit insert verb.
"First" is the first import REQUESTED, not the first to finish probing, and each
import job result says `placed` and, when not placed, why (`placement.reason`).
For human UI placement, Assets **Add at playhead** and normal timeline drops target the
base story timeline and push later clips right (Insert). Use an explicit overlay path only when the clip
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
`edit.overwrite {asset, at_ms, video_track?, audio_track?, create_video_track?, create_audio_track?, src_range_ms?}` to
replace the fixed source-duration interval without moving downstream material.
Use `create_video_track:true` or `create_audio_track:true` instead of the matching
existing-track field to create that destination in the same atomic undo step.
Assets drags use overwrite plainly and insert with Alt; the dropped lane chooses
the track, and only dropping on an overlay lane puts video on top.
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
Settings > Video & performance, open the manual guide, or re-check after
installation.
When ffmpeg or ffprobe is missing, every media verb that needs them (58 verbs: imports,
probes, waveforms, previews, frames, renders, exports, transcription and
analysis) refuses before it changes anything: `ok:false`, code
`tool_missing`, message `ffmpeg is not installed` (or ffprobe), and a
`suggested_action` with this platform's way to get them. Relay it to the person
and wait for their consent (a person's own request to download it is that
consent); do not retry in a loop. Where a built-in download
exists (Windows/Linux x86_64 and Apple silicon Macs) show the person the
consent text (`details.setup.download.consent` on the ffmpeg card: publisher,
version, size, GNU GPL version 3, source link) and only when they agree call
`system.fetch_tool {"tool":"ffmpeg","accept_license":true}`; without
`accept_license` the verb refuses and returns that text in `error.cause`. On an
Apple silicon Mac the download is Martin Riedl's notarized build, which has no
vid.stab: Stabilize then needs Homebrew's complete build. On Windows and Linux
x86_64 it is a pinned FFmpeg-Builds 9.0.2 build; if its publisher has removed
it the job fails and names the other ways. Linux arm64 has no download: the
distribution's package (`sudo apt install ffmpeg`; Fedora `sudo dnf install
ffmpeg-free`). Intel Macs have no
download (`unsupported_platform`): the person installs Homebrew
(https://brew.sh, needs their Mac password) and runs `brew install
ffmpeg-full`. Anywhere you may call `system.set_ffmpeg {path}` with a build
that has ffprobe beside it. The card's `details.setup` carries the steps and
copyable commands. Then `system.doctor {"refresh":true}` finds the new tools
without restarting Cut.
Which FFmpeg is in use and why: the ffmpeg card's `details.resolved` and
`details.selection` (`reason`: environment | chosen | automatic_hardware |
automatic_no_hardware | only_one_found | automatic_off | not_found;
`automatic`: on | on_by_environment | off_by_environment | off_source_build;
`found`; `text`, a sentence to relay). The engine picks a hardware-capable
build by itself, the same when you start `cutd serve` as when the app does. A
person's choice (`system.set_ffmpeg`) or `SHELLX_CUT_FFMPEG` wins.
The ffmpeg card's `details.version_advice` is null, or
`{in_use, tested_versions, message, upgrade}` for a readable major version below
7: relay it as optional upgrade advice, keep the person's choice, and do not
repeat it as an error or missing-tool warning.
`SHELLX_CUT_FFMPEG_AUTO=0` turns the automatic pick off. A source build in a
cargo target folder keeps it off unless `SHELLX_CUT_FFMPEG_AUTO=1`.
A step whose filter no installed FFmpeg has is refused up front the same way
(`tool_missing`): Stabilize without vid.stab, caption clips burned into an
export without libass, color management without zimg. The message names the
feature and library; the cause says what to remove to go on without it.
A file that exists but is not readable media is `unreadable_media`: an empty
file is refused by `media.import` itself (`ok:false`); anything else fails the
import job with that code, a message naming the file, ffprobe's reason in
`cause` (ending "the import was undone") and a `suggested_action`; the asset is
removed with a `media.remove` op, so never place it. `media.relink` refuses such
a target with the same code and keeps the old source. Relay the message; do not
retry the same file.
The first video placed in an untouched project sets the project format; the
import job result's `format` says what happened: `{adopted:true, width, height,
fps, variable_frame_rate, note?}` or `{adopted:false, reason}` (the format was
chosen before, or the clip is larger than the frame limit: then the job also
ends with that warning). Frames are 16 to 8192 px on each side, at most
8192x4320 pixels, with even sides for the project; `project.format`,
`project.create` settings, `render.final` width/height and
`resolution:"match_source"` all refuse beyond it with `invalid_args` naming the
limit (AV1 also needs at least 64x64). A project whose stored format is beyond
the limit (made elsewhere or edited by hand) still opens: the format is reduced
to the largest frame of the same shape within the limit, `project.open` returns
a `project_format_reduced` warning with `from`/`to`, and `project.health` lists
the same notice; `export.frame` refuses a still beyond the limit. A variable-frame-rate clip
(`probe.variable_frame_rate`, `probe.nominal_fps`) gives the timeline its
nominal standard rate (a phone's 60, not its 58.59 average), and renders place
its frames by their real timestamps.
Colour: every export is SDR BT.709 processed in 8 bits. `render.final` (plan,
job result and receipt) carries `encode.color` = `{output, processing_bit_depth,
file_bit_depth, sources:[{asset, file, transfer, bit_depth, handling}], note?}`;
an HDR source (transfer `arib-std-b67` HLG or `smpte2084` PQ) has handling
`tone_mapped_to_sdr` and the file is tagged BT.709. Keeping HDR in the export is
not available. The conversion runs before grades and effects. An SD source
tagged BT.601 (`smpte170m`/`bt470bg`) is converted to BT.709 too (handling
`converted_bt601_to_bt709`). The tone map needs ffmpeg's zscale (zimg); without
it every verb that composes the picture (render.final/queue/bundle/direct/
reframe/preview, export.range/gif/frame/publish, verify.scopes, render.frame and
render.storyboard with `compose:true`) refuses up front with `tool_missing`
naming the HDR clips (tagging a clip `rec709` with `edit.color_space` exports it
unconverted, handling `hdr_tagged_not_converted`). Relay `note` to the person.
An older project with an odd size (e.g. 641x361) cannot be exported as video
at that size: `render.final` (and its dry run) at the project size,
`render.queue` entries without their own size, `render.direct`,
`render.reframe`, `render.preview`, `export.range`, `export.gif` and
`autopilot.run` refuse up front with code `odd_project_size`; its
`suggested_action` is the exact call, e.g. `project.format
{"width":640,"height":360}`. Tell the person, and make the call only with their
agreement (it changes the project; Undo does not change the project size back).
Exports with their own even size (`export.publish`, `render.bundle`,
`render.final` with aspect/width/height/match_source) and stills
(`export.frame`, which then returns a `note` that the still is 640x360) are not
refused. An fps-only `project.format` still works.
After a video import's proxy job, `probe.decode_check` on the asset says how
much of the file really decodes (`decodable_picture_ms`, `decodable_sound_ms`
against the claimed lengths). When that comparison finds a shortfall it carries a
plain `warning` (also on the proxy job result, and a "File cut off" badge in
Assets): the clip still edits, but an export stops early where data is
missing. A file damaged INSIDE (zeroed or corrupt data in the middle) can reach
its end but lose frames: `code:"decode_gap"`, `claimed_frames` vs
`decodable_frames`, and a warning that says how many frames cannot be read
(`code:"decode_short"` is the cut-off case). No `decode_check` means it was not
checked (`proxy:false` imports). A render receipt then carries
`sources_decodable`, which fails by file name when a source on the timeline has
such a warning (an export can hold frames over a gap, so its own streams can
still look complete). This check counts frames; FFmpeg 6 may repeat frames to
conceal corrupt pictures, leaving no warning. `sources_decodable` cannot
certify damage hidden this way.
Perception/transcription sidecars inherit the same resolved FFmpeg/ffprobe
directory at `AppState` startup and after doctor re-scans, so captions and
analysis jobs reuse the engine's selected/Homebrew/app-data video tools instead
of depending on the sidecar process PATH.
Captions, transcription and the post-render checks run on Cut's own Python,
installed by Install captions (`system.setup_perception`); a Python the person
installed is not used, and the Windows Store `python` shortcut is not a Python.
Without it a sidecar step is refused with code `sidecar`, message `Captions and
transcription are not installed` and that action. The background enrich job of
an import then finishes done (not failed, not warned) with
`result.skipped:"captions_not_installed"`; a render finishes done too, its
`result.pass` is `null` (no verdict, not a failure) and its checks read `not
measured, because captions and transcription are not installed`
(`result.verification_reason`, receipt `details.reason`). Both results carry
`setup_needed:{missing, what, setup_verb:"system.setup_perception",
person_action:"Install captions", suggested_action}`; relay it once, do not
retry in a loop. Every `suggested_action` names a route of the installed app,
never a path or command of a source checkout.
If the captions analysis process starts but exits unsuccessfully, the `sidecar`
error keeps its diagnostic in `cause` and supplies `suggested_action`: install or
repair the captions tools in Settings > AI & transcription, or call `system.setup_perception`,
wait for its job, then retry. This applies to `media.transcribe` and
`media.perception`; inspect the completed job's error, not only its start reply.
Captions on Windows need the Microsoft Visual C++ Redistributable; ShellX Cut
tells you when it is missing and gets it from Microsoft for you. It is ONE tool:
doctor card `vc-runtime` (Windows only; `details`: `installed`, `version`,
`needed_by`, `official_page`, `download_url`, `about`, `license_url`, `fetch_action`,
`removed_by_uninstall:false`), shown in Settings > AI & transcription. While it
is missing, `system.setup_perception` and the premium `system.setup_matte` are
refused at once (code `tool_missing`, no job) with the entry and the verb in
`suggested_action`. Ask the person before downloading it and accepting
Microsoft's licence (`details.license_name`, `details.license_url`), then call
`system.fetch_tool {tool:"vc-runtime"}` (no `accept_license`). The job downloads
Microsoft's installer from aka.ms/vc14 (listed on
learn.microsoft.com/cpp/windows/latest-supported-vc-redist), runs it only after
Windows' signature check accepts it as Microsoft Corporation's, and Windows then
asks the PERSON for administrator permission: tell them to approve it; you cannot.
The job fails with the reason when the prompt was declined, the account cannot
approve it (ask an administrator), the download or the signature check failed;
the result says `restart_needed`. Then `system.setup_perception` works without
restarting Cut. Uninstalling Cut keeps the runtime.
The human Render button and FFmpeg-backed export choices also run
`verify.pregate {}` before starting output. All quality predictions show a concise
warning with collapsible details and an explicit Export anyway or Queue anyway
choice, including high-risk findings such as intentional still screen footage.
Missing media, FFmpeg, and output authorization failures remain actual errors.
The warning's Guide action opens the bundled
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

### 2b. Editing a recording in Edit

For new polished recordings, select the screen clip in Edit and open the Inspector
Zoom card. Select, add or delete zooms and adjust strength, target, timing or speed.
Redo automatic zooms keeps your own zooms. Press Update video to see the changes;
playback keeps the previous zooms until the update finishes. Old polished clips
and Raw MP4s do not gain editable recording zooms.

A separate camera track is on by default. Select the Camera track clip for the
Inspector Camera section: corners/centre, size, Circle / Rounded / Rectangle,
ring, and Back to as recorded. Drag the bubble on the preview; split at both ends
and delete a piece to hide it for a stretch. Before recording, turn off Edit the
camera later (own track) on the Record page to bake the bubble in instead.

#### Zoom verbs and recovery

Use `screen_record.zoom_list{clip}` for the recording's zooms. A segment is
`{id,start_ms,end_ms,scale,cx,cy,ramp_in_ms,ramp_out_ms,origin:"auto"|"manual"}`.
Times are recording-clock milliseconds; scale is 1–8, targets are clamped in-frame,
and segments cannot overlap. Each segment includes a server-mapped
`timeline:{start_ms,end_ms}|null` through trim, constant speed, speed ramp and
reverse. Do not copy that mapping into clients.

| Verb | Arguments | Result |
|---|---|---|
| `screen_record.zoom_list` | `{clip}` | `{clip,take,source:{w,h},duration_ms,segments:[segment & {timeline:{start_ms,end_ms}|null}],stale,applied_plan,pending_plan,missing_video,missing_video_reason,next_action}` |
| `screen_record.zoom_set` | `{clip,segments,rationale?}` | `{clip,segments,stale}`; empty disables zoom |
| `screen_record.zoom_add` | `{clip,at_ms? or timeline_ms?,duration_ms?=2000,scale?=2,cx?,cy?}` | `{segment,stale}`; target defaults to pointer |
| `screen_record.zoom_update` | `{clip,id,target?:"pointer",start_ms?,end_ms?,scale?,cx?,cy?,ramp_in_ms?,ramp_out_ms?}` | `{segment,stale}`; pins manual |
| `screen_record.zoom_remove` | `{clip,id? or ids?}` | `{removed,stale}` |
| `screen_record.zoom_auto` | `{clip,keep_manual?=true,config?}` | `{segments,stale}`; autoedit config, keeps manual segments |
| `screen_record.take_refresh` | `{clip}` | `{job_id,clip,asset,baked,cached,applied_plan}`; initial asset may be null, poll jobs.status |
| `screen_record.take_link` | Internal only | Polish provenance op; direct calls refuse |

For zoom_add send EXACTLY ONE of recording `at_ms` or absolute project
`timeline_ms`. The server maps the selected clip's timeline placement; positions
outside the clip refuse (move inside or use at_ms), and frozen stretches refuse
(unfreeze or use at_ms). Returned segment times remain recording-clock ms.
The default add is a two-second, 2× pointer-target zoom.

`screen_record.zoom_update{clip,id,target:"pointer"}` resets cx/cy to the last
recorded pointer sample at start_ms (centre fallback), normalized and clamped at
the chosen scale. It overrides explicit cx/cy and pins the segment as manual.
Editing any automatic zoom pins it as Mine; automatic redo keeps manual zooms by
default and accepts the same config as autoedit. Empty zoom_set disables zoom.
Each write is one concrete/resolved-list undoable op, returning stale and
segments, segment or removed (zoom_set also returns clip); replay needs no events
file. Undo/redo restore zooms and stale state.

The zoom_list result includes missing_video:boolean, missing_video_reason:string|null,
and next_action:string|null. The project.state per-clip recording annotation
also includes these fields alongside take, stale, zoom_count and camera_clip.
Missing video is independent of stale (pending/applied plan mismatch). A missing
bake has an explanatory reason and next_action
`"Update video rebuilds it (screen_record.take_refresh)."`; a present bake has
false/null/null. In the Zoom card: **Video file missing. Update video rebuilds it.**
This includes Undo to an older render whose file was deleted by hand or with the cache.

Update video calls `screen_record.take_refresh{clip}`; poll jobs.status for the
final asset (initially it may be null). It bakes first, refuses a duration mismatch
over one frame, then swaps the source behind EVERY split/trimmed take clip without
resetting trim, timing, speed/ramp/freeze/reverse, effects, transform or fades.
For a missing old file the duration reference is its stored probe; otherwise the
old file is probed if present, then recording plan/source duration is the fallback.
A cached target also repairs a missing current asset reference. Earlier renders
stay in the project's recording cache until the cache is cleared. An unchanged
take reuses the original plan bytes and cache filename.

`render.final` refuses stale takes with **Update video first** unless the caller
explicitly passes `allow_stale_recordings:true` to export the old baked picture.
`edit.auto_zoom` refuses recording clips: “this recording has its own zooms: use
screen_record.zoom_*”. `screen_record.take_link` is internal to polish and refuses
direct calls.

#### Camera verbs

`screen_record.polish{source,plan,camera?:"separate"|"baked"}` defaults to separate
when a validated camera artifact exists: a bubble-free screen and visible Camera
overlay on the recorded clock, starting at the measured first-frame offset. It
preserves recorded square-bubble shape, ring, size, position and hold position
keys, which appear in exact paused frames. camera:"baked" preserves the old baked
bubble and hidden compatibility track; raw:true and Raw MP4 mode are unchanged.
Edits to a retained baked-mode camera do not change the bubble baked into video.

Identify a camera clip by `clip.recording?.camera === true`, with the original
`project.state.recordings[].camera_clip` id as fallback. The take also carries
camera_asset. Use `edit.transform{clip,x,y,scale,opacity,shape,radius?,ring?}`,
including unchanged transform fields, to change its look. x/y are top-left frame
fractions, scale is width/frame width; shape is circle, rounded or rect. The
optional ring is {width,colour}, for example {width:0.02,colour:"#ffffffe6"}.
`screen_record.camera_reset{clip}` returns {clip,transform,keyframes} in the standard
mutation envelope with `op_ids` and `project_revision`, and restores
the recorded look and placement in one undoable op, leaving trim, timing and speed
alone. Each camera change/reset supports project.undo. Update video keeps a
separate screen bubble-free. Make a copy carries each take's source, events,
plans, camera and audio inputs, and counts their bytes in the copy plan.
If named take files cannot be carried, `recordings_not_carried` in the plan
and completed job result names each take and reason; its rendered clips remain.

### 3. Edit through the transcript



**One frame is the smallest clip.** Every `edit.*` verb is checked after it
runs: an edit that would leave a media clip shorter than one frame is refused
with the nearest valid value ("a clip must be at least one frame: 16.7 ms at
60 fps", "use src_out_ms 5034 or more"). Insert, move and paste points less
than a frame from a clip edge snap to the edge (the result carries
`snapped {from, to, why}`); `edit.split` there refuses and names the existing
cut. Older sub-frame clips do not block other edits: they are listed in
`project.state.subframe_clips` and drawn in the timeline as an amber marker the
person can select and delete.

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
- Remove pauses (`transcript.remove_silences {aggressiveness, min_ms?, padding_ms?}`) —
  Choose a pause removal amount (required `aggressiveness`): Light `"calm"`
  (1200 ms minimum, 200 ms kept each end), Standard `"natural"` (700/120 ms),
  or Strong `"jumpy"` (350/60 ms).
  The preset is part of your editorial intent and belongs in the record.
- Remove filler words (`transcript.remove_fillers {lexicon?}`) — um/uh runs (default lexicon:
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
  (`jobs.status`), wait for its `enrich_job`, then `audio.add_music {asset}`. By default it drops the bed on
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
  A move has two modes, with the standard editors' names: **Overwrite**
  (`ripple:false`, default; a plain drag in the UI) replaces what is under the
  clip and moves nothing else; **Insert** (`ripple:true`; Alt-drag) pushes the
  rest of every media track right. The result's `placement` names the mode.
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

- **Every op gets a rationale.** Pass `rationale` where the verb accepts it;
  the op record's rationale field is what makes
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

The receipt and the render job result carry the verdict in one place:
`outcome` (receipt) / `result.checks` (job) = `{verdict, summary, failed[],
unmeasured[], action?}`. `verdict:"problems"` means the file was written but is
NOT the timeline (`duration_matches_edl` or `sources_decodable` failed, e.g. a
cut-off source): the job ends `completion:"done_with_warnings"`,
`outcome_reason:"checks_failed"`, message "done, but it did not pass its
checks", the file is kept, and the status bar lists it until dismissed.
`verdict:"warnings"` = only quality checks failed (job "done with warnings").
A quality check with nothing to check on this export is NOT APPLICABLE
(`details.status:"not_applicable"`, `applies_when`, `reason`; `pass:true`;
listed in `outcome.not_applicable`), never a failure or a warning:
`caption_presence` without caption clips on a visible caption track,
`cut_on_word` without a transcript, `lufs`/`silence_at_edges` without sound;
`lufs` also without a loudness target (`normalize_loudness`, measured against
it) or an explicit `profile`, `silence_at_edges` also without an explicit
`profile` (as in Premiere/Media Encoder, a plain export is not held to a
delivery standard nobody asked for; `autopilot.run` asks for both),
`black_or_frozen_frames`/`uniform_border` without picture (frozen frames do not
apply when every picture is a still). `unmeasured` is different: the check
applies but could not be measured here (reason given, e.g. captions tools not
installed). An ordinary export with no captions ends plain "done".
"No sound" and "only stills" are decided on the engine's own probe of the
placed sources at render time, never on probe facts stored in the project; a
loudness target or explicit `profile` is measured even on an export without
sound (it never reads not applicable).
`render.queue` ends done when every delivery succeeds; when some fail it ends
"done, but N of M failed" (`outcome:"partial"`, `outcome_reason:"items_failed"`,
`result.failures`), when all fail the job fails ("None of the N deliveries was
written: ..."), and with no failed delivery but one that failed its checks it
ends `checks_failed`; `autopilot.run` ends "done with warnings" when checks still fail.

`jobs.status.completion:"done_with_warnings"` is terminal and may leave optional
post-render instrumentation unmeasured. Inspect `result.verification_status`,
`result.verification_error`, and the actual receipt before judging the media.
For a terminal `state:"failed"`, `outcome` and `outcome_reason` tell a
cancellation, a project switch, a restart interruption and supersession apart
from `true_failure`.
A missing/unmeasured check is **not a pass and not a content failure**; affected
rows carry `details.status:"unmeasured"` and `details.measured:false`, preserve
the runtime cause, and never produce `fix_actions`. Verify independently or
repair the instrumentation.

Use `verify.rerun {render_id}` only when you need fresh output-only evidence for
the exact persisted artifact. It returns `{job_id, render_id, output_hash}`;
poll `jobs.status` or cancel with `jobs.cancel`. The terminal result is bound to
the source receipt id, full output hash, content type for quality checks, exact five-check set,
and its own verification-receipt path. It does **not** run `render.final`, alter
the source RenderReceipt, or infer source/caption/word-cut/current-timeline facts.

| Check | What it proves | How to read it |
|---|---|---|
| `cut_on_word` | No EDL boundary lands inside a spoken word (boundaries vs STT word spans) | Any fail names the clip + at_ms — undo the bad op (`edit.restore` if latest, else `project.revert{to}`) + re-cut via transcript verb |
| `lufs` | Integrated loudness + true peak vs target (applies only with `normalize_loudness` or an explicit content type for quality checks) | The measured number is a fact — report it even on pass (e.g. −16.2 LUFS, TP −1.8 dB) |
| `caption_presence` | Captions exist where speech exists, and cue text is sane (`repeated_word_ratio` catches doubled-word generation) | Fail → re-run `captions.generate` or inspect the caption track |
| `black_or_frozen_frames` | No dead video in the output | Evidence points at timecodes — eyeball with `render.frame {at_ms}` |
| `uniform_border` | No baked-in letterbox/pillarbox beyond tolerance | Fix the source with `edit.crop` to its perception `content_bbox`, or use deliberate `fit:"cover"` |
| `silence_at_edges` | Output doesn't start/end on dead air | Usually a missed trim at timeline edges |
| `duration_matches_edl` | Every stream spans the timeline: container, each video stream (duration and frame count) and each audio stream, measured on their own; `details.reason` names a short stream; without measured streams it is `unmeasured`, never a pass | A short video stream usually means a source that is cut off (see the asset's `probe.decode_check.warning`); report it, don't ship |

`verify.judge {render_id?, backend?}` is the perceptual visual review — a JOB
(returns `{job_id}`). Its backend union remains Claude → Codex → Antigravity →
Grok, but detection is not admission. Claude requires restricted Read capability
at version 2.1.248 or later plus a safe copied-frame workspace; Grok requires
version 1.0.21 or later with its no-model-tools policy. Codex supports
direct-frame review
when Doctor reports `judge.codex.details.judge_ready:true` after image/schema
and tool-free admission; finding the binary is insufficient. Antigravity
remains unready for render judging. An explicit unready backend returns
`status:"not_run"` with its recorded reason before review work; this does not
alter Agent Chat or generation availability. Auto keeps that configured order,
skips unready rungs, and may continue after an attempted ready-rung
infrastructure error. A named backend never falls back. `not_run` means *not
reviewed*, never *passed*; report its recorded reason and never fabricate
evidence. Doctor admission reports readiness; a completed receipt records the
actual review. A job error starting `review not run` means a project file was
refused (a linked render, a render outside the project that is not the file
its receipt recorded, an analysis file that is a link or describes another
video, or a link at `.scratch/judge/<render_id>/envelope.json`); report it and
follow its suggested action rather than retrying unchanged.

### 6. Export

- Default file-writing exports use the configured export folder
  (`project.set_output_dir`) when present, otherwise `<project>/exports/`.
  When a default target already exists, Cut writes the next available sibling
  name such as `recording-2.mp4`; explicit `path` / Save As values stay exact,
  remain fenced, and may overwrite existing export media/sidecar files.
- In the UI, the status-bar `export folder:` chip shows the current destination
  and opens Settings at the folder row. The topbar Settings button and the
  Record workspace's Export folder settings button open the same setting; per-export Save As
  stays in export controls.
- **Captions / interchange:** `export.srt {path?}` (the ONLY SRT exporter),
  `export.vtt {path?}` (WebVTT for HTML5 `<track>`), `export.xml
  {format:"fcpxml"|"premiere"|"resolve", path?}` for handoff to a traditional
  NLE, and `export.otio {path?}` for OpenTimelineIO. Before replacing a timeline,
  call `import.otio {path, mode:"preview"}` and present its track/media summary;
  pass the returned `source_hash` to `mode:"replace"` so changed bytes conflict.
  Replacement is one undoable op, preserves project format, and represents
  unavailable media as timed gaps. Replace probes media EOF and refuses source
  out-points beyond it, except exact Cut frame-rounding drift of at most 50 ms;
  that case clamps to EOF with `otio_time_clamped`. Retrim a larger or nonmatching
  out-point explicitly. `export.chapters {path?}` (markers → YouTube/podcast chapter list — pair
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
  Empty `dir` clears it (back to `<project>/exports`). An APP-WIDE remembered
  preference held by the engine (it survives a restart and applies to every
  project until cleared; NOT a timeline op, never logged/replayed).
  For one-time carry-over of an older UI choice, `only_if_empty:true` with a
  nonempty `dir` sets it only if the engine has no folder; an existing choice
  is returned unchanged. Ordinary folder changes omit this option.
  `project.get_output_dir {}` reads it (`{dir, destination: chosen |
  project_exports, usable, problem, suggested_action}`), the status bar shows the
  same value, and the `output_folder_changed` event announces each change. If
  the chosen folder is deleted or becomes unwritable, an export that would use
  it is refused BEFORE it starts (folder named, way forward: choose another or
  clear it); an explicit `path` is never blocked by it. The engine re-checks the
  folder about every 10 s and sends `output_folder_changed` when it becomes
  unusable (the chip then says "unavailable"). On Linux and macOS a folder
  everyone can write to is refused ("export folder can be written by anyone on
  this computer: <path>") unless it has the sticky bit; group-writable is fine.

Exports are derived artifacts — render receipts stay the proof of the edit
itself. Explicit `path` args are fenced to the project / outputs dir.

### 7. Seeing the app (UI verification)

**The monitor preview is a LIVE composite.** The human's monitor
composites the timeline in real time: overlay video tracks render as stacked
PiP layers at their `edit.transform` geometry + opacity, caption clips as live
text, and per-clip `edit.grade` as an approximate CSS filter — all over the
stable first non-empty base track, played smoothly (the base `<video>` is the
master clock; overlays sync to it). A hidden or gapped base stays black; hidden
overlay and caption tracks are omitted. While playing it is deliberately
APPROXIMATE (grade is CSS-approximated; gamma + 3D LUT are not shown live);
while paused the monitor shows the EXACT composed frame by default (COMPOSED on;
`ui.preview {composed}` switches it), so read `ui.state.preview` (`mode`,
`showing`, `hidden_elements`, `composed_frame`) to know which picture a
screenshot shows — exact verification stays `render.frame {compose:true}` and
`render.final`. The agent's frame-exact eyes are still `render.frame`; the live
composite is the human's editing feedback. The **"Render selection"** monitor button (calling
`export.range {range_ms, to_asset:false}`) renders the EXACT composite over the
marked ruler span or selected clips and plays it back with full audio — the
"check exactly how the final looks for this part" path, then **Save to Assets** imports it
(add a layer over a span → render it → save the clip).

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
- `render.preview {draft: true}` — **Incremental draft preview:** the WHOLE
  timeline at draft size (the project's aspect inside 960x540) showing the
  export's frames: the same source frame at every output frame as `render.final`,
  the same length, speed, reversed clips, captions and overlays. It is made of
  pieces cut at the base track's clip boundaries; after an edit only the pieces
  whose content changed are remade (a piece moved by a whole number of frames is
  reused). The result names `rendered[]` / `reused[]`. DERIVED state — never a
  receipt; the render receipt still comes only from `render.final`. Window mode
  remains available as `render.preview {at_ms, duration_ms?}` for a fast low-res clip.
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
  registered selector exists. Right tools also require their actual body to
  finish painting. A blocked/failed body returns `applied:false` and a recovery
  selector; read `ui.state.right.body_status` and use the explicit recovery
  control rather than treating the selected tab as a loaded tool.
  Unknown/unavailable targets, missing selections,
  already-current no-ops, disconnects, and timeouts fail explicitly.
  `ui.state` returns `shellx-cut/ui-state/2` with active workspace/tabs,
  overlays/dialogs, available surface ids, selection, playhead, and path-safe
  project identity. It also says what the person faces outside the panels:
  `overlays.native_dialogs` (an OS Save/Open/confirm dialog the app waits
  behind, with `opened_by` and `title`: you cannot press it, so use the verb
  that takes the path or the answer), `record_hotkey` (why global F9 is not
  registered), `export_folder {destination, usable, folder}` (where exports go,
  as the status bar shows it; `folder` is a name, `project.get_output_dir` has
  the path), `preview.media_problem` (the monitor cannot decode the video;
  its message names the package), and `notices` (what the person is being told right now:
  `action_feedback` with the `source` verb and `code` of a refusal they saw,
  `odd_project_size`, `recording_recovery`, `failed_job`). Read the wording
  from the verb or record an entry names, and say it to the person in your
  own words.
- `ui.preview {composed}` sets the preview monitor's COMPOSED mode (the
  COMPOSED button; ON by default): `true` draws the exact composed frame
  (titles, effects, fades, grades) over the paused live preview, `false` shows
  the live preview only. `ui.state.preview` says what the monitor shows:
  `mode`, `showing` (`composed_frame` | `live_preview` | `source_frame` |
  `none`), `hidden_elements` (what the shown picture leaves out at the
  playhead, e.g. `titles`, `fades`), `media` (`proxy` | `original` | null:
  the file the live preview plays for the clip at the playhead) and
  `composed_frame` (`state` off | waiting | pending | shown | failed, `at_ms`,
  `reason`, `latency_ms`, `requests`).
  Read it before judging a `ui.screenshot` of the preview.
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
# {"ok":true,"result":{"asset_id":"a1","job_id":"job_001","enrich_job":"job_enrich","op":{…}},"op_ids":["op_000002"]}

curl -s $V/jobs.status -d '{"job_id":"job_001"}'   # poll 2-5s until state is done or failed
# {"ok":true,"result":{"job_id":"job_001","kind":"import_chain","state":"done","outcome":"succeeded","progress":1.0}}

curl -s $V/jobs.status -d '{"job_id":"job_enrich"}' # wait for successful enrichment before transcript edits

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

<!-- generated:requirements:start (node scripts/generate-requirements-docs.mjs) -->
## What needs what

Every verb needs only what the requirements registry lists for it (an open project follows the verb's
`project_state`). Before a verb, `system.doctor {feature:"<verb>"}` says whether it can run and what blocks it;
a refusal for something missing carries `error.requirement {id, name, get}`. A person's licence, sign-in or
system permission is theirs: tell them what `get.person` says and never do it for them.

The full `system.doctor` report also includes fixed `availability` booleans: `pause`, `camera`,
`live_timer`, `microphone_choice`, `source_preview`, `managed_motion`. These describe this system and
executable without listing or opening devices; they are not setup or permission checks. Older engines
omit the field: missing means unknown, never unsupported. Do not query recorder devices just to learn these facts.

| Tool or condition | Needed for | Get it |
|---|---|---|
| Video processing (FFmpeg) `ffmpeg` | Import and relink media; Transcription; Silences, scenes, beats and loudness; Word edits, transcript search, chapters and generated captions; Speaker labels; Dubbing; Motion tracking and the visual search index; Remove background; Color match, auto balance and multicam audio sync; Titles, shapes and Generate templates on the timeline; Generate templates, previews and plans; Preview frames, comparisons, timeline contact sheets and scopes; Export and render; Smart reframe and directed cuts; Output checks; Render review; Stock media and b-roll; Generated media; ShellX Motion templates and tracking; Recording results and recovery | `system.fetch_tool {"tool":"ffmpeg","accept_license":true}`; Download (`ui.open {panel:"settings-video-performance"}`); before downloading, tell the person it is FFmpeg under the GNU General Public License version 3 (the consent text in system.doctor's ffmpeg card names publisher, version and size) and ask whether you may download it and accept that licence; if the person asked for the download themselves, that is the go-ahead, and your reply says which licence applied. In the app the person chooses Download or Choose ffmpeg; an ffmpeg they already have works through system.set_ffmpeg {path}; licence: GNU General Public License version 3 (https://www.gnu.org/licenses/gpl-3.0.html) |
| Media inspection (ffprobe) `ffprobe` | Import and relink media; Word edits, transcript search, chapters and generated captions; Color match, auto balance and multicam audio sync; Titles, shapes and Generate templates on the timeline; Preview frames, comparisons, timeline contact sheets and scopes; Export and render; Smart reframe and directed cuts; Output checks; Render review; Stock media and b-roll; Generated media; ShellX Motion templates and tracking; Recording results and recovery | `system.fetch_tool {"tool":"ffmpeg","accept_license":true}`; Download (`ui.open {panel:"settings-video-performance"}`); comes with FFmpeg: the same download, licence and question as FFmpeg; licence: GNU General Public License version 3 (https://www.gnu.org/licenses/gpl-3.0.html) |
| Stabilization in FFmpeg (vid.stab) `ffmpeg_vidstab` | Stabilize | `system.set_ffmpeg`; Choose ffmpeg (`ui.open {panel:"settings-video-performance"}`); a complete FFmpeg build is needed (on a Mac: Homebrew's ffmpeg-full); the steps are in system.doctor's ffmpeg card, details.setup |
| Caption burn-in in FFmpeg (libass) `ffmpeg_libass` | Export and render (optional) | `system.set_ffmpeg`; Choose ffmpeg (`ui.open {panel:"settings-video-performance"}`); a complete FFmpeg build is needed; the steps are in system.doctor's ffmpeg card, details.setup |
| Color management in FFmpeg (zimg) `ffmpeg_zimg` | Project color management (optional); Preview frames, comparisons, timeline contact sheets and scopes (optional); Export and render (optional) | `system.set_ffmpeg`; Choose ffmpeg (`ui.open {panel:"settings-video-performance"}`); a complete FFmpeg build is needed; the steps are in system.doctor's ffmpeg card, details.setup |
| Faster exports (GPU encoder) `gpu_encoder` | Export and render (optional) | `system.set_ffmpeg`; Choose ffmpeg (`ui.open {panel:"settings-video-performance"}`); needs a supported graphics card, its driver and an FFmpeg built with its encoder; the steps are in system.doctor's gpu-encode card, details.enable_help |
| GPU render path (CUDA) `cuda_render` | Export and render (optional) | `system.set_ffmpeg`; Choose ffmpeg (`ui.open {panel:"settings-video-performance"}`); needs an NVIDIA GPU with its driver and an FFmpeg with CUDA filters |
| Captions and transcription `captions_runtime` | Transcription; Silences, scenes, beats and loudness; Translation of captions and transcripts; Speaker labels; Dubbing; Motion tracking and the visual search index; Smart reframe and directed cuts; Output checks; Import and relink media (optional); Redaction (blur or cover a region, faces, text) (optional); Visual search (optional); Export and render (optional) | `system.setup_perception`; Install captions (`ui.open {panel:"settings-ai-transcription"}`); a download of several hundred MB that needs no Python of the person's own. On macOS and Linux it needs no administrator permission; on Windows Microsoft's Visual C++ runtime must be installed first (its own Install in the same Settings page), and Windows asks for administrator permission for that step |
| Speech-to-text model `speech_model` | Transcription | `system.setup_perception`; Install captions (`ui.open {panel:"settings-ai-transcription"}`); comes with Install captions; system.set_stt_model chooses the model |
| Microsoft Visual C++ Redistributable `vc_runtime` (windows) | Transcription; Silences, scenes, beats and loudness; Motion tracking and the visual search index; Smart reframe and directed cuts; Setup and environment | `system.fetch_tool {"tool":"vc-runtime"}`; Install (`ui.open {panel:"settings-ai-transcription"}`); Microsoft's own installer from Microsoft (aka.ms/vc14), checked for Microsoft's signature; Windows asks for administrator permission; or install it from https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist; licence: Microsoft Software License Terms for the Microsoft Visual C++ Redistributable (https://aka.ms/VCRedistLicense) |
| Background removal `matte_runtime` | Remove background | `system.setup_matte`; Install (`ui.open {panel:"settings-ai-transcription"}`) |
| Premium background removal `matte_premium` | Remove background (optional) | `system.setup_matte {"model":"matanyone","accept_noncommercial":true}`; Install premium (`ui.open {panel:"settings-ai-transcription"}`); the premium model is licensed for non-commercial use only (NTU S-Lab License 1.0): tell the person and ask before installing; accept_noncommercial:true states their yes. It needs an NVIDIA GPU; licence: NTU S-Lab License 1.0 (non-commercial) |
| Render review (a signed-in agent CLI) `review_judge` | Render review | the person installs the CLI and signs in to it themselves; Cut never signs in for anyone. Then Re-scan (system.doctor {refresh:true}) |
| Agent CLI for chat and translation `chat_cli` | Translation of captions and transcripts; Agent Chat | the person installs the CLI and signs in to it themselves; Cut never signs in for anyone. Then Re-scan (system.doctor {refresh:true}) |
| Media generation provider `generation_provider` | Generated media; Generate templates, previews and plans (optional) | the person installs the provider's CLI and signs in themselves; Cut never signs in for anyone |
| Dubbing service `dub_service` | Dubbing | start the dubbing service and set its address (CUT_DUB_ENDPOINT); see system.doctor's dub card |
| Speaker labelling service `diarize_service` | Speaker labels | start the speaker labelling service and set its address (CUT_DIARIZE_ENDPOINT); see system.doctor's diarize card |
| ShellX Motion `motion_runtime` | ShellX Motion templates and tracking | `system.motion_status`; ShellX Motion is installed separately |
| An open project `open_project` | Project, sequences, history and timeline edits; Project color management; Import and relink media; Transcription; Silences, scenes, beats and loudness; Edits from the analysis (scenes, beats, silences, emphasis, speaker energy); Word edits, transcript search, chapters and generated captions; Translation of captions and transcripts; Speaker labels; Dubbing; Motion tracking and the visual search index; Redaction (blur or cover a region, faces, text); Visual search; Remove background; Color match, auto balance and multicam audio sync; Stabilize; Titles, shapes and Generate templates on the timeline; Generate templates, previews and plans; Preview frames, comparisons, timeline contact sheets and scopes; Export and render; Smart reframe and directed cuts; Output checks; Render review; Stock media and b-roll; ShellX Motion templates and tracking; Screen recording; Recording results and recovery; Voiceover | `project.open`; open or create a project (project.open {path} or project.create {name}) |
| A usable export folder `export_folder` | Export and render; Recording results and recovery | `project.set_output_dir`; choose another export folder, or clear the choice to use the project's exports folder |
| Free disk space `disk_space` | Setup and environment; Export and render (optional) | `project.cache_purge`; free space on the drive; Settings > Health & recovery can preview and remove Cut's aged cache files (Preview cache cleanup) |
| A transcript of the asset `transcript` | Word edits, transcript search, chapters and generated captions; Translation of captions and transcripts; Dubbing; Output checks | `media.transcribe`; a transcript is made after a clip is imported while Captions and transcription is installed. A person can select a clip and use Transcript Tools > Transcribe clip or Transcribe again. Repeating transcription replaces the previous transcript after confirmation in the app. |
| An analysis of the asset `analysis` | Edits from the analysis (scenes, beats, silences, emphasis, speaker energy) | `media.perception`; silences, scenes, beats and loudness are measured after a clip is imported while Captions and transcription is installed. For a clip imported before that, the recipes First edit and Edit for clarity measure it; there is no separate button |
| Screen recording permission `screen_permission` | Screen recording | a person must allow it: macOS System Settings > Privacy & Security > Screen & System Audio Recording, then reopen ShellX Cut; on Linux the screen-sharing portal asks |
| Microphone access `microphone_permission` | Voiceover; Screen recording (optional) | a person must allow it in the system's privacy settings (Windows: Settings > Privacy & security > Microphone; macOS: Privacy & Security > Microphone) |
| System sound capture `system_audio` | Screen recording (optional) | a person must allow it (macOS asks separately for Audio Capture) |
| The editor window `ui_client` | Driving the editor window | open the app; system.doctor gives the local address to open in a browser |
| Internet connection `network` | Stock media and b-roll; Setup and environment; Visual search (optional) | connect to the internet and retry |
<!-- generated:requirements:end -->
