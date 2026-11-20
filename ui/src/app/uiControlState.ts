import { exportFolderState, type ExportFolderState, type OutputFolderStatus } from '../lib/outputFolderStatus'
import { runningLines } from './quitQuestionModel'
import { readEnvironmentCards, type EnvironmentCardState } from './uiEnvironmentCards'
import type { LayoutState } from '../layout/useLayout'
import type { Project } from '../lib/client'
import type { HighlightSpec } from '../HighlightOverlay'
import type { SettingsCategoryId } from '../panels/Environment/settingsModel'
import type { GenerateWorkspaceTab } from '../panels/GenerateTemplates'
import type { AppDrawer } from './AppDrawerStack'
import type { NativeDialogEntry } from '../lib/nativeDialogs'
import type { UiNotice } from '../lib/uiNotices'
import type { RecordHotkeyCapability } from '../lib/tauri'
import {
  UI_OPEN_SURFACE_IDS,
  UI_SURFACES,
  type UiSurfaceAction,
  type UiSurfaceDefinition,
} from './uiSurfaceRegistry'

export interface UiDomState {
  activeReviewTab: 'ops' | 'receipts' | 'qc' | 'scopes' | 'diff' | null
  rightBodyTab: LayoutState['rightTab'] | null
  rightBodyStatus: 'loaded' | 'loading' | 'blocked' | 'failed' | null
  rightRecoverySelector: string | null
  dialogs: string[]
  /** Exact About version only when the visible text matches its DOM attribute. */
  aboutVersion: string | null
  /** Build-stamped source identity from the generated document element. */
  uiSourceContentManifestSha256: string | null
  /** The preview monitor's visible media problem (e.g. no H.264 decoder). */
  previewMediaProblem: PreviewMediaProblemState | null
  /** The question shown when the app is closed while an export runs. */
  quitQuestion?: QuitQuestionState | null
  /** What the preview monitor shows, as rendered (null: no monitor). */
  previewView: PreviewViewState | null
  /** The Settings tool entries on screen (uiEnvironmentCards.ts). */
  environmentCards?: EnvironmentCardState[]
}

/** The open "An export is still running" question, as visibly rendered. */
export interface QuitQuestionState {
  message: string
  running: string[]
  choices: ['keep_working', 'quit_and_stop']
}

/** What the preview monitor shows, read from its rendered attributes, so an
 * agent knows what a screenshot of the preview means. */
export interface PreviewViewState {
  /** 'composed': the exact frame is drawn while paused (default); 'live': the person turned it off. */
  mode: 'composed' | 'live'
  /** composed_frame | live_preview | source_frame | none */
  showing: string
  /** Elements at the playhead the shown picture leaves out (titles, fades, ...). */
  hidden_elements: string[]
  /** File the live preview plays at the playhead; null: no live clip (gap, image). */
  media: 'proxy' | 'original' | null
  /** The exact frame over the paused live preview: off | waiting | pending | shown | failed. */
  composed_frame: {
    state: string
    at_ms: number | null
    reason: string | null
    latency_ms: number | null
    requests: number
  } | null
}

/** What the preview monitor says it cannot show, as visibly rendered. */
export interface PreviewMediaProblemState {
  kind: string
  asset_ids: string[]
  message: string
}

