# ShellX Cut Feature Inventory

<!-- shellx-cut-release-truth: candidate; version=0.6.113; published=0.6.112 -->

This feature view is bundled with installed ShellX Cut builds for users and
agents discovering the application on a new machine.

For the exact machine-readable contract, use `schema/verbs.json`. For agent
workflow details and full verb arguments, use `skill/shellx-cut/SKILL.md` and
`skill/shellx-cut/reference.md`.

## v0.6.113 release notes (candidate source; qualification pending)

This source line is a candidate and is not a published, signed, or
installed-qualified release. v0.6.112 remains the latest published release.

- Transcript is now a timeline-linked phrase and chapter list with exact
  start-end ranges and one shared time control for clips, captions, markers,
  comments, cited search results, and review evidence. Reused or unavailable
  source media explains when an exact occurrence must be chosen or cannot be
  opened.
- Recording Studio adds separate **Display** and **Window** choices, a
  cancellable 3- or 5-second countdown, **Screen** and **Presenter** scenes with
  an elapsed or countdown timer, and camera recording as a separate editable
  track on supported Windows and macOS builds. Supported macOS builds also
  offer durable **Pause & resume** for compatible display recordings.
- Timeline Voiceover records into an unlocked audio track after a visible
  count-in, follows the playhead or selected In-Out range, supports Stop and
  Cancel, and places the sealed take as one Undoable edit.
- Caption Inspector Find & Replace previews **Contains** or **Whole word**
  matches on one track and an optional timeline range, then applies the reviewed
  batch as one Undoable change without moving caption timing.
- Preview **Compare** pauses playback and shows exact composed **Before** and
  **Current** frames at the same playhead position without invoking Undo.
- Health & Recovery can rebuild missing or stale Cut-owned editing proxies and
  filmstrips with progress and cancellation. An unchanged failed output
  verification can also expose one exact retry while its revision, receipt, and
  rendered bytes still match.
- Bugfixes.

## v0.6.112 release notes

- Cut presents the editor and local API before background FFmpeg hardware
  discovery begins, so a slow driver probe no longer holds the opening UI.
- User-facing media-folder, b-roll-folder, LUT, and Render Queue destinations
  use native desktop pickers and path-light labels instead of editable local
  filesystem paths.
- Audio-bearing media in Source Monitor now has a seekable waveform with
  current-position and In/Out markers, sharing the timeline waveform cache.
- Recording adds 24/25/30/50/60 FPS presets, validated custom 1–240 FPS input,
  and an app-local microphone preference: System Default preserves existing
  behavior, while Windows/macOS can save a privately resolved selected endpoint.
  The Record UI receives only expiring opaque tokens and generic `Microphone N`
  category-and-ordinal labels; an
  unavailable saved selection refuses mic-enabled start instead of falling back.
  Linux remains System Default only.
- Recording internals now also have a durable private Windows Pause session and
  a private camera-session spine. They preserve exact monitor/device identity,
  measured screen-clock timing, verified pause artifacts, Stop dominance,
  exactly-once camera cleanup, truthful zero-frame cancellation, and replay-
  safe evidence. These foundations are deliberately unwired: v0.6.112 does not
  expose Pause or Camera controls.
- Long-form timeline scrolling isolates unchanged track rows and coalesces
  scroll updates. Further thumbnail-candidate reduction remains planned.

These are v0.6.112 published-release capabilities.

- Projects → Make a copy opens a preview-first portable-copy flow: choose a
  destination with the native folder picker, inspect used-reference and unique
  media counts, dedupe savings, cache exclusion, offline refusal, and current
  destination collision truth, then explicitly confirm the copy. It never
  changes the source project or original media.

## v0.6.111 release notes

- A bundled interactive Manual uses the real editor frontend, opens and
  highlights indexed controls, and keeps embedded exploration read-only.
- Find → Moment uses `media.intelligence_status`,
  `media.intelligence_rebuild`, and `media.intelligence_search` to search one
  cited project index across spoken words, existing
  visual embeddings, scenes, beats, markers, and media metadata. Results keep
  source and timeline time distinct, disclose partial/stale coverage, and can
  be previewed, opened on the current sequence, or explicitly handed to Agent
  Chat for discussion without applying an edit. `inspect.media` and
  `inspect.range` provide the path-light, current evidence agents use for those
  citations.
- Assets can recover several moved sources through an exact-hash
  `media.relink_preview` / `media.relink_apply` review, while the underlying
  `project.package_plan` / `project.package_create` portable-package API
  preserves the open project. The later Projects UI is documented in the
  v0.6.112 release notes above.
- Health & Recovery adds a read-only `project.cache_preview` and explicitly
  confirmed, cancellable `project.cache_purge` for aged, unreferenced,
  Cut-owned proxy and filmstrip files; source media, exports, recordings, and
  foreign files are never candidates.
- A failed default Recording Studio export can expose one guarded
  `jobs.retry` attempt when its project revision, capture inputs, edit plan, and
  output lease are still identical. Changed, cancelled, explicit Save As,
  successful, legacy, and already-retried jobs remain ineligible.
- Reliability and UI fixes across native testing, provider routing, the Manual
  button, and release packaging.

## Visible Surface Map

The stable names below are the complete `ui.open` contract for human-visible
workspaces, tabs, settings categories, and drawers. Human controls and agents
route to the same surface registry.

- Editor and media: `timeline`, `preview`, `projects`, `assets`, `transcript`,
  `library`, and `record`.
- Generate and Find: `generate`, `generate-prompt`, `generate-storyboard`,
  `generate-media`, `find-media`, `find-moment`, and `sequence-index`.
- Right tools and Review: `properties`, `color`, `audio`, `chat`, `review`,
  `review-ops`, `receipts`, `qc`, `scopes`, `diff`, and `comments`.
