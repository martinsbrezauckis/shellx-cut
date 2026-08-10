# The debug API — REST, WebSocket, and MCP

Role: the single-page operator reference for driving ShellX Cut from outside
the UI — every endpoint, the security model, and MCP client setup. The verb
catalog itself lives in `schema/verbs.json` (contract) and
`skill/shellx-cut/reference.md` (the full per-verb argument reference).

## Starting the server

```bash
cutd serve --project x.cutproj            # REST + WS + UI at http://127.0.0.1:6161
cutd serve --headless                     # API only (UI optional, attach any time)
cutd serve --addr 127.0.0.1:6169 …        # non-default port (loopback only)
cutd mcp                                  # MCP over stdio — proxies the running serve
cutd verb project.state '{}'              # one-shot CLI escape hatch (no server needed)
```

The desktop app runs the same `cutd` internally — an agent can drive the
installed app and a headless dev server identically.

**Port discovery:** on startup `cutd` writes its bound address to
`engine.addr` in the app-data dir and removes it on graceful shutdown.
Clients (MCP proxy, CLI) read this file first and fall back to `127.0.0.1:6161`.

| OS | discovery file |
|---|---|
| Linux | `$XDG_DATA_HOME/ShellX Cut/engine.addr` (default `~/.local/share/ShellX Cut/engine.addr`) |
| macOS | `~/Library/Application Support/ShellX Cut/engine.addr` |
| Windows | `%LOCALAPPDATA%\ShellX Cut\engine.addr` |

## Security model — loopback-only, no token (by design)

There is **no API token or auth header**. The supported default is **one
personal workstation / one trusted interactive environment**. Its trust
boundary is the **whole local machine**, not a same-user or per-process
authentication boundary:

- `cutd` **refuses to bind a non-loopback address** (`0.0.0.0`, LAN IPs, `::`).
- Any local process or OS account that can connect to that loopback TCP port can
  operate the open editor. A native caller can omit `Origin` and can forge
  `Origin` or `Host`; loopback TCP does not report a caller identity.
- Browser-driven cross-origin and DNS-rebinding requests are rejected by an
  Origin + Host guard on every request (a non-loopback `Origin` or `Host` gets
  403). Those headers mitigate browser attacks; they do **not** authenticate
  native local callers.
- Native LAN/public listening is unsupported and refused by default.
  `SHELLX_CUT_ALLOW_NON_LOCAL=1` changes only the bind check: Cut does not add
  or verify a remote token, capability, or identity. Remote use is supported
  only through an independently authenticated and authorized SSH/VPN/external
  ShellX broker or equivalent transport. That protection belongs to the
  transport and must be separately evidenced; without it, remote access must be
  refused. Directly exposing the port publishes the full mutation surface.
- Shared/multi-user machines, untrusted local apps/services, containers sharing
  host networking, and exposed ports are outside the supported default. Native
  per-caller/per-user capability authentication is future hardening; it is not
  in v0.6.108. Under this documented deployment assumption, its absence is
  **NOT A DEFECT**.

`cutd mcp` is a stdio transport that proxies the running server; it has no
additional caller authentication and inherits this machine-wide boundary. The
contained Claude `agent.chat` broker separately limits that provider's native
tools and allowed Cut verbs; it does not make REST/MCP per-user authenticated.
See the [local-machine threat model](shellx-cut-threat-model.md) for the
repository-grounded assets, abuse paths, and residual risk.

## Endpoint catalog

All verbs go through one route; the rest are read-side support surfaces.

| Route | Method | What it serves |
|---|---|---|
| `/api/verb/{name}` | POST | dispatch any of the schema's verbs; body = args JSON; returns the envelope `{ok, result?, op_ids?, project_revision?, warnings?[], error?{code,message,cause,…}}` |
| `/api/state` | GET | current project/timeline state snapshot |
| `/api/verbs` | GET | the live verb registry (generated from `schema/verbs.json`) |
| `/api/events` | GET (WS) | event stream: `op_applied · job_progress · render_done · receipt_ready · project_changed · ui_state · doctor_updated`. `op_applied` carries `revision`, `from_revision`, and `{delta:{kind:"op",count:1}}`; clients repair a missed frame through bounded `project.state{since_revision}` deltas or an explicit snapshot fallback. (`project_changed` refreshes visible clients after REST/CLI/MCP create, open, or close; `doctor_updated` refreshes environment capabilities; agents key on `receipt_ready`) |
| `/api/frame?at_ms=` | GET | composited frame at a timeline position — the agent's eyes |
| `/api/agent` | GET | `shellx-cut/agent-docs/2`: machine-readable API/docs discovery, exact running executable, MCP proxy/standalone metadata, copyable client config, and self-test contract |
| `/api/agent-doc/*path` | GET | serves the agent docs (e.g. `skill/shellx-cut/SKILL.md`) over HTTP for installed-app onboarding |
| `/api/export/*path` | GET | download rendered artifacts by PROJECT-RELATIVE path, resolved against the open project's `exports/` subtree. Refuses (409) when the same relative name also matches a different file in the chosen output folder — an ambiguous request is never answered with a guess |
| `/api/export-file?path=` | GET | download ONE exact export by absolute path — the shape that can name a file in the folder chosen with `project.set_output_dir`. Fenced to the authorized export roots (project `exports/` subtree, `CUTD_OUTPUTS_DIR`, the chosen output folder); a path that is missing or outside them is refused, never substituted |
| `/api/source/{asset}` | GET | stream a registered asset's original source (seekable, chunked; fenced to the asset registry) |
| `/api/library-blob/{file}` | GET | fenced blob serving for library-stored media |
| `/api/library-poster?id=` | GET | library item poster/thumbnail (id-fenced via the library manifest) |
| `/proxies/{file}` · `/frames/{file}` · `/filmstrip/{file}` | GET | preview proxies, extracted frames, and timeline filmstrips from the current project dir |
| `/` | GET | the UI (when `ui/dist` was built; the API works headless regardless) |

