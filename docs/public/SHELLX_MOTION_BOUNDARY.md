# ShellX Cut / ShellX Motion Boundary

Status: working integration contract.

Purpose: keep ShellX Cut aligned with ShellX Motion without duplicating a second
motion renderer or standalone motion editor inside Cut.

## 2026-08-25 Self-Describing Discovery And Generic Lifecycle Checkpoint

Motion MCI-1 is source-accepted at implementation commit
`81bcaaa07e5daaba285f22e64fad197edb3d4eb6` and documentation checkpoint
`06abe54cfed2a40bc23f6bf7045dde7960d6d282`. It adds three read-only CLI routes:
`runtime-probe`, `connector catalog`, and `connector describe <capability-id>`.
The accepted contract identities are:

- `shellx-motion/runtime-probe@1` schema SHA-256
  `ef43bbccc5fb58aa347bb72a2ab2a89187a14060b9410099c01d828b7f89b35a`;
- `shellx-motion/capability-catalog@1` schema SHA-256
  `d689c6b1f013cf8ea8c81d4a3825a13e1c5ee8ed188c6971931e3da0c303b066`;
- current produced catalog fingerprint
  `9f19e5e8f87ab9a5c4e5e7673814653463c3517bbe8cf6574a62b9fc90f3bdcf`.

This checkpoint changes discovery, not Cut execution. Cut's existing blocking named connectors and
`motion.job.get/list` remain unchanged. Every MCI-1 generic connector-job descriptor is
`not-admitted`; Cut must not add submit/events/cancel/retry until Motion MCI-2/MCI-3 accepts one
durable connector lifecycle and typed failure contract. Source and ordinary packed runtimes remain
unmanaged and distribution-unverified. Scene3D, C6 physics, and C7 orchestration remain refused for
Cut, while current P2A/P2B rendered-media routes retain their declared Linux-only limits.

The future Cut consumer must branch on negotiated request, lifecycle, trust, artifact, receipt, and
editor-operation classes, never on a growing list of capability IDs. A new Motion render, formula,
template, or scene-orchestration operation requires no Cut code change when those protocol classes
stay the same: Motion adds the descriptor, bounded request metadata, output roles, and documentation
resource. Cut changes only for a protocol-major revision or a genuinely new interaction, authority,
artifact, or editor-operation class. Until that generic Cut consumer is implemented and an exact
Cut-plus-Motion process pair passes, catalog visibility is not Cut readiness or execution admission.

Motion MCI-2 is now source-accepted at implementation commit
`a96960a712a27ff770841bfd13249864db5d72d1`. Its current contract identities are:

- `shellx-motion/capability-catalog@2` schema SHA-256
  `0fa8bdb4606fa341a89e55cb7554bd365f783c8e4b63b370d054f10d92df787c`;
- produced catalog fingerprint
  `848014c36389ebece97712feb6b027380eacc8603814b1b0d67a21b99c564388`;
- Template-to-Cut descriptor fingerprint
  `e6f762650e0494e01d2f14adc997dcadb67263d17640d21532f624d25091df61`;
- generic lifecycle identities `shellx-motion/capability-descriptor@2`,
  `shellx-motion/connector-job@2`, and `shellx-motion/connector-job-binding@1`;
- fixed authenticated coordinator command `motion.connector.submit`, followed by the shared
  `motion.job.get`, `motion.job.list`, `motion.job.events`, `motion.job.cancel`, and
  `motion.job.retry` controls advertised by the descriptor.

Cut should integrate this as one generic adapter. It validates the runtime protocol, catalog,
descriptor revision/fingerprint, closed request fields, lifecycle class, permission class, output
roles, receipt schemas, and Cut import-plan class. It must not add a verb, dispatch branch, or source
switch for each Motion capability id. When Motion publishes a later render, formula, template, or
scene-orchestration descriptor using those same classes, that feature becomes discoverable and
callable through catalog data without a Cut code change. Cut changes only for a new protocol major
or a genuinely new request, interaction, authority, lifecycle, output, receipt, or editor-operation
class.

The host configures this generic path once with a stable authenticated caller, a caller-scoped
opaque-reference authority, and an immutable binding journal. Connector requests carry opaque
reference tokens and bounded scalar values, never paths or URLs. Motion journals the exact catalog,
descriptor, request, caller, and job binding before queueing or resolving a token. Explicit retry
may reconstruct only a terminal retryable failed binding; interrupted pending/running work and
cancelled work are never automatically resurrected.

