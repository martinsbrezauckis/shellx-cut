# The debug API — REST, WebSocket, and MCP

<!-- shellx-cut-release-truth: candidate; version=0.6.114; published=0.6.113 -->

Role: the single-page operator reference for driving ShellX Cut from outside
the UI — every endpoint, the security model, and MCP client setup. The verb
catalog itself lives in `schema/verbs.json` (contract) and
`skill/shellx-cut/reference.md` (the full per-verb argument reference).
This reference describes the Debug API in v0.6.114 candidate source. v0.6.113
remains the latest published release.

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
| Linux | `$XDG_DATA_HOME/shellx-cut/engine.addr` (default `~/.local/share/shellx-cut/engine.addr`) |
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
- Native LAN/public listening is unsupported and refused by default. In a
  debug build only, `SHELLX_CUT_ALLOW_NON_LOCAL=1` permits a non-loopback bind
  and skips the browser Origin/Host/Fetch-Metadata guard. Packaged builds ignore
  the flag. Cut does not add or verify a remote token, capability, or identity.
  Remote use is supported
  only through an independently authenticated and authorized SSH/VPN/external
  ShellX broker or equivalent transport. That protection belongs to the
  transport and must be separately evidenced; without it, remote access must be
  refused. Directly exposing the port publishes the full mutation surface.
- Shared/multi-user machines, untrusted local apps/services, containers sharing
  host networking, and exposed ports are outside the supported default. Native
  per-caller/per-user capability authentication is future hardening; it is not
  in the current source. Under this documented deployment assumption, its absence is
  **NOT A DEFECT**.

`cutd mcp` is a stdio transport that proxies the running server; it has no
additional caller authentication and inherits this machine-wide boundary. The
brokered `agent.chat` routes separately limit their allowed Cut verbs; this does
not make REST/MCP per-user authenticated or restrict a normal user-configured
MCP server.
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
| `/api/frame?at_ms=[&h=][&compose=1]` | GET | bounded composited/scrub JPEG at a timeline position — `h` defaults to 540 and may not exceed 2160; derived width and pixels are capped at 4K UHD. Use `export.frame` for an explicit full-resolution still. |
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

`project.open` reads `ops.jsonl` as the durable editing history. It rejects a
journal over 128 MiB or a single operation record over 8 MiB with `invalid_args`
and leaves the journal and recovery evidence unchanged. Preserve the full
project folder for repair or migration; truncating `ops.jsonl` loses history.
Oversized derived `project.json` and snapshot caches are skipped and rebuilt
from an admitted journal.

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

### Deterministic editing-cache rebuild and cleanup

`project.cache_preview {}` is a separate, read-only path-free check. It only
accepts flat `proxies/` and `filmstrip/` entries that have matching durable
ShellX Cut ownership-ledger records, are not referenced by current asset
metadata, and have been unchanged for at least 24 hours. Any legacy/unowned or
foreign file, symlink, unexpected directory, malformed/stale ledger, oversized
root, journal drift, or cooperating producer makes the operation fail closed;
it does not return a plan. It neither modifies `project.health` nor treats file
age as last-use evidence.

`project.cache_rebuild {asset_ids?,estimate_only?}` is a separate, bounded
backfill for missing or stale base proxies and filmstrips. `estimate_only:true`
runs the same current-source-hash admission without reserving an output or
creating a job; it reports verified source-input bytes, proxy/filmstrip output
counts, and proxy media duration as work units—not a wall-clock promise. A
scheduling request accepts at most 64 registered asset ids (or every registered
asset only when the project has at most 64), verifies the current source hash
before admission and publication, and writes a durable ownership reservation
before creating an output. Every path-free result reports `status`, work units,
queued asset/output totals, and queued/up-to-date/items-needing-attention
counts. Existing legacy or unowned files, source changes, unavailable sources,
and unsupported media are reported or refused; they are never adopted or
removed. Poll the returned `cache_rebuild` job with `jobs.status` and use
`jobs.cancel` for cooperative stop. Cancellation or a restart leaves only the
pending reservation, so a later identical rebuild can resume safely; source
media, exports, captures, and receipts are untouched.