### Project-sync reconnect rule

WebSocket delivery is best effort. On reconnect, request a revision delta from
`project.state{since_revision}` and use the bounded `project.ops` cursor pages
only when complete durable history is required. A `no_project` error from that
state request is authoritative confirmation that the project was closed: discard
every cached cursor and in-flight page, then reset project-scoped UI state before
showing Projects. A transport or other transient error is not proof of closure;
keep the cached workspace and retry rather than falsely erasing it. REST, CLI,
and MCP clients share these same verb/error semantics.

### Health & Recovery read

`project.health {cursor?, revision?, limit?:1..128}` is the separate, read-only
filesystem check for Settings → Health & Recovery. It first strictly validates
the live journal identity. If that validation finds an external change or is
unavailable, it returns an honest `journal.status:"attention"|"unavailable"`
report with `media.status:"unavailable"`, zero checked assets, and no
`project_revision`; close and reopen the project before trying again.

When the journal is current, the first page returns the opaque
`project_revision`; every continuation must send that same `revision` plus the
previous `next_cursor`. Each response checks at most 128 registered assets in
stable asset-id order and contains only path-free source/proxy/filmstrip state
and page counts. A client must aggregate every revision-bound page before
claiming a whole-project healthy result. The first page also includes a bounded,
path-free `editing_cache` inventory for only rebuildable `proxies/` and
`filmstrip/` thumbnail files. It reports apparent bytes, file counts, recognized
outputs no longer referenced by current asset metadata, and the latest
cache-file change time; it does not mean "last used." Only the product's flat
proxy/base-strip/window-strip filename forms are counted. Symlinks and
unexpected directories are never followed, foreign files make the scan partial,
and exports, captures, receipts, and source media are excluded. Reclaimable
counts are informational only. A nested `cleanup_preview` separates files that
have not changed for at least 24 hours from newer unreferenced files, and blocks
when the bounded scan is partial. File-change age is not last-use evidence and
does not prove that a producer is inactive; a future cleanup must revalidate the
same project revision and active jobs. This verb never repairs, relinks, deletes,
promotes, purges cache files, or follows an unregistered derived path. Job-record
persistence notices remain on `jobs.list`; capture-recovery inventory is
separately exposed by `screen_record.recovery_status`. Settings → Health &
Recovery reads that inventory independently of `screen_record.doctor`, using
only one complete lexical traversal for its capture result. Because the recorder
API has no revision, its UI copy says the evidence was reported/read in that
check, not that it is a timeless snapshot. A malformed, partial, or failed
inventory is attention, never a capture-health pass, and the page offers no
repair action.

### Executable argument contract

Every public verb's `args` entry in `schema/verbs.json` is an executable JSON
Schema Draft 7 contract, not documentation-only metadata. The server compiles
all 264 schemas once at startup and applies the selected schema at the shared
dispatch boundary. Direct/internal dispatch, REST, `cutd verb`, and
`cutd mcp` therefore reject the same malformed input before a handler runs.

Every live input schema also includes optional mutation controls:

- `request_id` is a caller-generated retry identity. When a call emits project
  ops, Cut persists the caller, request ID, and canonical payload fingerprint
  with those ops and writes an atomic response receipt before replying.
- `expected_revision` requires `request_id` and must match the latest durable
  op ID exposed as `project_revision`. Cut checks it before work and again at
  the journal append boundary.
- Repeating the same caller/request/payload returns the original envelope and
  op IDs. Reusing the ID with changed input conflicts. If an op committed but
  its response receipt did not, Cut reports the committed op IDs and refuses to
  duplicate the mutation.

Validation failures use the normal `invalid_args` envelope and identify the
verb, exact JSON Pointer, failed keyword, concise constraint, and recovery:

```json
{
  "ok": false,
  "error": {
    "code": "invalid_args",
    "message": "invalid args for verb 'ui.playhead' at '/at_ms' (at_ms): minimum",
    "cause": "schema keyword 'minimum' failed: use a number greater than or equal to 0",
    "suggested_action": "correct '/at_ms' to use a number greater than or equal to 0; GET /api/verbs shows the exact input schema"
  }
}
```

Use JSON values with the declared types; stringly booleans and numbers are not
coerced. Handler-level checks still enforce project-dependent rules after the
schema passes. Errors never echo argument values, and unknown-property names
are bounded.

### Executable behavior contract

Every registry entry also carries `behavior` metadata generated and validated
with the schema: one `mutation_class` (`read`, `project_metadata`,
`asset_metadata`, `timeline`, `navigation`, or `external_side_effect`), a
`project_state` (`none`, `optional`, or `required`), an internal `dispatch`
target, plus `idempotency`, `replayability`, `async_job`, `ui_exposure`,
`agent_chat`, `risk`, and UI facets. `timeline` is the only class that advances
history; workspace switching is `navigation`, and operations outside normal
project history are `external_side_effect`.