The current v2 catalog admits Canvas-to-Cut, Script-to-Cut, Source-to-Cut, and Template-to-Cut to
that generic lifecycle on their declared Linux route. Canvas bridge export, Canvas-to-MP4, and Cut
Generate-to-Cut remain named compatibility routes. Scene3D, C6 physics, and C7 scene orchestration
remain refused. The generic submit command stays discoverable when its host resolver/journal is not
configured, but returns typed `capability_unavailable`; the route must not disappear and invite a
feature-specific fallback.

PC2 Linux x64 Node 24.18.0 and Mac arm64 Node 26.0.0 passed the focused lifecycle/refusal tests,
affected package typechecks, and complete contract, argument, documentation, source-hygiene,
architecture, packed-build, and pack gates on byte-identical source. This is Motion source/packed
evidence, not installed Cut evidence.

Motion MCI-3 is source-accepted at implementation commit
`dc49ce1f1c66bd6719410180db7e83fc927664bd`. Its `job-status-contract@1` authority SHA-256 is
`d729280cd1508f9df7680fe3c648ee55d02567cf719f039c8beac12ef5843e26`; the canonical
render-job and prompt-job handoff schema hashes are
`06051f3ccf862231d766236f00bc6a9223f7c9f7979afff66efce346b6bdaf01` and
`68567a971866d23454c4eba9882bb965a54162eb5464d70e1b54938e1938b1d3`. The catalog schema,
ten descriptors, produced catalog fingerprint and generic submit command remain unchanged.

The Cut generic adapter must accept a bounded future Motion error code as an opaque string and
preserve its `message`, `retryable`, optional `remedy`, `retryAfterMs`, and `suggestedAction` through
its own job projection. It branches on this metadata, never on capability-specific error names;
unknown codes are not translated to `invalid_args` or `connector_failed`. Canonical handoff state is
`pending`, not `queued`. A terminal job's `outcome` is authoritative even when an artifact path is
present, while `job_unknown`, `job_expired`, and `job_not_visible` describe lookup failure rather
than render failure.

PC2 and Mac independently passed the focused Core/connector/Debug/MCP/type gates and the complete
contract, documentation, architecture, installed-bin and packed-package gates from the exact MCI-3
source tree. Each packed 19 packages, 4,528 files, and 1,480 reachable shipping modules. This is
Motion source/packed evidence, not an installed Cut process-pair result. The generic Cut consumer,
exact Cut-server plus Motion-runtime process-pair test, managed/signed distribution, clean-host and
Windows qualification, Scene3D/C6/C7 admission, and release qualification remain open.

Cut now has a compiled, source-only structural consumer for this contract. It discovers the full
truthful catalog, projects conditional/compatibility-only/refused availability, prepares only the
admitted rendered-media import class, and uses one injected fixed connector/job transport. It does
not add a Cut public verb, one-shot CLI lifecycle, managed installation claim, or process-pair
claim. Scene3D/C6/C7 remain visible but unpreparable. Its path-free
`shellx-cut/motion-connector-delivery@1` value is a Cut-owned trusted bridge seam for a later
authenticated coordinator: after a succeeded `job-status@1`, that bridge resolves the private
Motion receipt/output authority and returns only descriptor-bound artifacts, attestation, and a
rendered-media Cut import plan. It is not a Motion command or public runtime protocol. Before a
submit it re-describes the selected descriptor and refuses catalog/descriptor drift; explicit retry
also re-negotiates the original bound runtime/catalog/descriptor identity without upgrading it.

## ShellX Motion Owns

- MotionIR, TemplateIR, AssetIR, package manifests, and `.shellxmotion` package
  validation.
- Renderer lanes: native, browser, FFmpeg, hosted later, plus renderer-specific
  preview and final-render receipts.
- Motion package jobs: validate, inspect, preview, render, batch render, import
  planning, export, support bundles, and action/debug coverage for Motion
  surfaces.
- Prompt-to-motion workflows routed through Motion's action/debug API and local
  CLI agent adapters.
- Motion package receipts proving inputs, lane, runtime versions, output files,
  warnings, unsupported features, and validation checks.

## ShellX Cut Owns

- Timeline editing, media import, project assets, op log, undo/redo, review
  rail, delivery settings, render queue, and final Cut render receipts.
- Cut-native editable operations: title, caption, shape, transform, effects,
  audio, transcript, dubbing, diarization, matte, grade, and timeline assembly.
- Generate's Cut-native template path: `generate.list`, `generate.describe`,
  `generate.preview`, `generate.insert`, `generate.from_prompt`, and
  `generate.storyboard`.
