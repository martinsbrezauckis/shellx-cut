// One typed inventory for human navigation, ui.open, ui.state, the command
// palette, selectors, and contract tests. A surface entry describes how it
// opens; React-specific setters live in useAppSurfaceEvents.

export type UiDrawerId =
  | 'music'
  | 'title'
  | 'kinetic'
  | 'layer'
  | 'clips'
  | 'autopilot'
  | 'assemble'
  | 'recipes'
  | 'matte'
  | 'shape'
  | 'mask'

export type UiSurfaceAction =
  | { kind: 'focus'; panel: 'timeline' | 'preview' }
  | { kind: 'left'; tab: 'transcript' | 'assets' | 'projects' }
  | { kind: 'find'; surface: 'find-media' | 'find-moment' | 'sequence-index' }
  | { kind: 'generate'; tab: 'templates' | 'prompt' | 'storyboard' | 'media' }
  | { kind: 'workspace'; workspace: 'library' | 'record' }
  | { kind: 'right'; tab: 'properties' | 'color' | 'audio' | 'chat' }
  | { kind: 'review'; tab: 'ops' | 'receipts' | 'qc' | 'scopes' | 'diff' }
  | { kind: 'settings'; category: string }
  | { kind: 'overlay'; overlay: 'wizard' | 'comments' }
  | { kind: 'drawer'; drawer: UiDrawerId }

export interface UiSurfaceDefinition {
  id: string
  label: string
  kind: 'panel' | 'workspace' | 'tab' | 'drawer' | 'overlay' | 'dialog' | 'alias'
  selector: string
  humanRoute: string
  action?: UiSurfaceAction
  /** Why a visible surface is intentionally unavailable through ui.open. */
  agentOnlyReason?: string
}

function surface<const Id extends string>(
  id: Id,
  label: string,
  kind: UiSurfaceDefinition['kind'],
  selector: string,
  humanRoute: string,
): Omit<UiSurfaceDefinition, 'id' | 'action'> & { id: Id }
function surface<const Id extends string, const Action extends UiSurfaceAction>(
  id: Id,
  label: string,
  kind: UiSurfaceDefinition['kind'],
  selector: string,
  humanRoute: string,
  action: Action,
): Omit<UiSurfaceDefinition, 'id' | 'action'> & { id: Id; action: Action }
function surface(
  id: string,
  label: string,
  kind: UiSurfaceDefinition['kind'],
  selector: string,
  humanRoute: string,
  action?: UiSurfaceAction,
): UiSurfaceDefinition {
  return { id, label, kind, selector, humanRoute, ...(action ? { action } : {}) }
}

