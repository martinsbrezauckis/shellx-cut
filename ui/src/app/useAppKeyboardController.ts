import { useEffect, type Dispatch, type SetStateAction } from 'react'
import type { LayoutState } from '../layout/useLayout'
import { shouldIgnoreGlobalShortcut } from '../lib/dom'
import { matchesFixedAction } from '../lib/keymap'

interface AppKeyboardControllerArgs {
  setLayout: Dispatch<SetStateAction<LayoutState>>
  setCommentsOpen: Dispatch<SetStateAction<boolean>>
  onUndo: () => void
  onRedo: () => void
}

/** Owns global app-shell keyboard shortcuts; mutation callbacks stay in App. */
export function useAppKeyboardController({ setLayout, setCommentsOpen, onUndo, onRedo }: AppKeyboardControllerArgs) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (shouldIgnoreGlobalShortcut(e)) return
      if (e.key === '\\' && !e.ctrlKey && !e.metaKey && !e.altKey) {
        e.preventDefault()
        setLayout((l) => ({ ...l, workspaceMode: 'edit', railCollapsed: !l.railCollapsed }))
      } else if (matchesFixedAction(e, 'comments.toggle')) {
        e.preventDefault()
        setLayout((l) => ({ ...l, workspaceMode: 'edit' }))
        setCommentsOpen((v) => !v)
      } else if (!e.ctrlKey && !e.metaKey && !e.altKey && (e.key === 'r' || e.key === 'R')) {
        setLayout((l) => ({ ...l, workspaceMode: 'edit', railCollapsed: false, railPinned: true }))
        setTimeout(() => document.querySelector<HTMLElement>('[data-cut-panel="review"]')?.focus(), 0)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [setCommentsOpen, setLayout])

  useEffect(() => {
    const onUndoRedo = (e: KeyboardEvent) => {
      if (shouldIgnoreGlobalShortcut(e)) return
      if (!(e.ctrlKey || e.metaKey)) return
      const isZ = e.key === 'z' || e.key === 'Z'
      const isY = e.key === 'y' || e.key === 'Y'
      if (isZ && !e.shiftKey) {
        e.preventDefault()
        onUndo()
      } else if ((isZ && e.shiftKey) || (isY && !e.shiftKey)) {
        e.preventDefault()
        onRedo()
      }
    }
    window.addEventListener('keydown', onUndoRedo)
    return () => window.removeEventListener('keydown', onUndoRedo)
  }, [onRedo, onUndo])
}