`idempotency` is `request_key` only for a durable operation whose persisted
caller/request key can deduplicate retries; `natural` is safe repeated
inspection or derivation; `not_applicable` is a passive projection with no
operation identity; and `none` is a probe, provider call, or output-producing
action that Cut must execute independently. `replayability:"replayable"` is
reserved for durable project metadata, asset metadata, or timeline operations;
reads are never journal-replayed. `async_job` names a job this call starts or
owns, not a `job_id` it merely reports (`jobs.status` and `jobs.list` are
reads). `ui_exposure` distinguishes human, agent-only, internal, and rig-only
verbs. `risk` summarizes impact rather than I/O: `none` has no durable or host
impact, `reversible`/`destructive` describe durable mutations, and `external`
means a non-history provider, process, OS permission, or fenced-output action.
Facets are small generated labels for shared UI projections.

`side_effects` is deliberately literal about direct bounded engine
interactions: `filesystem` means the handler reads or writes a project,
registered-asset, index, or fenced output path; `process` means it starts a
local helper such as ffmpeg; `network` means it contacts a provider or remote
service; `ui` means it invokes a UI/desktop bridge or an OS permission-prone
native probe. These flags are not a
mutation-only label. For example, `media.check` reads registered source-file
metadata, and `edit.color_match` reads registered clips and runs ffmpeg while
still committing a replay-safe grade.

`agent_chat` is a separate broker capability, not an assertion that a Cut
handler is pure. It controls whether the contained Claude turn can discover or
call that verb; the broker still limits calls to the open project and registered
asset IDs. A permitted bounded edit may therefore accurately declare
`filesystem:true` or `process:true`, while arbitrary provider-native file,
shell, network, and other MCP access remains denied. The generated core
contract rejects an unknown journal verb instead of guessing that it is an
undoable timeline edit. The generated dispatcher target is not a caller option;
it makes the schema name-to-handler route exhaustively checked at build time.

Release installers include the start-here guide, agent rules, public feature and
Debug API docs, verb schema, feature workflow, Motion boundary, and the complete
ShellX Cut skill directory (reference plus every craft guide). Package checks
verify that every served `/api/agent-doc/*path` file is
byte-identical to the candidate source, preventing a stale or partial docs bundle.

Long-running verbs return `{job_id}` immediately — poll `jobs.status`, list via
`jobs.list`, abort via `jobs.cancel`. Cancellation does not claim success until
tracked blocking workers and their synchronous child processes have finished.
If that bounded drain is still in progress, `job_cancel_pending` asks the
caller to wait and retry. A project switch uses the same fail-closed boundary:
the next project is not attached while an old worker is alive. File-writing
verbs are fenced to the project/export directories (schema the output-fencing contract).
`JobRecord.state` remains compatible (`queued`, `running`, `done`, `failed`).
Active records also retain the latest optional human-readable `message` reported
by the worker, so `jobs.status`/`jobs.list` can restore a current phase after a
reload without waiting for another event. Older queued or persisted records may
omit it. A limited queued job also carries
`queue:{resource,max_running}` while it waits for shared local capacity. A job
orchestrating another active job may also carry `waiting_on:{job_id,kind}` only
for the child it currently awaits; this is relationship evidence, not a retry
promise. `queue` clears when its slot is acquired, `waiting_on` clears when the
child returns, and both clear when the owning job becomes terminal. Clients can
explain a wait without guessing from a zero progress value.
New terminal records also report `outcome` and `outcome_reason`: a user cancel,
project-switch cancel, restart interruption, supersession, and true failure
stay distinct even though non-success outcomes retain `state:"failed"` for
existing clients. Older persisted records omit these fields.
Job JSON is written atomically. On project reopen, a malformed job record is
kept under the project's `jobs/quarantine/` folder and reported through
`jobs.list.result.persistence_notices`; it is never silently ignored or reused
as a future job ID.

Job-owned external workers (translation local/CLI, dubbing, diarization,
Generate and draft adapters, judge review, Motion CLI commands, Agent Chat,
generated-media providers, and Motion artifact validation) share an
operation-wide deadline and cancellation signal. `render.final`,
`reframe.render`, and `reframe.direct` also pass one two-hour render-wide
control into every cut-media ffmpeg phase, including stabilization, segmented
windows, concat, and mux. Their stdout and stderr are drained while retained
diagnostics are capped; cancellation closes stdin, requests a graceful stop,
hard-stops the owned tree, and waits for the leader before a terminal job outcome
is published. Unix workers run in a new process group; Windows workers are
suspended, assigned to a kill-on-close Job Object, then resumed, so an eager
child cannot escape the ownership claim. `cancelled_by_user`, `project_switch`,
`restart`, and `superseded` remain distinct terminal reasons even when a worker
observes the stop before the caller returns from `jobs.cancel`.