- Setup and Settings: `wizard`, `environment`, `settings-general`,
  `settings-editing`, `settings-video-performance`,
  `settings-ai-transcription`, `settings-recording`,
  `settings-services-integrations`, `settings-agent-control`,
  `settings-storage-privacy`, `settings-health-recovery`, and `settings-about`.
- Editing drawers: `music`, `title`, `kinetic`, `layer`, `clips`, `autopilot`,
  `assemble`, `recipes`, `matte`, `shape`, and `mask`.
- Compatibility aliases: `stock` opens `find-media`; `search` opens
  `find-moment`.
- Storage & privacy includes a plain-language Network activity section. The
  installed app discloses its once-per-launch GitHub release-metadata check and
  persists an opt-out in the native shell; Cut adds no project, media, history,
  or analytics payload to that check.

## Core Editing

- Project lifecycle: Projects is the initial workspace; its human controls cover
  create, open, save, rename, recent-project list, forget (single or bulk
  clear-missing), and delete. The agent/debug contract additionally exposes
  close, checkpoint creation, diff, revert, and operation history; guarded
  workflows create checkpoints automatically, but there is no standalone
  checkpoint-create button. Dropping a video,
  audio file, or image while no project is open creates a sensibly named
  project and places the media on its timeline. The first video adopts source
  geometry and frame rate when the new project still has its untouched default
  format; delivery aspect, output size, codec, and bitrate stay per-render.
- Sequence Index: search clip and marker metadata across every active or inactive
  timeline from Find → Sequence, filter by sequence/result/track kind or live
  status (issues, offline, gaps, effects, hidden, locked, muted), then open the
  correct sequence at the result time. Result rows expose basenames, stable ids,
  effect names and track state without disclosing source paths; the currently
  shown bounded rows can be copied as escaped, spreadsheet-safe CSV for QC
  handoff.
- Timeline editing: split, trim, ripple delete, move, insert, paste, add track,
  restore, markers (with labels and colors), speed, fades, crossfades,
  transforms, crops, and undoable operation replay. The selected-clip Inspector
  offers both quick speed-ramp presets and a compact custom curve editor with
  ordered, millisecond-accurate source-time points from 0.25× to 4×; invalid
  or out-of-range curves stay unapplied with a concrete inline reason.
- Imported picture and sound are linked by default. Moving or trimming either
  half moves or trims its exact counterpart atomically; deliberate split edits
  can opt out with `linked:false`.
- Ripple trims are available from the toolbar and default Q/W
  bindings: Q removes from the playhead to the selected clip's start, W removes
  from the playhead to its end, and the remaining linked picture and sound close
  the gap together instead of deleting the whole selected clip.
- The timeline toolbar exposes Add Video Track and Add Audio Track. Empty
  user-created tracks remain available through unrelated edits and deletes.
- Timeline placement follows NLE layer semantics: normal Insert and normal
  drag/drop place media on the base story timeline with ripple; Alt-drag or
  dropping on an existing overlay lane places video on top without rippling the
  base.
- Video layers use bottom-to-top compositing: the first
  non-empty video track is the stable base canvas, later video tracks render in
  track order above it, and an empty track does not steal the base role. A hidden
  or gapped base stays black instead of promoting an overlay. Transform and
  opacity work on the base (against black) as well as on picture-in-picture
  overlays, including after masks and power windows.
- Timeline track headers expose common controls directly in the lane: video and
  caption tracks can be hidden or shown, any track can be locked against
  accidental edits in the timeline, Layer drawer, and Inspector; video tracks
  can be sent backward or brought forward within the video stack; and
  audio-bearing tracks can be muted, soloed, listened to, panned,
  or gain-adjusted without destructive rewrites.
- Timeline width: selected-clip tools open from the right-edge Tools strip as a
  contextual overlay by default, so choosing a clip does not shrink the timeline.
  Users who prefer a persistent inspector can pin the rail back into the layout.
- Precision trims: slip, slide, and roll — via the Inspector trim stepper
  popover, Alt+arrow nudges, or the drag trim TOOL (`t` cycles
  select→slip→slide→roll on the timeline).
- Selection and reuse: marquee rubber-band selection on empty lanes
  (Shift-additive) and paste-attributes (Ctrl/Cmd+Alt+V) copying grade, speed,
  effects, and more between clips through a checkbox dialog.
- Non-destructive audio muting: clip mute ranges glued to SOURCE time (they
  survive trims, slips, and splits), word-level mute/unmute from the
  Transcript panel, and per-track mute/solo.
- Audio pan/balance per track with center-neutral semantics, available in the
  mixer and as a compact timeline-header shortcut.
- The selected audio clip has a **Clip volume lane** over its waveform. Use
  **Add point** at the playhead or Ctrl/Cmd-click the curve, then drag named
  control points between their neighbours over a fixed −60…+12 dB display.
  Dragging previews locally; pointer-up sends one complete, undoable automation
  edit. Escape and cancelled pointers leave the clip unchanged. The first point
  seeds the current static Gain, and Clear returns to that unchanged static
  Gain. The Inspector remains the alternative for exact time, level, and
  interpolation editing. Automation saves are revision-protected: if another
  edit changes the project first, Cut reloads the current points instead of
  overwriting them.
- Viewing aids: fullscreen preview (`f`), rule-of-thirds and title/action safe
  guides (`g`).
- Keyboard remapping: a central keymap with a Settings editor
  (press-to-rebind, conflict detection, reset), a versioned portable JSON
  profile with atomic validation and forward-compatible unknown-key handling,
  familiar Cut/Premiere-style/Resolve-style/Final Cut-style presets scoped to
  commands Cut actually supports, and the `?` overlay derived from the live
  bindings. Shortcut profiles contain only command IDs and keys—never project
  or media data.
- Transcript editing: word-level cuts, non-destructive Ignore for words that
  should be skipped by captions/reels, silence removal, filler removal, search,
  and transcript-based assembly.
