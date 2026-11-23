# ShellX Cut Feature Inventory

<!-- shellx-cut-release-truth: published; version=0.6.120; published=0.6.120 -->

This feature view is bundled with installed ShellX Cut builds for users and
agents discovering the application on a new machine.

For the exact machine-readable contract, use `schema/verbs.json`. For agent
workflow details and full verb arguments, use `skill/shellx-cut/SKILL.md` and
`skill/shellx-cut/reference.md`.

## v0.6.120 published

v0.6.120 remains the latest published release.

- Controls that do not apply are no longer shown. The timeline toolbar follows the selected clip: no Speed for captions; no Color, Transform, Remove background or GIF for sound. The source monitor shows only the targets a source has. Pause is shown only on macOS, where it exists.
- Dragging a clip from Assets onto the timeline overwrites at the drop position; hold Alt to insert it and push the rest to the right. A label shows which will happen.
- Record page: messages say in plain words what to do; a check that has not run says "Not checked"; markers and the timer follow the recorded time after a pause; a Start that is refused keeps you on the Record page and says why, and one refused by the checks leaves no empty project; the camera size slider is reachable in a small window.
- Export: the warning before an export says in plain words what was found or was not checked, and it also opens when the checks could not run. You always choose whether to export.
- A damaged picture file is refused when you add it instead of stalling the preview; in an existing project it shows as a file that cannot be read.
- Premium background removal uses updated tools and shows the real download size before the install. It needs a current NVIDIA driver, and cards of the 2017 Volta generation (Titan V, Quadro GV100) are no longer supported by it; existing installs keep working.
- In a wide window the preview stays visible beside Properties.
- Windows: saving the recent projects list, the Library and the search index no longer fails while another program reads the file.
- For agents: every refusal names the next step, including pause on Windows and Linux; the system report says what this system can do.
- Many smaller fixes.

## v0.6.119 published

This section records the published v0.6.119 release.

- Screen recordings: the zooms can be edited after recording. Select the recording in Edit and open Zoom in the Inspector: change a zoom's strength, target and timing, add or delete zooms, or redo the automatic ones, then press Update video.
- Screen recordings with a camera: the camera is on its own Camera track. Move it, resize it, change its shape, cut it or hide it for a stretch, or put it back as it was recorded. A switch on the Record page keeps the camera drawn into the video as before.
- Overlay clips can be a circle or a rounded square, with an optional ring.
- "Make a copy" of a project carries the original recordings, so their zooms stay editable in the copy.
- Record page: a paused recording says so in plain words and its timer stands still; the list of technical checks shows one check per line and closes when you click outside it.
- Rebuilding missing previews: one number says how many items need attention, and each skipped item says why.
- For agents: commands to list and edit a recording's zooms and to reset its camera.
- Many smaller fixes.

## Editing new polished recordings

- Select a new recording's screen clip in Edit and open the Inspector Zoom card.
  Select, add or delete zooms; adjust strength, target, timing and speed. Redo
  automatic zooms keeps your own zooms. Press Update video to apply changes to the
  picture; playback keeps the previous zooms until it finishes. Old recordings
  and Raw MP4 files stay as they are.
- A separate camera track is on by default. Select the Camera track clip for the
  Inspector Camera section: four corners and centre, size, Circle / Rounded /
  Rectangle, ring, and Back to as recorded. Drag the bubble on the preview, or
  split at both ends and delete a piece to hide it for a stretch. Camera changes
  support Undo. Before recording, turn off Edit the camera later (own track) on the
  Record page to bake the bubble in instead.

## v0.6.118 published

This section records the published v0.6.118 release.

- Clearer wording across the app and the manual, following the terms of the leading video editors.
- Light theme: every label is readable, and keyboard focus is visible.
- On Linux, recording says when no microphone is available before trying to use it.
- Reversing a video clip also reverses its linked sound.
- A 1080p recording at 60 fps is labelled High frame rate instead of 4K source.
- The setup status always shows the newest check and says when FFmpeg is older than the tested versions.
- Fixed: exports could lose a picture where clips that are not whole frames meet.
- Fixed: a new project lost the size you chose after a 4K import.
- Many smaller fixes.

## v0.6.117 published

This section records the published v0.6.117 release.

- Fixed: screen recordings on macOS keep moving as they are saved in parts.
- Fixed: Windows screen recordings keep moving between saved parts.
- On Windows, Stop responds at once while the recording finishes saving in the background.
- On Windows, recording checks video support before starting and leaves minimized windows alone during readiness checks.
- The Record screen shows whether the current recording is paused.
- On Linux Wayland, screen recording no longer requires keyboard input access.
- Fixed: Add to timeline messages follow the right recording and project.
- Fixed: recordings already added to the timeline stay marked as added during checks and no longer keep offering recovery.
- On Windows and macOS, recording says when no microphone is available before trying to use it.
- Fixed: the chosen export folder is kept when updating from 0.6.115.

## v0.6.116 published

This section records the published v0.6.116 release.

- The preview shows the finished picture when you pause, with titles, effects
  and fades.
- Exports match the timeline exactly: picture and sound stay in sync on
  timelines with many cuts or mixed frame rates.
- Closing or quitting during a recording saves the recording first; during an
  export, ShellX Cut asks before stopping it.
- A recording or export cut short by a crash or power loss is reported at the
  next start; recordings come back with their sound and say how much was
  recovered.
- Stopping a screen recording is quicker, and a recording made while an export
  runs is kept.
- One export folder for all projects, remembered between sessions and shown in
  the status bar.
- FFmpeg can be downloaded from inside the app on Apple-silicon Macs too, and
  every download shows its publisher, version, size and licence first.
- When a feature needs a tool that is not installed, ShellX Cut says what is
  missing and offers the download in Settings, including Microsoft's Visual C++
  runtime for captions on Windows.
- In the timeline, dragging a clip overwrites and Alt-dragging inserts, and the
  timeline names which one you are doing.
- An export that stops making progress ends with a clear message, and a render
  queue tells you when some of its exports failed.