`project.cache_purge {plan_id, confirm:true}` consumes that one preview plan
and returns a cancellable `cache_purge` job. The job takes an exclusive cache
lease, rechecks journal/root/file identity before removal, removes only the
previewed ledger-owned files, and updates the ledger durably after each removal.
Its terminal `jobs.status.result` carries exact path-free
`before`/`planned`/`removed`/`after` files and bytes, plus a balance check; a
cooperative cancellation after deletion retains the same partial-progress
accounting. Normal terminal results use a strict post-scan; a project-switch
cancellation can instead report an exact delta under the exclusive lifecycle
lease after its ProjectStore has been removed. If a cache file was unlinked but
the replacement ownership ledger could not be published, the job is explicitly
`failed` with `ledger_recovery_required:true`: the old ledger entry stays
visible and blocks another preview instead of concealing partial cleanup. Use
`jobs.status` for progress, `jobs.cancel` to request cooperative stop, and
`project.cache_preview` again after a terminal record when it is not blocked to
obtain a new deletion plan. Source media, exports, captures, receipts, and every
unowned path remain outside both verbs' deletion roots.

### Executable argument contract

Every public verb's `args` entry in `schema/verbs.json` is an executable JSON
Schema Draft 7 contract, not documentation-only metadata. The server compiles
all 306 schemas once at startup and applies the selected schema at the shared
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

`project.group_preview {op_id}` is the bounded review read for one existing
adjacent durable compound action. When its `reject.status` is `ready`, submit
the returned first/last operation ids and `preview_hash` unchanged to
`project.group_reject` with a fresh `request_id` and that exact revision. The
reject route is tip-only and appends one materialized-prefix restore record;
it refuses newer history rather than attempting a generic selected-operation
replay. One `project.undo` restores the complete group.

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
handler is pure. It controls whether a brokered Agent Chat turn can discover or
call that verb; the broker still limits calls to the open project and registered
asset IDs. A permitted bounded edit may therefore accurately declare
`filesystem:true` or `process:true`. The filter governs Cut MCP verbs; native
file, shell, and network policy remains provider-specific. The generated core
contract rejects an unknown journal verb instead of guessing that it is an
undoable timeline edit. The generated dispatcher target is not a caller option;
it makes the schema name-to-handler route exhaustively checked at build time.

Release installers include the start-here guide, agent rules, public feature and
Debug API docs, verb schema, feature-surface contract, Motion boundary, and the complete
ShellX Cut skill directory (reference plus every craft guide). Package checks
verify that every served `/api/agent-doc/*path` file is
byte-identical to the candidate source, preventing a stale or partial docs bundle.

Long-running verbs return `{job_id}` immediately — poll `jobs.status`, list via
`jobs.list`, abort via `jobs.cancel`. An engine-eligible failed default-output
`screen_record.export` can start exactly one linked child through `jobs.retry`;
it validates the active revision, source/EditPlan/capture-audio SHA-256 inputs,
and a fresh default-output lease, then gives the queued renderer private
no-follow staged copies of those exact bytes rather than mutable project input
paths. An eligible failed `verify.rerun` can likewise
start one linked child only after the active revision, immutable RenderReceipt,
and exact rendered-output hash are revalidated. Neither route replays raw old
arguments; explicit Save As exports, changed inputs, and all unowned job kinds
are refused. Cancellation does not claim success until
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
independent process is `motion.open`, which hands ShellX Motion to the desktop
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
identity. While Settings > About is open, its optional
`about.displayed_version` is the exact version whose visible text has committed;
it is `null` while the doctor report is pending and absent on older connected UI
clients. The server adds `connected:true` and `ui_clients`; after the last UI
socket disconnects it returns `no_ui_client` instead of stale state.