- Captions and titles: caption generation, text cards, kinetic captions (Lines
  from caption cues or One word at a time from a timeline transcript), styling,
  a caption style preset gallery (built-in looks + save-your-own,
  replay-independent apply), range controls, shifting, reflow, and animated
  titles.
- Mask / privacy drawer: quick actions for Blur face, Blur rectangle, and Hide
  plate/text; manual rectangle/ellipse/polygon regions; blur, pixelate, or black
  box effects; whole-clip masks or timed redaction from the current playhead.

## Media And Library

- Media import with probe, proxy, filmstrip, waveform, perception enrichment,
  and first-import auto-placement on an empty timeline. Later imports wait in
  Assets until the user clicks Insert or drags them into the timeline. Library
  media uses explicit Add to project or Insert at playhead actions.
- Assets is media attached to the current project; Library is reusable media
  shared across projects. The human Assets Import action mirrors its successful
  imports into Library. Direct `media.import` automation stays project-local by
  design so generated and pipeline-internal media do not silently pollute the
  reusable collection; follow it with `library.add {asset}` when reuse is
  intentional. Both surfaces show when the same content exists on the other.
- Offline media handling: `media.check` reports sources gone from disk
  (computed live, never a stored flag), the Assets tray badges offline clips,
  and `media.relink` repoints an asset — same-content relinks keep proxies and
  transcripts, changed content re-derives them.
- Assets → Recover missing media handles several moved sources through one
  bounded folder review. `media.relink_preview` permits only uniquely exact
  full-file hashes to be selected; `media.relink_apply` rechecks those choices
  and appends one grouped project-metadata operation with an immutable receipt.
  A unique, strongly matching metadata candidate is labeled “Possible
  replacement — review individually” and routes to normal one-file Relink;
  it remains disabled and cannot enter grouped apply. That hint requires exact
  basename, kind, stored byte size, and duration (audio/video) or dimensions
  (still), and available dimension/container/codec disagreement refuses it.
  Missing stored probe/size and tied private ranks are refused. Preview exposes
  safe matched-fact labels only, never candidate paths, roots, raw probes,
  timestamps, scores, or metadata values.
- Projects → Make a copy uses `project.package_plan` to inventory every
  referenced asset across all sequences, deduplicate exact bytes, exclude
  rebuildable caches, and refuse offline or stale inputs before a user confirms.
  A matching `project.package_create` publishes a new verified `.cutproj`
  without replacing a destination or changing the source project.
- Media Health in the Assets tray summarizes missing source files, proxy/source
  playback state, and large 4K/camera clips, with a one-click relink action,
  per-asset readiness badges, a "needs action" filter, and Advanced counts kept
  out of the default view.
- Proxy import controls are discoverable from Assets and command search so
  casual users can turn on smoother playback for future 4K/phone/camera imports.
- Smart bins: saved per-project asset filters (kind, name text, unused,
  4K+/high-resolution, missing/offline, and recently modified sources) whose
  membership is computed at list time, shown as live-count chips in the
  Assets tray.
- Hover-scrub thumbnails: Assets cards render real filmstrip frames and scrub
  them under the pointer.
- Source monitor: open any online video or audio asset independently of Program
  playback, using its editing proxy when one is ready so large or
  platform-unsupported source codecs remain auditionable; use the explicit
  keyboard-accessible Play/Pause transport, mark source In/Out,
  and insert that exact range at the timeline playhead. Audio-bearing sources
  also show a display-only waveform projection: amber In/Out marks and a white
  current-position line make the selected range readable; click it or use its
  keyboard slider controls to seek the real source transport. Video assets with audio
  create aligned linked picture and sound. The same compact V and A destination
  selectors can target either unlocked track or Off, then **Overwrite range**
  replaces only the selected destinations at the playhead; native selects work
  with Tab and arrow keys, while the action remains disabled until a marked
  range and at least one destination are available. From a selected footage clip or
  video-track header, **Match Frame** opens the Source monitor at the exact
  source frame under the playhead for normal, reverse, and freeze playback;
  unavailable/offline sources and speed ramps stay visibly disabled rather
  than guessed. A track header also refuses a crossfade or other overlap until
  one clip is selected directly. **All uses** is a compact UI-only navigator over the existing
  per-asset Sequence Index: it switches to the selected sequence and seeks its
  laid clip start after any upstream crossfades, never claiming a ramp-exact
  source mapping; its exact asset filter is applied before the 500-occurrence
  cap.
  Online still images open in the same monitor as a real image preview, not
  pretend timed media: set a bounded duration (0.1 seconds to one hour), choose
  an unlocked video track or Off, and **Overwrite still** replaces that exact
  interval at the visible playhead. Stills never offer source In/Out, transport,
  range insert, or an audio destination.
  The monitor and a footage clip menu also offer **Reveal in Project** and
  **Reveal in Library**:
  each clears local filters and selects the same registered asset in the
  existing surface. **Reveal Source File** is desktop-only and resolves that
  registered identity again in the native shell before asking File Explorer,
  Finder, or the Linux file manager to show a local regular file. Browser,
  removed, offline, and non-file cases stay closed with a visible reason; the
  UI neither supplies nor displays a raw source path.
- Cited Find moment search: **Prepare search** derives a rebuildable, path-light
  index from analysis the project already owns—transcript words and speakers,
  existing visual embeddings, scenes, beats, markers, and media metadata. It
  does not silently run missing transcription, perception, or visual analysis.
  Coverage says what is ready, missing, or stale; stale evidence is excluded.
  Search supports evidence-kind and current-sequence scope, returns source-time
  excerpts with exact provenance, previews online sources, and jumps only to a
  real occurrence in the active sequence. Selected citations become visible,
  index-bound Agent Chat attachments as well as an editable draft. Cut resolves
  them through the same read-only `inspect.range` contract before provider
  launch, so changed or stale evidence is refused instead of trusted from copied
  prose. Nothing is sent or edited until the user deliberately continues.