- ShellX Cut opens in one window that fits your screen.
- Fixed: playback looping or jumping back to 00:00 when the middle part of a
  split clip is on a second video track (#2).
- Many smaller fixes.

## v0.6.115 published

This section records the published v0.6.115 release.

- Linux Camera: Recording Studio gains a selectable camera source with explicit
  device readiness and capture status.
- Bugfixes: export, job status, project switching, and Voiceover recovery are
  more reliable.
- Export warnings offer an explicit Export anyway or Queue anyway choice for
  intentional still footage.
- Windows and macOS window recordings preserve the native mouse cursor.
- Finishing-recording messages stay clear of the top-bar buttons.

The v0.6.115 release is published with native installed qualification recorded in the release evidence.

## v0.6.114 published

This section records the published v0.6.114 release.

- Recording Studio keeps the shared **Edit | Record** switch in the editor
  header while giving capture a focused workspace. Capture readiness, source
  selection, and rehearsal lead the flow; its technical checks stay under
  Details, and the reserved recording transport does not cover the scrollable
  setup. Editor-only chrome stays out of Record. Its live status is
  process-local evidence, not a timer or file-growth guess: it distinguishes an
  armed exact-source close from an ordinary Cut-owned stop and discloses only a
  safe controller-placement conclusion, never a native window, process, or
  source identity.
- An explicit native source preview can show the exact current display or
  window on Windows and macOS, or a fresh user-chosen Portal source on Linux.
  Frames stay bounded in memory; permission, source loss, recursion, pause,
  hide, and stop states remain visible and release their native owner.
- Recording now shows continuous microphone and system-audio confidence from
  the already-admitted capture streams themselves, without monitoring playback
  or a second input stream. The live status keeps stale, stopped, device-lost,
  and unsupported inputs distinct from a current level; unsupported input
  meters remain labeled unavailable.
- A video-only 3–5 second rehearsal records into an owned temporary area for
  immediate playback, then discards without creating a project, recovery item,
  or timeline media. During a normal take, compact live controls expose the
  real Stop, marker, scene, timer, and audio-meter state. Pause/Resume is
  available only on macOS.
- Health & recovery can retry one eligible failed recorder export or
  verification rerun only while its durable lineage and immutable inputs still
  match. Recorder retry work renders from private staged copies, so a queued
  retry cannot silently reopen changed inputs.
- Editing-cache rebuild now offers a verified work-unit estimate and uses
  cancellation-aware renderer ownership. Confirmed cleanup reports exact
  before/removed/after reconciliation, including truthful partial progress and
  ledger-recovery state after a failure.
- Bulk relink evidence now survives project reopen and is required by the
  portable-copy workflow when the current revision is the relink operation.
  Project/revision switches clear stale relink and package UI results.
- Settings > Video & performance can run a bounded read-only ShellX Motion runtime, catalog, and
  fixed connector-descriptor check. It verifies structural compatibility only:
  connector execution remains unqualified. One-click Install, Repair, Update,
  and Remove are deliberately unavailable until `MOTION-DIST-01` supplies a
  verified immutable platform distribution manifest and matching artifact; a
  source checkout, PATH binary, or npm package is not presented as a managed
  installation.

These capabilities belong to the published v0.6.114 release. This feature
inventory describes supported behavior; release qualification is recorded separately.

## v0.6.113 published

This section preserves v0.6.113 feature history.

- Transcript is now a timeline-linked phrase and chapter list with exact
  start-end ranges and one shared time control for clips, captions, markers,
  comments, cited search results, and review evidence. Reused or unavailable
  source media explains when an exact occurrence must be chosen or cannot be
  opened.
- Recording Studio adds separate **Display** and **Window** choices, a
  cancellable 3- or 5-second start countdown, an independently configurable
  on-video timer, and a separate retained camera take when Doctor lists a current
  device on Windows or macOS. The camera track is hidden, and its bubble is baked
  into the polished clip; editing the retained take does not update that bubble.
  With no connected camera or unavailable discovery, Camera stays disabled
  while screen recording remains available. Supported macOS builds also offer durable **Pause & resume**
  for compatible display recordings.
- Timeline Voiceover records into an unlocked audio track after a visible
  count-in, follows the playhead or selected In-Out range, supports Stop and
  Cancel, and places the sealed take as one Undoable edit.
- Caption Inspector Find & Replace previews **Contains** or **Whole word**
  matches on one track and an optional timeline range, then applies the reviewed
  batch as one Undoable change without moving caption timing.
- Preview **Compare** pauses playback and shows exact composed **Before** and
  **Current** frames at the same playhead position without invoking Undo.
- Health & recovery can rebuild missing or stale Cut-owned editing proxies and
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
- Health & recovery adds a read-only `project.cache_preview` and explicitly
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
  Only files are imported by a drop: folders and files of an unsupported type are
  named in a notice that says why they were left out (open the folder and drop the
  files inside it).
- Sequence Index: search clip and marker metadata across every active or inactive
  timeline from Find → Sequence, filter by sequence/result/track kind or live
  status (issues, offline, gaps, effects, hidden, locked, muted), then open the
  correct sequence at the result time. Result rows expose basenames, stable ids,
  effect names and track state without disclosing source paths; the currently
  shown bounded rows can be copied as escaped, spreadsheet-safe CSV for quality-check
  handoff.
- Timeline editing: split, trim, ripple delete, move, insert, paste, add track,
  restore, markers (with labels and colors), speed, fades, crossfades,
  transforms, crops, and undoable operation replay. The selected-clip Inspector
  offers both quick speed-ramp presets and a compact custom curve editor with
  ordered, millisecond-accurate source-time points from 0.25× to 4×; invalid
  or out-of-range curves stay unapplied with a concrete inline reason.
- Color: the selected visual clip's **Color** tab can tag its input as Rec.709,
  Rec.2020, sRGB, or scene-linear; the project can set matching working and
  output spaces. Cut converts those explicit tags through its lightweight
  color-management path before grade/effects. This does not claim camera Log
  interpretation or an end-to-end HDR mastering or delivery workflow.
- A probed video with audio is inserted as one linked picture-and-sound action.
  Cut validates both destinations (or creates both) and the shared source range
  before committing either clip, so an invalid second leg leaves no partial
  picture, sound, or track. Moving or trimming either half also changes its exact
  counterpart atomically; deliberate split edits can opt out with `linked:false`.
- Dragging a clip moves it in one of two modes, named as in Premiere Pro,
  DaVinci Resolve, Final Cut and Avid. A plain drag is **Overwrite**: the clip
  replaces what is under it and nothing else moves on any track. Hold Alt for
  **Insert**: the clip goes in and pushes the rest of every track right. The
  label next to the pointer names the mode while you drag; the old place stays
  as a gap in both modes.
- One frame is the smallest clip. An edit that would leave a clip shorter than
  one frame is refused with the nearest value that works, and edit points less
  than a frame from a clip edge snap to the edge. A sub-frame clip left by an
  older version is drawn as an amber marker you can select and delete.
- Ripple trims are available from the toolbar and default Q/W
  bindings: Q removes from the playhead to the selected clip's start, W removes
  from the playhead to its end, and the remaining linked picture and sound close
  the gap together instead of deleting the whole selected clip.
- The timeline toolbar exposes Video track and Audio track. Empty
  user-created tracks remain available through unrelated edits and deletes.
- Add at playhead inserts media on the base story timeline and pushes later
  clips right. Dragging from Assets overwrites what is under the drop; hold Alt
  to insert and push later clips right. The dropped lane chooses the track:
  video goes on top only when dropped on an overlay lane. Empty space uses the
  base track. The preview shows Overwrite or Insert before release.
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
  mixer and as a compact timeline-header shortcut. A mono source plays on both
  channels at its own level, as a stereo source at centre does.
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
- The preview shows the finished picture when paused: COMPOSED is on by
  default, so the paused frame is the exact composed frame with titles,
  effects, fades and grades. While playing, the live preview draws what it can
  and a note on the picture names anything that appears only when paused
  ("Playback preview: fades appear when paused"). With COMPOSED off the preview
  is live only and says what it leaves out; the choice is remembered. On Linux,
  on a heavily loaded machine the picture can hold briefly at a cut while
  playing; the paused frame and exports are exact.
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
  and first-import auto-placement on an empty timeline (the first import
  requested is placed, whatever order the files finish analysis in; each import
  result says whether it was placed and why not). Later imports wait in
  Assets until the user clicks Add at playhead or drags them into the timeline. Library
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
- Opening a project checks its proxies in the background (status bar
  "Checking proxies"): proxies this computer made are used again, one it has no
  record of (from an older version or another computer) is made again once, and
  the ones its footage lacks are made; the preview plays the original media
  meanwhile. After the update to v0.6.116 an existing project therefore makes
  its proxies again once, at its first open, because the older proxies have no
  such record; for that first open the preview shows the original media (or
  "making proxy") for a while. On Linux the live preview may show a still frame
  of a large original file until its proxy is ready; the paused frame and
  exports are exact. Missing title movies are drawn again when they are needed.
  The same open also reads each clip's media facts (kind, sound, size, frame
  rate) again from its own file, and draws again every title movie and
  timeline thumbnail strip this computer did not make for exactly its asset
  and settings; a proxy counts only for the asset and source it was made for.
  After the update the title movies, thumbnail strips and proxies of an
  existing project are therefore made once more at its first open (a short
  wait; nothing to do); facts of a missing or changed file count as unknown, so
  a quality check is measured rather than skipped.
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
- Speaker diarization: **Transcript Tools > Label speakers** runs
  `media.diarize` after checking the configured Sortformer v2 Doctor card,
  binds the selected current-project asset, and polls the exact returned
  job/receipt before refreshing transcript speaker labels. Multicam switching
  can use those labels with `mode:"speaker"`. One run accepts up to 2 hours of
  audio; a longer asset, or one with no readable audio, is refused with a plain
  reason (trim or split it, or choose an asset with speech audio), and the
  service's reachability advice is given only for a real service failure.