- Motion-backed Generate templates and agent/debug workflows through
  `motion.template_to_cut` and `motion.script_to_cut`: Cut passes either a
  Motion package alias/path plus params, or a `shellx-motion/scripted-video@1`
  JSON object/file, to the local ShellX Motion CLI, receives preview/render
  receipts and artifacts, and imports rendered MP4 output through normal Cut
  verbs for timeline insert.
- Provider-backed media generation through `assets.generate`, surfaced in
  Generate > AI media, then imported like any other project asset.
- The host UX for placing Motion output on the Cut timeline and showing the
  resulting Cut-side receipt/evidence.

## Integration Contract

Cut should consume Motion through stable package/job contracts, not by parsing
renderer internals.

Every Cut-owned Motion process supplies a stable, path-private
`cut:<workspace-hash>` caller identity. A new Cut process working on the same
project therefore rejoins the same Motion owner bucket, while independent Cut
projects do not collapse into Motion's `unattributed` bucket.

Motion process results are parsed even when the CLI exits nonzero. A
`cancelled:true` / `render_cancelled` result is surfaced as stopped and is never
auto-retried. `job_queue_timeout` means the machine-wide Motion capacity was
busy before work started and carries a wait-and-retry action. Other failed
envelopes remain sidecar failures.

Cut exposes Motion's read-only live status as `motion.job.get` and
`motion.job.list`. The render verbs remain blocking, so a caller that needs
progress chooses a valid `job_id` before starting `motion.template_to_cut`,
`motion.script_to_cut`, or `motion.link.refresh`, runs that request on one
connection, and polls from another. Both calls use the same internally derived
active-project caller identity. Cut accepts neither a caller-id argument nor
Motion's operator-only all-callers scope.

Status handling follows Motion without translation: `pending` means waiting
for capacity and has no `startedAtMs`; `running` means work has begun; and
`succeeded`, `failed`, `cancelled`, or `skipped` are terminal. Poll no faster
than `pollAfterMs` and stop when that field is absent. `job_unknown`,
`job_expired`, and `job_not_visible` remain distinct query errors. Cut's own
`jobs.*` vocabulary remains `queued | running | done | failed`; the two job
models are not coerced into each other.

Motion receipt status is a separate contract from job state. Cut accepts
`passed` and `warning` as successful receipt attestations, rejects `failed` or
any other value, and returns the receipt warnings for the caller to present.
Repeated warning text from plan, receipt, and unsupported-feature diagnostics
is surfaced once.

Initial acceptable Cut modes:

- rendered media import: Motion renders an output file, Cut imports it as normal
  media.
- live overlay clip: Cut references a Motion package/output as a timeline layer
  when preview/render support exists.
- editable lowering: supported Motion template layers lower into Cut-native ops
  such as title, caption, shape, transform, and media insert operations.
- final render bridge: Cut's render queue may request Motion output and then
  continue through Cut's normal `render.final`/`render.queue` receipt model.

Cut must not store React/JSX, browser-runtime implementation details, or private
renderer state as its project truth. If a Motion package cannot lower cleanly,
Cut should show a rendered-media or live-overlay path with an unsupported-feature
receipt.

## UI And Debug Surface Rules

- Any Motion bridge visible in Cut needs a `ui.open` path, `ui.state` evidence,
  stable `data-cut-*` selectors, and a CDP/Playwright coverage path.
- Generate remains Library-adjacent: Templates, Native prompt, Storyboard, and AI
  media are Cut host surfaces. Motion-backed templates appear there only as
  package-backed items that cross the Motion CLI contract.
- Agents should be able to open the relevant Cut surfaces directly:
  `ui.open{panel:"generate"}`, `generate-prompt`, `generate-storyboard`, and
  `generate-media`.

## Current Cut Behavior

- Cut's native `generate.*` verbs remain editable-template features; Motion
  templates lower through explicit bridge verbs instead of embedding a renderer
  inside Cut.
- `motion.template_to_cut` and `motion.script_to_cut` call the local ShellX
  Motion CLI for preview/render work, then Cut imports rendered MP4 output
  through normal project/checkpoint/media/timeline verbs. Current connectors
  also pass their package directory into the atomic import so a generated clip
  is immediately bound to its editable source; omission remains accepted only
  for legacy connector compatibility.
- Template aliases resolve from an explicit `SHELLX_MOTION_TEMPLATE_ROOT` or a
  discovered Motion checkout's promoted `templates/shellx-product-pack` before
  legacy fixtures. Generate exposes fog density, wave height, and spill
  suppression as bounded decimal controls; Motion TemplateIR remains the final
  validator and renderer source of truth.