- Dedicated global Library workspace: All/Recent/Favorites/Missing collections,
  tags, folders, search/sort/type filters, list/grid density, bulk organization,
  add-to-project, explicit Insert at playhead, and honest dead-link reporting
  (`media_ok` per item). Results are server-filtered and paged 100 at a time
  with visible Previous/Next controls and exact totals, so large collections do
  not create an unbounded DOM. Missing linked sources expose Relink and accept
  only the same content at its new location; different media stays a separate
  Library item.
- Asset sources: Find media reads the matching Cut server's source catalog for
  local folders, Openverse, Internet Archive, Wikimedia, NASA, and built-in
  shape stickers. It limits kinds to the selected source, shows its license and
  credit before import, and lets the offline sticker catalog browse with an
  empty query; network sources are contacted when you search or import a
  result. Provider-
  backed generation uses the user's configured generation CLI: Codex images,
  Grok Imagine images/video, or Antigravity (`agy`) images. Antigravity uses a
  native sandboxed non-interactive CLI contract and keeps its existing login and
  settings in place; Cut never reads or rewrites them.

## AI-Assisted Editing

- Local transcription models: Parakeet default, Canary weak-language tier with
  MMS_FA forced-aligned word timestamps, and Whisper large-v3 compatibility
  fallback.
- Perception: speech words, silences, scenes, beats, face detection, OCR,
  subject tracking, matte runners, and reusable media facts.
- Speaker diarization: `media.diarize` labels who spoke when through the
  configured Sortformer v2 service and refreshes transcript speaker labels.
  Multicam switching can use those labels with `mode:"speaker"`.
- Dubbing and translation: `audio.dub` creates a new translated voice track
  through the configured OmniVoice service; text translation uses the CLI agent
  first, then local translation only as fallback.
- Assemble: the human-visible `assemble` drawer turns existing footage into
  highlights/repurposed edits, plans vertical shorts, matches a script to
  footage, or fills a b-roll slot. `assemble.repurpose`, `assemble.shorts`,
  `assemble.from_script`, and `assemble.broll` return normal reviewable timeline
  operations rather than an opaque generated movie.
- Repurpose / Clip candidates: the `clips` drawer uses `clip.candidates` plus
  model-free `score.clip` explanations to rank standalone moments, then hands
  selected windows to social delivery without changing the source edit.
- Autopilot: the `autopilot` drawer previews or runs `autopilot.run`, a bounded
  render → verify → low-risk fix → re-verify loop under one checkpoint. It
  stops on no progress and never converts a failed or unmeasured receipt into a
  pass.
- Recipes: all 11 bundled workflows are First edit, Edit for clarity, Podcast
  repurpose, Talking-head cleanup, Screen-demo polish, Phone clip cleanup,
  Social short bundle, Blur or mask an area, Add captions, Export for YouTube,
  and Export for TikTok. Timeline-changing recipes show their exact plan before
  Run.
- Agent chat: a CLI agent can operate the live project through cutd's MCP verb
  surface, producing normal reversible edit operations. A turn can attach up to
  eight registered project assets as references; cutd validates their IDs and
  keeps source paths behind `project.state`. Each launched turn records a stable
  pre-edit history baseline, uniquely attributes its ops, computes the exact
  Review diff, and exposes Preview, Diff, Accept, Revert, and safe retry controls.
  Concurrent human/system edits are reported separately and disable whole-turn
  revert rather than risking rollback of someone else's work. A categorized
  prompt library pre-fills eight common Polish, Repurpose, Speech, and Review
  outcomes without sending or spending an agent turn until the user presses Send.
  Agent Chat launches the user's installed Claude Code, Codex, Grok, or Antigravity CLI. Claude uses
  Cut's contained capability route in a disposable cwd with native CLI tools
  disabled. Provider version text is informational only and each route's required
  containment flags are verified before each turn. Codex keeps the user's normal configuration, native sandbox, and
  permissions; Cut neither copies nor rewrites its login files. Grok receives a
  disposable config/home with native tools disabled and only Cut's filtered MCP
  route, while its existing login file remains in place. Cut grants trust only
  to that newly created empty workspace for the duration of the turn so Grok
  can start its project-scoped Cut MCP server. Antigravity keeps its
  normal settings and login while Cut creates a new disposable sandboxed project
  containing one Cut-only MCP plugin. Headless approval is bounded by that empty
  workspace and Cut's filtered server-side verb policy; the resolved CLI's full
  launch contract is verified before each turn on every supported platform. Each route can inspect
  the open project and apply reversible in-project edits. Review every resulting
  edit, especially when using a local CLI that retains its own native tools and
  integrations.

## Generate

- Native editable Generate workspace beside Library:
  `generate.list`, `generate.describe`, `generate.preview`, `generate.insert`,
  `generate.from_prompt`, and `generate.storyboard`.
- Generate previews are non-mutating; inserts create normal undoable timeline
  edits.
- The prompt/storyboard PLANNER ships as bundled adapter scripts that route the
  planning request to the user's own local CLI subscription agent (Claude Code /
  Codex / Grok — the `agent` arg picks one, `auto` takes the first installed).
  No hosted API and no key: plans cost nothing beyond the user's existing CLI
  subscription. With no CLI agent installed the verbs return an honest
  `not_run` (never a fabricated plan); cutd validates every returned plan or
  storyboard against the local catalog before anything can be previewed or
  inserted.
