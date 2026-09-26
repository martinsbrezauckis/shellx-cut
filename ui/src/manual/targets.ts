import { uiSurface, type UiSurfaceId } from '../app/uiSurfaceRegistry'

export interface ManualFeatureTarget {
  featureId: string
  surface?: UiSurfaceId
  openSelector?: string
  selector?: string
  precision: 'exact' | 'surface'
  unavailable?: string
}

/**
 * An exact entry names the real control; its optional surface is the same
 * registered opener used by ui.open. Controls that require a selection, a
 * loaded result, or a conditional warning may still be absent after opening
 * their surface, in which case the bridge reports surface-only rather than
 * pretending that it found the control.
 */
function exact(
  featureId: string,
  selector: string,
  surface?: UiSurfaceId,
  openSelector?: string,
): ManualFeatureTarget {
  return {
    featureId,
    selector,
    precision: 'exact',
    ...(surface ? { surface } : {}),
    ...(openSelector ? { openSelector } : {}),
  }
}

/** A documented workflow or contextual menu has a truthful nearest surface, not one exact control. */
function surface(featureId: string, id: UiSurfaceId): ManualFeatureTarget {
  return {
    featureId,
    surface: id,
    selector: uiSurface(id)?.selector,
    precision: 'surface',
  }
}

/** Keep non-UI capabilities explicit instead of inventing a visible control. */
function unavailable(featureId: string, reason: string): ManualFeatureTarget {
  return { featureId, precision: 'surface', unavailable: reason }
}