All Cut-owned finite foreground tools use the same bounded tree owner: media
probe/analysis/hardware checks, doctor checks (including the recorder doctor's
two-second login/session, ffmpeg, and GStreamer probes), the MCP self-test, Claude
capability checks, Python perception helpers, consented setup, archive helpers,
and finite Screen Record ffmpeg work. The shared foreground budget is 30 minutes
unless a shorter probe budget is documented by its caller; JSON sidecar stdin is
written under that same cancellation/deadline boundary, and perception progress
lines are streamed while bounded diagnostics are drained. The only intentionally
independent process is `motion.open`, which hands ShellX Canvas to the desktop
rather than starting a Cut job. Screen Record export/raw mux retain the recorder
crate's separately implemented 30-minute owner because `record-render` cannot
depend on the server job crate; it has the equivalent deadline/cancellation,
pipe-drain, descendant-tree termination, and direct-child wait/reap contract.
Native Screen Record capture backends retain their backend-native lifecycle
owners: their start/stop paths must stop and reap the backend before reporting a
terminal recording state, but they are not represented as common-owner command
children. There is no new verb, REST endpoint, MCP tool, or plugin permission for
process ownership: existing `jobs.status`/`jobs.cancel` and their terminal outcome
fields are its only API and MCP projection.

`media.import` attaches media to the current project's Assets and intentionally
does not populate the global cross-project Library. Automation that is importing
user media for later reuse should explicitly follow a successful import with
`library.add {asset:<asset_id>, source:"agent"}`. Generated and internal pipeline
imports should remain project-local.

The agent-only plugin gateway is a permission fence over this same registry,
not a parallel API. Use `plugins.list` to inspect the built-in
`openverse-assets` and `matte-runtime` scopes, `plugins.enable` to persist their
enabled state, and `plugins.call` to dispatch an allowed verb under that scoped
identity. Disabled, out-of-scope, corrupt/unavailable permission state, and
recursive `plugins.*` calls fail closed. When `plugins.list` reports a corrupt
state, use `plugins.enable` with the exact plugin name and `enabled:true` to
atomically repair it; that explicit grant enables only the named plugin and
leaves all other plugins disabled until separately approved.

## Confirmed UI control

`ui.open`, `ui.playhead`, `ui.select`, and `ui.highlight` are correlated
request/response operations, not fire-and-forget notifications. Success means
the exact WebSocket client that received the request reported a later,
committed state revision:

```json
{
  "ok": true,
  "result": {
    "applied": true,
    "verb": "ui.open",
    "request_id": 42,
    "requested": {"panel": "settings-agent-control"},
    "surface": "settings-agent-control",
    "selector": "[data-cut-settings-body=\"agent-control\"]",
    "state": {"schema": "shellx-cut/ui-state/2", "state_revision": 18}
  }
}
```

Unknown or unavailable targets, missing clip ids, already-current no-ops,
disconnects, and confirmation timeouts never return `ok:true`. UI-declined
commands return the normal `ok:false` envelope and retain a bounded
`result.applied:false` payload with the resulting state and error. A different
tab, stale request id, wrong frame type, or wrong verb cannot satisfy the
pending request.

The `ui.open.panel` enum from `GET /api/verbs` is generated from one typed UI
surface registry. It covers editor panels, workspaces, left/right/Review tabs,
Comments, every Settings destination, and editing drawers. Human-only dialogs
such as the command palette and render queue remain in the same registry with
stable selectors and an explicit agent-control alternative.

`ui.state {}` returns `shellx-cut/ui-state/2`: active workspace, left/right and
Review tabs, overlays/dialogs, open/available/agent-openable surface ids,
playhead, selection, export range, state revision, and path-safe project
identity. The server adds `connected:true` and `ui_clients`; after the last UI
socket disconnects it returns `no_ui_client` instead of stale state.

## Linked A/V timeline edits

Imported video with audio is represented as aligned clips on video and audio
tracks. Live `edit.move` and `edit.trim` calls default `linked` to `true`: when
there is one exact opposite-kind counterpart with the same asset, source range,
and timeline span, both clips change atomically and one undo restores the pair.
Pass `linked:false` only for a deliberate independent move or trim, such as a
split edit. Ambiguous or locked counterparts are rejected instead of silently
desynchronizing media.

```bash
curl -sS http://127.0.0.1:6161/api/verb/edit.move \
  -H 'content-type: application/json' \
  -d '{"clip":"c1","to_track":"v1","at_ms":2500}'

curl -sS http://127.0.0.1:6161/api/verb/edit.trim \
  -H 'content-type: application/json' \
  -d '{"clip":"c1","src_in_ms":1200}'
```

The human timeline's Q and W bindings are playhead-to-edge ripple trims, not
whole-clip deletes: Q trims the selected linked pair from the playhead back to
its start; W trims from the playhead forward to its end. The remaining timeline
closes the removed span. The toolbar also exposes explicit Add Video Track and
Add Audio Track controls.

## Native recording permissions

### Crash-resilient recordings

`screen_record.start` creates a private, project-local checkpoint journal before a
backend starts. Its output is not an open live MP4: each Linux, Windows, or macOS
segment must finalize, hash, and fully decode before recovery may use it. On daemon or
project open, Cut scans dead capture owners without sending signals. It can salvage only
the contiguous verified prefix to `recovered.mp4`; a corrupt segment or malformed
journal is quarantined and an open final segment reports an unknown lost tail. Normal
recordings atomically publish `project.json` before their complete journal receipt; a
restart repairs that one sealed projection boundary rather than misclassifying it as
interrupted. Capture-root components are checked as local plain directories before
Cut creates, scans, or reads a capture; the local marker is atomically published and
never redirects `screen_record.stop` away from that capture's own `project.json`.
The backend releases one shared capture clock only
after its native session is ready; segment restart gaps and a closed segment's missing
native frame-delivery time are real cloned-frame video time on that same clock. Event
timing, mic first-packet silence padding, and system-audio placement therefore remain
aligned to the playable source instead of an earlier portal/setup clock.