## Linked A/V timeline edits

Imported video with audio is represented as aligned clips on video and audio
tracks. Live `edit.move` and `edit.trim` calls default `linked` to `true`: when
there is one exact opposite-kind counterpart with the same asset, source range,
and timeline span, both clips change atomically and one undo restores the pair.
Pass `linked:false` only for a deliberate independent move or trim, such as a
split edit. Ambiguous or locked counterparts are rejected instead of silently
desynchronizing media.

## Atomic linked A/V insert

`edit.insert_linked` is the only insertion route for a probed video asset with
audio. It needs an editorial `at_ms`, plus exactly one video destination
strategy (`video_track` or `create_video_track:true`) and exactly one audio
strategy (`audio_track` or `create_audio_track:true`). An optional
`src_range_ms` applies to both clips and `ripple` opens timeline time once.
Audio-only, video-without-audio, and still-image placement continue to use
`edit.insert`.

Cut validates the source, both typed destinations or requested track creations,
the shared range, and the crossfade-adjusted editorial coordinate before it
commits. It then returns both clip and track IDs in one replayable, undoable
operation. A rejected second leg leaves the project and event stream unchanged;
no partial clip or track is published.

## Atomic overwrite edit

`edit.overwrite` is the fixed-time overwrite operation. It is deliberately not
`edit.insert {ripple:false}`: insert still splices and moves the target track;
overwrite replaces `[at_ms, at_ms + source_duration)` and leaves all later
timeline coordinates unchanged. Choose `video_track`, `audio_track`, or both.
Supplying both commits one linked A/V operation, undoable and replayable as one
unit; supplying one is an intentional video-only or audio-only overwrite.

`at_ms` is **editorial** time: the cumulative duration of clips on each target
track, like `edit.insert`, `edit.split`, and `edit.move`. A crossfade shortens
the rendered playhead without changing that editorial clock, so callers reading
a visible playhead must convert it through the target track's crossfade layout.
For one atomic V+A overwrite, both selected tracks must resolve the visible
position to the same editorial value; otherwise overwrite each track separately
or align their transitions first.

The engine refuses an overwrite that would shorten, remove, or otherwise lose a
live right-owned crossfade: its rendered overlap is part of the fixed-time
layout. Move the edge outside that transition owner, or rebuild the transition
deliberately in a separate edit; a rejected linked edit leaves every target
unchanged.

A rendered point inside the overlap itself is multiply covered by both sides of
the dissolve. Source Monitor treats that as an ambiguous overwrite start and
disables overwrite rather than choosing the left clip's editorial clock: that
choice could move the dissolve and change pixels before the visible playhead.
Move to either non-overlapped side first.

The source window may be explicit (`src_range_ms`) or Source Monitor marks
(`source_in_ms` plus `source_out_ms`). Every overlapping clip or gap on each
chosen track is consumed; partial boundary clips are trimmed, and an overwrite
beyond a track tail uses only the needed preceding gap/tail extension. Captions,
markers, duck windows, and unselected tracks do not move. The receipt lists the
new clip and exactly which clips/gaps/tail interval each target consumed.

```bash
curl -sS http://127.0.0.1:6161/api/verb/edit.overwrite \
  -H 'content-type: application/json' \
  -d '{"asset":"a2","at_ms":12000,"video_track":"v1","audio_track":"a1t","source_in_ms":1500,"source_out_ms":4500}'
```

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

`screen_record.start` accepts optional `expected_project_identity` copied unchanged
from `project.state.project_identity`. Under the project-transition lock it refuses
a closed or changed project before rehearsal cleanup, native admission, marker,
or capture reservation.

The installed Record UI and global F9 handle the no-project case before calling
this verb: they create a uniquely named recording project through
`project.create`, bind its returned identity, and then pass that identity as
`expected_project_identity`. Direct Debug API callers still need to create or
open a project and provide the matching identity themselves.