- Motion-backed Generate templates lower through `motion.template_to_cut` for
  package templates and `motion.script_to_cut` for scripted-video JSON, calling
  the local ShellX Motion CLI, returning preview receipt/artifact evidence, and
  importing rendered MP4 output through normal Cut media/timeline verbs when
  inserted. The visible catalog includes promoted cinematic fog, editorial
  liquid-surface, keyed-subject promo, and tracked-callout families with bounded
  text, color, duration, and decimal effect controls. Their production media is
  replaced through Edit in Motion; Cut keeps the linked render and editorial
  identity rather than pretending the rich effects are native filters. Current
  Motion connectors retain their generated package as the clip's local editable
  source binding; legacy connectors without that field still import, but require
  an explicit relink before Edit in Motion is available. Every Motion CLI call
  carries a stable path-private Cut workspace identity. A deliberately stopped
  Motion render returns `render_cancelled` and is never retried; a
  `job_queue_timeout` reports that machine-wide Motion capacity is busy and may
  be retried later.
- Agents can name a Motion-backed render up front with `job_id`, then inspect it
  through `motion.job.get` or `motion.job.list` while the original request is
  still running. Cut keeps the Motion `pending | running | ended` lifecycle and
  terminal outcome vocabulary intact, derives the caller from the open project,
  and exposes no cross-caller scope. Polling stops when `pollAfterMs` disappears.
- Connector import plans enter through `motion.map_import` / `motion.apply_import`.
  Real artifacts are attested first, then the whole plan commits as one
  idempotent operation. Background apply reports progress through `jobs.*`, can
  be cancelled before commit, and undo/revert includes imported assets and clips.
  Motion receipt status `warning` remains a successful advisory just like
  `passed`; Cut rejects failed receipts, returns the warning text, and removes
  duplicates introduced by the plan, receipt, and unsupported diagnostics.
  Current Motion SDK handoffs expose a path-free `verified` lineage proof that
  binds manifest/Motion hashes (plus preserved/normalized/lowering hashes for
  glTF) through the handle, render receipt, and Cut-plan receipt. Older
  template/script connectors remain usable as explicit `legacy-unverified`
  imports. Real rendered clips retain the immutable proof as
  `motion_link.originAttestation` across replay/reopen and later refresh/relink.
  When `packageDir` is supplied, that proof also records an import-time,
  independently derived `currentPackage` comparison: `exact`, `changed`, or
  `unavailable`, with path-free changed hash fields and no effect on immutable
  artifact authorization.
  Receipt-bound Motion text, document backgrounds, and basic vector shapes can instead lower to
  normal editable Cut titles/shapes with stable source-layer bindings and one
  grouped undo action. Uniform opacity, horizontal-position, and
  vertical-position keyframes lower to native clip automation; Motion pixel
  positions are normalized against the source document and may remain
  intentionally off-screen. Exact non-overlapping fade-in/out transitions use
  the same path. A single Cut-origin
  video reference can round-trip as a normal media clip without exposing an
  editable-plan filesystem path; the same applies to an unprocessed Cut-origin
  audio clip at normal speed.
  Changed plans for the same package/motion identity update
  those objects in place while keeping native clip IDs stable; layer-set, kind,
  timing, mixed per-segment easing, transform scale/rotation, and other
  unsupported dynamic-field changes fail closed.
- Rendered Motion clips retain source/render provenance and expose current,
  changed, missing-source, and render-error states in the Timeline/Inspector.
  Environment simulations such as rain, water, and snow remain Motion-owned
  rendered media: **Edit in Motion** opens their full controls and curves in
  Canvas, while **Refresh render** replaces the linked Cut clip in place.
  The launch creates a project-local, path-private return request; Canvas writes
  a new immutable ready descriptor only after a verified copy-on-write render.
  Refresh adopts the newest matching package/motion identity and exact authored
  source revision, so stale, changed, or mismatched handbacks leave the last good
  clip untouched.
  `motion.link.relink` validates and repairs the local package binding;
  `motion.link.refresh` creates and verifies a new immutable render before one
  atomic in-place clip replacement and retains Motion's on-disk render
  `receiptPath` in the replay-backed link. The last good render remains available on
  failure and `project.undo` restores it after success.
  **Edit in Motion** uses `motion.link.edit` to launch the verified package and
  trusted return request into Canvas's SDK-backed, path-free Motion intake;
  availability is reported
  honestly when Canvas is not installed/configured.
  The Inspector and `project.state` also show a bounded source summary for
  chroma-keyed layers, spill/matte cleanup, animated roto, and tracked roto.
  Raw geometry, tracking identities, paths, and unknown fields are not exposed;
  these controls remain Motion-owned and stale pixels still require refresh.
  The same Inspector now exposes local **Track & stabilize** controls for linked
  packages: choose manifest-declared footage and a visual target, seed a point
  or planar region, analyze, inspect source freshness, apply ordinary Motion
  transform keyframes, verify, or detach back to the exact prior keyframes.
  Package changes are copy-on-write and receipt/identity/race checked; the last
  good Cut render stays untouched until **Refresh render** is selected.
- AI media generation remains separate through `assets.generate`, uses immutable
  content-addressed outputs with provenance/reuse metadata, and imports
  provider-backed media like any other asset. Up to four registered project
  images/videos can be copied into the isolated run as visual references;
  explicit variation labels create distinct immutable takes in one family while
  an unchanged request still reuses without provider cost. `assets.generated_list`
  powers the path-light project history, re-checking sidecar and media integrity
  before a take is offered for reference or retry. Requests use the persisted job
  queue, expose progress, and can be cancelled before or during the provider run.
  Antigravity is deliberately image-only because Cut has no source-proven
  Antigravity video-generation capability to advertise.

## Review, Verification, And Delivery