const TARGETS: ManualFeatureTarget[] = [
  // Setup
  exact('cut.setup.ffmpeg', '[data-cut-env-card="ffmpeg"]', 'settings-video-performance'),
  exact('cut.setup.doctor', '[data-cut-environment-refresh]', 'environment'),
  exact('cut.setup.ffmpeg_path', '[data-cut-env-ffmpeg-change]', 'settings-video-performance'),
  exact('cut.setup.captions', '[data-cut-env-setup-perception="perception"]', 'settings-ai-transcription'),
  unavailable('cut.setup.cli_install', 'CLI installation is unavailable in Cut: it is an external provider setup, not an editor control.'),
  exact('cut.setup.cli_connect', '[data-cut-agent-control-copy-mcp]', 'settings-agent-control'),

  // Workspaces and top bar
  exact('cut.record.open', '[data-cut-panel="record"]', 'record'),
  exact('cut.top.projects', '[data-cut-left-tab="projects"][aria-selected="true"]', 'projects'),
  exact('cut.top.library', '[data-cut-panel="library"]', 'library'),
  exact('cut.top.settings', '[data-cut-settings-body="overview"]', 'environment'),
  exact('cut.top.manual', '[data-cut-manual-link]'),

  // Header tools and their real drawers
  exact('cut.header.title', '[data-cut-title-open="true"]', 'title'),
  exact('cut.header.shape', '[data-cut-shape-open="true"]', 'shape'),
  exact('cut.header.region_mask', '[data-cut-mask-open="true"]', 'mask'),
  exact('cut.header.music', '[data-cut-musicbed-open="true"]', 'music'),
  exact('cut.header.mixer', '[data-cut-right-tab="audio"][aria-selected="true"]', 'audio'),
  exact('cut.header.repurpose', '[data-cut-clips-open="true"]', 'clips'),
  exact('cut.header.autopilot', '[data-cut-autopilot-open="true"]', 'autopilot'),
  exact('cut.header.recipes', '[data-cut-recipes-open="true"]', 'recipes'),
  exact('cut.header.assemble', '[data-cut-assemble-open="true"]', 'assemble'),
  exact('cut.header.storyboard', '[data-cut-storyboard-btn]'),
  exact('cut.header.comments', '[data-cut-panel="comments"]', 'comments'),
  exact('cut.header.gpu', '[data-cut-gpu-toggle]'),

  exact('cut.header.title.add', '[data-cut-title-apply]', 'title'),
  exact('cut.header.title.templates', '[data-cut-title-template]', 'title'),
  exact('cut.header.title.lower_third', '[data-cut-title-preset]', 'title'),
  exact('cut.header.shape.rectangle', '[data-cut-shape-kind-opt="rect"]', 'shape'),
  exact('cut.header.shape.circle', '[data-cut-shape-kind-opt="ellipse"]', 'shape'),
  exact('cut.header.shape.arrow', '[data-cut-shape-kind-opt="arrow"]', 'shape'),
  exact('cut.header.region_mask.face', '[data-cut-mask-preset="face"]', 'mask'),
  exact('cut.header.region_mask.rectangle', '[data-cut-mask-preset="rectangle"]', 'mask'),
  exact('cut.header.region_mask.plate', '[data-cut-mask-preset="plate"]', 'mask'),
  exact('cut.header.region_mask.duration', '[data-cut-mask-duration-mode="timed"]', 'mask'),
  exact('cut.header.region_mask.custom', '[data-cut-mask-preset="custom"]', 'mask'),
  exact('cut.header.region_mask.apply', '[data-cut-mask-apply]', 'mask'),
  exact('cut.header.music.add', '[data-cut-musicbed-apply]', 'music'),
  exact('cut.header.music.duck', '[data-cut-musicbed-duck]', 'music'),
  exact('cut.header.mixer.gain', '[data-cut-mixer-fader]', 'audio'),
  exact('cut.header.mixer.pan', '[data-cut-mixer-pan]', 'audio'),
  exact('cut.header.mixer.mute_solo', '[data-cut-mixer-mute], [data-cut-mixer-solo]', 'audio'),
  exact('cut.header.mixer.eq', '[data-cut-inspector-eq]', 'properties'),
  exact('cut.header.mixer.cleanup', '[data-cut-inspector-cleanup]', 'properties'),
  exact('cut.header.repurpose.highlights', '[data-cut-clips-list]', 'clips'),
  exact('cut.header.repurpose.vertical', '[data-cut-clips-platform="9:16"]', 'clips'),
  exact('cut.header.repurpose.variants', '[data-cut-clips-platforms]', 'clips'),
  exact('cut.header.autopilot.plan', '[data-cut-autopilot-run]', 'autopilot'),
  exact('cut.header.autopilot.apply', '[data-cut-autopilot-apply]', 'autopilot'),
  exact('cut.header.recipes.browse', '[data-cut-recipes-list]', 'recipes'),
  exact('cut.header.recipes.edit_for_clarity', '[data-cut-recipe="edit-for-clarity"]', 'recipes'),
  exact('cut.header.recipes.describe', '[data-cut-recipe]', 'recipes'),
  exact('cut.header.recipes.run', '[data-cut-recipe-run]', 'recipes'),
  exact('cut.header.recipes.phone_cleanup', '[data-cut-recipe="phone-clip-cleanup"]', 'recipes'),
  exact('cut.header.recipes.social_bundle', '[data-cut-recipe="social-short-bundle"]', 'recipes'),
  exact('cut.header.recipes.privacy_mask', '[data-cut-recipe="area-privacy-mask"]', 'recipes'),
  exact('cut.header.recipes.captions', '[data-cut-recipe="add-captions"]', 'recipes'),
  exact('cut.header.recipes.youtube_export', '[data-cut-recipe="youtube-export"]', 'recipes'),
  exact('cut.header.recipes.tiktok_export', '[data-cut-recipe="tiktok-export"]', 'recipes'),
  exact('cut.header.assemble.prompt', '[data-cut-assemble-prompt]', 'assemble'),
  exact('cut.header.assemble.sources', '[data-cut-assemble-asset]', 'assemble'),
  exact('cut.header.assemble.draft', '[data-cut-assemble-run]', 'assemble'),
  // Storyboard planning is the Generate > Storyboard surface; the header
  // Storyboard button above is the separate contact-sheet viewer.
  exact('cut.header.storyboard.generate', '[data-cut-generate-storyboard-plan]', 'generate-storyboard'),
  exact('cut.header.storyboard.review', '[data-cut-generate-storyboard-scenes]', 'generate-storyboard'),
  exact('cut.header.storyboard.preview', '[data-cut-generate-storyboard-preview]', 'generate-storyboard'),
  exact('cut.header.storyboard.insert', '[data-cut-generate-storyboard-insert]', 'generate-storyboard'),
  exact('cut.header.comments.add', '[data-cut-comment-input]', 'comments'),
  exact('cut.header.comments.make_changes', '[data-cut-action="comment-make-changes"]', 'comments'),
  exact('cut.header.comments.resolve', '[data-cut-action="comment-done"]', 'comments'),

  // Render and export. Only the two menu toggles are passive bridge openers.
  exact('cut.top.render', '[data-cut-render-menu]', undefined, '[data-cut-render-opts]'),
  exact('cut.top.export', '[data-cut-export-menu]', undefined, '[data-cut-export-btn]'),
  exact('cut.top.projects.open', '[data-cut-project-open]', 'projects'),
  exact('cut.top.projects.create', '[data-cut-projects-create]', 'projects'),
  exact('cut.top.library.saved', '[data-cut-library-grid]', 'library'),
  exact('cut.top.library.add', '[data-cut-library-toproject]', 'library'),
  exact('cut.top.render.preview', '[data-cut-render-preset]', undefined, '[data-cut-render-opts]'),
  exact('cut.top.render.full', '[data-cut-render-btn]'),
  exact('cut.top.render.range', '[data-cut-action="render-section"]', 'preview'),
  exact('cut.top.export.video', '[data-cut-export-option="video"]', undefined, '[data-cut-export-btn]'),
  exact('cut.top.export.bundle', '[data-cut-export-group="publish"]', undefined, '[data-cut-export-btn]'),
  unavailable('cut.top.export.archive', 'Project archive export is unavailable in this release; copying or zipping the project folder is outside the editor UI.'),
  unavailable('cut.export.preflight', 'Preflight warnings are unavailable until a render or export request finds a risk; the manual does not start output jobs.'),
  unavailable('cut.export.preflight.black_tail', 'This conditional preflight warning is unavailable until an output check finds a black ending; the manual does not start output jobs.'),
  unavailable('cut.export.preflight.dead_frames', 'This conditional preflight warning is unavailable until an output check finds black or frozen footage; the manual does not start output jobs.'),
  unavailable('cut.export.preflight.pacing', 'This conditional preflight warning is unavailable until an output check finds long holds; the manual does not start output jobs.'),
  unavailable('cut.export.preflight.silent_audio', 'This conditional preflight warning is unavailable until an output check finds silent audio; the manual does not start output jobs.'),
  unavailable('cut.export.preflight.tiny_clips', 'This conditional preflight warning is unavailable until an output check finds tiny clips; the manual does not start output jobs.'),
  unavailable('cut.export.preflight.borders', 'This conditional preflight warning is unavailable until an output check finds black borders; the manual does not start output jobs.'),

  // Left sidebar and Generate
  exact('cut.left.transcript', '[data-cut-left-tab="transcript"][aria-selected="true"]', 'transcript'),
  exact('cut.left.assets', '[data-cut-left-tab="assets"][aria-selected="true"]', 'assets'),
  exact('cut.left.asset_filters', '[data-cut-asset-filters]', 'assets'),
  exact('cut.left.asset_needs_action', '[data-cut-asset-attention-filter]', 'assets'),
  exact('cut.left.media_health', '[data-cut-media-health]', 'assets'),
  exact('cut.left.proxies', '[data-cut-proxy-toggle]', 'assets'),
  exact('cut.left.import', '[data-cut-import-cta]', 'assets'),
  exact('cut.left.add_at_playhead', '[data-cut-action="insert-asset"]', 'assets'),
  exact('cut.left.generate', '[data-cut-generate-tab="templates"][aria-selected="true"]', 'generate'),
  exact('cut.left.generated_history', '[data-cut-generate-history]', 'generate-media'),
  exact('cut.left.generated_references', '[data-cut-generate-references]', 'generate-media'),
  exact('cut.left.motion_edit', '[data-cut-motion-edit]', 'properties'),
  exact('cut.left.motion_refresh', '[data-cut-motion-refresh]', 'properties'),
  exact('cut.left.motion_tracking', '[data-cut-motion-tracking]', 'properties'),
  exact('cut.left.find', '[data-cut-left-tab="find"][aria-selected="true"]', 'find-media'),
  exact('cut.left.find.media', '[data-cut-find-tab="find-media"][aria-selected="true"]', 'find-media'),
  exact('cut.left.find.moment', '[data-cut-find-tab="find-moment"][aria-selected="true"]', 'find-moment'),
  exact('cut.left.sequence_index', '[data-cut-find-tab="sequence-index"][aria-selected="true"]', 'sequence-index'),

  // Preview
  exact('cut.preview.monitor', '[data-cut-panel="preview"]', 'preview'),
  exact('cut.preview.ffmpeg_setup', '[data-cut-preview-ffmpeg-setup]', 'preview'),
  exact('cut.preview.transport', '[data-cut-transport]', 'preview'),
  exact('cut.preview.frame', '[data-cut-action="snapshot-frame"]', 'preview'),
  exact('cut.preview.render_selection', '[data-cut-action="render-section"]', 'preview'),
  exact('cut.preview.audio', '[data-cut-audio-toggle]', 'preview'),
  exact('cut.preview.composed', '[data-cut-composed]', 'preview'),
  exact('cut.preview.compare', '[data-cut-preview-compare-trigger]', 'preview'),
  exact('cut.preview.guides', '[data-cut-guides]', 'preview'),
  exact('cut.preview.fullscreen', '[data-cut-action="fullscreen-toggle"]', 'preview'),

  // Timeline
  exact('cut.timeline.timecode', '[data-cut-tc-readout]', 'timeline'),
  exact('cut.timeline.razor', '[data-cut-tool="razor"]', 'timeline'),
  exact('cut.timeline.trim', '[data-cut-tool="trim"]', 'timeline'),
  exact('cut.timeline.snap', '[data-cut-tool="snap"]', 'timeline'),
  exact('cut.timeline.ripple', '[data-cut-tool="ripple-del"]', 'timeline'),
  exact('cut.timeline.lift', '[data-cut-tool="lift-del"]', 'timeline'),
  exact('cut.timeline.speed', '[data-cut-speed-control]', 'timeline'),
  exact('cut.timeline.sync', '[data-cut-action="sync-by-audio"]', 'timeline'),
  exact('cut.timeline.multicam', '[data-cut-action="multicam-switch"]', 'timeline'),
  exact('cut.timeline.beat', '[data-cut-action="cut-to-beat"]', 'timeline'),
  exact('cut.timeline.cleanup_tools', '[data-cut-timeline-automation-trigger]', 'timeline'),
  exact('cut.timeline.trim_dead_air', '[data-cut-tool="trim_edges"]', 'timeline'),
  exact('cut.timeline.split_scenes', '[data-cut-tool="split_scenes"]', 'timeline'),
  exact('cut.timeline.mark_scenes', '[data-cut-tool="mark_scenes"]', 'timeline'),
  exact('cut.timeline.grade', '[data-cut-action="open-grade"]', 'timeline'),
  exact('cut.timeline.ask_agent', '[data-cut-action="timeline-ask-agent"]', 'timeline'),
  exact('cut.timeline.track_controls', '[data-cut-track-header]', 'timeline'),
  exact('cut.timeline.track_visibility', '[data-cut-visibility-track]', 'timeline'),
  exact('cut.timeline.track_lock', '[data-cut-lock-track]', 'timeline'),
  exact('cut.timeline.track_order', '[data-cut-track-order]', 'timeline'),
  exact('cut.timeline.track_mute', '[data-cut-mute-track]', 'timeline'),
  exact('cut.timeline.track_solo', '[data-cut-solo-track]', 'timeline'),
  exact('cut.timeline.track_listen', '[data-cut-listen-track]', 'timeline'),
  exact('cut.timeline.track_gain', '[data-cut-gain-track]', 'timeline'),
  exact('cut.timeline.track_pan', '[data-cut-pan-track]', 'timeline'),
  exact('cut.timeline.voiceover', '[data-cut-voiceover-control]', 'timeline'),
  surface('cut.timeline.base_overlay', 'timeline'),
  exact('cut.timeline.save_assets', '[data-cut-action="save-range"]', 'timeline'),

  // Context menus are selection- and pointer-position-specific. Reveal their
  // owning surface without fabricating an always-open menu.
  surface('cut.context.menu', 'timeline'),
  surface('cut.context.assets', 'assets'),
  surface('cut.context.library', 'library'),
  surface('cut.context.projects', 'projects'),
  surface('cut.context.preview', 'preview'),
  surface('cut.context.timeline_clip', 'timeline'),
  surface('cut.context.timeline_surface', 'timeline'),
  surface('cut.context.timeline_track', 'timeline'),
  surface('cut.context.timeline_marker', 'timeline'),

  // Inspector and review rail
  exact('cut.inspector.properties', '[data-cut-right-tab="properties"][aria-selected="true"]', 'properties'),
  exact('cut.inspector.color', '[data-cut-right-tab="color"][aria-selected="true"]', 'color'),
  exact('cut.inspector.audio', '[data-cut-right-tab="audio"][aria-selected="true"]', 'audio'),
  exact('cut.inspector.chat', '[data-cut-right-tab="chat"][aria-selected="true"]', 'chat'),
  exact('cut.inspector.chat_assets', '[data-cut-chat-attach]', 'chat'),
  exact('cut.inspector.chat_change', '[data-cut-chat-review]', 'chat'),
  exact('cut.inspector.chat_clear_target', '[data-cut-action="chat-clear-timeline-target"]', 'chat'),
  surface('cut.inspector.tools_overlay', 'properties'),
  exact('cut.inspector.pin', '[data-cut-rail-pin]', 'properties'),
  surface('cut.inspector.fades', 'properties'),
  surface('cut.inspector.transform', 'properties'),
  exact('cut.review.ops', '[data-cut-review-tab="ops"][aria-selected="true"]', 'review-ops'),
  exact('cut.review.receipts', '[data-cut-review-tab="receipts"][aria-selected="true"]', 'receipts'),
  exact('cut.review.scopes', '[data-cut-scopes]', 'scopes'),

  // Record workspace. Camera capture stays explicit and keeps its own editable take.
  exact('cut.record.studio', '[data-cut-studio-preview]', 'record'),
  exact('cut.record.scenes', '[data-cut-rec-settings-tab="camera"]', 'record'),
  exact('cut.record.scene_timer', '[data-cut-rec-settings-tab="timer"]', 'record'),
  exact('cut.record.pause_resume', '[data-cut-rec-settings-tab="timing"]', 'record'),
  exact('cut.record.camera_enable', '[data-cut-rec-camera-toggle]', 'record'),
  exact('cut.record.camera_visible', '[data-cut-rec-camera-layout-preview]', 'record'),
  exact('cut.record.camera_position', '[data-cut-rec-camera-layout]', 'record'),
  exact('cut.record.camera_size', '[data-cut-rec-camera-size]', 'record'),
  exact('cut.record.camera_shape', '[data-cut-rec-camera-shape]', 'record'),
  exact('cut.record.background', '[data-cut-studio-background-select]', 'record'),
  exact('cut.record.raw_streams', '[data-cut-rec-recording-details]', 'record'),
  exact('cut.record.hotkeys', '[data-cut-action="record-start"]', 'record'),
  exact('cut.record.raw_mode', '[data-cut-rec-mode="raw"]', 'record'),
  exact('cut.record.autoedit', '[data-cut-rec-mode="auto"]', 'record'),
  unavailable('cut.record.studio_event_api', 'Studio event API is unavailable as an editor control; it is a Debug API integration.'),

  // Multi-step workflows retain only their nearest real entry surface.
  surface('cut.workflow.import', 'assets'),
  surface('cut.workflow.base_overlay', 'timeline'),
  surface('cut.workflow.split_trim', 'timeline'),
  exact('cut.workflow.captions', '[data-cut-inspector-group="caption-bulk"]', 'properties'),
  surface('cut.workflow.edit_for_clarity', 'recipes'),
  surface('cut.workflow.generate', 'generate'),
  surface('cut.workflow.generated_media', 'generate-media'),
  surface('cut.workflow.motion_roundtrip', 'properties'),
  surface('cut.workflow.sequence_index', 'sequence-index'),
  surface('cut.workflow.agent_change', 'chat'),
  surface('cut.workflow.recording', 'record'),
  surface('cut.workflow.review', 'receipts'),
  exact('cut.workflow.export', '[data-cut-render-btn]'),

  // The API reference is intentionally not represented as simulated editor UI.
  unavailable('cut.api.debug', 'Debug API overview is unavailable as one editor control; it documents Cut’s local interface.'),
  unavailable('cut.api.rest', 'REST verbs are unavailable as one editor control; they document Cut’s local interface.'),
  unavailable('cut.api.state', 'Project state API is unavailable as one editor control; it documents Cut’s local interface.'),
  unavailable('cut.api.events', 'WebSocket events are unavailable as one editor control; they document Cut’s local interface.'),
  unavailable('cut.api.mcp', 'MCP server is unavailable as one editor control; it documents Cut’s local interface.'),
  unavailable('cut.api.motion_jobs', 'Motion job observation is unavailable as one editor control; it is API and MCP integration.'),
  unavailable('cut.api.catalog', 'Verb catalog is unavailable as one editor control; it documents Cut’s local interface.'),
]

const BY_ID = new Map(TARGETS.map((target) => [target.featureId, target]))

export function manualFeatureTarget(featureId: string): ManualFeatureTarget {
  return BY_ID.get(featureId) ?? unavailable(
    featureId,
    'This manual entry is unavailable because it has no registered editor target.',
  )
}

export function manualFeatureForElement(element: Element): string | null {
  const declared = element.closest<HTMLElement>('[data-cut-manual-id]')?.dataset.cutManualId
  if (declared) return declared
  for (const target of TARGETS) {
    if (target.selector && element.closest(target.selector)) return target.featureId
  }
  return null
}