On Windows, `screen_record.start` calculates the exact compact WGC checkpoint output
path before it creates a capture marker or starts a worker. A project whose checkpoint
would exceed the legacy 260 UTF-16-code-unit path limit (including its terminator)
fails synchronously with `invalid_args` and an instruction to use a shorter project
path. The private WGC stage keeps 128-bit random base64url names; finalized deep-path
audio/checkpoint files still use the existing durable no-replace publication contract
through extended-length `MoveFileExW` paths.

`screen_record.stop` waits with a bounded capture-work-derived budget: twice the
marker-declared or journal-observed capture span plus 15 seconds, with a 45-second
minimum and 15-minute maximum. That allows real checkpoint stitch/audio finalization
without calling it a failure after a fixed short poll; a truly stuck finalizer still
returns an explicit timeout. On macOS, Core Audio collection is stopped at the video
capture boundary before sparse-checkpoint stitch work, so `system.wav` contains real
capture-period samples (plus measured leading padding), not a trimmed stitching tail.

`screen_record.recovery_status{after?,limit?}` is the read-only, paginated recovery
projection. It reports capture ids and receipt/loss facts, never cache paths or arbitrary
journal paths, and cannot itself probe, remux, repair, or signal a capture. Its `after`
cursor is an exact capture id emitted by the prior page's `next_cursor`, not a path; an
unknown well-formed id is rejected rather than silently skipping rows. Both top-level and
receipt `state` values are stable lowercase snake case. Capture states are `complete`,
`recovered`, `quarantined`, `interrupted`, `owner_ambiguous`, `torn_journal`, or
`corrupt`; receipt states are `complete`, `recovered`, `quarantined`, or `interrupted`.
A recovered MP4 is playable salvage media, not an automatic completed project or timeline
edit. Settings requests sequential 100-row pages, accepts at most 4,096 rows, verifies
strict lexical/cursor continuity and safe source basenames, and discards all partial rows
on any failure.

```bash
curl -sS http://127.0.0.1:6161/api/verb/screen_record.recovery_status \
  -H 'content-type: application/json' \
  -d '{"limit":50}'
```

`screen_record.doctor` reports a native backend as `ok` only after a bounded,
discarded frame reaches Cut. It does not persist image content. On Linux, the
XDG ScreenCast portal could open a source picker or a new consent request, so
doctor reports `unknown` instead of triggering it; `unknown` is never ready/green.
`degraded` and `missing` are evidenced failures. The Doctor response separately
reports `start_allowed`: it remains false for every missing, degraded, or arbitrary
unknown required card, but on Linux is true for the one exact prompt-deferred XDG
ScreenCast portal observation. That lets `screen_record.start` open the user-selected
source picker without manufacturing a green Doctor result. On macOS,
drive these verbs through the installed signed app: ScreenCaptureKit uses Screen
Recording permission and `system_audio:true` uses the separate Audio Capture
permission declared by the app bundle. The first request can show either system
prompt; restart Cut after granting it, then retry the capture. A successful
`screen_record.stop` reports `system.wav` through `raw_streams.system`.
Doctor exposes that optional audio path as a separate `system_audio` card. It
stays `unknown` for a compiled backend because Doctor never starts a live
loopback/tap stream; on macOS this also guarantees Doctor cannot trigger the
Audio Capture prompt. The optional card does not change `ready` or
`start_allowed` for an ordinary screen-only recording. Packet delivery is
proved only by a user-started recording and its finalized timing/artifact.
For a short, explicit delivery check instead of a full recording, use
`screen_record.system_audio_probe{max_ms?}` or the **Test system audio** button in
Record. The caller should play a sound during the 0.5–5 second window. This
consenting action can trigger the separate macOS Audio Capture prompt, returns
`live:true` only after a real packet, and separately sets `signal_detected:true`
only when that stream is not all-silent. Green UI readiness requires both facts.
It returns no audio bytes or path. Its
temporary Linux/Windows WAV is removed before the response; macOS samples never
leave memory.

```bash
curl -sS http://127.0.0.1:6161/api/verb/screen_record.system_audio_probe \
  -H 'content-type: application/json' \
  -d '{"max_ms":2500}'
```

`raw_has_system` describes the optional combined `raw_path` only, so it becomes
true only when `mux_raw:true` successfully muxes that system stream into the raw
output. On Windows, the WAV's sibling `system-audio.json` records the
first real WASAPI packet offset from capture start; `screen_record.polish` uses
that offset and clips the separate `a_system` track to the remaining video span. macOS
stops its Core Audio tap at the video capture boundary, then physically pads the saved
real PCM from the measured first callback before publishing `system.wav`. Linux captures the native PipeWire default-sink monitor and records its first
nonempty packet on the same capture clock before WAV I/O. A successfully finalized WAV with
no packet has a null offset and is not inserted automatically; a PipeWire connection, format,
or capture failure removes the partial WAV rather than claiming a raw artifact exists.
Older captures without the sidecar retain zero-offset placement. A capture with
the temporary `system-audio.json.pending` recovery marker is rejected rather
than being placed at zero: retry it so the WAV and timing receipt publish as a
pair. A headless
development `cutd` process does not inherit the installed app's TCC grants.