export interface UiObservableState {
  schema: 'shellx-cut/ui-state/2'
  state_revision: number
  active_workspace: LayoutState['workspaceMode']
  left: { collapsed: boolean; active_tab: LayoutState['leftTab']; find_surface: LayoutState['findSurface']; generate_tab: GenerateWorkspaceTab }
  right: {
    collapsed: boolean
    pinned: boolean
    active_tab: LayoutState['rightTab']
    body_status: UiDomState['rightBodyStatus']
    recovery_selector: string | null
  }
  review: { active_tab: UiDomState['activeReviewTab'] }
  /** Additive v2 field for the About version visibly committed by the UI. */
  about: { displayed_version: UiDomState['aboutVersion'] }
  /** Additive debug evidence for the packaged UI bundle rendered by this client. */
  runtime: { ui_source_content_manifest_sha256: UiDomState['uiSourceContentManifestSha256'] }
  /** Additive v2 field: a preview media problem shown on the monitor (null = none). */
  preview: { media_problem: PreviewMediaProblemState | null } & Partial<PreviewViewState>
  overlays: {
    wizard: boolean
    settings: SettingsCategoryId | null
    settings_cards: EnvironmentCardState[]
    comments: boolean
    drawer: AppDrawer | null
    highlight: string | null
    dialogs: string[]
    /** Operating-system dialogs the app opened and is waiting behind (the webview cannot show them). */
    native_dialogs: NativeDialogEntry[]
    /** Additive: the app was asked to close while an export runs and waits for the person. */
    quit_question: QuitQuestionState | null
  }
  /** Additive: how the global record shortcut is registered; `reason` says why it is not. */
  record_hotkey: { state: RecordHotkeyCapability['state'] | 'unknown'; scope: RecordHotkeyCapability['scope'] | 'unknown'; reason: string | null }
  /** Additive: where exports go as the status bar shows it (folder name only; see lib/outputFolderStatus.ts). */
  export_folder: ExportFolderState | null
  /** Additive: the person-facing notices shown right now (structured, path-free; see lib/uiNotices.ts). */
  notices: UiNotice[]
  open_surface_ids: string[]
  available_surface_ids: string[]
  agent_openable_surface_ids: string[]
  playhead_ms: number
  selected_clip_ids: string[]
  export_range?: [number, number]
  project: { open: boolean; name?: string; active_sequence?: string }
}

export interface UiStateSource {
  revision: number
  layout: LayoutState
  generateTab: GenerateWorkspaceTab
  wizardOpen: boolean
  envOpen: boolean
  envCategory: SettingsCategoryId
  commentsOpen: boolean
  activeDrawer: AppDrawer | null
  highlight: HighlightSpec | null
  playheadMs: number
  selectedClipIds: string[]
  exportRange: [number, number] | null
  project: Project | null
  dom: UiDomState
  nativeDialogs?: readonly NativeDialogEntry[]
  recordHotkey?: RecordHotkeyCapability | null
  notices?: readonly UiNotice[]
  exportFolder?: OutputFolderStatus | null
}

function actionIsOpen(action: UiSurfaceAction, state: UiStateSource): boolean {
  switch (action.kind) {
    case 'focus':
      return state.layout.workspaceMode === 'edit'
    case 'left':
      return state.layout.workspaceMode === 'edit'
        && !state.layout.leftCollapsed
        && state.layout.leftTab === action.tab
    case 'find':
      return state.layout.workspaceMode === 'edit'
        && !state.layout.leftCollapsed
        && state.layout.leftTab === 'find'
        && state.layout.findSurface === action.surface
    case 'generate':
      return state.layout.workspaceMode === 'edit'
        && !state.layout.leftCollapsed
        && state.layout.leftTab === 'generate'
        && state.generateTab === action.tab
    case 'workspace':
      return state.layout.workspaceMode === action.workspace
    case 'right':
      return state.layout.workspaceMode === 'edit'
        && !state.layout.railCollapsed
        && state.layout.rightTab === action.tab
        && state.dom.rightBodyTab === action.tab
        && state.dom.rightBodyStatus === 'loaded'
    case 'review':
      return state.layout.workspaceMode === 'edit'
        && !state.layout.railCollapsed
        && state.layout.railPinned
        && state.dom.activeReviewTab === action.tab
    case 'settings':
      return state.envOpen && state.envCategory === action.category
    case 'overlay':
      return action.overlay === 'wizard' ? state.wizardOpen : state.commentsOpen
    case 'drawer':
      return state.activeDrawer === action.drawer
  }
}

export function surfaceIsOpen(surface: UiSurfaceDefinition, state: UiStateSource): boolean {
  return surface.action ? actionIsOpen(surface.action, state) : state.dom.dialogs.includes(surface.id)
}