export const UI_SURFACES = [
  surface('timeline', 'Timeline', 'panel', '[data-cut-panel="timeline"]', 'Editor timeline', { kind: 'focus', panel: 'timeline' }),
  surface('preview', 'Preview', 'panel', '[data-cut-panel="preview"]', 'Editor preview', { kind: 'focus', panel: 'preview' }),
  surface('transcript', 'Transcript', 'tab', '[data-cut-left-tab="transcript"][aria-selected="true"]', 'Left sidebar > Transcript', { kind: 'left', tab: 'transcript' }),
  surface('assets', 'Assets', 'tab', '[data-cut-left-tab="assets"][aria-selected="true"]', 'Left sidebar > Assets', { kind: 'left', tab: 'assets' }),
  surface('projects', 'Projects', 'tab', '[data-cut-left-tab="projects"][aria-selected="true"]', 'Left sidebar > Projects', { kind: 'left', tab: 'projects' }),
  surface('generate', 'Generate templates', 'tab', '[data-cut-generate-tab="templates"][aria-selected="true"]', 'Left sidebar > Generate > Templates', { kind: 'generate', tab: 'templates' }),
  surface('generate-prompt', 'Generate prompt', 'tab', '[data-cut-generate-tab="prompt"][aria-selected="true"]', 'Left sidebar > Generate > Prompt', { kind: 'generate', tab: 'prompt' }),
  surface('generate-storyboard', 'Generate storyboard', 'tab', '[data-cut-generate-tab="storyboard"][aria-selected="true"]', 'Left sidebar > Generate > Storyboard', { kind: 'generate', tab: 'storyboard' }),
  surface('generate-media', 'Generate media', 'tab', '[data-cut-generate-tab="media"][aria-selected="true"]', 'Left sidebar > Generate > Media', { kind: 'generate', tab: 'media' }),
  surface('find-media', 'Find media', 'tab', '[data-cut-find-tab="find-media"][aria-selected="true"]', 'Left sidebar > Find > Media', { kind: 'find', surface: 'find-media' }),
  surface('find-moment', 'Find moment', 'tab', '[data-cut-find-tab="find-moment"][aria-selected="true"]', 'Left sidebar > Find > Moment', { kind: 'find', surface: 'find-moment' }),
  surface('sequence-index', 'Sequence index', 'tab', '[data-cut-find-tab="sequence-index"][aria-selected="true"]', 'Left sidebar > Find > Sequence', { kind: 'find', surface: 'sequence-index' }),
  surface('library', 'Library', 'workspace', '[data-cut-panel="library"]', 'Top bar > Library', { kind: 'workspace', workspace: 'library' }),
  surface('record', 'Record', 'workspace', '[data-cut-panel="record"]', 'Top bar > Record', { kind: 'workspace', workspace: 'record' }),
  surface('properties', 'Properties', 'tab', '[data-cut-right-tab="properties"][aria-selected="true"]', 'Right tools > Properties', { kind: 'right', tab: 'properties' }),
  surface('color', 'Color', 'tab', '[data-cut-right-tab="color"][aria-selected="true"]', 'Right tools > Color', { kind: 'right', tab: 'color' }),
  surface('audio', 'Audio', 'tab', '[data-cut-right-tab="audio"][aria-selected="true"]', 'Right tools > Audio', { kind: 'right', tab: 'audio' }),
  surface('chat', 'Agent chat', 'tab', '[data-cut-right-tab="chat"][aria-selected="true"]', 'Right tools > Chat', { kind: 'right', tab: 'chat' }),
  surface('review', 'Review operations', 'tab', '[data-cut-review-tab="ops"][aria-selected="true"]', 'Review > Ops', { kind: 'review', tab: 'ops' }),
  surface('review-ops', 'Review operations', 'tab', '[data-cut-review-tab="ops"][aria-selected="true"]', 'Review > Ops', { kind: 'review', tab: 'ops' }),
  surface('receipts', 'Render receipts', 'tab', '[data-cut-review-tab="receipts"][aria-selected="true"]', 'Review > Receipts', { kind: 'review', tab: 'receipts' }),
  surface('qc', 'Quality checks', 'tab', '[data-cut-review-tab="qc"][aria-selected="true"]', 'Review > QC', { kind: 'review', tab: 'qc' }),
  surface('scopes', 'Video scopes', 'tab', '[data-cut-review-tab="scopes"][aria-selected="true"]', 'Review > Scopes', { kind: 'review', tab: 'scopes' }),
  surface('diff', 'Edit comparison', 'tab', '[data-cut-review-tab="diff"][aria-selected="true"]', 'Review > Diff', { kind: 'review', tab: 'diff' }),
  surface('comments', 'Comments', 'panel', '[data-cut-panel="comments"]', 'Top bar > Comments', { kind: 'overlay', overlay: 'comments' }),
  surface('wizard', 'Start setup', 'overlay', '[data-cut-wizard-open="true"]', 'First-run setup', { kind: 'overlay', overlay: 'wizard' }),
  surface('environment', 'Settings overview', 'overlay', '[data-cut-settings-body="overview"]', 'Settings > Overview', { kind: 'settings', category: 'overview' }),
  surface('settings-general', 'General settings', 'overlay', '[data-cut-settings-body="general"]', 'Settings > General', { kind: 'settings', category: 'general' }),
  surface('settings-editing', 'Editing settings', 'overlay', '[data-cut-settings-body="editing"]', 'Settings > Editing', { kind: 'settings', category: 'editing' }),
  surface('settings-video-performance', 'Video and performance settings', 'overlay', '[data-cut-settings-body="video-performance"]', 'Settings > Video & performance', { kind: 'settings', category: 'video-performance' }),
  surface('settings-ai-transcription', 'AI and transcription settings', 'overlay', '[data-cut-settings-body="ai-transcription"]', 'Settings > AI & transcription', { kind: 'settings', category: 'ai-transcription' }),
  surface('settings-recording', 'Recording settings', 'overlay', '[data-cut-settings-body="recording"]', 'Settings > Recording', { kind: 'settings', category: 'recording' }),
  surface('settings-services-integrations', 'Services settings', 'overlay', '[data-cut-settings-body="services-integrations"]', 'Settings > Services & integrations', { kind: 'settings', category: 'services-integrations' }),
  surface('settings-agent-control', 'Agent control settings', 'overlay', '[data-cut-settings-body="agent-control"]', 'Settings > Agent control', { kind: 'settings', category: 'agent-control' }),
  surface('settings-storage-privacy', 'Storage and privacy settings', 'overlay', '[data-cut-settings-body="storage-privacy"]', 'Settings > Storage & privacy', { kind: 'settings', category: 'storage-privacy' }),
  surface('settings-about', 'About', 'overlay', '[data-cut-settings-body="about"]', 'Settings > About', { kind: 'settings', category: 'about' }),
  surface('music', 'Music bed', 'drawer', '[data-cut-musicbed-open="true"]', 'Command palette > Music bed', { kind: 'drawer', drawer: 'music' }),
  surface('title', 'Add title', 'drawer', '[data-cut-title-open="true"]', 'Top bar > Title', { kind: 'drawer', drawer: 'title' }),
  surface('kinetic', 'Kinetic captions', 'drawer', '[data-cut-kinetic-open="true"]', 'Transcript > Caption styles', { kind: 'drawer', drawer: 'kinetic' }),
  surface('layer', 'Transform and layer', 'drawer', '[data-cut-layer-open="true"]', 'Command palette > Transform & layer', { kind: 'drawer', drawer: 'layer' }),
  surface('clips', 'Clip candidates', 'drawer', '[data-cut-clips-open="true"]', 'Top bar > Repurpose', { kind: 'drawer', drawer: 'clips' }),
  surface('autopilot', 'Autopilot', 'drawer', '[data-cut-autopilot-open="true"]', 'Top bar > Autopilot', { kind: 'drawer', drawer: 'autopilot' }),
  surface('assemble', 'Assemble', 'drawer', '[data-cut-assemble-open="true"]', 'Top bar > Assemble', { kind: 'drawer', drawer: 'assemble' }),
  surface('recipes', 'Recipes', 'drawer', '[data-cut-recipes-open="true"]', 'Top bar > Recipes', { kind: 'drawer', drawer: 'recipes' }),
  surface('matte', 'Remove background', 'drawer', '[data-cut-matte-open="true"]', 'Command palette > Remove background', { kind: 'drawer', drawer: 'matte' }),
  surface('shape', 'Add shape', 'drawer', '[data-cut-shape-open="true"]', 'Top bar > Shape', { kind: 'drawer', drawer: 'shape' }),
  surface('mask', 'Mask and privacy', 'drawer', '[data-cut-mask-open="true"]', 'Top bar > Mask', { kind: 'drawer', drawer: 'mask' }),
  surface('stock', 'Find media', 'alias', '[data-cut-find-tab="find-media"][aria-selected="true"]', 'Compatibility alias for Find media', { kind: 'find', surface: 'find-media' }),
  surface('search', 'Find moment', 'alias', '[data-cut-find-tab="find-moment"][aria-selected="true"]', 'Compatibility alias for Find moment', { kind: 'find', surface: 'find-moment' }),
  {
    ...surface('command-palette', 'Command palette', 'dialog', '[data-cut-command-palette]', 'Ctrl/Cmd+K'),
    agentOnlyReason: 'Human shortcut surface; ui.open is the direct agent equivalent.',
  },
  {
    ...surface('shortcut-reference', 'Shortcut reference', 'dialog', '[data-cut-keymap]', '? shortcut'),
    agentOnlyReason: 'Human reference overlay; agent instructions live in the installed skill.',
  },
  {
    ...surface('render-queue', 'Render queue', 'dialog', '[data-cut-render-queue]', 'Top bar > Render queue'),
    agentOnlyReason: 'Human job-management modal; agents use jobs.list/status/cancel.',
  },
  {
    ...surface('storyboard-dialog', 'Storyboard', 'dialog', '[data-cut-storyboard-open="true"]', 'Top bar > Storyboard'),
    agentOnlyReason: 'Human presentation modal; agents call render.storyboard directly.',
  },
  {
    ...surface('smart-reframe-dialog', 'Smart reframe director', 'dialog', '[data-cut-director]', 'Export > Smart reframe'),
    agentOnlyReason: 'Human confirmation workflow; agents use the reframe verbs and receipts.',
  },
  {
    ...surface('otio-import-dialog', 'OTIO import preview', 'dialog', '[data-cut-otio-import]', 'Project > Import OTIO'),
    agentOnlyReason: 'Desktop file-picker confirmation; agents call import.otio with an explicit path.',
  },
  {
    ...surface('paste-attributes-dialog', 'Paste attributes', 'dialog', '[data-cut-paste-attributes]', 'Timeline clip context menu'),
    agentOnlyReason: 'Human convenience dialog; agents call the edit verbs directly.',
  },
  {
    ...surface('source-monitor-dialog', 'Source monitor', 'dialog', '[data-cut-source-monitor]', 'Assets > Open source'),
    agentOnlyReason: 'Human preview dialog; agents inspect frames through render.frame.',
  },
  {
    ...surface('generated-compare-dialog', 'Generated take comparison', 'dialog', '[data-cut-generated-compare-dialog]', 'Generate > Compare takes'),
    agentOnlyReason: 'Human visual comparison; generated assets remain verb-addressable.',
  },
] as const satisfies readonly UiSurfaceDefinition[]

export type UiSurfaceId = (typeof UI_SURFACES)[number]['id']
export type UiOpenSurfaceId = Extract<(typeof UI_SURFACES)[number], { action: UiSurfaceAction }>['id']

export const UI_OPEN_SURFACE_IDS = UI_SURFACES
  .filter((entry): entry is Extract<(typeof UI_SURFACES)[number], { action: UiSurfaceAction }> => 'action' in entry)
  .map((entry) => entry.id) as UiOpenSurfaceId[]

const BY_ID = new Map<string, UiSurfaceDefinition>(UI_SURFACES.map((entry) => [entry.id, entry]))

export function uiSurface(id: string): UiSurfaceDefinition | undefined {
  return BY_ID.get(id)
}

/** Human callers dispatch the same stable id ui.open consumes. */
export function openUiSurface(id: UiSurfaceId): void {
  document.dispatchEvent(new CustomEvent('cut:open-ui-surface', { detail: { id } }))
}
