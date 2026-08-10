---
name: shellx-cut
description: Use when editing video with ShellX Cut or its cutd server — video edit, cut video, trim footage, remove silences, remove filler words, transcript editing, text-based video editing, captions/SRT, talking-head or screen-demo cleanup, render receipt, verify a render, export FCPXML, shellx cut, cutd. Covers driving the verb API (REST + MCP) headless and verifying results with receipts.
---

# ShellX Cut — agent-first video editing

> **Engine v0.6.107.** Synced to the contract (`schema/verbs.json` — the single
> machine-readable source of truth; if this guide and that file disagree, trust
> the file): **262 verbs across 32 domains** under the public verb contract.
> **`reference.md` is the full 262-verb table —
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
>   key by speaker).
> - **AI dubbing + subtitle translation** — `audio.dub` (re-voice an asset's
>   speech into another language in a cloned voice, time-fit to the original,
>   added as a NEW audio track; reuses `transcript.translate` + the OmniVoice TTS
>   service). `transcript.translate` / `captions.translate` are TEXT-only
>   translation (CLI-primary, local Opus-MT/MADLAD fallback — no dubbing).
> - **Agent chat (natural-language editing)** — `agent.chat` launches the user's
>   installed Claude Code or Codex CLI. Claude uses Cut's pinned `2.1.224`
>   contained contract. Codex keeps the user's normal configuration, native
>   sandbox, and permissions; Cut adds its filtered MCP server without copying or
>   rewriting Codex login files. Grok is planned for the next release. Every Cut
>   verb applied by either route is a normal reversible op.
>   `attachments` can carry up to eight registered project asset IDs as references;
>   the server validates them against the open project and exposes no arbitrary
>   source-path input. Each launched turn returns `plan` plus a `review` artifact
>   with its pre-turn baseline, post-turn tip, computed diff, and a `revert_safe`
>   verdict. Agent ops carry a unique per-turn actor. Concurrent human/system ops
>   are listed separately and disable whole-turn revert. The UI uses those exact
>   refs for Preview, Diff, shared Accept markers, tip-guarded atomic Revert, and
>   retry-prefill, so later edits make a stale revert refuse instead of rolling back.
>   The Agent Chat prompt library offers eight curated Polish, Repurpose, Speech,
>   and Review requests mapped only to existing verbs/recipes. Choosing one merely
>   pre-fills the editable composer; it never launches a CLI turn by itself.
> - **Scoped plugins (agent-only)** — `plugins.list`, `plugins.enable`, and
>   `plugins.call` expose the built-in Openverse-assets and matte-runtime scopes
>   as a permission fence over the SAME verb registry. A disabled, out-of-scope,
>   or corrupt/unavailable-state call fails closed; this is not a second
>   extension API. Inspect `plugins.list` for recovery guidance; an explicit
>   `plugins.enable {name,enabled:true}` repairs only that named grant.
> - **Native editable Generate** — `generate.list` / `generate.describe` /
>   `generate.preview` / `generate.insert` / `generate.from_prompt` /
>   `generate.storyboard` power the editor-side Generate tab: reusable
>   templates, prompt-planned editable visuals, non-mutating PNG previews,
>   undoable timeline inserts, and multi-scene storyboard IR. The
>   prompt/storyboard PLANNER ships as bundled adapters that route to the
>   user's LOCAL CLI subscription agent (claude/codex/grok, `agent:"auto"` =
>   first installed; no CLI → honest `not_run`) — an agent driving the verbs
>   can either call them directly (the engine plans) or produce the plan/IR
>   itself per `craft/generate-storyboard-planning.md`. Motion-backed
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
>   Canvas Motion Studio through **Edit in Motion**, then refresh the same linked
>   Cut clip. Fog, water, keying, matte cleanup, and tracked-callout motion remain
>   Motion-owned rendered effects rather than fake native Cut controls.
>   Rendered Motion imports retain a stable `motion_link` on the live Cut clip:
>   `clipId` is the editorial identity, `packageId`/`motionId` are the source
>   identity, and the attested render digest is replaceable derived media. The
>   timeline `M` badge and Inspector link section expose this state; do not imply
>   rain, water, snow, shaders, 3D, particles, Motion blur, or film controls are
>   native Cut edits. Open those controls and their curves through
>   `motion.link.edit`, render the edited copy-on-write revision in Canvas, then use
>   `motion.link.refresh` to update the same Cut clip. Cut creates a path-private
>   return request for the launch; Canvas publishes an immutable ready descriptor
>   only after a verified render, and refresh rechecks identity plus source revision
>   before adopting it.
>   Use `motion.link.relink` to repair a missing local package only after its
>   package/motion identity matches. Use `motion.link.refresh` to render a new
>   immutable project-owned artifact and atomically replace the same Cut clip;
>   receipt/digest/source races fail without disturbing the last good render.
>   `motion.link.edit` opens that same identity in Canvas Motion Studio through
>   its `--motion-package` host intake and trusted `--motion-cut-return-request`
>   handback; configure `SHELLX_CANVAS_BIN` if needed. Never invent or expose either
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
>   own generation CLI. Generation accepts up to four registered image/video
>   references and an explicit variation label; `assets.generated_list` exposes
>   a path-light, integrity-checked project history for reference and retry.
> - **Recipe layer** — `recipe.list` / `recipe.describe` / `recipe.run`:
>   declarative, gated pipeline MANIFESTS over the existing verbs (built-ins
>   in `schema/recipes.json`: `first-project`, `edit-for-clarity`,
>   `podcast-repurpose`, `talking-head-cleanup`,
>   `screen-demo-polish`, `phone-clip-cleanup`, `social-short-bundle`,
>   `area-privacy-mask`, `add-captions`, `youtube-export`, `tiktok-export`).
>   `run` is a pure orchestrator — one auto-checkpoint,
>   per-stage gates, stops + reports on the first failed verb or gate.
> - **Recording Studio** — the `screen_record.*` domain wires the integrated
>   Cut recorder crates in process: `doctor` / `start` / `stop` /
>   `recovery_status` / `studio_event` / `autoedit` / `polish` / `export` (live screen/audio
>   capture, raw streams, auto-edit plan, content-addressed bake). Live camera
>   capture is unavailable in this release; camera composition accepts an
>   existing project-local camera file only.
>   Doctor health stays strict: on Linux `start_allowed:true` means only the
>   deliberate prompt-deferred XDG ScreenCast portal card may enter the
>   user-initiated source picker; it does not make `ready` true, and missing,
>   degraded, or other unknown required cards still refuse `start`.
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
> - **Director / reframe / delivery** — `render.reframe` (subject-tracked moving
>   crop to a platform aspect — the HONEST alternative to a static centre-crop),
>   `render.direct` (director-model pass: a per-scene contact sheet the foundation
>   model reads to choose WHICH subject each shot is about, fed back into
>   `render.reframe{direction}`), `render.queue` (batch delivery), `render.bundle`
>   (social repurposing pack), `verify.pregate` (PRE-render predictive quality
>   gate, no render spent).
> - **Repurposing / assembly** — `assemble.repurpose` (auto-highlight selection),
>   `assemble.shorts` (vertical-short planner), `assemble.from_script`
>   (script→timeline matching), `assemble.broll` (slot→retrieve→place),
>   `clip.candidates`, `score.clip` (engagement score, model-free).
> - **AI matte (no green screen)** — `edit.matte` (+ `system.setup_matte`):
>   background removal/replace, RVM auto default or premium target-assigned
>   MatAnyone2 (SAM2 click-to-pick subject).
> - **Advanced color** — the `grade` gallery (`grade.save`/`apply`/`list`),
>   `edit.grade_stack` (layered grades), `edit.grade_window` (power window),
>   `project.color` / `edit.color_space` (working/output/input colour management).
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
default; `SHELLX_CUT_ALLOW_NON_LOCAL=1` changes only the bind check and does not
make Cut authenticate a remote caller. Remote use is supported only through an
independently authenticated and authorized SSH/VPN/ShellX broker or equivalent
transport; without it, refuse remote access. Do not use shared/multi-user
machines, untrusted local services, host-network containers, or exposed ports
for this mode. The contained Claude `agent.chat` route narrows that provider's
tools; it does not change local REST/MCP authentication. Read
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
- Antigravity CLI currently has no `agy mcp add`
  subcommand. Add
  `{"mcpServers":{"shellx-cut":{"command":"/absolute/path/to/cutd","args":["mcp"]}}}`
  to `~/.gemini/config/mcp_config.json` or `.agents/mcp_config.json`, then use
  `/mcp` to inspect or reload it. Headless `agy --print` auto-denies permission
  prompts; put only exact unattended grants such as
  `mcp(shellx-cut/system_mcp_test)` under `permissions.allow` in
  `~/.gemini/antigravity-cli/settings.json`. Do not use a global MCP wildcard or
  `--dangerously-skip-permissions` just to test Cut.

