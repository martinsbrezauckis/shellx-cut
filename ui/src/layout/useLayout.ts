// layout/useLayout.ts — persisted panel-size state.
// Role: owns the user-resizable dimensions of the app shell and their
// localStorage persistence ("Persist sizes to localStorage"):
//   txFrac        left-sidebar share of the upper split  (default 0.40)
//   tlH           timeline height in px                  (default 280, min 160)
//   railW         review-rail width in px                (340 default, 280–480)
//   railCollapsed right review rail hidden (`\` or its collapse button)
//   railPinned    right rail consumes layout width instead of floating as a
//                 contextual overlay
//   leftCollapsed left sidebar hidden (its collapse button) — preview goes wide
//   leftTab       active left-sidebar tab (Transcript | Assets | Generate | … | Find),
//                 persisted so a reload returns to the tab the user last worked in
//   rightTab      active RIGHT-sidebar tab (Properties | Color | Audio) — the
//                 Inspector + the (formerly drawer-bound) Grade + Mixer surfaces,
//                 now docked as tabs so they never overlap the topbar menus
// Pixel mins/maxes that depend on the live container size (sidebar 320px,
// preview 480px, timeline max 60% height) are clamped at drag time in App.tsx
// and re-enforced by CSS min-/max- rules; this hook only sanity-clamps loaded
// values so a stale/garbage localStorage entry can never wedge the layout.
// Callers: App.tsx. Dependencies: react only.

import { useEffect, useState } from 'react'

/** Which tab the tabbed left sidebar (LeftPanel) is showing. Library is a
 * dedicated workspace; persisted legacy `leftTab:"library"` values migrate to
 * Assets in `load()` instead of recreating the old cramped rail. */
export type LeftTab = 'transcript' | 'assets' | 'generate' | 'projects' | 'find'

/** Which Find surface the LEFT sidebar's Find tab is showing:
 *  'find-media' (assets.search), 'find-moment' (media.index/search), or the
 *  cross-timeline 'sequence-index' (project.sequence_index).
 *  (Generated-media placement: AI Generate was a third surface here; it CREATES media, so it is now a
 *  project-media-adjacent left tab — Find stays pure search.) */
export type FindSurface = 'find-media' | 'find-moment' | 'sequence-index'

/** Workspace mode. The mode switch swaps
 *  the layout while the project persists, in production order. Edit is the full
 *  NLE; Record is the flagship capture surface. Color + Audio used to be modes too,
 *  but a non-fullscreen window let the topbar Projects menu overlap their labels —
 *  they are now RIGHT-SIDEBAR TABS (rightTab below), not modes. */
export type WorkspaceMode = 'edit' | 'record' | 'library' | 'export'

/** Which tab the RIGHT sidebar is showing: Properties = the Inspector,
 *  Color = the grade controls (edit.grade), Audio = the per-track mixer (edit.gain). */
export type RightTab = 'properties' | 'color' | 'audio' | 'chat'

export interface LayoutState {
  /** Left-sidebar fraction of the sidebar|preview split width (0–1). */
  txFrac: number
  /** Timeline panel height, px. */
  tlH: number
  /** Review rail width, px. */
  railW: number
  /** Right review rail collapsed (state survives reload like the sizes do). */
  railCollapsed: boolean
  /** Right rail pinned into layout; false keeps selected-clip tools as an overlay. */
  railPinned: boolean
  /** Left sidebar collapsed — hidden so the preview/timeline take full width. */
  leftCollapsed: boolean
  /** Active left-sidebar tab, persisted across reloads. */
  leftTab: LeftTab
  /** Which Find surface the left sidebar's Find tab shows. */
  findSurface: FindSurface
  /** Active workspace mode (Edit · Record), persisted. */
  workspaceMode: WorkspaceMode
  /** Active right-sidebar tab (Properties · Color · Audio), persisted. */
  rightTab: RightTab
}

// v2: added leftCollapsed + leftTab (tabbed left sidebar). A v1 entry is missing
// these keys; `load()` fills them from defaults, so the bump is non-breaking and
// we keep the same key (defensive merge below tolerates either shape).
const KEY = 'cut.layout.v1'

/** defaults: 40/60 split · timeline 280px · rail 340px · left sidebar on the
 *  Projects tab. A fresh user should see where work is created/reopened before
 *  encountering project-local Assets or Transcript surfaces.
 *
 *  Receipt-philosophy default: the review rail
 *  (OPS · RECEIPTS · QC · DIFF) is the AGENT's instrument panel, not the
 *  user's — so it starts COLLAPSED. Normal use shows a clean editor + the
 *  status-bar one-line "what changed"; the rail is the deliberate Inspect /
 *  Advanced surface, one click (the right-edge "Inspect" strip, `\`, `R`, or
 *  the receipt chip) away. Changing only the DEFAULT keeps any persisted user
 *  choice intact (load() merges localStorage over these). */
