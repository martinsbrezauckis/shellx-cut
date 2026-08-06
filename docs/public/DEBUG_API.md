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

There is **no API token or auth header**. The trust boundary is the local
machine:

- `cutd` **refuses to bind a non-loopback address** (`0.0.0.0`, LAN IPs, `::`).
- Browser-driven cross-origin and DNS-rebinding requests are rejected by an
  Origin + Host guard on every request (a non-loopback `Origin` or `Host` gets
  403).
- The only opt-out is `SHELLX_CUT_ALLOW_NON_LOCAL=1`, intended **solely** for a
  trusted reverse proxy or tunnel that adds its own authentication in front.
  Setting it and exposing the port directly publishes the full mutation surface
  — don't.

Anything on the same machine (any local process or agent) can drive the editor.
That is the product: the debug API is the co-editing surface, not an admin
backdoor.

## Endpoint catalog

All verbs go through one route; the rest are read-side support surfaces.

| Route | Method | What it serves |
|---|---|---|
| `/api/verb/{name}` | POST | dispatch any of the schema's verbs; body = args JSON; returns the envelope `{ok, result?, op_ids?, warnings?[], error?{code,message,cause,…}}` |
| `/api/state` | GET | current project/timeline state snapshot |
| `/api/verbs` | GET | the live verb registry (generated from `schema/verbs.json`) |
| `/api/events` | GET (WS) | event stream: `op_applied · job_progress · render_done · receipt_ready · project_changed · ui_state · doctor_updated` (`project_changed` refreshes visible clients after REST/CLI/MCP create, open, or close; `doctor_updated` refreshes environment capabilities; agents key on `receipt_ready`) |
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

### Executable argument contract

Every public verb's `args` entry in `schema/verbs.json` is an executable JSON
Schema Draft 7 contract, not documentation-only metadata. The server compiles
all 260 schemas once at startup and applies the selected schema at the shared
dispatch boundary. Direct/internal dispatch, REST, `cutd verb`, and
`cutd mcp` therefore reject the same malformed input before a handler runs.

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

Release installers include the start-here guide, agent rules, public feature and
Debug API docs, verb schema, feature workflow, Motion boundary, and the complete
ShellX Cut skill directory (reference plus every craft guide). Native release
qualification verifies that every served `/api/agent-doc/*path` file is
byte-identical to the candidate source, preventing a stale or partial docs bundle.

Long-running verbs return `{job_id}` immediately — poll `jobs.status`, list via
`jobs.list`, abort via `jobs.cancel`. Cancellation does not claim success until
tracked blocking workers and their synchronous child processes have finished.
If that bounded drain is still in progress, `job_cancel_pending` asks the
caller to wait and retry. A project switch uses the same fail-closed boundary:
the next project is not attached while an old worker is alive. File-writing
verbs are fenced to the project/export directories (schema the output-fencing contract).

`media.import` attaches media to the current project's Assets and intentionally
does not populate the global cross-project Library. Automation that is importing
user media for later reuse should explicitly follow a successful import with
`library.add {asset:<asset_id>, source:"agent"}`. Generated and internal pipeline
imports should remain project-local.

The agent-only plugin gateway is a permission fence over this same registry,
not a parallel API. Use `plugins.list` to inspect the built-in
`openverse-assets` and `matte-runtime` scopes, `plugins.enable` to persist their
enabled state, and `plugins.call` to dispatch an allowed verb under that scoped
identity. Disabled, out-of-scope, and recursive `plugins.*` calls fail closed.

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

`screen_record.doctor` reports the linked native backend, while
`screen_record.start` performs the permission-sensitive capture. On macOS,
drive these verbs through the installed signed app: ScreenCaptureKit uses Screen
Recording permission and `system_audio:true` uses the separate Audio Capture
permission declared by the app bundle. The first request can show either system
prompt; restart Cut after granting it, then retry the capture. A successful
`screen_record.stop` returns `raw_streams.system`, `system.wav`, and
`raw_has_system:true`. A headless development `cutd` process does not inherit the
installed app's TCC grants.

On Windows 10 build 20348 or newer, `system_audio:true` uses native process
loopback without opening the physical render driver. Security software may ask
to approve the new signed binary; if it blocks the audio worker, video and
microphone capture continue and `raw_has_system` remains false.

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
| Antigravity CLI | Add the `mcpServers` block below to `~/.gemini/config/mcp_config.json` or the workspace's `.agents/mcp_config.json` | Open `/mcp` to inspect status, reload the server, and read connection logs | The qualified `agy` 1.1.9 client has no `agy mcp add` subcommand. Global and workspace-local JSON configuration are supported. |

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
read-only check proves protocol negotiation, ping, all 260 tools, and that the
MCP proxy resolves to the same running Cut engine.

Coverage of both surfaces is enforced: `scripts/coverage-audit.sh` asserts
every verb answers on REST **and** appears in MCP `tools/list`.

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
