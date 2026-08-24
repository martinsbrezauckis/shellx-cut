// app/useSourceNavigationController.ts — app-shell owner of source reveals.

import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from 'react'
import type { LayoutState } from '../layout/useLayout'
import {
  sourceNavigationEvent,
  sourceNavigationRequest,
  type SourceNavigationDestination,
} from './sourceNavigation'

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
  setLayout: Dispatch<SetStateAction<LayoutState>>,
): SourceNavigationState | null {
  const [navigation, setNavigation] = useState<SourceNavigationState | null>(null)
  const sequence = useRef(0)

  useEffect(() => {
    const onReveal = (event: Event) => {
      const request = sourceNavigationRequest((event as CustomEvent<unknown>).detail)
      if (!request) return
      sequence.current += 1
      setNavigation({ ...request, nonce: sequence.current })
      setLayout((current) => request.destination === 'project'
        ? { ...current, workspaceMode: 'edit', leftTab: 'assets', leftCollapsed: false }
        : { ...current, workspaceMode: 'library' })
    }
    document.addEventListener(sourceNavigationEvent, onReveal)
    return () => document.removeEventListener(sourceNavigationEvent, onReveal)
  }, [setLayout])

  return navigation
}