For every client, call `system.mcp_test {}` through the configured MCP server as
the final proof of protocol negotiation, ping, all 262 tools, and same-engine
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
returns the full materialized timeline (assets, tracks, markers, checkpoints). Drop a
checkpoint before any editing pass: `project.checkpoint {name:"pre-edit"}`.

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

For multi-sequence projects, use `project.sequence_index {query?, kind?,
sequence?, track_kind?, status?, limit?}` to search clips and markers across
active and inactive timelines without switching through them. `status` can
isolate `issues`, `offline`, `gaps`, `effects`, `hidden`, `locked`, or `muted`;
offline is checked live, and issue rows never reveal source paths. Results carry
stable sequence/track/item ids, timeline ranges, effect names, and track state.
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

**First import auto-places**: on an empty timeline the chain inserts the asset
onto v1/a1t as real system-actor `edit.insert` ops — import and start editing.
Later imports (b-roll) are NOT placed; add them with explicit `edit.insert`.
For human UI placement, Assets **Insert** and normal timeline drops target the
base story timeline with ripple. Use an explicit overlay path only when the clip
should sit above the base picture: Alt-drop/new overlay lane, drop on an
existing overlay lane, or `edit.add_track {kind:"video"}` followed by
`edit.insert {track:<overlay>, ripple:false}`. Linked audio for a placed video
lands on an audio track; the video insert owns the base ripple so the linked
audio is inserted into the opened gap without a second ripple.
To preview an unused timed asset before placement, open its **Source monitor**
from Assets, seek and mark In/Out, then choose **Insert range**. The monitor
prefers a ready editing proxy so large or host-unsupported source codecs stay
auditionable, with an explicit keyboard-accessible Play/Pause control. This
uses one source range for both picture and linked audio at
the current timeline playhead; it does not change Program playback while
auditioning the source.
Find moment results are source-relative. Use the result's **Timeline** action to
jump to the nearest real occurrence after trims, gaps, reuse, constant speed, or
reverse; use **Source** to inspect the indexed frame directly. An unused asset or
a variable-speed ramp has no honest direct timeline jump and stays Source-only.

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
and a deliberate Continue button. The warning's Guide action opens the online
manual anchor `cut.export.preflight`.

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
(`edit.split`, `edit.ripple_delete`, `edit.trim`, `edit.move`, `edit.insert`,
`edit.gain`, `edit.speed` (per-clip retime / slow-mo, pitch-preserved, 0.25–4×),
`edit.grade` (color), `edit.add_marker`, `edit.remove_marker`,
`edit.move_marker`) exist for non-speech work — but if the cut is about *what
was said*, use a transcript verb so the word-boundary guarantee holds.

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
- Checkpoint between passes (`project.checkpoint`), and **always run
  `project.diff {from: last_checkpoint, to: "now"}` before rendering** — read the summary (clips
  added/removed/moved, `duration_delta_ms`, `tracks_touched`) and confirm it
  matches what you intended to do. A diff you can't explain means stop and
  inspect `project.ops {since}`.
