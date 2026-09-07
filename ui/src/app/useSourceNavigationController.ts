// app/useSourceNavigationController.ts — app-shell owner of source reveals.

import { useEffect, useRef, useState } from 'react'
import {
  sourceNavigationEvent,
  sourceNavigationRequest,
  type SourceNavigationDestination,
} from './sourceNavigation'
import type { RequestLayout } from './useRecordingWorkspaceNavigation'

export interface SourceNavigationState {
  assetId: string
  destination: SourceNavigationDestination
  nonce: number
}

/**
 * Moves to an existing surface and carries an exact registered asset id. The
 * nonce makes a second reveal of the same asset observable to that surface.
 */
export function useSourceNavigationController(
  setLayout: RequestLayout,
): SourceNavigationState | null {
  const [navigation, setNavigation] = useState<SourceNavigationState | null>(null)
  const sequence = useRef(0)

  useEffect(() => {
    const onReveal = (event: Event) => {
      const request = sourceNavigationRequest((event as CustomEvent<unknown>).detail)
      if (!request) return
      const moved = setLayout((current) => request.destination === 'project'
        ? { ...current, workspaceMode: 'edit', leftTab: 'assets', leftCollapsed: false }
        : { ...current, workspaceMode: 'library' })
      if (!moved) return
      sequence.current += 1
      setNavigation({ ...request, nonce: sequence.current })
    }
    document.addEventListener(sourceNavigationEvent, onReveal)
    return () => document.removeEventListener(sourceNavigationEvent, onReveal)
  }, [setLayout])

  return navigation
}