Record's current setup choices remain active after switching to Edit; app-wide
F9 revalidates that draft rather than the last successful take. An invalid draft
refuses capture without changing workspace. The UI's live Pause/Resume
acknowledgement also survives that navigation. Direct API clients own their
setup and must inspect the actual Pause/Resume replies themselves;
`screen_record.status` does not project Pause state.

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

Automation that must act only after live capture is real uses
`screen_record.status{capture_id}`. The result becomes `ready:true` only after
the platform's screen path delivers a real frame to Cut (including encoder
acceptance on Windows and observable PipeWire/GStreamer delivery on Linux).
Continue only while `terminal:false`; once the capture terminalizes, readiness
cannot become true again. Process startup, elapsed time, or output-file growth
is not equivalent evidence, and inactive captures return `not_found`.
`controller_placement.state:"not_excluded"` means the native display source
allows Cut to appear when unobscured. It does not claim Cut was actually visible
in any frame; inspect captured pixels for that fact. A selected-window source
retains its existing picker admission and reports display placement inapplicable.

`screen_record.live_frame{capture_id}` reads optional real pixels from the
same active native source stream as the recording. Its frame is a generation-bound,
in-memory BMP sampled to at most 640×360 pixels (with a 4 MiB hard ceiling);
`frame:null` means no current frame and never pretends that capture succeeded.
A checkpoint rollover clears the previous
generation, and Stop or failure clears all retained pixels. `recursion` and
`controller_exclusion` are independent facts: a `ready` frame can have possible
controller recursion when the native source does not prove Cut-window exclusion.
Whole-display capture includes Cut when it is visible and omits it naturally
when another window covers it. Any admitted source that contains Cut can show
recursive pixels; the result reports that possibility instead of substituting
a fake frame. A preview readback failure does not stop or change the recording.
The latest frame becomes
`stale` after five seconds without a new native frame; sample cost fields help
qualify 4K callback load. The same contract supports native macOS and Linux taps;
each backend must report its actual exclusion state.
The Record workspace must display these exact capture-owned pixels during a
take, or an explicit unavailable/stale state. A schematic image is not a live
preview. An empty or stale frame does not prove that capture stopped; query
`screen_record.status` for the exact capture and keep Stop accessible.

Source preview is a separate opt-in native lifecycle, not evidence that an
ordinary recording is active. Read `screen_record.preview_capability {}` first:
`source_selection:"exact"` accepts only a current opaque Doctor monitor/window
id, while `"portal"` accepts only `{source:{kind:"portal"}}` and opens a fresh
Linux system picker. `preview_start` replaces and releases any prior source;
`preview_status` becomes `ready` only after a real native frame; and
`preview_frame` returns only the latest generation-bound memory BMP, capped at
4 MiB before base64. Pause releases native capture but remembers the exact
source, Resume advances generation, and Hide/Stop release frames and device
ownership. While a generation is active, Start, Status, and Frame status carry
its opaque lowercase-32-hex `lease_nonce`. Pause, Resume, Hide, and Stop each require the current positive
`expected_generation` and matching
`expected_lease_nonce` as an exact pair; a stale generation or nonce is rejected without changing
a newer or restarted-server preview. The nonce identifies only the current
Cut-server owner, changes after that server restarts, and is neither authentication
nor a native target or ticket. No preview verb accepts a title, ordinal,
coordinate, file, restore token, or browser-capture fallback. Permission,
source loss, recursion, and unavailable states remain explicit and path-free.
An `unavailable_reason` appears only when the process cannot prove it released
a prior native preview; it states the required process-restart recovery.
Reopening Record does not clear that owner.

The Recording UI's continuous mic/system meters are computed from the already
admitted capture callbacks. They open no monitoring playback and no second
stream; status snapshots use nonblocking access and label silence, stale input,
loss, clipping, and unsupported paths truthfully. Current macOS system-audio
metering is unavailable because tap PCM is only finalized at Stop, even though
the normal capture may still return a verified system stream.