- Render preview, frame extraction, final render, render queue, social bundles,
  storyboard/contact sheets, and subject-aware reframe. Social bundles include
  an atomic hashed manifest and an honest ready/needs-review/blocked package
  verdict across platform QC, caption writes, thumbnails, and brand checks.
  Active background work stays visible after a UI reload through the durable job
  list, with plain-language task names, the worker's latest durable phase,
  queued/running progress, and a direct cancellation control in the status bar.
  Cancellation shows its bounded stopping phase and prevents duplicate requests
  while Cut drains the task's owned worker and child processes. Limited queued
  work names its stable position among jobs waiting for the same local capacity
  and that resource's slot count; the durable list is refreshed even when a
  progress event was missed. A running batch also names the exact active render
  job it is currently waiting on; Cut does not imply that every job is retryable.
  `jobs.retry` is deliberately narrow: it permits one linked retry for an
  eligible failed default-output Recording Studio export after revalidating the
  project revision, capture inputs, edit plan, and output lease, or for failed
  receipt-bound output verification after revalidating the active project,
  immutable render receipt, and exact rendered bytes. It never generically
  replays historical job arguments.
- Settings > Health & Recovery explains whether the disposable project cache
  matched or was rebuilt from durable history, whether a replay snapshot was
  accepted, and how many newer journal records still replay on reopen. It is a
  read-only projection of the existing revision-bound health check. The same
  page reports the bounded apparent size and latest file-change time of only
  recognized flat rebuildable proxies and thumbnails, including the portion no
  longer referenced by current project assets. Its cleanup preview shows which
  unreferenced files have crossed a 24-hour file-change safety window, keeps
  newer files visibly inside that window, and blocks on a partial scan. It does
  not call file-change time "last used" or infer that active work has stopped,
  does not follow symlinks or unexpected directories, excludes foreign files,
  exports, captures, receipts, and source media. A non-destructive Editing
  cache control can queue only a bounded deterministic `project.cache_rebuild`:
  it verifies current source identity, reserves exact output ownership before
  work, reports queued/up-to-date/items-needing-attention counts, and keeps unfinished
  reservations resumable after cancellation/restart. A separate cleanup control
  first asks the server for a read-only durable-ownership
  `project.cache_preview`, then requires in-panel confirmation before starting
  a cancellable `project.cache_purge` job. That job has an exclusive lease and
  revalidates its revision, flat roots,
  ledger, and file identity; it can remove only aged, unreferenced,
  ledger-owned proxy/filmstrip files and fails closed on legacy, foreign,
  symlinked, partial, or changed cache state.
- Verification receipts: deterministic checks, judge review, pregate, pacing,
  captions, delivery, brand, loudness, video scopes, and related fix loops.
  Review can re-run the output-only checks for one exact persisted render; Cut
  revalidates its receipt/hash/profile, never re-renders, and writes a separate
  immutable verification receipt without replacing the original evidence.
- `verify.judge` ships with its access-ladder adapter instead of requiring a
  hidden external script. It samples the rendered output and drives the first
  working local subscription CLI in the order Claude, Codex, Antigravity, then
  Grok; a detected rung that fails infrastructure checks falls through in auto
  mode, while a named backend forces one rung. Settings reports the CLI and
  adapter runtime independently, and an absent CLI/Python runtime records
  `not_run` rather than fabricating a review.
- Project brand kits: Review → QC stores validated font, palette, caption
  position/size, and delivery-aspect constraints in the project operation log.
  Brand verification reads the saved kit by default, and social bundles enforce
  it automatically while recording whether constraints were stored or explicit.
- Review Scopes tab: run `verify.scopes` on a timeline frame, read luma,
  saturation, white-balance, broadcast-range, and clipping warnings, and
  optionally generate vectorscope, waveform, and histogram image evidence.
- Render and video-like export actions run a preflight check before starting
  the job. High-risk issues block the export, while lower-risk warnings can be
  reviewed with collapsible details, continued, or opened in the manual at
  `cut.export.preflight`. The default banner names user-facing issues such as
  black ending, black/frozen footage, silent export, tiny clips, and black
  borders; raw pregate detail stays under Details.
- Review loop: clip-anchored comments, draft suggested verb changes, apply
  drafted changes under an auto-checkpoint, resolve review comments, export an
  offline render-bound review page, and atomically import its timecoded feedback.
  Adjacent edits from one durable compound action appear in the OPS feed as one
  collapsible, human-labelled unit; expanding it keeps every individual edit and
  its existing review or undo action available.
- Export: NLE XML, OTIO, EDL, SRT, VTT, chapters, transcript, frame, range,
  audio, GIF, and platform publish presets. Desktop OTIO import is opened from
  Assets, runs a read-only track/media preflight, confirms a source hash, then
  replaces the active timeline in one replay-safe operation; offline clips
  remain timed gaps.

## Recording

- Recording Studio surface with a large composition preview, background
  choice, raw-stream status, and focused hotkeys (`F9` record, `F12` marker).
- Capture frame rate keeps the 30 FPS default, offers one-click 24/25/30/50/60
  choices, and accepts a validated custom 1–240 FPS value before recording.
  Each new capture also retains its exact reduced requested decimal and the v1
  nearest-integer backend request separately from the legacy editing timebase.
  After finalization, an optional FFprobe record may show independently measured
  average and nominal rates, decoded frames, and duration; a failed, invalid, or
  unavailable probe is shown as not measured rather than guessed.
- Where Doctor explicitly advertises it (currently the Linux final-source
  normalizer), Recording Studio also offers Source, 1080p, or 720p with a
  Standard or High profile. The selected height is a downscale-only limit, and
  Stop reports the verified final dimensions, cadence, and libx264 encoder under
  Advanced facts. Other recorder backends omit the control and refuse a direct
  request rather than silently changing or pretending to honor output quality.
- **Test microphone** opens the current OS-default input for a bounded sample
  window, reports a real peak without inventing a silence floor, and keeps the
  Start action unavailable until the test finishes. The device name is
  display-only; individual stable device selection is not yet claimed.