export function createUiObservableState(source: UiStateSource): UiObservableState {
  const openSurfaceIds = UI_SURFACES
    .filter((entry) => surfaceIsOpen(entry, source))
    .map((entry) => entry.id)
  const highlightTarget = source.highlight?.selector ?? source.highlight?.clip ?? source.highlight?.panel ?? null
  return {
    schema: 'shellx-cut/ui-state/2',
    state_revision: source.revision,
    active_workspace: source.layout.workspaceMode,
    left: {
      collapsed: source.layout.leftCollapsed,
      active_tab: source.layout.leftTab,
      find_surface: source.layout.findSurface,
      generate_tab: source.generateTab,
    },
    right: {
      collapsed: source.layout.railCollapsed,
      pinned: source.layout.railPinned,
      active_tab: source.layout.rightTab,
      body_status: source.dom.rightBodyTab === source.layout.rightTab ? source.dom.rightBodyStatus : null,
      recovery_selector: source.dom.rightBodyTab === source.layout.rightTab ? source.dom.rightRecoverySelector : null,
    },
    review: { active_tab: source.dom.activeReviewTab },
    about: { displayed_version: source.dom.aboutVersion },
    runtime: { ui_source_content_manifest_sha256: source.dom.uiSourceContentManifestSha256 },
    preview: { media_problem: source.dom.previewMediaProblem, ...(source.dom.previewView ?? {}) },
    overlays: {
      wizard: source.wizardOpen,
      settings: source.envOpen ? source.envCategory : null,
      settings_cards: source.envOpen ? source.dom.environmentCards ?? [] : [],
      comments: source.commentsOpen,
      drawer: source.activeDrawer,
      highlight: highlightTarget,
      dialogs: source.dom.dialogs,
      native_dialogs: (source.nativeDialogs ?? []).map((entry) => ({ ...entry })),
      quit_question: source.dom.quitQuestion ?? null,
    },
    record_hotkey: {
      state: source.recordHotkey?.state ?? 'unknown',
      scope: source.recordHotkey?.scope ?? 'unknown',
      reason: source.recordHotkey?.reason ?? null,
    },
    export_folder: exportFolderState(source.exportFolder),
    notices: (source.notices ?? []).map((entry) => ({ ...entry })),
    open_surface_ids: openSurfaceIds,
    available_surface_ids: UI_SURFACES.map((entry) => entry.id),
    agent_openable_surface_ids: [...UI_OPEN_SURFACE_IDS],
    playhead_ms: source.playheadMs,
    selected_clip_ids: [...source.selectedClipIds],
    ...(source.exportRange ? { export_range: source.exportRange } : {}),
    project: {
      open: source.project !== null,
      ...(source.project ? { name: source.project.name } : {}),
      ...(source.project?.active_sequence ? { active_sequence: source.project.active_sequence } : {}),
    },
  }
}

type UiDomDocument = Pick<Document, 'querySelector'> & {
  documentElement?: Pick<HTMLElement, 'dataset'> | null
}

/** Read the version from the badge's rendered text, rejecting the doctor fallback or an attribute mismatch. */
export function readRenderedAboutVersion(documentRef: UiDomDocument = document): string | null {
  const about = documentRef.querySelector<HTMLElement>('[data-cut-about]')
  const version = documentRef.querySelector<HTMLElement>('[data-cut-about-version]')
  if (!about || !version) return null
  const displayed = version.textContent?.trim().replace(/^v/, '').trim()
  const declared = about.dataset.cutAppVersion?.trim()
  return displayed && displayed !== '—' && (!declared || declared === displayed) ? displayed : null
}

/** Read only the generated document marker, never an API or build-time fallback. */
export function readRenderedUiSourceContentManifest(documentRef: UiDomDocument = document): string | null {
  const value = documentRef.documentElement?.dataset.cutUiSourceContentManifestSha256?.trim()
  return typeof value === 'string' && /^[a-f0-9]{64}$/.test(value) ? value : null
}

/** The monitor's media-problem notice, read from what is rendered. */
export function readPreviewMediaProblem(documentRef: UiDomDocument = document): PreviewMediaProblemState | null {
  const notice = documentRef.querySelector<HTMLElement>('[data-cut-preview-media-problem]')
  const kind = notice?.dataset.cutPreviewMediaProblem?.trim()
  if (!notice || !kind) return null
  const assets = notice.dataset.cutPreviewMediaAssets?.trim()
  const text = (selector: string) => (notice.querySelector?.(selector)?.textContent ?? '').replace(/\s+/g, ' ').trim()
  const title = text('[data-cut-preview-media-problem-title]')
  const detail = text('[data-cut-preview-media-problem-detail]')
  return {
    kind,
    asset_ids: assets ? assets.split(/\s+/) : [],
    // Title and detail are separate blocks on screen; keep them apart here too.
    message: title && detail ? `${title}. ${detail}` : (notice.textContent ?? '').replace(/\s+/g, ' ').trim(),
  }
}