- Dubbing and translation: **Transcript Tools > Dub audio** checks the
  configured OmniVoice Doctor card, binds the selected asset and target
  language, then accepts only the matching track and receipt before refreshing
  the project. `audio.dub` creates a new translated voice track; text translation
  uses the CLI agent first, then local translation only as fallback.
- Assemble: the human-visible `assemble` drawer turns existing footage into
  highlights/repurposed edits, plans vertical shorts, matches a script to
  footage, or fills a b-roll slot. `assemble.repurpose`, `assemble.shorts`,
  and `assemble.from_script` return a revision- and transcript-bound review
  plan first. The
  visible **Add reviewed plan to timeline** action commits approved ranges as
  one editable, normal-Undo operation; `assemble.shorts` also applies its
  planned source crop and transcript-derived captions only when the open
  project already has the requested aspect. If the reviewed transcript word
  spans change without a timeline revision, Cut refuses Apply and asks for a
  new review.
  `assemble.broll` remains its
  direct retrieve-and-place workflow. If a delayed search finds that the selected
  project or revision changed, it returns the origin identity, last accepted revision,
  and checkpoint as a partial result; verify the identity, compare the current
  revision, and review intervening changes before restore. It
  never directs a revert at the now-current project. None produces an opaque generated movie.
- Repurpose / Find highlights: the `clips` drawer uses `clip.candidates` plus
  model-free `score.clip` explanations to rank standalone moments, then hands
  selected windows to social delivery without changing the source edit.
- Autopilot: the `autopilot` drawer previews or runs `autopilot.run`, a bounded
  render → verify → low-risk fix → re-verify loop under one checkpoint. It
  stops on no progress and never converts a failed or unmeasured receipt into a
  pass. If checks still fail when it stops, the job reads as partly done.
- Recipes: all 11 bundled workflows are First edit, Edit for clarity, Podcast
  repurpose, Talking-head cleanup, Screen-demo polish, Phone clip cleanup,
  Social short bundle, Blur or mask an area, Add captions, Export for YouTube,
  and Export for TikTok. Timeline-changing recipes show their exact plan before
  Run.