`screen_record.rehearsal_start` records one video-only 3–5 second native test
take into an owned temporary root and returns only an opaque same-origin
playback capability. It creates no project, journal, recovery entry, timeline
asset, or promotion route. `screen_record.rehearsal_discard` revokes and removes
that take; unmount, a replacement rehearsal, and ordinary Recording Start use
the same cleanup boundary. Deletion failures retain private ownership and retry
only through the bounded cleanup owner instead of exposing or orphaning bytes.

On Windows and macOS, passive Doctor enumeration may advertise opaque camera
choices without opening a device or prompting for permission. Camera use is
explicit through `screen_record.start{camera_id}` and is limited to Auto-edit
mode. Start revalidates the current opaque identity and admits it only after a
real native first frame. Stop returns camera media only as a validated
`CameraArtifact` with its capture id, relative video leaf, content hash, terminal
state, and shared-clock range. The screen source and camera take remain separate
editable assets; Cut never substitutes a different camera after permission,
busy-device, disappearance, or no-frame failure.

`screen_record.doctor` also advertises the bounded Recording Scenes contract
without opening a device. `screen_record.start{scenes}` freezes 1–32 uniquely
named Screen or Presenter PiP presets and exactly one capture-wide Off,
Elapsed, or Countdown timer. Its `scenes.saved:true` acknowledgement means the
catalog and initial scene header are durably journaled; it does not claim that
the camera or timer has started. During capture, `screen_record.scene_activate`
and `screen_record.scene_timer` return only after the reducer-validated event is
saved at a logical time issued by the capture's shared `CaptureClock`.
Presenter PiP activation fails closed until the selected camera is admitted.
Presenter PiP accepts top-left, top-right, bottom-right, and bottom-left
corners. A `screen_record.studio_event` camera `transform` chooses the camera
position, size, or shape for the polished output. That choice carries across
later internal scene switches. A camera `reset` event with only `t_ms`,
`source:"camera"`, and `kind:"reset"` restores the active frozen scene layout
without changing camera visibility or the recording timer. This metadata
changes the polished composition; the separate raw camera take stays intact.

On Windows, `screen_record.start` calculates the exact compact WGC checkpoint output
path before it creates a capture marker or starts a worker. A project whose checkpoint
would exceed the legacy 260 UTF-16-code-unit path limit (including its terminator)
fails synchronously with `invalid_args` and an instruction to use a shorter project
path. The private WGC stage keeps 128-bit random base64url names; finalized deep-path
audio/checkpoint files still use the existing durable no-replace publication contract
through extended-length `MoveFileExW` paths.

The Windows and macOS in-app window pickers use the live rows returned by
`screen_record.doctor.windows`. Their `id` field is an opaque native identity;
`title` and `app` exist only for display. Pass the `id` unchanged to
`screen_record.start{window}` or `debug.screenshot{window}`. Capture revalidates
that exact identity and returns a clear error if the window closed or was
replaced; it never searches by title or silently falls back to a whole display.

On Windows and macOS, `screen_record.doctor.monitors[]` includes an opaque
versioned native `id` when Cut can derive an exact native identity. Pass that
`id` unchanged as `screen_record.start{monitor_id}` to require a fresh native
re-enumeration and exact match before capture. A stale display fails explicitly:
Cut never substitutes display copy, list position, primary state, dimensions,
or the legacy `monitor` ordinal. The existing full-display picker retains that
ordinal path only when Doctor did not provide an `id`. Linux deliberately
returns no in-app monitor rows because the system portal owns source choice.

`screen_record.stop` waits with a bounded capture-work-derived budget: twice the
marker-declared or journal-observed capture span plus 15 seconds, with a 45-second
minimum and 15-minute maximum. That allows real checkpoint stitch/audio finalization
without calling it a failure after a fixed short poll; a truly stuck finalizer still
returns an explicit timeout. On macOS, Core Audio collection is stopped at the video
capture boundary before sparse-checkpoint stitch work, so `system.wav` contains real
capture-period samples (plus measured leading padding), not a trimmed stitching tail.

