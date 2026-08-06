// components/PanelErrorBoundary.tsx — honest fallback for a right-rail tab
// whose render throws (including a failed lazy-chunk load surfacing through
// Suspense). Part of the 2026-08-06 Color-panel fix: the JS-CATCHABLE class of
// render failures shows a notice inside the rail instead of white-screening the
// tree, and the failed tab is blocklisted (panelPersistGuard) so the next
// launch never restores it. The WebKit web-process crash class is NOT catchable
// here — that is what the arm/confirm sentinel in AppRightRail covers.
//
// Class component on purpose: React error boundaries require
// getDerivedStateFromError/componentDidCatch, which have no hook equivalent.
//
// Callers: app/AppRightRail.tsx (wraps every right-tab body, keyed by tab so a
// tab switch resets the boundary). Dependencies: layout/panelPersistGuard.

import { Component, Fragment, type ErrorInfo, type ReactNode } from 'react'
import { recordPanelRenderFailure } from '../layout/panelPersistGuard'
// Shared drawer visual language (cd-*): the notice must style itself even when
// the failed panel never got to import its own stylesheet.
import '../panels/drawer.css'

export interface PanelErrorBoundaryProps {
  /** Persisted tab id ('properties' | 'color' | …) — the blocklist key. */
  tab: string
  /** Human label for the notice copy ("Color", "Audio", …). */
  label: string
  children: ReactNode
}

interface PanelErrorBoundaryState {
  failed: boolean
  /** Bumped by "Try again" to remount the children. */
  attempt: number
}

export default class PanelErrorBoundary extends Component<PanelErrorBoundaryProps, PanelErrorBoundaryState> {
  state: PanelErrorBoundaryState = { failed: false, attempt: 0 }

  static getDerivedStateFromError(): Partial<PanelErrorBoundaryState> {
    return { failed: true }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // Blocklist the tab so loadLayout() boots into Properties next launch;
    // the console line keeps the real error auditable on rigs (CDP logs).
    recordPanelRenderFailure(this.props.tab)
    console.error(`[cut] right-tab "${this.props.tab}" failed to render:`, error, info.componentStack)
  }

  render() {
    if (this.state.failed) {
      const { tab, label } = this.props
      return (
        <section className="cd-embed" data-cut-panel-render-failed={tab} aria-label={`${label} unavailable`}>
          <div className="cd-body">
            <div className="cd-empty">
              The {label} tools hit a drawing error and were turned off. Your project and edits are safe.
            </div>
            <button
              type="button"
              className="cd-btn cd-btn--ghost"
              data-cut-panel-render-retry={tab}
              onClick={() => this.setState((s) => ({ failed: false, attempt: s.attempt + 1 }))}
            >
              Try again
            </button>
          </div>
        </section>
      )
    }
    // key remounts the subtree on retry so the failed component tree is rebuilt
    // from scratch rather than re-rendered in its broken state.
    return <Fragment key={this.state.attempt}>{this.props.children}</Fragment>
  }
}