When `screen_record.stop{autoedit:true}` creates the normal EditPlan, it carries
the finalized capture FPS into `out_fps`; the plan therefore preserves the
captured elapsed time instead of using the engine's generic default. A direct
`screen_record.export` MP4 validates the same capture-local mic/system leaves
before rendering. It mixes a delayed system stream with its recorded
first-packet offset through the planned renderer (not the raw stream-copy path),
while a current-format null-timed system WAV stays omitted rather than being
invented at time zero. Before compositing, sparse or variable-rate capture
frames are resampled to the plan FPS using their timestamps, so a source with a
higher nominal codec rate cannot duplicate frames and lengthen the export.

On GNOME Wayland, global evdev button events do not provide an absolute captured-frame
position. Cut pairs each click only with the nearest `SPA_META_Cursor` sample on the
same capture clock when it is at most 100 ms away, then transforms the compositor's
monitor origin and logical size into negotiated frame pixels. On X11 and native monitor
capture on Windows/macOS, rdevin global desktop points likewise become exact only after
the selected portal/native monitor origin and coordinate size are transformed into the
final encoded output frame. Linux waits for `source.mp4` dimensions rather than
assuming the portal's logical stream size; if they cannot be established, rdevin
positions are unavailable. Missing, stale, outside, or unsupported geometry remains
`approximate` or `unavailable` in `screen_record.stop.cursor_correlation`; those click
transitions never seed auto-zoom. A capture with no button transitions is
`unavailable`, rather than a vacuous exact claim. This lets a client distinguish a
truthful degraded cursor track from precise capture.

Windows Graphics Capture and macOS ScreenCaptureKit window recording currently expose
only a launch-time window rectangle to this backend, not timestamped geometry samples
on the capture clock. Because a selected window can move or resize, Cut reports its
rdevin cursor/click/scroll positions as `unavailable` rather than reusing that stale
rectangle; monitor capture remains eligible for the validated exact transform.

On Windows 10 build 20348 or newer, `system_audio:true` uses native process
loopback without opening the physical render driver. The captured WAV contains
only packets WASAPI actually supplied; Cut does not pad a delayed first packet
or the capture tail with generated silence. Security software may ask to approve
the new signed binary; if it blocks the audio worker, video and microphone capture
continue and `raw_has_system` remains false.

## Sequence Index and QC status

`project.sequence_index` is the path-light cross-timeline table used by Find →
Sequence. It searches active and inactive sequences without switching them and
can isolate live media/timeline state without returning source paths:

```bash
curl -sS http://127.0.0.1:6161/api/verb/project.sequence_index \
  -H 'content-type: application/json' \
  -d '{"query":"vignette","status":"effects","track_kind":"video"}'

curl -sS http://127.0.0.1:6161/api/verb/project.sequence_index \
  -H 'content-type: application/json' \
  -d '{"status":"issues","limit":500}'
```

`status` accepts `all`, `issues`, `offline`, `gaps`, `effects`, `hidden`,
`locked`, or `muted`. `issues` combines offline media and explicit timeline
gaps. Offline state is computed from the filesystem for the call; effect and
track-state fields come from the materialized sequence. Anonymous gaps appear
only for `status:"gaps"`, `status:"issues"`, or a query containing `gap`/`gaps`,
so the default clip/marker index remains stable. Non-`all` status filters exclude
marker rows.

Clip rows include `effects`, `offline`, `track_visible`, `track_locked`,
`track_muted`, and `issues` plus their stable sequence/track/timeline location.
The app can copy the currently returned rows as escaped, spreadsheet-safe CSV;
this is a bounded path-light handoff, not a second filesystem export surface.

## Motion integration quick path

Cut is the editing/orchestration owner; ShellX Motion owns Motion-package
authoring and rendering. Discover the promoted rich generators instead of
guessing IDs, preview before insertion, then inspect the resulting Cut state:

```bash
curl -sS http://127.0.0.1:6161/api/verb/generate.list \
  -H 'content-type: application/json' \
  -d '{"kind":"motion"}'

curl -sS http://127.0.0.1:6161/api/verb/generate.preview \
  -H 'content-type: application/json' \
  -d '{"id":"builtin.motion.cinematic-fog-title","params":{"title":"CREATE BEYOND THE FRAME"}}'

curl -sS http://127.0.0.1:6161/api/verb/generate.insert \
  -H 'content-type: application/json' \
  -d '{"id":"builtin.motion.cinematic-fog-title","params":{"title":"CREATE BEYOND THE FRAME"}}'

curl -sS http://127.0.0.1:6161/api/verb/project.state \
  -H 'content-type: application/json' -d '{}'
```

Agent Chat accepts registered project asset IDs as references, never source paths:

```bash
curl -sS http://127.0.0.1:6161/api/verb/agent.chat \
  -H 'content-type: application/json' \
  -d '{"message":"match this reference","attachments":["a1"]}'
```