- **Timeline voiceover** is a compact control on each unlocked audio track. It
  stays disabled unless the current Record Doctor explicitly admits native
  capture; selecting a microphone, a playhead or In–Out range, and Record does
  not create a browser-capture fallback. After real readiness and a visible
  three-second count-in, the server issues a memory-only owner claim for the
  active Timeline tab and requires it for tick, Stop, Cancel, and correlated
  Preview Out. It correlates exact Preview request/fingerprint/epoch playback,
  stops at observed Out, verifies a sealed private WAV, and places one asset
  plus clip atomically as the one Undoable history entry. Direct microphone
  monitoring is off. An exact active retry reattaches before ordinary project
  revision changes are considered; a different B remains refused while A is
  active. Only `voiceover_start_retry_rejected` proves no matching active owner
  remained and lets the UI discard its non-secret retry identity. Cancel, zero
  samples, and an unusable terminal add no edit. This is candidate-source
  behavior, not installed/native-release qualification.
- Recording Studio can explicitly add one current Windows/macOS camera in
  Auto-edit mode. Doctor exposes only opaque, expiring choices with safe labels;
  Start revalidates the chosen device, waits for a real first frame, and refuses
  missing permission, a busy device, or no-frame delivery without substituting
  another camera. The finalized camera remains a separate editable take bound to
  the screen capture clock, with position, size, visibility, and shape retained
  as replayable Studio events. This is candidate source pending native and
  installed qualification on both hosts.
- Recording Scenes provides a compact named scene strip for **Screen** and
  **Presenter PiP** layouts. The initial scene catalog is frozen and saved
  before Start acknowledges it; live scene switches and the one capture-wide
  elapsed/countdown timer are journaled on the shared recording clock before
  the UI reports them as saved. Presenter PiP remains unavailable until the
  selected camera has been admitted, while a Screen-first catalog can start
  without opening a camera. This is candidate source pending native and
  installed qualification.
- Screen recorder doctor, system-audio probe, start, status, stop, studio-event, autoedit, polish, and
  export verbs. `screen_record.autoedit` is the plan step reached through the
  Stop/auto-edit workflow and agent API; it is not a separate visible button.
  The short-lived `screen_record.status{capture_id}` readiness seam admits
  automation only after a real native screen frame reaches Cut and only while
  that capture remains non-terminal; process start, elapsed time, and output
  growth do not substitute for pixel delivery.
- On Windows and macOS, Doctor monitor rows include an opaque native display ID
  only when the exact identity is available. The Record picker passes it
  unchanged to `screen_record.start{monitor_id}`, which revalidates that exact
  display before capture and never substitutes an ordinal, label, primary state,
  or geometry. The visible source control keeps **Display** and **Window** as
  separate first-level choices and never mixes their targets. The familiar
  ordinal path remains only when no ID is available. Region remains hidden from
  the public UI while its private GPU-cropped native path awaits compiled and
  installed qualification; no coordinate form is exposed. Linux
  keeps source selection in its system portal.
- Doctor reports system audio as a separate optional card. A compiled backend
  remains `unknown` until an actual recording proves packets; Doctor never opens
  a loopback/tap stream or triggers macOS Audio Capture consent, and this
  passive card does not block ordinary screen recording.
- Record includes an explicit **Test system audio** action. It opens only the
  native loopback/process-tap path for 0.5–5 seconds, may show macOS Audio
  Capture consent, and reports green only after a real packet arrives with a
  detected signal; silent packets remain a routing warning. Play a short sound
  during the test. Temporary Linux/Windows WAV data is held in an
  exclusive private directory and removed before return; macOS samples stay in
  memory. The action never starts screen capture or creates a project.
- Stop/auto-edit preserves the finalized capture frame rate in its plan. MP4
  recorder export normalizes sparse or variable-rate source timestamps to that
  planned rate, so a high nominal codec rate cannot lengthen the finished clip;
  it validates capture-local microphone/system artifacts and mixes system audio
  at its measured first-packet offset through the normal plan render.
- On Linux, Doctor keeps a prompt-deferred XDG ScreenCast portal `unknown` and
  non-green; its separate `start_allowed` field permits only that exact state to
  open the user-initiated source picker. Missing, degraded, and other unknown
  required cards still block recording.
- Live Studio background/marker events are appended to the bounded
  `studio-events.jsonl` journal beside the capture and replayed into the
  polished plan. Cut still reads legacy `studio-events.json` captures.
- Recording output is converted into normal Cut media and timeline edits with
  recoverable cached artifacts.
- Raw stream discovery reports screen, camera, mic, system audio, and Studio
  metadata. A finalized camera take carries an integrity-checked `CameraArtifact`
  and shared-clock range; Auto-edit lowers it to the existing editable camera
  overlay rather than baking it into the screen source.
- Crash-resilient recordings write independently finalized, verified checkpoint
  segments. Restart recovery can salvage a playable `recovered.mp4` prefix with a
  receipt and explicit lost-tail bounds; it never promotes an open encoder MP4 or
  represents salvage as a completed timeline project. Normal `source.mp4` stitching
  materializes measured encoder-restart gaps to preserve the event/mic/system-audio
  wall clock. `screen_record.recovery_status` is a read-only, project-scoped,
  paginated receipt view: it returns safe capture ids and loss state but never paths
  or recovery side effects. Settings → Health & Recovery reads this inventory rather
  than inferring recovery from the recorder doctor; it treats a complete lexical read
  as evidence reported in that check, and exposes only the existing Open Record route.
  Its cursor must be an emitted capture id, and both its capture and nested receipt
  states use lowercase snake case.
- When a sealed capture receipt contains a measured system-audio first-packet
  offset, Health & Recovery reports that historical packet-timing evidence for
  the project. It does not reinterpret the evidence as a current machine-level
  permission grant, and it never opens an audio stream while checking.
- Microphone and system-audio samples stream to same-directory partial
  WAV files with bounded memory; Cut publishes each raw stream only after its
  header finalizes, and ends the shared capture before classic WAV capacity can
  corrupt or desynchronize a long recording.
