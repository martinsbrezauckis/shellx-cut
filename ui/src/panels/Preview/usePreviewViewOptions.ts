// usePreviewViewOptions.ts — Preview presentation state: full-screen + frame guides.
//
// Role: owns the two UI-only view toggles for the Preview monitor:
//   • full-screen — the Fullscreen API on the whole Preview panel (monitor +
//     transport stay together; Esc exits natively, state re-syncs on the
//     `fullscreenchange` event so the button never lies about the real state).
//   • guides — framing overlays (rule-of-thirds / safe areas), persisted to
//     localStorage (the proxyPref pattern) so the choice survives reloads.
//
// UI-only by design (no verb): both are pure presentation state of THIS
// client's monitor — they never touch the project, the op-log, or the render,
// so there is nothing for an agent to co-edit or replay. Agents inspect them
// via the `data-cut-*` selectors on the transport buttons / stage overlay.
// Callers: panels/Preview/index.tsx. Deps: react only.

import { useCallback, useEffect, useState, type RefObject } from 'react'
import {
  isAppWindowFullscreen,
  isTauri,
  setAppWindowFullscreen,
} from '../../lib/tauri'

/** Guide overlay mode, cycled by the transport button / the G key. */
export type GuideMode = 'off' | 'thirds' | 'safe' | 'both'

const GUIDE_ORDER: GuideMode[] = ['off', 'thirds', 'safe', 'both']
const GUIDE_KEY = 'cut.previewGuides'

function loadGuideMode(): GuideMode {
  try {
    const v = localStorage.getItem(GUIDE_KEY)
    return GUIDE_ORDER.includes(v as GuideMode) ? (v as GuideMode) : 'off'
  } catch {
    return 'off'
  }
}

/** Human label for the current guide mode (button copy + tooltips). */
export function guideLabel(mode: GuideMode): string {
  switch (mode) {
    case 'off': return 'Guides off'
    case 'thirds': return 'Rule of thirds'
    case 'safe': return 'Safe areas'
    case 'both': return 'Thirds + safe areas'
  }
}

export function usePreviewViewOptions(rootRef: RefObject<HTMLElement | null>) {
  // --- full-screen -----------------------------------------------------------
  const [fullscreenMode, setFullscreenMode] = useState<'off' | 'dom' | 'native' | 'overlay'>('off')
  const isFullscreen = fullscreenMode !== 'off'

  useEffect(() => {
    // The event is the single source of truth: it fires for our toggle, for
    // Esc, and for any programmatic exit, so the state can never go stale.
    const onChange = () => {
      setFullscreenMode((current) => (
        document.fullscreenElement != null
          ? 'dom'
          : current === 'dom'
            ? 'off'
            : current
      ))
    }
    document.addEventListener('fullscreenchange', onChange)
    return () => document.removeEventListener('fullscreenchange', onChange)
  }, [])

  useEffect(() => {
    if (!isTauri()) return
    let active = true
    const syncNativeState = async () => {
      const nativeFullscreen = await isAppWindowFullscreen()
      if (!active || nativeFullscreen == null) return
      setFullscreenMode((current) => (
        nativeFullscreen
          ? 'native'
          : current === 'native'
            ? 'off'
            : current
      ))
    }
    window.addEventListener('focus', syncNativeState)
    window.addEventListener('resize', syncNativeState)
    void syncNativeState()
    return () => {
      active = false
      window.removeEventListener('focus', syncNativeState)
      window.removeEventListener('resize', syncNativeState)
    }
  }, [])

  const toggleFullscreen = useCallback(async () => {
    const el = rootRef.current
    if (!el) return

    if (fullscreenMode === 'native') {
      if (await setAppWindowFullscreen(false)) setFullscreenMode('off')
      return
    }
    if (fullscreenMode === 'overlay') {
      setFullscreenMode('off')
      return
    }
    if (document.fullscreenElement) {
      await document.exitFullscreen().catch(() => {})
      return
    }

    // Installed WebViews use the native window command. This works uniformly
    // across WKWebView, WebView2, and WebKitGTK and avoids depending on a DOM
    // API that WKWebView may omit or reject.
    if (isTauri() && await setAppWindowFullscreen(true)) {
      setFullscreenMode('native')
      return
    }

    if (typeof el.requestFullscreen === 'function') {
      try {
        await el.requestFullscreen()
        setFullscreenMode('dom')
        return
      } catch {
        // Browser permission policy can reject fullscreen even after a gesture.
      }
    }

    // Last-resort in-window immersive view. It still gives the editor the full
    // viewport and remains explicitly reversible by the same button / F key.
    setFullscreenMode('overlay')
  }, [fullscreenMode, rootRef])

  // --- guides ----------------------------------------------------------------
  const [guides, setGuides] = useState<GuideMode>(loadGuideMode)

  const cycleGuides = useCallback(() => {
    setGuides((g) => {
      const next = GUIDE_ORDER[(GUIDE_ORDER.indexOf(g) + 1) % GUIDE_ORDER.length]
      try {
        localStorage.setItem(GUIDE_KEY, next)
      } catch {
        /* storage unavailable — session-only, still works */
      }
      return next
    })
  }, [])

  return { isFullscreen, toggleFullscreen, guides, cycleGuides }
}