The server validates every ID against the open project, rejects duplicates, and
caps each turn at eight attachments. The response echoes the validated IDs in
`result.attachments` on both the success and structured no-edit paths.

Headless editing supports installed Claude Code, Codex, Grok, and Antigravity CLIs. Claude uses the
pinned 2.1.224 contained contract: Cut verifies its version and policy flags,
uses a disposable cwd and sanitized environment, and disables native CLI tools.
Codex keeps the user's normal configuration, native sandbox, and permissions;
Cut adds the live project's MCP server and does not copy or rewrite Codex login
files. Grok receives a disposable config/home with native tools disabled and
only the live Cut MCP server; its existing auth file remains in place and is
never copied or rewritten. Antigravity keeps its normal settings, native
sandbox, permissions, and login while Cut adds a workspace-local MCP entry;
that route currently requires macOS or Linux. See [SECURITY.md](../../SECURITY.md).

Every launched turn also returns a review contract:

- `result.plan` records the request, registered reference IDs, and execution
  policy shown by the Chat rail.
- `result.review.baseline` is the pre-turn op/checkpoint ref;
  `result.review.tip` is the observed post-turn history head.
- `result.review.diff` is the same computed artifact as `project.diff`.
- `result.review.revert_safe` is true only when all ops after the baseline belong
  to this uniquely attributed Agent Chat turn. Use
  `project.revert {to: baseline, if_tip: tip}` only in that case; the tip guard
  atomically refuses if newer work landed after the review was prepared.
- `result.review.concurrent_actions` names human/system/other-agent ops observed
  during the turn. Their presence disables whole-turn revert so those changes are
  never silently rolled back with the agent's work.

The Chat rail exposes the current composed Preview, exact Review Diff, shared
Accept markers, atomic Revert, and Try again (revert then prefill, never auto-send).
Timeout/CLI failure responses keep `actions` and `review` when partial edits landed.

The four promoted rich template IDs are returned by `generate.list`; current
families cover cinematic fog titles, editorial liquid surfaces, keyed-subject
promotions, and tracked callout overlays. Use `generate.describe` for their
exact parameters.

For canonical Motion packages, the lower-level bridge is:

- `motion.template_to_cut` / `motion.script_to_cut` — create the package and
  import it into Cut.
- `motion.job.get` / `motion.job.list` — inspect only the open Cut project's
  Motion renders. For live observation, choose `job_id` on the blocking
  template/script/linked-refresh request first, then poll from another REST,
  CLI, or MCP request. `pending` means waiting for capacity; `running` means
  work has begun; the remaining four states are terminal. Wait at least
  `pollAfterMs` and stop when it is absent. Cut derives caller identity and
  offers no all-callers argument.
- `motion.map_import` / `motion.apply_import` — inspect and apply native
  lowering versus rendered fallback deliberately. Map first and inspect the
  path-free `lineageProofs`: `verified` binds the current Motion SDK's two base
  package hashes (plus three glTF hashes when applicable) through artifact,
  render-receipt, and Cut-plan identities; `legacy-unverified` is compatibility
  for older render+connector handoffs, not a verified-lineage claim. A real
  `packageDir` also yields `currentPackage.status` as `exact`, `changed`, or
  `unavailable` from bounded package-owned bytes; inspect `changedFields` and
  never reinterpret unavailable comparison evidence as an exact match. A real
  rendered apply persists the same proof at
  `project.state.tracks[].clips[].motion_link.originAttestation`.
- `motion.link.edit` / `motion.link.refresh` / `motion.link.relink` — keep the
  source package and Cut clip synchronized. Edit launches a path-private return
  request; Canvas publishes immutable ready descriptors after verified renders,
  and refresh adopts one only after exact identity/revision checks.
- `motion.link.tracking.*` — configure, run, and apply tracking on a linked
  Motion clip without transferring tracking ownership to Cut.

Use `motion.link.edit` when the installed Canvas editor should open a linked
package visually. Complex environments, shaders, particles, 3D, keying/roto,
compositing graphs, and procedural effects remain Motion-rendered unless the
receiver reports an exact native lowering. See
[`SHELLX_MOTION_BOUNDARY.md`](SHELLX_MOTION_BOUNDARY.md) for the complete
ownership and fallback contract and `skill/shellx-cut/reference.md` for exact
request/result shapes.

`render.final` and its receipt are Cut-owned. A successful media render can be
followed by optional post-render perception instrumentation; if that sidecar
fails, the API must report the checks as unmeasured and preserve the runtime
cause, rather than implying a Motion connector or content failure.

The async `jobs.status` result reports `verified` and `verification_status`.
`verification_status:"complete"` means the output battery ran (the receipt may
still contain real measured failures). `verification_status:"unmeasured"`
means the artifact rendered but optional output instrumentation failed; the
result includes the structured `verification_error`, and affected receipt rows
carry `details.status:"unmeasured"` plus `details.measured:false`. The legacy
`checks_skipped` summary remains for older clients. Unmeasured checks never
produce `fix_actions`.

`verify.rerun {render_id}` is the narrow historic-artifact recheck path. It
returns a cancellable job handle, selects the exact persisted RenderReceipt,
re-fences and full-hashes its output around one owned sidecar/probe run, and
atomically publishes a separate `verify_rerun_<job_id>.json` receipt. It never
calls `render.final`, rewrites the source `render_*.json`, or claims checks that
depend on source words, captions, edit boundaries, or the current timeline.

