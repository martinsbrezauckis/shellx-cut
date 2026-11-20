import { useOutputFolderStatus } from '../lib/useOutputFolder'
import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react'
import type { LayoutState } from '../layout/useLayout'
import type { Project } from '../lib/client'
import { events } from '../lib/events'
import { getOpenNativeDialogs, subscribeNativeDialogs } from '../lib/nativeDialogs'
import { getUiNotices, subscribeUiNotices } from '../lib/uiNotices'
import { useRecordHotkeyCapability } from './useRecordHotkeyCapability'
import type { HighlightSpec } from '../HighlightOverlay'
import type { SettingsCategoryId } from '../panels/Environment/settingsModel'
import type { GenerateWorkspaceTab } from '../panels/GenerateTemplates'
import type { AppDrawer } from './AppDrawerStack'
import {
  createUiObservableState,
  readUiDomState,
  type UiDomState,
  type UiObservableState,
} from './uiControlState'

interface UiStatePublisherArgs {
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
}

const EMPTY_DOM: UiDomState = {
  activeReviewTab: null,
  rightBodyTab: null,
  rightBodyStatus: null,
  rightRecoverySelector: null,
  dialogs: [],
  aboutVersion: null,
  uiSourceContentManifestSha256: null,
  previewMediaProblem: null,
  quitQuestion: null,
  previewView: null,
}

/** Publish a path-safe, revisioned state snapshot after each React commit.
 * A small DOM observer adds self-owned dialogs and the Review tab without
 * duplicating their local state in App. */
export const UI_DOM_MUTATION_OBSERVER_OPTIONS: MutationObserverInit = {
  subtree: true,
  childList: true,
  characterData: true,
  attributes: true,
  attributeFilter: ['aria-selected', 'data-cut-app-version', 'data-cut-ui-source-content-manifest-sha256', 'data-cut-preview-media-assets', 'data-cut-preview-mode', 'data-cut-preview-showing', 'data-cut-preview-hidden', 'data-cut-preview-media', 'data-cut-composed-frame-state', 'data-cut-composed-frame-at', 'data-cut-composed-frame-error', 'data-cut-composed-frame-latency', 'data-cut-quit-question-running'],
}

export function useUiStatePublisher(args: UiStatePublisherArgs) {
  const revision = useRef(0)
  const nativeDialogs = useSyncExternalStore(subscribeNativeDialogs, getOpenNativeDialogs)
  const notices = useSyncExternalStore(subscribeUiNotices, getUiNotices)
  const recordHotkey = useRecordHotkeyCapability()
  const exportFolder = useOutputFolderStatus()
  const [dom, setDom] = useState<UiDomState>(EMPTY_DOM)
  const stateRef = useRef<UiObservableState>(createUiObservableState({
    revision: 0,
    ...args,
    dom: EMPTY_DOM,
  }))

  useEffect(() => {
    let last = ''
    let queued = false
    const refresh = () => {
      queued = false
      const next = readUiDomState()
      const signature = JSON.stringify(next)
      if (signature !== last) {
        last = signature
        setDom(next)
      }
    }
    const schedule = () => {
      if (queued) return
      queued = true
      window.requestAnimationFrame(refresh)
    }
    refresh()
    const observer = new MutationObserver(schedule)
    observer.observe(document.body, UI_DOM_MUTATION_OBSERVER_OPTIONS)
    return () => observer.disconnect()
  }, [])

  useLayoutEffect(() => {
    revision.current += 1
    const next = createUiObservableState({
      revision: revision.current,
      ...args,
      dom,
      nativeDialogs,
      recordHotkey,
      notices,
      exportFolder,
    })
    stateRef.current = next
    events.pushUiState(next)
  }, [
    args.activeDrawer,
    args.commentsOpen,
    args.envCategory,
    args.envOpen,
    args.exportRange,
    args.generateTab,
    args.highlight,
    args.layout,
    args.playheadMs,
    args.project,
    args.selectedClipIds,
    args.wizardOpen,
    dom,
    nativeDialogs,
    recordHotkey,
    notices,
    exportFolder,
  ])

  return stateRef
}