- `motion.map_import` and `motion.apply_import` are the receiving side of the
  Motion import-plan connector. They validate or apply
  `shellx-motion/cut-import-plan@1` files without duplicating Motion connector
  packages in the Cut repository. Dry-run operations expose only a
  `plannedPath`. Real operations must provide a
  `shellx-motion/artifact-handle-ref@1`; Cut resolves legacy paths relative to
  the plan directory and current SDK paths relative to the canonical
  `<artifactRoot>/.shellx-motion/cut/` layout's artifact root. It verifies the
  descriptor and media hashes/size/magic, requires successful (`passed` or
  `warning`) receipt attestations bound to the same operation, and runs bounded `ffprobe`
  before exposing the media path. Current SDK handles carry
  `shellx-motion/package-render-lineage@1`: Cut recomputes the lineage-bound
  artifact handle ID, requires the exact render input-hash set, and binds the
  descriptor/operation plus the two base package hashes (and all three
  `adapter.gltf` provenance hashes when present) through the exact Cut-plan
  receipt. These return a path-free `lineageProofs[].status:"verified"`; the
  SDK's Cut-plan receipt is the connector commitment, so its handle needs only
  the render attestation. If the caller supplies `packageDir`, Cut independently
  hashes the bounded manifest/Motion bytes and all three `adapter.gltf` evidence
  files when present. `lineageProofs[].currentPackage.status` is `exact` when
  those bytes match the immutable artifact lineage, `changed` with deterministic
  `changedFields` when readable bytes differ, and `unavailable` when comparison
  evidence cannot be derived. This report never authorizes or invalidates the
  already attested artifact. Legacy template/script connector handles have no
  package lineage and still require both render and connector receipts; they
  return `legacy-unverified` and must never be described as lineage-verified.
  Reference, rendered-media, source, media, and operation shapes are closed.
  Real apply stages the
  complete plan and commits one replayable `motion.apply_import` operation;
  validation failure commits nothing, the plan hash makes retries idempotent,
  and `background:true` exposes progress/cancellation through `jobs.*`.
  Undo/redo and the returned pre-plan checkpoint include the plan-owned assets
  as well as its timeline clips. Direct real-media paths,
  changed bytes or receipts, traversal, and symlink escapes fail closed.
- Cross-repository integration checks produce a current-SDK handoff, map it
  through Cut's Debug API, apply it through the MCP proxy, and require the
  identical path-free proof in replayed project state.
- A verified rendered-media apply also records
  `shellx-cut/motion-link@1` provenance in the atomic op. `project.state`
  projects that replay-backed link onto the live clip as `motion_link`: stable
  Cut `clipId`, Motion package/motion identity, source/plan revision, attested
  render digest/handle, last receipt, fallback path, and current link state.
  Its immutable, path-free `originAttestation` is the exact proof returned by
  map/apply: handle/operation/descriptor hashes, package lineage, and the
  render/optional-connector/Cut-plan receipt identities. Replay, reopen,
  undo/redo, refresh, and relink preserve this origin proof even when the
  current source revision or render changes.
  The renderer still consumes a normal Cut asset; Motion metadata never
  replaces native timeline truth. The Timeline exposes an `M` badge and link
  state, while the Inspector shows source/render identity plus a bounded,
  path-free keying/roto summary (counts, safe labels, spill/matte presence,
  frame counts, and tracking model). Raw vertices, tracking ids, and unknown
  package fields never enter the Cut state surface. `project.state`
  revalidates source/plan/render/fallback availability on this PC without
  persisting machine-local filesystem facts. A missing package keeps the last
  rendered fallback usable and reports `missing-source`. A changed plan reports
  `source-dirty`; after a validated relink or refresh establishes a
  `motion-package` revision, later package changes do too. An initial optional
  `packageDir` now records an import-time `exact | changed | unavailable`
  current-package comparison inside the immutable origin attestation. It is not
  a live watcher: later source truth still comes from package revision checks,
  relink, and refresh.
- Rain, water, snow, shaders, particles, 3D, blur, and film stay on this linked
  rendered-media path. Cut exposes their ownership and state, launches the full
  controls/curve editor in ShellX Motion, and refreshes the attested fallback in place;
  it does not pretend those controls are native Cut effects.
- `motion.link.relink` repairs only the local package binding after validating
  the durable package/motion identity; it never changes pixels and marks the
  source dirty. `motion.link.refresh` renders to a new immutable project-owned
  artifact, bounds CLI output, verifies package identity plus the exact receipt
  SHA-256, reads the supported `receiptPath` from Motion's render envelope and
  retains it in the replay-backed link, detects source/project/link races, then atomically imports and lowers
  to an in-place `edit.replace`. The stable Cut clip id, slot, look, and previous
  fallback survive. A failed refresh commits nothing; the successful refresh is
  one replayable op and one-step undo/redo.
