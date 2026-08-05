// App.tsx — ShellX Cut app shell (UI contract).
// Role: owns the WS connection + the verb-fetched snapshots, lays out the
// shell — 54px top bar / middle (transcript LEFT ~40% | preview RIGHT ~60%
// over timeline, review rail full-height right) / 40px status bar — with
// three draggable, persisted dividers, and routes panel callbacks into VERBS
// (the UI is an API client; no panel mutates
// state locally).
// Dependencies: lib/client (verbs), lib/events (WS), layout/* (dividers +
// persisted sizes), topbar/, panels/*, statusbar/. Callers: main.tsx.

import { Suspense, lazy, useCallback, useEffect, useRef, useState, type CSSProperties } from 'react'
import {
  callVerb,
  fetchProject,
  type OpRecord,
  type Project,
  type RenderReceipt,
  type Transcript as TranscriptData,
} from './lib/client'
import { events, type CutEvent } from './lib/events'
import { fetchDoctor, shouldAutoPopWizard, type DoctorReport } from './lib/doctor'
import KeymapOverlay from './KeymapOverlay'
import type { SettingsCategoryId } from './panels/Environment/settingsModel'
import HighlightOverlay, { type HighlightSpec } from './HighlightOverlay'
import { CommandPalette } from './palette/CommandPalette'
import DropZone from './DropZone'
import type { GenerateWorkspaceTab } from './panels/GenerateTemplates'
import StatusBar from './statusbar'
import TopBar from './topbar'
import AppDrawerStack, { type AppDrawer } from './app/AppDrawerStack'
import AppRightRail from './app/AppRightRail'
import AppWorkspace from './app/AppWorkspace'
import { useAppImportEvents } from './app/useAppImportEvents'
import { useAppLayoutController } from './app/useAppLayoutController'
import { useAppClipboardController } from './app/useAppClipboardController'
import { useAppKeyboardController } from './app/useAppKeyboardController'
import { useAppSurfaceEvents } from './app/useAppSurfaceEvents'
import { useUiCommandController } from './app/useUiCommandController'
import { useUiStatePublisher } from './app/useUiStatePublisher'
import { preferredProjectLeftTab, shouldReturnToProjectsAfterResync } from './app/model'
import UserActionFeedback from './components/UserActionFeedback'
import { runUserVerb } from './lib/userActionFeedback'
import { OfflineMediaProvider } from './app/OfflineMediaContext'

const EnvironmentPanel = lazy(() =>
  import('./panels/Environment').then((module) => ({ default: module.EnvironmentPanel })),
)
const StartWizard = lazy(() =>
  import('./panels/Environment').then((module) => ({ default: module.StartWizard })),
)

function SurfaceLoading({ label = 'Loading' }: { label?: string }) {
  return <div className="app__loading" data-cut-loading>{label}</div>
}