- Agent chat: a CLI agent can operate the live project through cutd's MCP verb
  surface, producing normal reversible edit operations. A turn can attach up to
  eight registered project assets as references; cutd validates their IDs and
  keeps source paths behind `project.state`. Timeline **Ask agent** and Comment
  **Make changes** attach an immutable project identity/revision plus the exact
  range, clips, or point. Cut validates that target again before launch and
  returns it with the turn rather than substituting a later selection. A turn
  applies its validated result directly and records a stable pre-edit baseline,
  tip, and exact diff. **Step back** is available only while the guarded whole-
  turn revert is safe; **Ask replacement** re-resolves the retained target and
  visibly refuses a deleted or stale one. Concurrent human/system edits disable
  Step back rather than risking rollback of someone else's work. There is no
  Draft, Accept, or editor review gate. A bounded local-device history keeps
  validated requests, targets, replies, actions, and revisions for each exact
  project identity, and marks interrupted turns honestly. A categorized
  prompt library pre-fills eight common Polish, Repurpose, Speech, and Review
  outcomes without sending or spending an agent turn until the user presses Send.
  Each preset advertises only the contained Chat inspect/edit verbs. Presets that
  need an external speech service, rendering, delivery, or verification tell the
  user to open the relevant Transcript Tools, Clips, or Review > Quality checks surface;
  pre-filling text does not grant or execute those capabilities.
  Agent Chat launches the user's installed Claude Code, Codex, Grok, or Antigravity CLI.
  The optional Model field passes a model name only to the selected provider;
  leaving it empty uses that CLI's configured default. Claude uses
  Cut's contained capability route in a disposable cwd with native CLI tools
  disabled. Provider version text is informational only and each route's required
  containment flags are verified before each turn. Codex uses the selected
  account's configuration, native sandbox, and permissions; an enrolled alternate
  account home reaches only the Codex child through `CODEX_HOME`. Cut neither
  copies nor rewrites its login files. Grok receives a
  disposable config/home with native tools disabled and only Cut's filtered MCP
  route, while its existing login file remains in place. Cut grants trust only
  to that newly created empty workspace for the duration of the turn so Grok
  can start its project-scoped Cut MCP server. Antigravity keeps its
  normal settings and login while Cut creates a new disposable sandboxed project
  containing one Cut-only MCP plugin. Headless approval is bounded by that empty
  workspace and Cut's filtered server-side verb policy; the resolved CLI's full
  launch contract is verified before each turn on every supported platform. Each
  route can inspect the open project and apply reversible in-project edits.

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

### Optional Motion bridge (connection unqualified)

Read-only discovery is the qualified v0.6.114 baseline. The entire bridge
subsection below describes existing optional CLI integration, including Edit in
Motion, refresh, tracking and round-trip routes. Connection qualification and
managed distribution/lifecycle remain deferred; these descriptions do not
establish a tested editing workflow for the current release.

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
### AI media generation

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
  Timeline contact sheets, and subject-aware reframe. Social bundles include
  an atomic hashed manifest and an honest ready/needs-review/blocked package
  verdict across platform quality checks, caption writes, thumbnails, and brand checks.
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
  replays historical job arguments. A background job that fails and cannot be
  retried, whoever started it (the person or an agent), stays visible in the
  status bar with what failed, the plain reason and the suggested next step
  until it is dismissed; the newest three are shown and older ones are counted.
  Details opens Settings > Health & recovery, which lists the full message.
  Cancelled, replaced and restart-interrupted jobs are not shown as failures.
  Rows reset when another project opens and can take up to 3 seconds to
  appear; failures from before the window opened are listed in Health &
  recovery. A render queue in which some exports failed says so ("done, but 1
  of 3 failed") and shows a status-bar row; when all failed, the queue itself
  failed. A stopped recipe is a failed job with its report kept. For agents,
  `outcome:"partial"` marks a job that ran to its end but did not fully
  deliver.
- Settings > Health & recovery explains whether the disposable project cache
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
  cache control can first request an `estimate_only` bounded deterministic
  `project.cache_rebuild` admission: current source identity is verified and
  the UI receives source bytes, required outputs, and proxy duration as work
  units without a job or reservation. Scheduling verifies again, reserves exact
  output ownership before work, reports queued/up-to-date/items-needing-attention
  counts with each skipped asset's reason and next action (an asset whose previews
  already match its source is reported as up to date, not as skipped), and keeps
  unfinished reservations resumable after cancellation/restart. Freshly generated previews
  retain their verified source identity. Existing files without matching
  engine-origin evidence stay in place and are reported for attention.
  A separate cleanup control first asks the server for a read-only durable-ownership
  `project.cache_preview`, then requires in-panel confirmation before starting
  a cancellable `project.cache_purge` job. That job has an exclusive lease and
  revalidates its revision, flat roots,
  ledger, and file identity; it can remove only aged, unreferenced,
  ledger-owned proxy/filmstrip files and fails closed on legacy, foreign,
  symlinked, partial, or changed cache state. Its terminal job result reconciles
  exact before/removed/after bytes and files; a cancellation after a removal
  discloses the partial progress instead of implying no cleanup occurred. A
  normal terminal result uses a strict post-scan; when a project switch has
  removed its ProjectStore, the cancellation reports the exact exclusive-lease
  delta instead. If a file was unlinked but its ledger publish failed, the job
  is explicitly failed, records the known partial removal, and leaves the stale
  ledger entry visible so later cleanup stays blocked until recovery.
- Verification receipts: deterministic checks, judge review, pregate, pacing,
  captions, delivery, brand, loudness, video scopes, and related fix loops.
  Review can re-run the output-only checks for one exact persisted render; Cut
  revalidates its receipt/hash/profile, never re-renders, and writes a separate
  immutable verification receipt without replacing the original evidence.
- `verify.judge` ships with its access-ladder adapter instead of requiring a
  hidden external script. It samples rendered frames for an optional visual
  review. Its accepted backend IDs remain Claude, Codex, Antigravity, and Grok,
  with Claude requiring version 2.1.248 or later, restricted Read capability,
  and a safe copied-frame workspace, and Grok requiring version 1.0.21 or later
  and its no-model-tools policy. Codex offers direct-frame review when Doctor confirms image/schema
  and tool-free admission with `judge.codex.details.judge_ready:true`.
  Installation alone is insufficient. Antigravity remains unready for render
  judging and an explicit request returns `not_run` before review work; this
  does not change Agent Chat or generation availability. Auto skips unready
  rungs in its configured order;
  an attempted ready-rung infrastructure error may continue to the next ready
  rung, while a named backend never falls back. Doctor admission reports
  readiness; a completed receipt records the actual review.
- Project brand kits: Review → Quality checks stores validated font, palette, caption
  position/size, and delivery-aspect constraints in the project operation log.
  Brand verification reads the saved kit by default, and social bundles enforce
  it automatically while recording whether constraints were stored or explicit.
- Review Scopes tab: run `verify.scopes` on a timeline frame, read luma,
  saturation, white-balance, broadcast-range, and clipping warnings, and
  optionally generate vectorscope, waveform, and histogram image evidence.