An unsuccessful `screen_record.stop` is not a completed-recording receipt: retain
the same `capture_id`, do not issue `screen_record.start` for a replacement, and
query `screen_record.status{capture_id}` before classifying the result. Offer
Retry Stop only when that exact capture is still live (`terminal:false`). A
terminal or `not_found` result must not restart a recording timer; uncertain
ownership is reported as unknown. `screen_record.recovery_status` remains the
read-only durable inventory for interrupted captures. The visible Record
workspace keeps unresolved ownership explicit. The Record UI uses the default
export folder for both outcome modes; optional file selection belongs to an
after-Stop Export or Save a copy action. The API retains optional `raw_path`
for explicit callers. An acknowledged Stop releases capture ownership before
downstream auto-edit/polish, so a later processing failure does not imply that
a live capture persists.

`mux_raw:true` and `autoedit:true` are independent Stop options. Together they save
an unedited MP4 in the default export folder and return an EditPlan for the
subsequent editable `screen_record.polish` step. Repeated default saves receive
numbered names instead of replacing an earlier take. After a successful Stop,
`screen_record.copy_raw{source:raw_path,path?}` can save another byte-for-byte MP4
copy without an EditPlan. Its source must remain a plain file in the active
project's authorized export folder; the copy job reserves a fenced destination,
publishes only a complete file, and removes its private stage on cancellation.
The Record UI calls Stop with `mux_raw:true` for Raw and Polished modes, and
adds `autoedit:true` only for Polished. It then polishes the editable clip with
`raw:false` while retaining the unchanged MP4. Show keystrokes is an optional
Polished setting that starts off; there is no independent auto-polish choice.
The on-video Countdown uses an applied duration of 00:00:01–23:59:59; its zero
does not stop capture. Camera transform controls expose all four corners and
retain the chosen placement across internal scene switches until explicit reset.

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

Doctor also reports `quality:{supported,output_sizes,profiles,detail}`. This is
an admission capability, not a generic encoder inventory. Current Linux capture
may offer Source/1080p/720p with Standard/High because its final-source
normalizer owns downscale-only sizing and libx264. Windows/macOS currently
return empty choices. Pass an advertised pair as
`screen_record.start{quality:{output_size,profile}}`; direct requests on an
unsupported backend fail. `screen_record.stop.quality` appears only when the
same final verification still matches the requested height cap, nonzero output
dimensions, H.264 container facts, and the actual libx264 selection.

For microphone setup, `screen_record.doctor{warm_mic:true}` is the bounded
user-visible test path for the resolved System Default or selected input. Doctor
returns generic `Microphone N` labels plus short-lived opaque tokens only;
Windows/MMDevice and macOS/Core Audio identities stay private, while Linux is
System Default only. `mic_warm` returns real bounded signal facts without a
device/backend name. An unavailable saved choice refuses mic-enabled start
instead of silently changing source. On device change, idle Record refreshes
safe enumeration only; recording never opens a competing microphone stream.

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

For an exact per-asset occurrence list, pass `asset` instead of relying on a
text match. That filter is applied before `limit`; returned `at_ms` and
`end_ms` use the laid layout shared by rendering and
`ui.playhead`, including upstream crossfade overlap.

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

`system.motion_status {}` is the read-only management preflight. It runs only a
bounded shell-free runtime probe, connector catalog, and fixed Template-to-Cut
descriptor check—no caller id, provider login, connector execution, download,
or filesystem mutation. A discovered source checkout, PATH binary, or npm
package remains unmanaged and execution-unqualified. The returned distribution
card keeps Install/Repair/Update/Remove unavailable with blocker
`MOTION-DIST-01` until Motion publishes a verified immutable manifest and a
matching platform artifact; Cut does not invent lifecycle authority from local
discovery.

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
An optional `model` string selects the model for that turn on the named `agent`;
omit it to use the CLI's configured default. Agent Chat's Model field sends this
same argument and keeps each provider's entered value separate while the panel
is open.