- Windows system audio uses native endpoint-independent process loopback on
  Windows 10 build 20348 or newer, so Cut does not open the physical render
  driver. If security software denies the audio worker, screen and microphone
  capture continue and the missing system-audio stream is reported explicitly.
- Linux system audio uses the native PipeWire default-sink monitor. Its first
  nonempty packet is measured on the recording clock before WAV I/O; a finalized
  no-packet WAV is explicitly null-timed and not inserted automatically, while a
  PipeWire connection, format, or capture failure removes the partial WAV.
- macOS 14.2 or newer captures system audio through a Core Audio process tap
  alongside ScreenCaptureKit video. The installed signed app requests Screen
  Recording and Audio Capture permission separately on first use; restart Cut
  after granting a prompt if macOS asks. A successful capture exposes
  `system.wav` through `raw_streams.system`; `raw_has_system` is true only when
  `mux_raw:true` also included it in the optional combined raw output. Otherwise
  screen capture continues and the absent system-audio stream remains explicit.
- Exports and recordings can use a default export folder or per-action Save As;
  default filename collisions are resolved with a numbered sibling file, while
  confirmed Save As targets can replace existing export media/sidecar files.
- Range exports render through a hidden sibling temp file and publish only after
  the MP4 finishes successfully, so a failed/aborted run does not leave a broken
  final export path.
- Agent/API project mutations accept durable request IDs and optimistic project
  revisions. Identical lost-response retries return the original operation IDs;
  changed payloads or stale revisions fail without duplicating edits.
- The status bar shows the current export folder; click the export-folder chip
  to open Settings at the folder setting.
- Preview → Compare pauses playback and shows a locked side-by-side Before and
  Current pair at one playhead position. Both are exact composed frames: Current
  is the live durable revision and Before is the state before its latest
  timeline-mutating edit (trailing metadata remains part of Current).
  Comparison is read-only, never invokes Undo, and refuses
  rather than displaying a mismatched sequence, frame format, revision, or time.

## Environment And Setup

- `system.doctor` reports compact cards for ffmpeg, perception, dubbing,
  diarization, judge CLIs, and disk health.
- Installable tools and model runtimes are shown as user-outcome cards with a
  status, primary action, and advanced details for paths or diagnostics.
- First-run setup leads with a plain three-step path: video tools first, add
  media next, and CLI agents only when Generate/chat workflows are needed.
- When FFmpeg is confirmed missing, the Preview monitor shows a direct setup
  notice with Install FFmpeg, Guide, and Re-check actions; Install opens
  Settings and highlights the Video processing card.
- The Render/Export area also shows the same plain FFmpeg setup actions and
  guards video-like render/export choices before they fall through to raw
  engine errors.
- Transcription/perception sidecars inherit the resolved FFmpeg/ffprobe
  directory at startup and after tool re-scans, so a selected, Homebrew, or
  app-data FFmpeg is reused by captions and analysis jobs.
- `system.fetch_tool`, `system.setup_perception`, `system.setup_matte`,
  `system.set_ffmpeg`, and `system.set_stt_model` cover the main setup paths.
- Settings > Agent control discovers the exact installed executable and offers
  a copyable MCP client config plus a read-only `system.mcp_test`. The check
  proves initialize/ping/tools-list compatibility and that MCP proxies back to
  the same running engine; it never creates a second project authority.

## Debug And Agent Surface

- UI/debug verbs: `ui.state`, `ui.open`, `ui.screenshot`, `ui.playhead`,
  `ui.select`, `ui.highlight` (dismissible close button/Escape), and
  `debug.screenshot`.
- UI control is confirmed rather than optimistic: `ok:true` is returned only
  after the exact connected UI commits and exposes the requested state.
  Already-open/no-op, unknown, unavailable, disconnected, and timed-out
  requests remain explicit failures. One typed surface registry drives human
  openers, `ui.open`, `ui.state`, palette routes, selectors, and browser tests.
- Command search includes user-facing setup/help entries such as Media Health,
  Proxy imports, Video tools setup, and CLI agent setup; results open the real
  surface and use the same highlight overlay as `ui.highlight`.
- The bundled app contains an indexed interactive manual. Contextual Guide
  actions open its requested article; selecting or reading an article does not
  open, reveal, or execute editor UI. **Show in Cut** is the separate explicit
  reveal-and-highlight action. The currently published online page at
  `https://docs.theshellx.com/manual/cut/` remains legacy reference material;
  separately validated real-frontend online publication is pending.
- Fresh installed builds expose:
  - `GET /api/agent`
  - `GET /api/agent-doc/<path>`
  - `GET /api/verbs`
  - `POST /api/verb/<name>`
  - `cutd mcp`
- Local REST/MCP trust is the supported one personal workstation / one trusted
  interactive environment, not same-user authentication: the default loopback
  server has no token and any local process/account able to connect can drive
  the open editor. Remote use requires an independently authenticated and
  authorized SSH/VPN/ShellX broker or equivalent transport; without it, remote
  access is refused. The brokered Agent Chat routes are separate provider-tool
  policies. See `DEBUG_API.md` and
  `shellx-cut-threat-model.md` before changing deployment.
- Agent-only plugins are a permission fence over the same verb registry, not a
  second extension API. `plugins.list`, `plugins.enable`, and `plugins.call`
  expose the built-in Openverse-assets and matte-runtime scopes to agents while
  rejecting disabled, out-of-scope, or corrupt/unavailable-state calls. A
  corrupt state is visible in `plugins.list`; an explicit `plugins.enable`
  repairs only the named plugin and leaves other plugins disabled.
- WebSocket events are `op_applied`, `job_progress`, `render_done`,
  `receipt_ready`, `project_changed`, `ui_state`, and `doctor_updated`.
- `FEATURE_SURFACE_CONTRACT.md` defines the public human, agent, debug, and
  documentation surfaces a supported capability may expose.