- Render, render queue, and video-like export actions run a preflight check before starting
  the job. Quality warnings, including high-risk predictions, can be
  reviewed with collapsible details and explicitly accepted with Export anyway
  or Queue anyway. Missing media and video tools remain actual errors. Open the manual at
  `cut.export.preflight`. The default banner names user-facing issues such as
  black ending, black/frozen footage, silent export, tiny clips, and black
  borders; raw pregate detail stays under Details.
- The render receipt, the job result and the dry-run plan name the encoder
  that ran (`encode`). When hardware encoding was asked for and software ran,
  `encode.fallback_reason` says why in plain words. A frame larger than the
  hardware encoder takes is named with that encoder's limit (H.264 hardware
  encoders stop at 4096 px; HEVC is offered when it reaches the size), and a
  hardware encoder that fails during the render makes Cut run the whole render
  again in software. `render.bundle` and `render.reframe` receipts carry the
  same record. When the experimental GPU render path was asked for, the record
  says whether it ran and, if not, why.
- Before a render starts, Cut estimates its memory and output size and compares
  them with free memory and disk; the export preflight shows a shortfall with
  its numbers. Software H.264 uses all of the computer's encoder threads when
  the memory is there; only when it is not does the render use fewer threads
  (as few as needed), and the progress message and the receipt say how many
  and why. A render that cannot fit is refused with `out_of_memory` or
  `not_enough_space` and the way out, and a render the system stops for memory
  says so instead of a bare exit code. The estimate counts a clip used several
  times once when its frames pass through unchanged, and a long render by its
  largest window. Every clip of an export carries exactly the frames of its
  place on the timeline (the timeline's frame grid, also across frame-rate
  and speed changes), so the picture stays on the sound after any number of
  cuts, a long render in parts is frame-identical to one pass, and
  `render.frame` shows the frame the export has at that time. A render that stops working (no new output and almost no
  processor use for 2 minutes) ends with `render_stalled`, naming the part of
  the timeline and the clip, instead of staying "running". A long reversed
  clip renders part by part, so it no longer needs memory for the whole clip
  at once.
- A mono source plays on both channels at its own level (in v0.6.115 it came
  out 3 dB quieter per channel).
- The Mixer's meter stems, the Preview monitor mix, Listen exports and the join
  files of long renders live in a private, owner-only work folder of the app,
  removed when the project closes or the engine stops, so `exports/` holds only
  what you render. The segment windows of a long render are still cached in
  the project's `.cache/` folder.
- Review loop: clip-anchored comments can route their text and immutable target
  through Agent Chat with **Make changes**; Chat applies its normal direct,
  reversible edit receipt, and **Step back** remains the guarded recovery path.
  Comments can still be resolved, exported as an offline render-bound review
  page, and imported atomically with timecoded feedback. Legacy
  `comment.draft`/`comment.apply` remain agent-only compatibility and diagnostic
  APIs, never an editor approval flow. Adjacent edits from one durable compound
  action appear in the Edit history feed as one collapsible, human-labelled unit; expanding
  it keeps every individual edit and its existing review or undo action available.
- Export: NLE XML, OTIO, EDL, SRT, VTT, chapters, transcript, frame, range,
  audio, GIF, and platform export presets (files saved locally). Desktop OTIO import is opened from
  Assets, runs a read-only track/media preflight, confirms a source hash, then
  replaces the active timeline in one replay-safe operation; offline clips
  remain timed gaps. Replace probes media EOF and refuses overlong source ranges,
  except exact Cut frame-rounding drift of at most 50 ms, which clamps with a
  warning; larger or nonmatching out-points require explicit retrimming.
  Interchange carries cuts and media references with frame rounding.
  `export.xml`, `export.otio`, and `export.edl` verb responses warn
  when a format drops nondefault playback
  state such as muted ranges, hidden tracks, or audio mix settings. Use a
  rendered delivery when those edits must be preserved in the output. The
  Export menu shows the complete warning list after a successful interchange
  export, so a saved file is not mistaken for a lossless transfer.

## Recording

- Recording Studio keeps the shared **Edit | Record** switch and a persistent
  Start/Stop transport. Screen and sound, real selected-source preview, and the
  Camera, Background, Video timer, Start, length & pause, and Video quality settings
  are visible in the Record workspace. The source preview shows actual native
  pixels before Start; during capture its live view uses frames owned by the
  exact active capture. If frames stop arriving, Record says so while keeping
  Stop usable. A selected whole display includes Cut when Cut is visible.
  `F9` is one Cut-wide Start/Stop action, including from Edit or while another
  app has focus when the OS has admitted the global shortcut. Start rechecks
  the current Record choices against the current project, source, devices, and
  permissions. Those choices survive switching to Edit; invalid choices show
  a reason without changing workspace or starting capture.
  On first use with no saved setup, F9 checks the current primary display (or
  the Linux screen-sharing picker) and starts a screen-only polished take with
  microphone, system audio, camera, and keystrokes off. Later F9 presses use
  the current Record settings, or the saved setup when Record has not been opened.
  Record cannot be left during an active take, including while paused.
  When no project is open, Start or F9 creates a new recording project first;
  an already open project is used as-is. The action does not restore a
  minimized Cut window.
  Windows/macOS attempt registration automatically, and GNOME Wayland attempts
  an owned custom binding on first launch. A passive indicator reports global
  scope only after it is observed; conflict, failure, or an explicit opt-out
  leaves focused F9 with its reason. `F12` drops a marker during capture.
- **Readiness and source refresh** put the current capture outcome before the
  source controls; open Details to inspect individual checks. Use **Refresh
  sources** after a display, window, or device change. While it rechecks the
  current setup, Start stays unavailable; an unknown result is not ready to
  record.
- **Camera and background** controls sit in Recording settings. Camera offers
  explicit corner, shape, size, and Reset camera layout choices when an admitted
  device and Polished mode make them available. A disabled control shows why.