export default function App() {
  // --- server-truth snapshots (refreshed via verbs + WS events) -------------
  const [project, setProject] = useState<Project | null>(null)
  const [ops, setOps] = useState<OpRecord[]>([])
  const [receipts, setReceipts] = useState<RenderReceipt[]>([])
  const [transcripts] = useState<Record<string, TranscriptData>>({})
  // --- UI-local view state (mirrored to the server via ui_state pushes) -----
  const [playheadMs, setPlayheadMs] = useState(0)
  const [selectedClipIds, setSelectedClipIds] = useState<string[]>([])
  // Export RANGE [in,out] painted on the ruler (drag-select). The explicit span
  // "Save as clip" / Render section uses — no clip selection, no 30s fallback.
  const [exportRange, setExportRange] = useState<[number, number] | null>(null)
  // connection + job pills live in statusbar/ (self-subscribed to `events`)
  // --- Environment doctor (the start wizard + Settings>Environment) ----------
  const [doctor, setDoctor] = useState<DoctorReport | null>(null)
  const [wizardOpen, setWizardOpen] = useState(false)
  const [envOpen, setEnvOpen] = useState(false)
  const [envCategory, setEnvCategory] = useState<SettingsCategoryId>('overview')
  const [generateTab, setGenerateTab] = useState<GenerateWorkspaceTab>('templates')
  const agentChatPromptSeq = useRef(0)
  const [agentChatPrefill, setAgentChatPrefill] = useState<{ prompt: string; nonce: number } | null>(null)
  // The right-side control drawers (Music, Title, Kinetic captions, Grade) —
  // each a one-verb-convenience scrim modal launched from its natural home
  // (Music/Title from the topbar, Kinetic from the transcript, Grade from the
  // selected clip), not relay-claimed (the verbs are agent-callable directly —
  // zero-local-mutation contract). MUTUALLY EXCLUSIVE: they all dock at the same right-side
  // slot, so exactly ONE is open at a time — a single `activeDrawer` makes
  // opening one close any other (was 4 independent bools → they stacked on top
  // of each other, a reported UX bug). null = none open.
  // 'grade' + 'mixer' are no longer modal drawers; they are the right-sidebar
  // Color / Audio TABS (rightTab). They're dropped from this union; their former
  // launch points (Inspector tools, command palette, the timeline Grade button, the
  // topbar mixer button) now switch the right tab + expand the rail (openRightTab).
  // Generate is the native template/prompt/storyboard workspace. It lives as a
  // project-media-adjacent left tab; provider-backed assets.generate is folded into
  // that workspace's Media subtab.
  const [activeDrawer, setActiveDrawer] = useState<AppDrawer | null>(null)
  const closeDrawer = () => setActiveDrawer(null)
  // Open a right-sidebar tab (Properties · Color · Audio) and ensure the rail
  // is expanded so it's actually visible (the rail defaults collapsed). Used by the
  // Color/Audio launchers that used to open the Grade/Mixer drawers. A plain closure
  // (like toggleLeftTab) so it can reference setLayout declared just below.
  const openRightTab = (t: 'properties' | 'color' | 'audio' | 'chat') =>
    setLayout((l) => ({ ...l, workspaceMode: 'edit', rightTab: t, railCollapsed: false }))
  // A topbar launcher TOGGLES its drawer — clicking the active one closes it:
  // "second click closes it"). Distinct drawers replace each other (one slot).
  const toggleDrawer = (d: AppDrawer) => {
    setLayout((l) => l.workspaceMode === 'library' ? { ...l, workspaceMode: 'edit' } : l)
    setActiveDrawer((cur) => (cur === d ? null : d))
  }
  // Projects remains a left-sidebar destination. Library has its own workspace;
  // opening either from another mode returns the editor shell to a coherent
  // state instead of leaving a hidden rail selection behind it.
  const toggleProjects = () =>
    setLayout((l) =>
      l.workspaceMode === 'edit' && l.leftTab === 'projects' && !l.leftCollapsed
        ? { ...l, leftCollapsed: true }
        : { ...l, workspaceMode: 'edit', leftTab: 'projects', leftCollapsed: false },
    )
  const toggleLibraryWorkspace = () =>
    setLayout((l) => ({
      ...l,
      workspaceMode: l.workspaceMode === 'library' ? 'edit' : 'library',
    }))
  // Agent-driven element highlight (ui.highlight). The nonce makes re-highlighting
  // the same target re-trigger the overlay effect.
  const [highlight, setHighlight] = useState<HighlightSpec | null>(null)
  const highlightNonce = useRef(0)
  // The review-comment rail (left side), default hidden so the timeline
  // keeps full width; toggled with `]` or the topbar Review button.
  const [commentsOpen, setCommentsOpen] = useState(false)
  // A comment to focus (select + scroll to) when the rail opens — set by a
  // timeline pin click. Passed as a prop (not an event) because the panel mounts
  // AFTER the rail opens, so an event would fire before its listener attaches.
  const [focusComment, setFocusComment] = useState<{ id: string; n: number } | null>(null)
  // Surface the first-run wizard automatically AT MOST once per session — once
  // dismissed, the user reaches the env via the status-bar chip (never nags).
  const wizardAutoShown = useRef(false)

  /** Re-fetch the doctor (refresh=true forces a re-scan). Used by the wizard /
   *  env "Re-scan" buttons and after a fetch action. */
  const refreshDoctor = useCallback(async (refresh = true) => {
    const d = await fetchDoctor(refresh)
    if (d) setDoctor(d)
    return d
  }, [])

  const { layout, setLayout, middleRef, mainRef, splitRef, txWidth, dragSplit, dragTimeline, dragRail } =
    useAppLayoutController(selectedClipIds)
  const { clipboardHasContent, clipboardClipId, clipboardNotice, copyClip, cutClip, pasteClip, clearClipboard } = useAppClipboardController({
    project,
    playheadMs,
    selectedClipIds,
    setSelectedClipIds,
  })
  const openUiSurface = useAppSurfaceEvents({
    setLayout,
    setCommentsOpen,
    setFocusComment,
    setActiveDrawer,
    setGenerateTab,
    setWizardOpen,
    setEnvOpen,
    setEnvCategory,
    onRefreshDoctor: () => refreshDoctor(true),
    agentChatPromptSeq,
    setAgentChatPrefill,
  })
  const uiStateRef = useUiStatePublisher({
    layout,
    generateTab,
    wizardOpen,
    envOpen,
    envCategory,
    commentsOpen,
    activeDrawer,
    highlight,
    playheadMs,
    selectedClipIds,
    exportRange,
    project,
  })

  useEffect(() => {
    const onLocalHighlight = (e: Event) => {
      const a = ((e as CustomEvent<Partial<HighlightSpec> & { clear?: boolean }>).detail ?? {}) as Partial<HighlightSpec> & { clear?: boolean }
      if (a.clear || (!a.selector && !a.clip && !a.panel)) {
        setHighlight(null)
        return
      }
      highlightNonce.current += 1
      setHighlight({
        selector: a.selector,
        clip: a.clip,
        panel: a.panel,
        label: a.label,
        description: a.description,
        duration_ms: a.duration_ms,
        scroll: a.scroll,
        n: highlightNonce.current,
      })
    }
    document.addEventListener('cut:local-highlight', onLocalHighlight)
    return () => document.removeEventListener('cut:local-highlight', onLocalHighlight)
  }, [])

  // Out-of-order fetch guard. op_applied/job_progress each spawn a FULL project
  // refetch, and these resolve out of order — a stale snapshot can clobber a fresher
  // one. The recorder's a_system (system-audio) track is placed LAST, after the
  // screen_record.polish system.wav probe wait, so its op_applied fires several
  // seconds after the video/mic ops; an earlier refetch resolving late would drop it
  // → "the 3rd track shows up late / only after I interact". Tag every refetch; apply
  // only the most-recently-ISSUED one. A later-issued fetch always sees >= the server
  // state of an earlier one, so latest-issued-wins equals newest-state-wins.
  const projectFetchSeq = useRef(0)
  const refreshProject = useCallback(async (): Promise<Project | null | undefined> => {
    const id = ++projectFetchSeq.current
    const p = await fetchProject()
    // A newer refresh superseded this response. Keep that distinct from the
    // server's real `null` (no project open): resync uses the latter to return
    // to Projects, while a stale response must leave the current workspace
    // alone. Conflating both values could unmount Record just after a polished
    // clip landed, hiding its Done / Export controls.
    if (id !== projectFetchSeq.current) return undefined
    setProject(p)
    return p
  }, [])

  /** Full state resync — on connect and after reconnect (WS may have lagged). */
  const resync = useCallback(async () => {
    const activeProject = await refreshProject()
    if (shouldReturnToProjectsAfterResync(activeProject)) {
      setLayout((current) => (
        current.workspaceMode === 'edit'
        && current.leftTab === 'projects'
        && !current.leftCollapsed
          ? current
          : {
              ...current,
              workspaceMode: 'edit',
              leftTab: 'projects',
              leftCollapsed: false,
            }
      ))
    }
    const r = await callVerb('project.ops', {})
    if (r.ok && r.result) {
      // Merge by op_id; do not replace the whole collection. An op_applied event can land
      // during the `await` above and append to `ops` (:162); an unconditional
      // setOps(r.result.ops) would clobber it. Union the server snapshot with
      // any locally-appended ops not in it, ordered by op_id (zero-padded
      // sequential `op_000042` → lexicographic sort == natural/sequence order).
      const fresh = r.result.ops
      setOps((prev) => {
        const seen = new Set(fresh.map((o) => o.op_id))
        const extra = prev.filter((o) => !seen.has(o.op_id))
        if (extra.length === 0) return fresh
        return [...fresh, ...extra].sort((a, b) => (a.op_id < b.op_id ? -1 : a.op_id > b.op_id ? 1 : 0))
      })
    }
    // Receipts persist on disk (receipts/render_*.json) but receipt_ready
    // only fires once — a tab opened AFTER a render must still see the
    // verdict (open the UI at any moment and see live state).
    // verify.checks{} returns the latest receipt; errors (no project / no
    // renders yet) are the normal empty case.
    const rc = await callVerb('verify.checks', {})
    if (rc.ok && rc.result?.checks) {
      setReceipts((prev) => (prev.some((p) => p.render_id === rc.result!.render_id) ? prev : [...prev, rc.result!]))
    }
    // Pull the cached environment doctor on (re)connect. Auto-surface the first-run
    // wizard ONCE this session ONLY when the essential dep (ffmpeg) is CONFIRMED
    // missing — readiness tri-state: never on an UNVERIFIED probe-timeout ('unknown'), which
    // must not pop a sticky "essential missing" modal (shouldAutoPopWizard gates
    // this precisely, independent of the broader essential_ok flag).
    const d = await refreshDoctor(false)
    if (shouldAutoPopWizard(d) && !wizardAutoShown.current) {
      wizardAutoShown.current = true
      setWizardOpen(true)
    }
  }, [refreshDoctor, refreshProject, setLayout])

  useUiCommandController({
    stateRef: uiStateRef,
    project,
    setPlayheadMs,
    setSelectedClipIds,
    setHighlight,
    highlightNonce,
    openSurface: openUiSurface,
  })

  const historyNavRef = useRef<Promise<void>>(Promise.resolve())
  const enqueueHistoryNav = useCallback((verbName: 'project.undo' | 'project.redo') => {
    historyNavRef.current = historyNavRef.current
      .catch(() => undefined)
      .then(async () => {
        await callVerb(verbName, {})
        await resync()
      })
  }, [resync])

  useAppImportEvents({ project, onChanged: resync, setLayout })

  // Project SWITCH (New / Open) — a hard reset of all accumulated cross-project
  // state, then a fresh pull. op_ids restart per project (every project's first
  // op is op_000001), so resync()'s reconnect MERGE would KEEP the previous
  // project's ops as "extra" (they don't collide with the new project's single
  // create op) — that leaked the old project's history into the new one's Review
  // feed, and stale receipts/selection/playhead persisted (the "New project →
  // strange left sidebar" bug). REPLACE here, never merge.
  const onProjectSwitched = useCallback(async () => {
    setOps([])
    setReceipts([])
    setSelectedClipIds([])
    setExportRange(null)
    setPlayheadMs(0)
    setCommentsOpen(false)
    clearClipboard()
    const nextProject = await refreshProject() // guarded so a late old-project refetch can't clobber the switch
    if (nextProject) {
      setLayout((l) => ({
        ...l,
        leftTab: preferredProjectLeftTab(nextProject),
        leftCollapsed: false,
        workspaceMode: 'edit',
      }))
    } else {
      setLayout((l) => ({
        ...l,
        leftTab: 'projects',
        leftCollapsed: false,
        workspaceMode: 'edit',
      }))
    }
    const r = await callVerb('project.ops', {})
    if (r.ok && r.result) setOps(r.result.ops) // replace with THIS project's ops
  }, [clearClipboard, refreshProject, setLayout])

  const onSequenceChanged = useCallback(() => {
    setSelectedClipIds([])
    setExportRange(null)
    setPlayheadMs(0)
    clearClipboard()
    void resync()
  }, [clearClipboard, resync])

  // WS lifecycle: connect once, resync on every (re)open, fold events in.
  useEffect(() => {
    const offStatus = events.onStatus((s) => {
      if (s === 'open') void resync()
    })
    const offEvents = events.subscribe((ev: CutEvent) => {
      switch (ev.type) {
        case 'op_applied':
          setOps((prev) => [...prev, ev.op])
          void refreshProject() // snapshot follows every op
          break
        case 'job_progress':
          // Import-chain (probe→proxy→filmstrip→ready) and the background ENRICH
          // job (transcribe→perception, decoupled complete as JOB
          // progress, NOT ops — so no op_applied fires to refresh the project.
          // Without this the transcript panel stays "transcribe pending" and the
          // preview stays "PROXY RENDERING…" until a manual reload, even though
          // the data is ready (asset.transcript/proxy get set mid-chain). Re-fetch
          // as either chain advances (and on any job finishing) so the UI
          // converges live. Job PILLS render in statusbar/ (self-subscribed).
          if (ev.kind === 'import_chain' || ev.kind === 'enrich' || ev.progress >= 1) {
            void refreshProject()
          }
          break
        case 'render_done':
          break // receipts arrive via receipt_ready; pills via statusbar/
        case 'receipt_ready':
          // Deduplicate by render_id — the same guard the resync path uses.
          // (:143). Without it a receipt the resync already folded in (a tab
          // opened after the render) would re-append when receipt_ready fires,
          // producing a duplicate receipt card.
          setReceipts((prev) =>
            prev.some((p) => p.render_id === ev.receipt.render_id) ? prev : [...prev, ev.receipt],
          )
          break
        case 'ui_state':
          // Echo of our own pushes (the server rebroadcasts ui_state) —
          // agent-driven view changes arrive as ui_command below.
          break
        case 'project_changed':
          // Project create/open/close can originate from REST, CLI, MCP, or
          // another UI client. Refresh the visible workspace even though these
          // transitions do not append a project op.
          void onProjectSwitched()
          break
        case 'doctor_updated':
          // The env changed (a fetch completed, or a refresh). Update the cards
          // live; re-surface the wizard ONLY if an essential just went CONFIRMED
          // missing and we never showed it (we never re-nag after a dismiss).
          // readiness tri-state: shouldAutoPopWizard requires ffmpeg === 'missing', so an
          // UNVERIFIED ('unknown') re-scan timeout never pops the modal.
          setDoctor(ev.report)
          if (shouldAutoPopWizard(ev.report) && !wizardAutoShown.current) {
            wizardAutoShown.current = true
            setWizardOpen(true)
          } else if (ev.report.essential_ok) {
            // The essential (ffmpeg) just became available — e.g. a system.fetch_tool
            // download finished while the first-run wizard was up. Auto-close the
            // wizard so the user isn't left staring at a now-satisfied gate.
            setWizardOpen((open) => (open ? false : open))
          }
          break
      }
    })
    events.connect()
    return () => {
      offStatus()
      offEvents()
    }
  }, [onProjectSwitched, resync])

  // --- panel callbacks ------------------------------------------------------
  // Playhead and selection are UI-view state, not project mutations. Commit
  // them locally; useUiStatePublisher publishes the resulting observable state
  // to the engine. Calling ui.playhead/ui.select here would relay the command
  // back into this same UI after the state already changed, so the confirmation
  // controller would correctly reject it as an already-applied conflict.
  const onSeek = useCallback((atMs: number) => {
    setPlayheadMs(Math.max(0, Math.round(atMs)))
  }, [])

  const onSelect = useCallback((clipIds: string[]) => {
    setSelectedClipIds(clipIds)
  }, [])

  const onCutWords = useCallback(
    (asset: string, wordRange: [number, number], rationale?: string) => {
      void runUserVerb(
        'transcript.cut_words',
        { asset, word_range: wordRange, rationale },
        'Could not remove the selected transcript words.',
      )
    },
    [],
  )

  const onRestore = useCallback((opId: string) => {
    void runUserVerb('edit.restore', { op_id: opId }, 'Could not restore that edit.')
  }, [])

  // Linear undo/redo: move the in-memory history cursor (the engine appends a
  // project.undo / project.redo nav op and publishes op_applied, which refreshes
  // the project). Guardrail errors at the baseline / tip are harmless no-ops.
  const onUndo = useCallback(() => {
    enqueueHistoryNav('project.undo')
  }, [enqueueHistoryNav])
  const onRedo = useCallback(() => {
    enqueueHistoryNav('project.redo')
  }, [enqueueHistoryNav])

  useAppKeyboardController({ setLayout, setCommentsOpen, onUndo, onRedo })

  // grid: 54px top bar / 1fr middle / 40px status bar. Middle = left
  // column (transcript|preview split over timeline) + full-height review
  // rail right. Three draggable dividers; sizes persisted via useLayout.
  return (
    <OfflineMediaProvider project={project} onProjectChanged={resync}>
    <div className="app" data-cut-app-root>
      <TopBar
        project={project}
        onOpenMusic={() => toggleDrawer('music')}
        onOpenMixer={() => openRightTab('audio')}
        onOpenProjects={toggleProjects}
        onOpenLibrary={toggleLibraryWorkspace}
        onOpenClips={() => toggleDrawer('clips')}
        onOpenAutopilot={() => toggleDrawer('autopilot')}
        onOpenAssemble={() => toggleDrawer('assemble')}
        onOpenRecipes={() => toggleDrawer('recipes')}
        onOpenMask={() => toggleDrawer('mask')}
        onOpenTitle={() => toggleDrawer('title')}
        onToggleComments={() => {
          setLayout((l) => ({ ...l, workspaceMode: 'edit' }))
          setCommentsOpen((v) => !v)
        }}
        commentsOpen={commentsOpen}
        openCommentCount={project?.comments?.filter((c) => c.status === 'open').length ?? 0}
        onProjectChanged={() => void resync()}
        onSequenceChanged={onSequenceChanged}
        playheadMs={playheadMs}
        mode={layout.workspaceMode}
        onMode={(m) => setLayout((l) => ({ ...l, workspaceMode: m }))}
        doctor={doctor}
        onOpenSetup={() => {
          // Same handler as the status-bar env chip: never both env surfaces at once.
          setWizardOpen(false)
          setEnvCategory('overview')
          setEnvOpen(true)
        }}
      />

      <div
        className="app__middle"
        ref={middleRef}
        data-cut-overlay-rail-open={!layout.railCollapsed && !layout.railPinned ? 'true' : undefined}
        style={{ '--cut-overlay-rail-width': `${layout.railW}px` } as CSSProperties}
      >
        <AppWorkspace
          layout={layout}
          setLayout={setLayout}
          mainRef={mainRef}
          splitRef={splitRef}
          txWidth={txWidth}
          dragSplit={dragSplit}
          dragTimeline={dragTimeline}
          project={project}
          doctor={doctor}
          ops={ops}
          transcripts={transcripts}
          playheadMs={playheadMs}
          selectedClipIds={selectedClipIds}
          exportRange={exportRange}
          clipboardHasContent={clipboardHasContent}
          clipboardClipId={clipboardClipId}
          commentsOpen={commentsOpen}
          focusComment={focusComment}
          generateTab={generateTab}
          onGenerateTab={setGenerateTab}
          onCutWords={onCutWords}
          onRestore={onRestore}
          onSeek={onSeek}
          onSelect={onSelect}
          onExportRange={setExportRange}
          onCopyClip={copyClip}
          onCutClip={cutClip}
          onPasteClip={pasteClip}
          onCollapseComments={() => setCommentsOpen(false)}
          onReopenProject={() => void onProjectSwitched()}
          onLibraryAddedToProject={() => void resync()}
          onRecordClipAdded={() => void resync()}
          onOpenOutputSettings={() => {
            setWizardOpen(false)
            setEnvCategory('general')
            setEnvOpen(true)
          }}
        />

        <AppRightRail
          hidden={layout.workspaceMode === 'library'}
          layout={layout}
          setLayout={setLayout}
          dragRail={dragRail}
          project={project}
          doctor={doctor}
          ops={ops}
          receipts={receipts}
          selectedClipId={selectedClipIds[0] ?? null}
          playheadMs={playheadMs}
          agentChatPrefill={agentChatPrefill}
          onReject={onRestore}
          onUndo={onUndo}
          onRedo={onRedo}
        />
      </div>

      <div className="app__status">
        {/* StatusBar: self-subscribes to events for connection + job
            pills; receipts/playhead/selection passed down. */}
        <StatusBar
          project={project}
          receipts={receipts}
          playheadMs={playheadMs}
          selectedClipIds={selectedClipIds}
          opsCount={ops.length}
          doctor={doctor}
          clipboardNotice={clipboardNotice}
          onOpenEnvironment={(category = 'overview') => {
            setWizardOpen(false)
            setEnvCategory(category)
            setEnvOpen(true)
          }}
        />
      </div>
      {/* `?` keyboard-map modal — self-contained, any scope */}
      <KeymapOverlay />

      {/* Cmd-K / Ctrl-K command palette — the agent-first verb launcher.
          Self-contained: owns its open state and the global hotkey. */}
      <CommandPalette />

      {/* Agent-driven element highlight (ui.highlight) — an outline + description
          chip over whatever control the agent is driving (guided demos / debug). */}
      <HighlightOverlay spec={highlight} onClear={() => setHighlight(null)} />

      {/* Desktop drag-drop media import (Tauri only). With no project, the first
          supported file creates one and becomes its timeline. */}
      <DropZone
        project={project}
        onChanged={() => void resync()}
        onProjectCreated={() => void onProjectSwitched()}
      />

      {/* Environment surfaces — first-run wizard (auto on missing essential) and
          Settings>Environment drawer (status-bar chip). Both render the SAME
          EnvCards; both are ui.open-drivable + reported in ui.state. */}
      {wizardOpen && (
        <Suspense fallback={<SurfaceLoading />}>
          <StartWizard report={doctor} onRefresh={() => void refreshDoctor(true)} onClose={() => setWizardOpen(false)} />
        </Suspense>
      )}
      {envOpen && (
        <Suspense fallback={<SurfaceLoading />}>
          <EnvironmentPanel
            report={doctor}
            onRefresh={() => void refreshDoctor(true)}
            onClose={() => setEnvOpen(false)}
            initialCategory={envCategory}
            onOpenRecording={() => {
              setEnvOpen(false)
              setLayout((current) => ({ ...current, workspaceMode: 'record' }))
            }}
          />
        </Suspense>
      )}

      <AppDrawerStack
        activeDrawer={activeDrawer}
        project={project}
        selectedClipId={selectedClipIds[0] ?? null}
        playheadMs={playheadMs}
        onSeek={onSeek}
        onProjectSwitched={onProjectSwitched}
        onClose={closeDrawer}
      />
      <UserActionFeedback />
    </div>
    </OfflineMediaProvider>
  )
}
