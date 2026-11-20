// The export destination the engine holds, as React state. `useOutputFolderSync`
// (mounted once, by the status bar) keeps the mirror current: it asks the engine
// on mount, on every reconnect, when the window regains focus and when a project
// opens or closes, and applies each `output_folder_changed` event at once, so an
// agent's project.set_output_dir shows without a reload. `useOutputFolderStatus`
// reads it anywhere.

import { useEffect, useSyncExternalStore } from 'react'
import { events } from './events'
import {
  applyOutputFolderStatus,
  getOutputFolderStatus,
  refreshOutputFolder,
  subscribeOutputFolderStatus,
} from './exportDestination'
import type { OutputFolderStatus } from './outputFolderStatus'

/** Backstop re-check of the export folder (the engine's own watch is the primary path). */
export const OUTPUT_FOLDER_RECHECK_MS = 30_000

export function useOutputFolderStatus(): OutputFolderStatus | null {
  return useSyncExternalStore(subscribeOutputFolderStatus, getOutputFolderStatus, getOutputFolderStatus)
}

export function useOutputFolderSync(): void {
  useEffect(() => {
    void refreshOutputFolder()
    const offStatus = events.onStatus((state) => {
      if (state === 'open') void refreshOutputFolder()
    })
    const offEvents = events.onEvent((event) => {
      if (event.type === 'output_folder_changed') applyOutputFolderStatus(event.status)
      else if (event.type === 'project_changed') void refreshOutputFolder()
    })
    const onFocus = () => { void refreshOutputFolder() }
    window.addEventListener('focus', onFocus)
    // The engine also watches the folder and announces a change; this slow timer
    // is the backstop if that event is missed (no busy polling, and none while
    // the window is hidden).
    const timer = window.setInterval(() => {
      if (document.visibilityState !== 'hidden') void refreshOutputFolder()
    }, OUTPUT_FOLDER_RECHECK_MS)
    return () => {
      offStatus()
      offEvents()
      window.removeEventListener('focus', onFocus)
      window.clearInterval(timer)
    }
  }, [])
}