- **After Stop** offers exactly Raw MP4 and Polished clip in Edit. Both save the
  unchanged original MP4 in the default export folder. Polished also adds an
  editable clip to the current project (including the project created at Start
  when none was open) with framing and, when source geometry is valid,
  zoom-to-cursor and cursor smoothing. Windows/macOS window capture preserves
  one native cursor in Raw and Polished output without a duplicate synthetic
  cursor track; synthetic cursor styling remains unavailable. Fresh button input
  matched to accepted same-owner frame geometry can drive click highlights and
  zoom; missing or invalid geometry cannot. This does not establish full resized
  window coverage or arbitrary move/DPI correctness. Its
  optional **Show keystrokes** setting starts off and sits
  beneath the mode choice; there is no second polish toggle or pre-record
  destination picker. Export or Save a copy chooses an optional destination
  after Stop. The Recording details disclosure gives stream and pointer facts.
- **Returning to Edit and Stop recovery** keep the active capture in Recording Studio.
  The Edit switch stays unavailable during a countdown, pending Start, a live take,
  finalization, or an unresolved Stop; the visible reason tells you whether
  to cancel, wait, or check status. After a Stop error, Cut checks
  `screen_record.status{capture_id}` for that exact take. Retry Stop is offered
  only when it is still live; a terminal capture is reported as stopped, and
  uncertain ownership is labelled unknown. Later polish failure does not
  imply a live capture.
- **Closing or quitting during a recording** saves the take first, as Stop
  would ("Saving your recording before ShellX Cut closes…"); Polished also adds
  its clip. If the window does not answer within 6 seconds, the desktop shell
  stops the recording itself, and an engine that is shut down finishes a
  running recording first. If Cut ends without that chance (a crash, a forced
  quit, a power cut), the next start names the recording, for example "ShellX
  Cut closed while recording in “…”. What was saved can be added to the
  timeline.", and offers **Add to timeline** for what was saved, with its sound,
  or **Dismiss**, which keeps the files. Save points are written every 15
  seconds, so the video after the last one is lost in such a stop; the notice
  says how much can be recovered, for example "About 15 s of about 20 s can be
  recovered; the last 5 s were not saved before the recording stopped." System
  sound is saved about once a second and comes back too; if none was saved, the
  message says so. Opening the project starts a visible "Preparing interrupted
  recording" job instead of making you wait, and Add to timeline waits until it
  has finished.
  `screen_record.pending`, `screen_record.recover` and
  `screen_record.pending_dismiss` are the same route for agents.
- **Saving a recording.** A recording is saved in parts of 15 seconds. On
  Windows and macOS, the screen keeps recording as one part ends and the next
  begins. Tests at 30 and 60 fps found no picture hold at any of 48 Windows
  part boundaries. Windows Stop answered in 0.14–0.19 seconds, and the file
  finished saving within about 5 seconds in those tests. Cut shows saving
  progress before reporting the file saved; these timings are test results,
  not a limit on how long every recording can take to save.
- **Known Windows recording limits.** About half a second at the start of a
  take can repeat the first picture. Short, separate system sounds can drift
  out of sync with the picture.
- **Closing or quitting during an export** asks first, as large editors do:
  "An export is still running", naming the work, with **Keep working** (the
  default; Esc and clicking outside keep working too) or **Quit and stop it**,
  which cancels the export cleanly (its unfinished files are removed) and then
  quits. A recording is saved first. On macOS the app menu's Quit, Cmd+Q, Quit
  in the Dock and logging out ask the same way as closing the window. If Cut
  ends without that chance (a crash, a forced quit, a power cut, Windows ending
  the session), the next start says in the status bar which export was
  interrupted and removes its unfinished files. A stop that arrives after the
  finished file is written cannot undo it: that export ends done with its
  receipt.