/** The close-time export question, read from what is rendered. */
export function readQuitQuestion(documentRef: UiDomDocument = document): QuitQuestionState | null {
  const dialog = documentRef.querySelector<HTMLElement>('[data-cut-quit-question]')
  if (!dialog) return null
  // One entry per job: a JSON array the dialog writes from the job records.
  const running = runningLines(dialog.dataset.cutQuitQuestionRunning)
  return {
    message: dialog.dataset.cutQuitQuestionMessage?.trim() ?? '',
    running,
    choices: ['keep_working', 'quit_and_stop'],
  }
}

const numberAttr = (value: string | undefined): number | null =>
  value === undefined || value === '' || !Number.isFinite(Number(value)) ? null : Number(value)

/** The preview monitor's mode and what it shows, read from what is rendered. */
export function readPreviewView(documentRef: UiDomDocument = document): PreviewViewState | null {
  const root = documentRef.querySelector<HTMLElement>('[data-cut-panel="preview"][data-cut-preview-mode]')
  const mode = root?.dataset.cutPreviewMode
  if (!root || (mode !== 'composed' && mode !== 'live')) return null
  const hidden = root.dataset.cutPreviewHidden?.trim()
  const frame = documentRef.querySelector<HTMLElement>('[data-cut-composed-frame-state]')
  return {
    mode,
    showing: root.dataset.cutPreviewShowing ?? 'none',
    hidden_elements: hidden ? hidden.split(/\s+/) : [],
    media: root.dataset.cutPreviewMedia === 'proxy' || root.dataset.cutPreviewMedia === 'original' ? root.dataset.cutPreviewMedia : null,
    composed_frame: frame ? {
      state: frame.dataset.cutComposedFrameState ?? 'off',
      at_ms: numberAttr(frame.dataset.cutComposedFrameAt),
      reason: frame.dataset.cutComposedFrameError ?? null,
      latency_ms: numberAttr(frame.dataset.cutComposedFrameLatency),
      requests: numberAttr(frame.dataset.cutComposedFrameRequests) ?? 0,
    } : null,
  }
}

export function readUiDomState(documentRef: UiDomDocument = document): UiDomState {
  const activeReview = documentRef.querySelector<HTMLElement>('[data-cut-review-tab][aria-selected="true"]')
    ?.dataset.cutReviewTab
  const activeReviewTab = (['ops', 'receipts', 'qc', 'scopes', 'diff'] as const).find((tab) => tab === activeReview) ?? null
  const dialogs = UI_SURFACES
    .filter((entry) => !('action' in entry) && documentRef.querySelector(entry.selector))
    .map((entry) => entry.id)
  const rightTab = documentRef.querySelector<HTMLElement>('[data-cut-right-tab][aria-selected="true"]')?.dataset.cutRightTab
  const rightBodyStatus = rightTab
    ? documentRef.querySelector(`[data-cut-panel-render-blocked="${rightTab}"]`) ? 'blocked'
      : documentRef.querySelector(`[data-cut-panel-render-failed="${rightTab}"]`) ? 'failed'
        : documentRef.querySelector(`[data-cut-right-body-loaded="${rightTab}"]`) ? 'loaded'
          : 'loading'
    : null
  return {
    activeReviewTab,
    rightBodyTab: rightTab === 'properties' || rightTab === 'color' || rightTab === 'audio' || rightTab === 'chat'
      ? rightTab : null,
    rightBodyStatus,
    rightRecoverySelector: rightBodyStatus === 'blocked' || rightBodyStatus === 'failed'
      ? `[data-cut-panel-render-retry="${rightTab}"]`
      : null,
    dialogs,
    aboutVersion: readRenderedAboutVersion(documentRef),
    uiSourceContentManifestSha256: readRenderedUiSourceContentManifest(documentRef),
    previewMediaProblem: readPreviewMediaProblem(documentRef),
    quitQuestion: readQuitQuestion(documentRef),
    previewView: readPreviewView(documentRef),
    environmentCards: readEnvironmentCards(documentRef as Partial<Pick<Document, 'querySelectorAll'>>),
  }
}