Find > Moment citations use a separate index-bound attachment contract. Resolve
them provider-free first, then send the same ids and exact index snapshot:

```bash
curl -sS http://127.0.0.1:6161/api/verb/inspect.range \
  -H 'content-type: application/json' \
  -d '{"index_id":"idx_0123456789abcdef01234567","evidence_ids":["ev_0123456789abcdef01234567"]}'

curl -sS http://127.0.0.1:6161/api/verb/agent.chat \
  -H 'content-type: application/json' \
  -d '{"message":"compare this cited moment","evidence_index_id":"idx_0123456789abcdef01234567","evidence_ids":["ev_0123456789abcdef01234567"]}'
```

Cut re-runs the same current-range validation before launching a provider. A
changed index, missing asset, or stale authority refuses the attachment; prompt
text is never treated as a substitute for current evidence.

The eight Agent Chat prompt presets only pre-fill the editable composer. Their
verb metadata is limited to the contained Chat policy's inspect/edit routes; it
does not make an external speech service, render/delivery route, or verification
route available. Use **Transcript Tools** for speaker labels or dubbing,
**Clips** for candidates and delivery, and **Review > QC** for verification.

`agent.chat` can also carry one immutable
`shellx-cut/chat-timeline-target/1` from a Comment, selected timeline range,
selected clips, or the playhead. It contains the exact project identity and
revision plus clip spans and range/position data. Cut revalidates it before
launch, returns it with the turn, and never replaces it with a newer UI
selection. Timeline **Ask agent** opens Chat with this target and waits for the
normal prompt; Comment **Make changes** routes its note and target through the
same Chat route.

Headless editing supports installed Claude Code, Codex, Grok, and Antigravity CLIs. Provider version
text is informational only; Cut verifies each route's required policy flags before every turn. Claude uses a
contained capability contract with a disposable cwd and
sanitized environment, and disables native CLI tools.
Codex uses the selected account's configuration, native sandbox, and permissions;
an enrolled alternate account home reaches only the Codex child through
`CODEX_HOME`. Cut adds the live project's MCP server and does not copy or rewrite
Codex login files. Grok receives a disposable config/home with native tools
disabled and only the live Cut MCP server; its existing auth file remains in place and is
never copied or rewritten. Antigravity keeps its normal settings and login while
Cut creates a new disposable sandboxed project containing one Cut-only MCP
plugin. Its headless approval mode is bounded by that empty workspace and Cut's
filtered server-side verb policy; the resolved CLI must advertise the complete
launch contract before each turn, including on Windows. See [SECURITY.md](../../SECURITY.md).

Every launched turn applies its validated result directly and returns a turn
receipt:

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

The Chat rail exposes the current composed Preview and exact Diff. It offers
**Step back** only when `revert_safe` remains true, and **Ask replacement**
re-resolves the retained target before a new request; a stale or deleted target
refuses visibly. There is no Draft, Accept, or editor approval stage. Bounded
history stays on the local device for the exact project identity and retains
validated requests, targets, replies, actions, and revisions; interrupted turns
remain marked as interrupted. Timeout/CLI failure responses keep `actions` and
`review` when partial edits landed.

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
| Codex | `codex mcp add shellx-cut -- "/absolute/path/to/cutd" mcp` | `codex mcp get shellx-cut --json` or `codex mcp list --json` confirms the stored configuration | With the default account, Codex stores the entry in `~/.codex/config.toml`; for an enrolled alternate account, run the check in that account's `CODEX_HOME` environment. The add command has no scope flag. Configuration presence alone is not a live handshake. |
| Grok Build | `grok mcp add --scope user shellx-cut -- "/absolute/path/to/cutd" mcp` | `grok mcp doctor shellx-cut` performs command, handshake, and tool-discovery checks | Grok defaults to `user` and also supports `project`. |
| Antigravity CLI | `agy mcp add shellx-cut /absolute/path/to/cutd mcp` | `agy mcp list` confirms configuration; open `/mcp` for live status and connection logs | The command writes the user-level `~/.gemini/config/mcp_config.json` entry. Configuration presence alone is not a live handshake. |