export const LAYOUT_DEFAULTS: LayoutState = {
  txFrac: 0.4,
  tlH: 280,
  railW: 340,
  railCollapsed: true,
  railPinned: false,
  leftCollapsed: false,
  leftTab: 'projects',
  findSurface: 'find-media',
  workspaceMode: 'edit',
  rightTab: 'properties',
}

/** hard bounds (the container-relative ones live in the drag handlers). */
export const LAYOUT_BOUNDS = {
  txFrac: [0.15, 0.85],
  tlH: [160, 4000], // upper bound is the 60%-of-column clamp at drag time
  railW: [280, 480],
} as const

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))
const isObject = (v: unknown): v is object => v !== null && typeof v === 'object'
const isLeftTab = (v: string): v is LeftTab => ['transcript', 'assets', 'generate', 'projects', 'find'].includes(v)
const isRightTab = (v: string): v is RightTab => ['color', 'audio', 'chat'].includes(v)

/** Load + defensively clamp persisted layout (corrupt entry → defaults). */
function load(): LayoutState {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return LAYOUT_DEFAULTS
    const p = JSON.parse(raw)
    if (!isObject(p)) return LAYOUT_DEFAULTS
    const txFrac = 'txFrac' in p ? Number(p.txFrac) : LAYOUT_DEFAULTS.txFrac
    const tlH = 'tlH' in p ? Number(p.tlH) : LAYOUT_DEFAULTS.tlH
    const railW = 'railW' in p ? Number(p.railW) : LAYOUT_DEFAULTS.railW
    const leftTabValue = 'leftTab' in p ? String(p.leftTab) : ''
    // Library used to be a left-sidebar tab. Treat that persisted value like
    // any unknown legacy value; the dedicated workspace opens only on an
    // explicit current-session user/agent action.
    const leftTab: LeftTab = isLeftTab(leftTabValue) ? leftTabValue : LAYOUT_DEFAULTS.leftTab
    const findSurfaceValue = 'findSurface' in p ? String(p.findSurface) : ''
    const findSurface: FindSurface = findSurfaceValue === 'find-moment' || findSurfaceValue === 'sequence-index'
      ? findSurfaceValue
      : 'find-media'
    const rightTabValue = 'rightTab' in p ? String(p.rightTab) : ''
    const rightTab: RightTab = isRightTab(rightTabValue) ? rightTabValue : 'properties'
    return {
      txFrac: clamp(txFrac || LAYOUT_DEFAULTS.txFrac, ...LAYOUT_BOUNDS.txFrac),
      tlH: clamp(tlH || LAYOUT_DEFAULTS.tlH, ...LAYOUT_BOUNDS.tlH),
      railW: clamp(railW || LAYOUT_DEFAULTS.railW, ...LAYOUT_BOUNDS.railW),
      railCollapsed: 'railCollapsed' in p && p.railCollapsed === true,
      railPinned: 'railPinned' in p && p.railPinned === true,
      leftCollapsed: 'leftCollapsed' in p && p.leftCollapsed === true,
      // Restore any permanent left tab. Unknown/corrupt values use the fresh-layout
      // default instead of forcing users back to another surface.
      leftTab,
      // Generated-media placement: 'generate' is no longer a Find surface — a persisted 'generate' (or any
      // unknown value) migrates to the default 'find-media'.
      findSurface,
      // ALWAYS launch in the editor. workspaceMode is intentionally NOT restored:
      // Record and Library swap the whole work area, so reopening into either
      // feels broken. The editor is home; users re-enter those tasks deliberately.
      // (Color/Audio are no longer modes; they are right-sidebar tabs.)
      workspaceMode: 'edit',
      // The right-sidebar tab IS restored (it's a persistent inspector surface, not
      // a transient mode): a user who works in Color/Audio returns to it on reload.
      rightTab,
    }
  } catch {
    return LAYOUT_DEFAULTS // private mode / quota / bad JSON — never fatal
  }
}

/**
 * Layout state + setter, persisted to localStorage (debounced 250ms so a
 * 60Hz divider drag doesn't write per-frame; the trailing write always lands).
 */
export function useLayout() {
  const [layout, setLayout] = useState<LayoutState>(load)
  useEffect(() => {
    const t = setTimeout(() => {
      try {
        localStorage.setItem(KEY, JSON.stringify(layout))
      } catch {
        // storage unavailable — layout still works for the session
      }
    }, 250)
    return () => clearTimeout(t)
  }, [layout])
  return [layout, setLayout] as const
}