- Cut's own `jobs.*` lifecycle continues to use `queued | running | done |
  failed`. That is an internal Cut job record, not a Motion job handoff; Motion
  handoffs use `pending` and Cut does not author a `queued` Motion state.
- Current Motion connectors declare the exact
  `shellx-cut/motion_editable_import.rs` receiver. Cut accepts a
  populated `unsupported` list on a `rendered_media` plan and reports every
  reason as a warning; only an `editable_lowering` plan claiming unsupported
  content is refused. Cut has no `motion.screenshot` call site.
- `motion.link.edit` revalidates the source identity and launches ShellX Motion
  with fixed argv (`--motion-package <canonical-dir>` plus a canonical
  `--motion-cut-return-request`). ShellX Motion keeps both paths in its trusted
  host, SDK-validates the package, and opens a path-free stale editor revision in
  the same rich workspace. A verified render publishes a new immutable ready
  descriptor; refresh rechecks package/motion identity and the exact source
  revision before adopting that copy-on-write package. `SHELLX_CANVAS_BIN` and
  historical `shellx-canvas` executable names are backward-compatibility
  discovery fallbacks only; they are not the ShellX Motion product name or an
  ownership boundary. No shell interpolation or Cut timeline mutation is involved.
- Linked footage tracking stays Motion-owned while Cut owns the host workflow.
  `motion.link.tracking.inventory` exposes only package/video/layer/lifecycle ids;
  `request` converts a normalized seed to source pixels and asks the local Motion
  CLI for deterministic point or planar analysis; `inspect` proves source-byte
  freshness; `apply` compiles the track into ordinary Motion transform
  keyframes; `verify` checks the attachment and source; and `detach` restores the
  exact prior keyframes. Mutations write project-owned copy-on-write package
  revisions and attach them only after package/motion/receipt identity and link
  generation still match. The last rendered Cut asset is never replaced by
  analyze/apply/detach; `motion.link.refresh` remains the explicit pixel commit.
- Connector code follows an enforced non-growth rule: package reads live in
  `motion_package.rs`, replay projection in `motion_link_projection.rs`, shared
  CLI discovery/process limits in `motion_runtime.rs`, and tracking concerns in
  separate `motion_tracking/` modules. New feature modules stay within the
  documented 350-line source and 600-line focused-test ceilings.
- Native editable receiving is active for receipt-bound Motion text,
  document backgrounds, and rect/rounded-rect/ellipse/circle/line shape
  operations. Cut materializes the document background as a full-canvas native
  shape and maps layer operations to its
  normal `title.add` and `edit.add_shape` objects, records stable
  package/motion/source-layer-to-clip bindings, makes exact plan retries
  idempotent, and groups the whole import into one undo/redo action. Those
  objects remain editable through `title.update` / `shape.update`. A changed
  plan with the same package and motion identity updates the same bound objects
  in place and is also one undo/redo action. The source-layer set, native kinds,
  and timing must remain unchanged in this receiver; those changes fail
  closed rather than duplicating or retiming objects. Uniform opacity,
  `transform.x`, and `transform.y` keyframe tracks lower to clip-local
  `edit.keyframe` data (`opacity`, `pos_x`, and `pos_y`); pixel positions are
  normalized against the Motion document and off-screen values remain
  off-screen. Non-overlapping fade-in/out transitions with one Cut-compatible
  easing use the opacity path. All supported tracks are created, replaced,
  cleared, and reimported in the same undo group as their native object. Fade
  overlaps, mixed/unsupported easing, transform scale/rotation keyframes, and
  fades combined with explicit opacity keyframes fail closed. One Cut-origin
  video layer may also round-trip through a bounded
  `cut-asset:<id>` reference into a normal `edit.insert` clip; reimport uses
  `edit.replace` and keeps its clip identity. The same bounded reference can
  return an unprocessed, normal-speed audio asset to Cut's native audio track.
  Filesystem paths and portable
  package media never enter this path. Other dynamic keyframes, transitions,
  track state, media layers, captions, effects, masks,
  and unknown fields currently fail closed with a `rendered_media` recovery
  action; they are not silently approximated.
- `assets.generate` creates image/video assets through the selected generation
  provider and imports them into the current Cut project.
- Cut UI copy should describe Motion output as imported/rendered media unless an
  individual template explicitly lowers into Cut-native editable operations.