- **Acting on human review notes (the `comment` loop).** A reviewer
  leaves timecoded notes with `comment.add {at_ms, text, end_ms?}`; list the
  open ones with `comment.list {status:"open"}`. New comments may carry
  `anchor:{track_id,clip_id,offset_ms}`; resolve that against current
  `project.state` if content was rippled, and treat a missing clip as a stale
  note rather than seeking blindly. For each, `comment.draft {comment_id}` asks
  the drafting agent to PROPOSE a concrete editor-verb change set (stored on the
  comment, not yet applied), then `comment.apply {comment_id}` executes it as
  real ops — wrapped in an auto-checkpoint so the whole change reverts in one
  step (`project.revert {to: <returned checkpoint>}`) and returns the `diff` for
  the reviewer. `comment.resolve {comment_id, status}` closes the loop
  (`addressed`/`dismissed`). Comment ops are review metadata, not timeline edits
  — outside the undo stack. For an external handoff, render the current cut and
  run `comment.export {}` to create an offline HTML reviewer beside a verified
  render copy. Import the reviewer's downloaded JSON with
  `comment.import {path}`; Cut verifies its render hash/source op and appends the
  entire feedback batch atomically. Later comment/preset/name metadata does not
  stale unchanged rendered bytes; a later render-affecting edit is rejected by
  default and requires `allow_stale:true` plus a recorded `rationale`.

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
(returns `{job_id}`). The bundled access ladder uses the first working
subscription CLI in the order Claude → Codex → Antigravity → Grok; a named
backend forces that rung. In auto mode, a present rung that fails an
infrastructure check is recorded and the next detected rung is tried. With no
usable CLI or adapter Python runtime, the job completes with
`{status: "not_run", reason}` — that means *not reviewed*, never *passed*.
Report it as "judge: not_run" with the recorded reason. Treating a not_run
result as a pass is fabricating evidence.

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
  (default 540). It omits captions/overlays. When you need the EXACT composed
  frame for verification (captions burned in, PiP composited, project geometry),
  pass `compose: true`. Raw bytes also at `GET /api/frame?at_ms=[&h=][&compose=1]`
  (the `X-Cut-Frame-Fast` header says which path served it).
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