## MCP setup

`cutd mcp` speaks MCP over stdio and exposes **every** schema verb as a tool
(dots→underscores: `edit.split` → `edit_split`). It proxies to the running
`cutd serve` found via port discovery. Read-only discovery such as
`system.mcp_test`, `system.doctor`, and `project.list` works with no project
open; project-scoped verbs operate on the one live project authority.

The easiest installed-app setup is **Settings > Agent control**. It reads
`GET /api/agent`, copies a client configuration containing the exact packaged
executable path, and exposes **Test MCP**. That read-only test launches the
same executable and verifies `initialize`, `ping`, complete `tools/list`
generation within the supported payload budget, structured tool output, and a
`system.doctor` proxy call back to the same running engine. A failed check is
shown as failed; the UI never labels an untested MCP path as connected.

Every client launches the same stdio command: the exact Cut executable followed
by `mcp`. Only the client-side registration, scope, and diagnostic commands
differ. The commands below deliberately use the exact executable placeholder;
replace it with the value shown by `/api/agent` or Settings > Agent control.

| Client | Register ShellX Cut | Client-side check | Scope behavior |
|---|---|---|---|
| Claude Code | `claude mcp add --scope user shellx-cut -- "/absolute/path/to/cutd" mcp` | `claude mcp get shellx-cut` or `claude mcp list` health-checks approved servers | Claude defaults to `local`; the shown `user` scope makes Cut available across projects. `project` is also supported. |
| Codex | `codex mcp add shellx-cut -- "/absolute/path/to/cutd" mcp` | `codex mcp get shellx-cut --json` or `codex mcp list --json` confirms the stored configuration | Codex stores the entry in `~/.codex/config.toml`; its add command has no scope flag. Configuration presence alone is not a live handshake. |
| Grok Build | `grok mcp add --scope user shellx-cut -- "/absolute/path/to/cutd" mcp` | `grok mcp doctor shellx-cut` performs command, handshake, and tool-discovery checks | Grok defaults to `user` and also supports `project`. |
| Antigravity CLI | Add the `mcpServers` block below to `~/.gemini/config/mcp_config.json` or the workspace's `.agents/mcp_config.json` | Open `/mcp` to inspect status, reload the server, and read connection logs | The client has no `agy mcp add` subcommand. Global and workspace-local JSON configuration are supported. |

Claude Code may alternatively use a project `.mcp.json`:

```json
{
  "mcpServers": {
    "shellx-cut": { "command": "cutd", "args": ["mcp"] }
  }
}
```

Claude Desktop (`claude_desktop_config.json`) uses the same `mcpServers` block.
Antigravity CLI uses the same JSON shape. For the global or workspace-local
file, configure the exact packaged executable rather than relying on `PATH`:

```json
{
  "mcpServers": {
    "shellx-cut": {
      "command": "/absolute/path/to/cutd",
      "args": ["mcp"]
    }
  }
}
```

Interactive Antigravity sessions ask before using an MCP tool. Headless
`agy --print` cannot display that prompt, so it auto-denies an unlisted call.
Allow only the exact read-only tools needed for unattended checks in
`~/.gemini/antigravity-cli/settings.json`; for example, the Cut self-test is:

```json
{
  "permissions": {
    "allow": ["mcp(shellx-cut/system_mcp_test)"]
  }
}
```

Add other exact tool rules separately when the workflow needs them. A broad
`mcp(*)` rule or `--dangerously-skip-permissions` grants more authority than a
connection test needs and is not part of Cut's onboarding instructions.

Any other MCP client works the same way — stdio transport, command `cutd`, args
`["mcp"]`. If `cutd` is not on `PATH`, use its absolute path.

Client-side checks are not interchangeable: Grok Doctor explicitly exercises a
handshake and tool discovery; Claude health-checks approved entries; Codex
`get`/`list` confirms configuration but does not claim a live handshake; and
Antigravity's `/mcp` overlay exposes live status and connection logs. For
**all four clients**, finish by calling the MCP tool `system_mcp_test {}`
(`system.mcp_test` in Cut verb notation) through that client. That Cut-owned
read-only check proves protocol negotiation, ping, all 264 tools, and that the
MCP proxy resolves to the same running Cut engine.

REST and MCP are generated from the same canonical verb registry. Use
`scripts/mcp-probe.mjs` against a running engine to check the MCP handshake,
tool inventory, and same-engine proxy behavior.

Motion live-job tools follow the normal dots-to-underscores MCP mapping:
`motion.job.get` becomes `motion_job_get` and `motion.job.list` becomes
`motion_job_list`. Cut's own `jobs.status/list/cancel` is a separate background
job model; do not translate Motion `pending` into Cut `queued`.

For automation, the Settings check is the public read-only verb:

```bash
# Substitute the live engine address from engine.addr when Cut did not bind 6161.
curl -sS http://127.0.0.1:6161/api/verb/system.mcp_test \
  -H 'content-type: application/json' -d '{}'
```

Success reports `mode:"proxy"`, the exact executable and command, negotiated
protocol version, tool count and payload size, `ping:true`, the resolved engine
address, and `same_engine:true`. `--standalone` is an advanced testing mode
with separate state and is refused while a served engine is running.
