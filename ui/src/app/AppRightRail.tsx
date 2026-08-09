import { Suspense, lazy, useEffect, useLayoutEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from 'react'
import type { OpRecord, Project, RenderReceipt } from '../lib/client'
import type { DoctorReport } from '../lib/doctor'
import { isEditableTarget } from '../lib/dom'
import { Icon } from '../icons'
import Divider from '../layout/Divider'
import {
  armPanelAttempt,
  confirmPanelPainted,
  disarmPanelAttempt,
  disarmPanelAttemptOnOrderlyUnload,
  isPanelRenderBlocked,
} from '../layout/panelPersistGuard'
import type { LayoutState, RightTab } from '../layout/useLayout'
import PanelErrorBoundary from '../components/PanelErrorBoundary'
import Review from '../panels/Review'
import type { ReviewTab, ReviewTabRequest } from '../panels/Review'

const Inspector = lazy(() => import('../panels/Inspector'))
const AgentChat = lazy(() => import('../panels/AgentChat'))
const GradeDrawer = lazy(() => import('../panels/Grade'))
const MixerDrawer = lazy(() => import('../panels/Mixer'))

type AgentChatPrefill = { prompt: string; nonce: number } | null

interface AppRightRailProps {
  /** Keep event bridges mounted while a full workspace temporarily hides the rail. */
  hidden?: boolean
  layout: LayoutState
  setLayout: Dispatch<SetStateAction<LayoutState>>
  dragRail: (clientX: number, clientY: number) => void
  project: Project | null
  doctor: DoctorReport | null
  ops: OpRecord[]
  receipts: RenderReceipt[]
  selectedClipId: string | null
  playheadMs: number
  agentChatPrefill: AgentChatPrefill
  onReject: (opId: string) => void
  onUndo: () => void
  onRedo: () => void
}

function SurfaceLoading({ label = 'Loading' }: { label?: string }) {
  return (
    <div className="app__loading" data-cut-loading>
      {label}
    </div>
  )
}

const rightTabs: ReadonlyArray<readonly [RightTab, string]> = [
  ['properties', 'Properties'],
  ['color', 'Color'],
  ['audio', 'Audio'],
  ['chat', 'Chat'],
]

/** How long after the double-rAF paint proof we wait before declaring the tab
 *  body safely painted. Covers a compositor that dies a frame or two AFTER the
 *  first committed frame. Kept short so a normal app close rarely lands inside the window —
 *  a false positive only costs one honest notice + one click next launch. */
const PANEL_PAINT_SETTLE_MS = 350

/** Rendered as a SIBLING of the active tab body inside the same Suspense, so
 *  its effect runs in the same commit that mounts the (lazy) panel. Two
 *  requestAnimationFrame hops prove a frame containing the panel actually
 *  reached the screen; a settle delay then confirms the attempt. If the
 *  WebKit web process dies while painting a panel under software rendering,
 *  these callbacks never run, the sentinel armed
 *  by AppRightRail survives, and the next launch refuses to restore the tab. */
function PanelPaintConfirm({ tab }: { tab: string }) {
  useEffect(() => {
    let raf2 = 0
    let timer: number | undefined
    const raf1 = requestAnimationFrame(() => {
      raf2 = requestAnimationFrame(() => {
        timer = window.setTimeout(() => confirmPanelPainted(tab), PANEL_PAINT_SETTLE_MS)
      })
    })
    return () => {
      cancelAnimationFrame(raf1)
      if (raf2) cancelAnimationFrame(raf2)
      if (timer !== undefined) window.clearTimeout(timer)
    }
  }, [tab])
  return null
}

export default function AppRightRail({
  hidden = false,
  layout,
  setLayout,
  dragRail,
  project,
  doctor,
  ops,
  receipts,
  selectedClipId,
  playheadMs,
  agentChatPrefill,
  onReject,
  onUndo,
  onRedo,
}: AppRightRailProps) {
  const railRef = useRef<HTMLDivElement>(null)
  const railOpen = !layout.railCollapsed
  const railPinned = layout.railPinned
  const [reviewTabRequest, setReviewTabRequest] = useState<ReviewTabRequest | null>(null)

  // ---- crash-safe tab mounting ---------------------------------------------
  // A right-tab body that previously took the WebView down (blocklisted by
  // panelPersistGuard) is NOT mounted automatically: an honest notice with a
  // "load anyway" action shows instead. `retryTabs` records the user's/agent's
  // explicit choice to mount it anyway this session.
  const activeTab = layout.rightTab
  const activeTabLabel = rightTabs.find(([id]) => id === activeTab)?.[1] ?? activeTab
  const bodyMounted = !hidden && railOpen && layout.workspaceMode === 'edit'
  const [retryTabs, setRetryTabs] = useState<ReadonlySet<string>>(() => new Set())
  const tabBlocked = useMemo(
    () => bodyMounted && !retryTabs.has(activeTab) && isPanelRenderBlocked(activeTab),
    [bodyMounted, retryTabs, activeTab],
  )

  // Two-phase commit around the mounted tab body: arm the sentinel BEFORE the
  // browser can paint the panel (layout effect = post-commit, pre-paint), and
  // disarm on clean switch-away/unmount. PanelPaintConfirm (inside the
  // Suspense, below) clears it once a painted frame is proven. If the WebKit
  // web process dies in between, no cleanup runs, the sentinel survives, and
  // the NEXT launch boots into Properties instead of the killer panel.
  useLayoutEffect(() => {
    if (!bodyMounted || tabBlocked) return
    armPanelAttempt(activeTab)
    return () => disarmPanelAttempt(activeTab)
  }, [bodyMounted, tabBlocked, activeTab])

  // Orderly unloads (reload/navigation/quit) fire pagehide with JS alive —
  // not the crash class the sentinel hunts. Without this, a reload inside the
  // arm→confirm window would blocklist an innocent tab at the next boot
  // during a normal reload. Once per boot.
  useEffect(() => {
    disarmPanelAttemptOnOrderlyUnload()
  }, [])

  useEffect(() => {
    const requestTab = (tab: ReviewTab, diff?: { from: string; to: string }) => {
      setLayout((l) => ({ ...l, workspaceMode: 'edit', railCollapsed: false, railPinned: true }))
      setReviewTabRequest((prev) => ({ tab, diff, nonce: (prev?.nonce ?? 0) + 1 }))
    }
    const onOpenReceipts = () => requestTab('receipts')
    const onOpenReviewTab = (event: Event) => {
      const detail = (event as CustomEvent<ReviewTab | { tab: ReviewTab; from?: string; to?: string }>).detail
      const tab = typeof detail === 'string' ? detail : detail?.tab
      if (tab === 'ops' || tab === 'receipts' || tab === 'qc' || tab === 'scopes' || tab === 'diff') {
        const diff = typeof detail === 'object' && detail.from && detail.to ? { from: detail.from, to: detail.to } : undefined
        requestTab(tab, diff)
      }
    }
    document.addEventListener('cut:open-receipts', onOpenReceipts)
    document.addEventListener('cut:open-review-tab', onOpenReviewTab)
    return () => {
      document.removeEventListener('cut:open-receipts', onOpenReceipts)
      document.removeEventListener('cut:open-review-tab', onOpenReviewTab)
    }
  }, [setLayout])

  useEffect(() => {
    if (hidden || !railOpen || railPinned) return
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null
      if (isEditableTarget(t)) return
      if (e.key === 'Escape') {
        if (document.querySelector('[data-cut-timeline-automation-menu]:not([hidden])')) return
        e.preventDefault()
        setLayout((l) => ({ ...l, railCollapsed: true }))
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [hidden, railOpen, railPinned, setLayout])

  useEffect(() => {
    if (hidden || !railOpen || railPinned) return
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as HTMLElement | null
      if (!target) return
      if (railRef.current?.contains(target)) return
      if (target.closest('[data-cut-action="expand-rail"]')) return
      if (target.closest('[data-cut-timeline-automation]')) return
      setLayout((l) => ({ ...l, railCollapsed: true }))
    }
    document.addEventListener('pointerdown', onPointerDown, true)
    return () => document.removeEventListener('pointerdown', onPointerDown, true)
  }, [hidden, railOpen, railPinned, setLayout])

  if (hidden) return null

  const railClass = [
    'app__rail',
    layout.railCollapsed ? 'app__rail--collapsed' : '',
    railOpen && railPinned ? 'app__rail--pinned' : '',
    railOpen && !railPinned ? 'app__rail--overlay' : '',
  ].filter(Boolean).join(' ')

  return (
    <>
      {railOpen && railPinned && <Divider orient="v" id="rail" onDrag={dragRail} />}
      {layout.railCollapsed && (
        <button
          className={`app__side-expand app__side-expand--right${selectedClipId ? ' app__side-expand--has-selection' : ''}`}
          data-cut-action="expand-rail"
          data-cut-selected-clip={selectedClipId ?? undefined}
          onClick={() => setLayout((l) => ({ ...l, railCollapsed: false }))}
          title={selectedClipId ? 'Tools — a clip is selected; open Properties, Color, or Audio' : 'Tools — Properties · Color · Audio · Chat'}
          aria-label={selectedClipId ? 'Open tools for selected clip' : 'Open selected-clip tools'}
        >
          <Icon name="collapseLeft" size={14} />
          {selectedClipId && <span className="app__side-expand-selection" aria-hidden="true">1</span>}
          <span className="app__side-expand-label">Tools</span>
        </button>
      )}
      <div
        ref={railRef}
        className={railClass}
        style={{ width: layout.railCollapsed ? undefined : layout.railW }}
        data-cut-rail-overlay={railOpen && !railPinned ? 'true' : undefined}
        data-cut-rail-pinned={railOpen && railPinned ? 'true' : undefined}
      >
        {!layout.railCollapsed && layout.workspaceMode === 'edit' && (
          <div className="app__inspector">
            <div className="app__rtab-bar">
              <div className="app__rtabs" role="tablist" aria-label="Selected-clip tool tabs" data-cut-right-tabs={layout.rightTab}>
                {rightTabs.map(([id, label]) => (
                  <button
                    key={id}
                    role="tab"
                    aria-selected={layout.rightTab === id}
                    className={`app__rtab ${layout.rightTab === id ? 'app__rtab--active' : ''}`}
                    data-cut-right-tab={id}
                    onClick={() => setLayout((l) => ({ ...l, rightTab: id }))}
                  >
                    {label}
                  </button>
                ))}
              </div>
              <div className="app__rail-controls" aria-label="Selected-clip tool layout">
                <button
                  type="button"
                  className={`app__rail-control ${railPinned ? 'app__rail-control--active' : ''}`}
                  data-cut-rail-pin
                  aria-pressed={railPinned}
                  title={railPinned ? 'Unpin tools from the layout' : 'Pin tools beside the editor'}
                  aria-label={railPinned ? 'Unpin selected-clip tools' : 'Pin selected-clip tools'}
                  onClick={() => setLayout((l) => ({ ...l, railPinned: !l.railPinned, railCollapsed: false }))}
                >
                  <Icon name={railPinned ? 'panelRight' : 'lock'} size={14} />
                </button>
                <button
                  type="button"
                  className="app__rail-control"
                  data-cut-rail-close
                  title="Close tools"
                  aria-label="Close selected-clip tools"
                  onClick={() => setLayout((l) => ({ ...l, railCollapsed: true }))}
                >
                  <Icon name="close" size={14} />
                </button>
              </div>
            </div>
            <div className="app__rtab-body">
              {tabBlocked ? (
                /* This tab's last mount never confirmed a paint (or its render
                   threw): under software rendering it can blank the whole
                   WebView, so it stays OFF until explicitly requested. Same
                   cd-* grammar as the drawers; both selectors are stable agent
                   handles (Debuggability Rule). */
                <section
                  className="cd-embed"
                  data-cut-panel-render-blocked={activeTab}
                  aria-label={`${activeTabLabel} tools not loaded`}
                >
                  <div className="cd-body">
                    <div className="cd-empty">
                      The {activeTabLabel} tools didn&apos;t finish drawing last time, so they stayed off
                      to keep the editor usable. This usually happens under software rendering
                      (virtual machines and remote desktops). Your project and edits are safe.
                    </div>
                    <button
                      type="button"
                      className="cd-btn cd-btn--primary"
                      data-cut-panel-render-retry={activeTab}
                      onClick={() => setRetryTabs((prev) => new Set(prev).add(activeTab))}
                    >
                      Load {activeTabLabel} tools
                    </button>
                  </div>
                </section>
              ) : (
                /* keyed by tab: a tab switch resets a tripped boundary. The
                   PanelPaintConfirm sibling commits together with the lazy
                   panel, so its paint proof covers the panel's actual mount. */
                <PanelErrorBoundary key={activeTab} tab={activeTab} label={activeTabLabel}>
                  <Suspense fallback={<SurfaceLoading />}>
                    {activeTab === 'properties' && (
                      <Inspector
                        project={project}
                        selectedClipId={selectedClipId}
                        playheadMs={playheadMs}
                        doctor={doctor}
                      />
                    )}
                    {activeTab === 'color' && (
                      <GradeDrawer project={project} clipId={selectedClipId} />
                    )}
                    {activeTab === 'audio' && (
                      <MixerDrawer
                        project={project}
                        playheadMs={playheadMs}
                        headOpId={ops.length ? ops[ops.length - 1].op_id : ''}
                      />
                    )}
                    {activeTab === 'chat' && (
                      <AgentChat project={project} prefill={agentChatPrefill} />
                    )}
                    <PanelPaintConfirm tab={activeTab} />
                  </Suspense>
                </PanelErrorBoundary>
              )}
            </div>
          </div>
        )}
        {railPinned && (
          <Review
            project={project}
            playheadMs={playheadMs}
            ops={ops}
            receipts={receipts}
            onReject={onReject}
            onUndo={onUndo}
            onRedo={onRedo}
            reviewTabRequest={reviewTabRequest}
            onCollapse={() => setLayout((l) => ({ ...l, railCollapsed: true }))}
          />
        )}
      </div>
    </>
  )
}