Claude Code may alternatively use a project `.mcp.json`:

```json
{
  "mcpServers": {
    "shellx-cut": { "command": "cutd", "args": ["mcp"] }
  }
}
```

Claude Desktop (`claude_desktop_config.json`) uses the same `mcpServers` block.
Antigravity CLI stores the same JSON shape in its global user configuration.
If configuring that file directly, use the exact packaged executable rather than relying on `PATH`:

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
read-only check proves protocol negotiation, ping, all 291 tools, and that the
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

## B5 bulk offline-media relink

`media.relink_preview {root}` is a bounded, read-only, symlink-refusing folder
scan for offline assets in the open project. Its `plan_hash` is only actionable
for a unique candidate whose complete `sha256:` equals the asset's stored full
SHA-256. A unique candidate with exact basename, kind, stored raw byte size,
and duration (audio/video) or dimensions (still) can be marked
`metadata_review`: it is a disabled hint to use normal one-file Relink, never
an applyable bulk row. Available dimension, normalized container, or codec
mismatch refuses; missing stored probe/size is `metadata_insufficient`; equal
top private ranks are `ambiguous_metadata`. Duplicate exact candidates and
sampled/missing stored hashes remain refused. Preview diagnostics disclose only
safe matched-fact labels, never candidate paths, selected root, raw probes,
timestamps, scores, or metadata values.

`media.relink_apply {root, plan_hash, accept, request_id, expected_revision}`
revalidates the exact plan and writes one replayable project-local metadata op.
It returns immutable `shellx-cut/media-relink-receipt/1` data for B6 (project
identity, pre/post revisions, plan hash, grouped op id, expected hash, chosen
path, and disposition per accepted asset); the ordinary immutable mutation
request receipt is retained too. Neither verb touches the global Library or
starts import/proxy/enrichment work, and bulk recovery makes no Ctrl-Z promise.

## B6 portable package

`project.package_plan {destination, name, b5_receipt?}` is the required dry
run. It walks every sequence, hashes only referenced source media with complete
SHA-256, and returns a destination-bound `plan_hash`, byte/file totals, dedupe
plan, and package-relative member names. Offline media, symlinks/reparse points,
unsafe destinations, and Motion-linked provenance refuse. It never searches for
or relinks media. If the current project revision is B5's grouped relink, its
exact immutable receipt is required; do not reuse that receipt after any later
project operation. Cut checks its project identity, post-revision, grouped
journal operation, exact hashes, and chosen paths before retaining only the
receipt's canonical digest.

`project.package_create {destination, name, plan_hash, b5_receipt?}` recomputes
that plan, starts a `portable_package` job, copies each unique content digest
into a private same-parent stage, clears all derived-cache pointers, writes a
Cut-native replay baseline plus `package.manifest.json`, verifies every listed
member checksum, and atomically publishes `<name>.cutproj` without replacement.
Poll `jobs.status`; terminal success includes the destination and manifest
SHA-256. `published_with_warnings` means publication completed but a
post-publication parent-directory sync or private-stage cleanup warning was
retained. The source project's log, cache, asset paths, and media remain intact
(the normal persisted source-local job record is operational state, not a
project operation). Linux, macOS, and Windows each publish through a native
no-replace directory primitive; unknown targets fail closed. In the desktop
editor, Projects → Make a copy selects a destination through the native folder
picker, previews counts, dedupe, cache exclusion, and target availability, then
requires an explicit confirmation. Library “Keep a copy” remains a different
global-Library action.
