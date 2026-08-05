import type { LayoutState } from '../layout/useLayout'
import type { Project } from '../lib/client'
import type { HighlightSpec } from '../HighlightOverlay'
import type { SettingsCategoryId } from '../panels/Environment/settingsModel'
import type { GenerateWorkspaceTab } from '../panels/GenerateTemplates'
import type { AppDrawer } from './AppDrawerStack'
import {
  UI_OPEN_SURFACE_IDS,
  UI_SURFACES,
  type UiSurfaceAction,
  type UiSurfaceDefinition,
} from './uiSurfaceRegistry'

export interface UiDomState {
  activeReviewTab: 'ops' | 'receipts' | 'qc' | 'scopes' | 'diff' | null
  dialogs: string[]
}

export interface UiObservableState {
  schema: 'shellx-cut/ui-state/2'
  state_revision: number
  active_workspace: LayoutState['workspaceMode']
  left: { collapsed: boolean; active_tab: LayoutState['leftTab']; find_surface: LayoutState['findSurface']; generate_tab: GenerateWorkspaceTab }
  right: { collapsed: boolean; pinned: boolean; active_tab: LayoutState['rightTab'] }
  review: { active_tab: UiDomState['activeReviewTab'] }
  overlays: {
    wizard: boolean
    settings: SettingsCategoryId | null
    comments: boolean
    drawer: AppDrawer | null
    highlight: string | null
    dialogs: string[]
  }
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
    },
    review: { active_tab: source.dom.activeReviewTab },
    overlays: {
      wizard: source.wizardOpen,
      settings: source.envOpen ? source.envCategory : null,
      comments: source.commentsOpen,
      drawer: source.activeDrawer,
      highlight: highlightTarget,
      dialogs: source.dom.dialogs,
    },
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

export function readUiDomState(): UiDomState {
  const activeReview = document.querySelector<HTMLElement>('[data-cut-review-tab][aria-selected="true"]')
    ?.dataset.cutReviewTab
  const activeReviewTab = activeReview === 'ops'
    || activeReview === 'receipts'
    || activeReview === 'qc'
    || activeReview === 'scopes'
    || activeReview === 'diff'
    ? activeReview
    : null
  const dialogs = UI_SURFACES
    .filter((entry) => !('action' in entry) && document.querySelector(entry.selector))
    .map((entry) => entry.id)
  return { activeReviewTab, dialogs }
}