- **If the engine stops by itself**, ShellX Cut says so in a dialog ("ShellX
  Cut engine stopped") and offers **Restart the engine** or **Quit ShellX
  Cut**. After a restart the window reloads and says which work was
  interrupted; the project that was open is not reopened, so open it again.
- **One window per user.** Opening ShellX Cut again brings the running window
  forward instead of starting a second one, and the first window is sized to
  fit the screen it opens on.
- **Start and Stop say what happened.** The bar shows "Starting recording…"
  until the first screen frame arrives and the timer counts from that frame. A
  Stop before the first frame says that nothing was recorded and leaves nothing
  to recover. A window that is no longer available is refused at Start; a
  missing folder and a folder that cannot be written each have their own
  message. A recording started outside this window (for example by an agent
  through `screen_record.start`) is shown with the recording bar and Stop
  (`screen_record.active`).
- On macOS, opening Record or a project never starts a screen capture or the
  screen-recording prompt: Record reads the last check (`screen_record.doctor`
  `{mode:"status"}`), and the system prompt can appear only with an explicit
  check or a recording start.
- Capture frame rate keeps the 30 FPS default and offers one-click 24/25/30/50/60
  choices. Video quality → Advanced frame rate accepts a whole-number custom
  1–240 FPS value only after **Apply frame rate** or Enter; an invalid draft
  leaves the previously selected rate active.
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
- **Test microphone** opens the selected input or System Default for a bounded sample
  window, reports a real peak without inventing a silence floor, and refuses
  every recording start until the test finishes (the Record button, F9, the
  global hotkey and `screen_record.start`): the window says "Finish the audio
  test before recording." and an agent gets `conflict`. Where selection is supported,
  Cut keeps the resolved endpoint private and exposes only safe labels and
  expiring tokens; an unavailable saved selection refuses microphone-enabled
  Start instead of falling back. When microphone audio is on, a problem
  reported by the passive microphone readiness read refuses Start before
  capture, with the same reason, message, and action shown by Doctor. This
  readiness read does not open or test the microphone. Recording with
  microphone audio off does not require a ready microphone. On Linux, a
  completed inventory of an identifiable PipeWire-Pulse audio route blocks
  a mic-enabled take only when it finds no capture source. A silent real or virtual source is still present;
  an unavailable sound server or ambiguous audio route leaves readiness
  unknown and permits Start.
- **Timeline voiceover** is a compact control on each unlocked audio track. It
  needs only the microphone and never checks screen capture. Opening it reads
  the microphone choice and permission; if macOS has not asked yet, the first
  Record voiceover shows the system microphone prompt, and denied access names
  System Settings > Privacy & Security > Microphone. On Windows, opening it reads
  the Windows microphone privacy switches (Settings > Privacy & security >
  Microphone: Microphone access, Let apps access your microphone, Let desktop
  apps access your microphone) and says which one is off, or that no microphone
  was found. macOS and Linux also say when no input is present. A take that
  fails after it started says why in its status.
  The Record panel's microphone control shows the same message before a test.
  Selecting a microphone, a playhead or In–Out range, and Record does
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
  samples, and an unusable terminal add no edit.
- Recording Studio can explicitly add one current Windows, macOS, or compiled
  Linux camera with **Polished clip in Edit** selected when Doctor lists a device.
  No connected camera or unavailable discovery leaves Camera disabled while
  normal screen recording stays available.
  Doctor exposes only opaque, expiring choices with safe labels; Start revalidates
  the chosen device, waits for a real first frame, and refuses missing permission,
  a busy device, or no-frame delivery without substituting another camera. The
  finalized camera remains a separate take bound to the screen capture clock,
  with position, size, visibility, and shape retained as replayable Studio events.
  New polished recordings put the camera on a visible Camera track by default;
  its layout can be edited in the Camera section after recording. The Record
  page switch can bake the bubble in instead. Old recordings are unchanged.
- Camera settings show four explicit corners, Circle or Rounded rectangle,
  size, and a separate **Reset camera layout** action. A chosen transform
  survives internal scene changes until Reset. The on-video timer offers Off,
  Count up, or Count down with an applied custom duration from 00:00:01 to
  23:59:59; its zero does not stop the capture. Capture length and the separate
  start countdown live under Start, length & pause.
- Screen recorder doctor, system-audio probe, start, status, stop, studio-event, autoedit, polish, and
  export verbs. `screen_record.autoedit` is the plan step reached through the
  Stop/auto-edit workflow and agent API; it is not a separate visible button.
  The short-lived `screen_record.status{capture_id}` seam admits automation
  only after a real native screen frame reaches Cut and only while that capture
  remains non-terminal; process start, elapsed time, and output growth do not
  substitute for pixel delivery. Its selected-source lifecycle says
  `source_lost` only after an armed exact native close signal wins before a
  Cut-owned close; generic failure or Stop never receives that label. The same
  read projects no-identity controller-placement evidence: `not_excluded`
  when Windows or macOS confirms whole-display capture can include Cut while
  visible, `unavailable` when placement cannot be proved, or `refused` after a
  failed Windows shell readback. It also reports admitted-stream audio meters;
  it never opens a device, changes capture state, or turns a last packet into a
  live level.
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
  and shared-clock range. New polished recordings normally use a visible Camera
  track; turning off Edit the camera later (own track) bakes the bubble in instead.
- Crash-resilient recordings write independently finalized, verified checkpoint
  segments. Restart recovery can salvage a playable `recovered.mp4` prefix with a
  receipt and explicit lost-tail bounds; it never promotes an open encoder MP4,
  and salvage reaches the timeline only through Add to timeline
  (`screen_record.recover`). Normal `source.mp4` stitching
  materializes measured encoder-restart gaps to preserve the event/mic/system-audio
  wall clock. `screen_record.recovery_status` is a read-only, project-scoped,
  paginated receipt view: it returns safe capture ids and loss state but never paths
  or recovery side effects. Settings → Health & recovery reads this inventory rather
  than inferring recovery from the recorder doctor; it treats a complete lexical read
  as evidence reported in that check, and exposes only the existing Open Record route.
  Its cursor must be an emitted capture id, and both its capture and nested receipt
  states use lowercase snake case.
- When a sealed capture receipt contains a measured system-audio first-packet
  offset, Health & recovery reports that historical packet-timing evidence for
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
- Windows screen recording uses the standard system capture border. Cut does not
  request the separate borderless-capture permission when starting a recording.
- General exports can use a default export folder or per-action Save As; default
  filename collisions are resolved with a numbered sibling file, while
  confirmed Save As targets can replace existing export media/sidecar files.
  Recording always publishes the unchanged raw MP4 to the default export folder
  in both Raw and Polished modes. Optional destination selection is available
  only after Stop through Export or Save a copy.
- A completed raw recording can be copied byte-for-byte to another fenced MP4
  destination through a cancellable job; the saved source stays in its export
  folder and the copy does not change the timeline.
- Range exports render through a hidden sibling temp file and publish only after
  the MP4 finishes successfully, so a failed/aborted run does not leave a broken
  final export path.
- Agent/API project mutations accept durable request IDs and optimistic project
  revisions. Identical lost-response retries return the original operation IDs;
  changed payloads or stale revisions fail without duplicating edits.
- One export folder for all projects: the folder chosen in the Export menu or
  Settings is remembered across projects and restarts, and the status bar shows
  it; click the export-folder chip to open Settings at the folder setting. If the
  folder disappears or can no longer be written, the chip says "unavailable"
  within about 10 seconds, and an export that would use it is refused before it
  starts, naming the folder. On Linux and macOS a folder that everyone on the
  computer can write to is refused ("export folder can be written by anyone on
  this computer: <path>") unless its sticky bit is set; a folder only your group
  can write to is fine.
- Preview → Compare pauses playback and shows a locked side-by-side Before and
  Current pair at one playhead position. Both are exact composed frames: Current
  is the live durable revision and Before is the state before its latest
  timeline-mutating edit (trailing metadata remains part of Current).
  Comparison is read-only, never invokes Undo, and refuses
  rather than displaying a mismatched sequence, frame format, revision, or time.

## Environment And Setup

- If you run Cut inside a container, enable container nesting before installing captions. Without it, setup can fail with Permission denied.
- `system.doctor` reports compact cards for ffmpeg, perception, dubbing,
  diarization, judge CLIs, and disk health. A detected judge CLI is not a
  render-review claim: its card shows the separate admission result and reason;
  Agent Chat has its own readiness check.
- Installable tools and model runtimes are shown as user-outcome cards with a
  status, primary action, and advanced details for paths or diagnostics.
- First-run setup leads with a plain three-step path: video tools first, add
  media next, and CLI agents only when Generate/chat workflows are needed.
- When FFmpeg is confirmed missing, the first start opens the "Welcome to
  ShellX Cut" setup window, and the Preview monitor shows a direct setup
  notice that opens the Video processing card. On Windows/Linux x86_64 and on
  Apple silicon Macs that card offers Download, with the publisher, version,
  size, licence (GNU GPL version 3) and source link shown beside it; choosing
  Download is the consent. Windows and Linux x86_64 get FFmpeg 9.0.2 from
  FFmpeg-Builds (a GPL 9.0 monthly build), pinned by exact files, sizes and
  SHA-256; only its ffmpeg and ffprobe are unpacked and each is checked again.
  If the publisher has removed that build, nothing is installed and the card
  names the other ways (Windows: the publisher's page and Choose ffmpeg;
  Linux: the distribution's package). Apple silicon gets Martin
  Riedl's notarized FFmpeg 9.0.2, pinned by size and SHA-256 and accepted only
  when its Apple Developer ID signature verifies; it cannot stabilize video,
  which the card says. Homebrew's complete build stays offered as the other way
  (and the only way on Intel Macs): numbered steps with Copy buttons, Check
  again (no restart needed), and Choose ffmpeg for a build that has ffprobe
  beside it.
- On Linux, removing the DEB or RPM package keeps your projects, settings and downloaded tools; after quitting Cut, you can optionally erase settings and downloaded tools at `~/.shellx-cut`, `~/.config/lv.shellx.cut`, `~/.local/share/lv.shellx.cut` and `~/.local/share/shellx-cut` (use your custom `XDG_CONFIG_HOME` and `XDG_DATA_HOME` locations if set). Delete project folders (normally `~/ShellX Cut Projects`) only if you also want to erase those projects.
- On macOS, moving ShellX Cut.app to the Trash keeps your projects, settings and downloaded tools; after quitting Cut, you can optionally erase settings and downloaded tools at `~/.shellx-cut`, `~/Library/Application Support/lv.shellx.cut` and `~/Library/Application Support/ShellX Cut`. Delete project folders (normally `~/ShellX Cut Projects`) only if you also want to erase those projects.
- On Windows, uninstalling always removes what ShellX Cut downloaded or made
  (FFmpeg, the captions Python with its environment and models, the
  background-removal files, its work folder, `engine.addr`, `tools-doctor.json`)
  from its install and data folders, without following links. Ticking "Delete
  the application data" also removes its settings, the recent-projects list,
  the microphone choice, the export folder you chose and the notes about
  interrupted work. Projects, the
  Library, recordings, any uv or Python you installed yourself, and Microsoft's
  Visual C++ runtime always stay. An update keeps the downloaded tools.
- Stabilize, caption burn-in and color-managed export are refused up front,
  with the missing library and how to get it, when no installed FFmpeg has the
  filter they need; the edit or render does not start and fail later.
- Verbs that need ffmpeg or ffprobe refuse up front with `tool_missing` and the
  platform's way to get the tools when they are missing; nothing is imported
  or registered. In the app the same refusal shows a notice with Open setup.
- A file that is not a video, audio or image Cut can read (a renamed text file,
  an empty file, a damaged download) never stays in Assets. An empty file or a
  folder is refused at once; any other file is judged by the import job, which
  fails with `unreadable_media`, names the file and what to do, and undoes the
  import. The status bar shows the same sentence. Relink refuses such a file
  the same way and keeps the old source.
- The first video placed in a new project sets its size and frame rate. Frames
  can be up to 8192 px on a side and 8192x4320 pixels in total (8K UHD and DCI
  8K, including 8192x4096 camera footage); a larger clip keeps the project
  format and the import says why. A project whose stored size is larger than
  that still opens, at the largest size of the same shape within the limit,
  and says so. A first video with an odd size gives the project the even size
  just below it; an older project stored at an odd size opens unchanged and is
  not exported until its size is changed (see below). A phone clip
  with a variable frame rate gives the timeline its nominal rate (60 fps, not
  its 58.59 fps average) and shows a "Variable frame rate" note in Assets.
- A video file that is cut off part-way (a copy or download that stopped) can
  still claim its full length. When its editing proxy is made, Cut compares
  what really decodes with what the file claims; a short file gets a "File cut
  off" badge in Assets with the readable length, and the same sentence in
  `probe.decode_check.warning` for agents. A file damaged in the middle is
  flagged when its proxy has fewer decoded frames than the source claims.
  FFmpeg 6 can conceal corrupt pictures by repeating frames, so a damaged
  clip may have no decode warning. Imports with proxies turned off are
  not checked this way. A render that uses a source with a decode warning
  carries a failing `sources_decodable` check naming the file, and the clip still fills its place
  on the timeline: its last readable frame is held to the end.
  `sources_decodable` cannot certify damage concealed by the decoder.
- Every export is SDR (BT.709 unless `project.color` sets another output
  space), processed in 8 bits. An HDR source (HLG or PQ,
  usually 10-bit phone or camera footage) is tone-mapped to SDR and the file is
  tagged BT.709, so it never claims to be HDR; the editing proxy is converted
  the same way so the preview matches. The render plan, the receipt (Render history & checks
  tab) and the job result say what happened to each source, including a
  10-bit source exported in 8 bits. The conversion happens before grades and
  effects, so a grade works on the SDR picture. An SD source tagged BT.601 is
  converted to BT.709 the same way. The HDR conversion needs an ffmpeg with the
  zscale filter (zimg); without one every export, preview, composed frame and
  scope of a timeline with HDR clips is refused at once, naming the clips and
  saying what to install. Keeping HDR in the export is not available yet.
- A project from an older release with an odd frame size (for example
  641x361) cannot be exported as video at that size (video needs an even width
  and height). Cut says so when the project opens, with one button, "Change
  project to 640x360"; an export started anyway is refused with the same
  sentence and button, and nothing is rounded during the export. Stills
  (Export frame) still work and are 640x360. The frame rate can still be
  changed without touching the size.
- The render receipt's `duration_matches_edl` check measures each stream of
  the output on its own (video duration and frame count, each audio stream),
  so a picture that stops early under full-length sound fails with a plain
  reason instead of passing on the container length.
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
  `ui.select`, `ui.highlight` (dismissible close button/Escape), `ui.preview`
  (COMPOSED on or off), and
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
  reveal-and-highlight action. The stable online page remains available at
  `https://docs.theshellx.com/manual/cut/`; the compiled real-frontend manual is
  staged separately for interactive review before it replaces that route.
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
  `receipt_ready`, `project_changed`, `ui_state`, `doctor_updated`, and `output_folder_changed`.
- `FEATURE_SURFACE_CONTRACT.md` defines the public human, agent, debug, and
  documentation surfaces a supported capability may expose.
