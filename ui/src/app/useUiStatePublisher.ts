import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { LayoutState } from '../layout/useLayout'
import type { Project } from '../lib/client'
import { events } from '../lib/events'
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

const EMPTY_DOM: UiDomState = { activeReviewTab: null, dialogs: [] }

/** Publish a path-safe, revisioned state snapshot after each React commit.
 * A small DOM observer adds self-owned dialogs and the Review tab without
 * duplicating their local state in App. */
export function useUiStatePublisher(args: UiStatePublisherArgs) {
  const revision = useRef(0)
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
    observer.observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ['aria-selected'],
    })
    return () => observer.disconnect()
  }, [])

  useLayoutEffect(() => {
    revision.current += 1
    const next = createUiObservableState({
      revision: revision.current,
      ...args,
      dom,
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
  ])

  return stateRef
}
