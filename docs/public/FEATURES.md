# ShellX Cut Feature Inventory

This feature view is bundled with installed ShellX Cut builds for users and
agents discovering the application on a new machine.

For the exact machine-readable contract, use `schema/verbs.json`. For agent
workflow details and full verb arguments, use `skill/shellx-cut/SKILL.md` and
`skill/shellx-cut/reference.md`.

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
- Captions and titles: caption generation, text cards, kinetic captions,
  styling, a caption style preset gallery (built-in looks + save-your-own,
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
  and insert that exact range at the timeline playhead. Video assets with audio
  create aligned linked picture and sound.
- Visual search: index video frames and find moments by content. Results keep
  source time distinct from edited timeline time, jump to the nearest real use
  of a trimmed/reused clip, and open unused hits at the exact Source frame.
- Dedicated global Library workspace: All/Recent/Favorites/Missing collections,
  tags, folders, search/sort/type filters, list/grid density, bulk organization,
  add-to-project, explicit Insert at playhead, and honest dead-link reporting
  (`media_ok` per item). Results are server-filtered and paged 100 at a time
  with visible Previous/Next controls and exact totals, so large collections do
  not create an unbounded DOM. Missing linked sources expose Relink and accept
  only the same content at its new location; different media stays a separate
  Library item.
- Asset sources: local folders, Openverse, Internet Archive, Wikimedia, NASA,
  built-in shape stickers, and provider-backed generation through the user's
  configured generation CLI.

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
  Cut's pinned 2.1.224 contained route in a disposable cwd with native CLI tools
  disabled. Codex keeps the user's normal configuration, native sandbox, and
  permissions; Cut neither copies nor rewrites its login files. Grok receives a
  disposable config/home with native tools disabled and only Cut's filtered MCP
  route, while its existing login file remains in place. Antigravity keeps its
  normal settings, sandbox, and permissions while Cut adds a workspace-local MCP
  entry; that route currently requires macOS or Linux. Each route can inspect
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
  exports, captures, receipts, and source media, and offers no purge action.
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
- Live camera capture is parked for this release. The UI says so directly and
  does not show unusable enable/position/size controls; screen, microphone and
  supported system-audio recording remain available.
- Screen recorder doctor, system-audio probe, start, stop, studio-event, autoedit, polish, and
  export verbs. `screen_record.autoedit` is the plan step reached through the
  Stop/auto-edit workflow and agent API; it is not a separate visible button.
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
- Live Studio background/marker events are stored as `studio-events.json`
  beside the capture and replayed into the polished plan.
- Recording output is converted into normal Cut media and timeline edits with
  recoverable cached artifacts.
- Raw stream discovery reports screen, mic, system audio, and Studio metadata.
  The downstream compositor retains support for a pre-recorded camera file,
  but live capture backends emit no camera stream in this release.
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
- The bundled app points to the online manual at
  `https://docs.theshellx.com/manual/cut/`, including feature deep links such as
  `?feature=cut.left.media_health`.
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
  access is refused. The contained Claude Agent Chat broker is a separate
  provider-tool policy. See `DEBUG_API.md` and
  `shellx-cut-threat-model.md` before changing deployment.
- Agent-only plugins are a permission fence over the same verb registry, not a
  second extension API. `plugins.list`, `plugins.enable`, and `plugins.call`
  expose the built-in Openverse-assets and matte-runtime scopes to agents while
  rejecting disabled, out-of-scope, or corrupt/unavailable-state calls. A
  corrupt state is visible in `plugins.list`; an explicit `plugins.enable`
  repairs only the named plugin and leaves other plugins disabled.
- WebSocket events are `op_applied`, `job_progress`, `render_done`,
  `receipt_ready`, `project_changed`, `ui_state`, and `doctor_updated`.
- Feature changes must follow `docs/public/FEATURE_CHANGE_WORKFLOW.md` so the
  contract, engine, UI, debug, skill, docs, tests, and packaging stay in sync.
