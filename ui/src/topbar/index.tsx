// topbar — 54px app top bar:
//   ✕ ShellX CUT · project.cutproj      [jobs chip]      [Render] [Export]
// Role: brand mark (the canonical traced ShellX X — shared with
// branding/shellx-cut-icon.svg — in ink with the editor-blue playhead
// strike, 20px inline SVG per ), project identity, a live jobs chip
// (seeded via jobs.list so a tab opened mid-render still shows it; folded
// from WS job_progress/render_done after), and the two primary actions:
// Render → render.final{preset, profile?}, Export ▾ → export.xml{format} /
// export.srt. Render carries two small pickers: preset (quality tier,
// default standard) and footage profile (default auto = omit the arg — the
// server default applies and auto-detect PROPOSES in the receipt).
// Both buttons dispatch verbs and report ONLY what the envelope returned
// (transient mono note; no toasts — receipts stay first-class in the rail).
// Callers: App.tsx. Deps: lib/client (verbs), lib/events (jobs), topbar.css.

import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb, type PregateReport, type Project, type StoryboardResult } from '../lib/client'
import { envHealthLevel, isFfmpegMissing, type DoctorReport } from '../lib/doctor'
import type { WorkspaceMode } from '../layout/useLayout'
import { FPS_PRESETS, RES_PRESETS, resKey } from '../lib/formatPresets'
import { applyExportOutputDir, folderTail, getStoredOutputDir, setStoredOutputDir, withAuthorizedOutputPath } from '../lib/exportDestination'
import { openCutManual } from '../lib/manual'
import { activeJobLabel, activeJobProgress } from '../lib/jobPresentation'
import {
  openVideoToolsGuide,
  openVideoToolsSettings,
  recheckVideoTools,
} from '../lib/videoToolsSetup'
import { isTauri, pickExportOutput, pickFolder, pickOtio } from '../lib/tauri'
import DirectorModal from '../director/DirectorModal'
import RenderQueueModal from './RenderQueueModal'
import OtioImportModal, { type OtioImportPreview } from './OtioImportModal'
import { BrandMark, Icon } from '../icons'
import ThemeToggle from '../components/ThemeToggle'
import UpdateButton from './UpdateButton'
import StoryboardOverlay from './StoryboardOverlay'
import PreflightWarning from './PreflightWarning'
import SequenceSwitcher from './SequenceSwitcher'
import { useTopbarDismissibleMenu } from './useTopbarDismissibleMenu'
import { useTopbarJobs } from './useTopbarJobs'
import {
  ASPECTS,
  ASYNC_RENDER_IDS,
  EXPORT_GROUPS,
  EXPORT_OPTIONS,
  FORMAT_LABELS,
  FORMATS,
  LOUDNESS,
  LOUDNESS_LABELS,
  PRESETS,
  PROFILES,
  REFRAME_PRESETS,
  WORKSPACE_MODES,
  selectedOption,
  type Aspect,
  type FileFormat,
  type Loudness,
  type Preset,
  type Profile,
  type ReframePreset,
} from './model'
import './topbar.css'

export interface TopBarProps {
  project: Project | null
  /** Open the music-bed drawer (drives audio.add_music). */
  onOpenMusic?: () => void
  /** Open the audio mixer drawer (level via edit.gain, mute/solo via flags). */
  onOpenMixer?: () => void
  onOpenProjects?: () => void
  onOpenLibrary?: () => void
  onOpenClips?: () => void
  onOpenAutopilot?: () => void
  onOpenAssemble?: () => void
  /** open the Recipes drawer (recipe.list / describe / run). */
  onOpenRecipes?: () => void
  /** Q2: open the Region-mask drawer (drives edit.add_mask — on-canvas blur/pixelate/black). */
  onOpenMask?: () => void
  /** 0.5.0: open the title drawer (drives title.add). */
  onOpenTitle?: () => void
  /** toggle the left review-comment rail (`]`). */
  onToggleComments?: () => void
  /** Whether the comment rail is currently open (drives the pressed state). */
  commentsOpen?: boolean
  /** Open review comment count (badge on the Review button). */
  openCommentCount?: number
  /** Re-fetch project state after project.create (which emits no op_applied). */
  onProjectChanged?: () => void
  /** Reset clip-local editor context after activating a different sequence. */
  onSequenceChanged?: () => void
  /** Live playhead (ms) — the position the Export-menu "Still frame" extracts. */
  playheadMs?: number
  /** Active workspace mode and setter. */
  mode?: WorkspaceMode
  onMode?: (m: WorkspaceMode) => void
  /** Open the Setup & tools hub (Settings>Environment panel) — the always-on
   *  home for installing ffmpeg / perception / background removal + version &
   *  about. Same handler as the status-bar env chip. */
  onOpenSetup?: () => void
  /** Environment doctor report — drives the Setup button's health nudge dot
   *  (amber = a degraded card, red = an essential dep missing). */
  doctor?: DoctorReport | null
}

// Timeline composition FORMAT presets — RES_PRESETS / FPS_PRESETS / resKey
// live in lib/formatPresets. New projects auto-adopt the first video; these are
// expert corrections, not quality choices required during project creation.

export default function TopBar({ project, onOpenMusic, onOpenMixer, onOpenProjects, onOpenLibrary, onOpenClips, onOpenAutopilot, onOpenAssemble, onOpenRecipes, onOpenMask, onOpenTitle, onToggleComments, commentsOpen, openCommentCount = 0, onProjectChanged, onSequenceChanged, playheadMs = 0, mode = 'edit', onMode, onOpenSetup, doctor = null }: TopBarProps) {
  // New project + Import moved OUT of the topbar: create lives in the
  // Projects left-tab (panels/Projects), import lives in the Assets tray + Library.

  // Click-to-rename the project name in the title (project.rename). The op
  // refreshes the project snapshot, so the title updates from server truth.
  const [renaming, setRenaming] = useState(false)
  const [renameVal, setRenameVal] = useState('')
  const commitRename = async () => {
    const name = renameVal.trim()
    setRenaming(false)
    if (!name || name === project?.name) return
    const r = await callVerb('project.rename', { name, rationale: `rename project to "${name}"` })
    if (r.ok) onProjectChanged?.()
  }

  // Timeline output FORMAT (project.format): resolution + frame rate. Lowering
  // them can make renders + proxies faster on heavy footage.
  const setResolution = async (label: string) => {
    const r = RES_PRESETS.find((x) => x.label === label)
    if (!r) return
    const res = await callVerb('project.format', { width: r.w, height: r.h, rationale: `timeline resolution → ${label}` })
    if (res.ok) { setNote(`timeline ${r.w}×${r.h}`); onProjectChanged?.() }
  }
  const setFps = async (fps: number) => {
    const res = await callVerb('project.format', { fps, rationale: `timeline frame rate → ${fps}fps` })
    if (res.ok) { setNote(`timeline ${fps} fps`); onProjectChanged?.() }
  }

  const { jobList, renderRunning } = useTopbarJobs()
  const jobsChipTitle = jobList.length === 0
    ? 'no running jobs'
    : [
        ...jobList.slice(0, 4).map((j) => `${activeJobLabel(j.kind)} · ${activeJobProgress(j)} (${j.job_id})`),
        ...(jobList.length > 4 ? [`+${jobList.length - 4} more running jobs`] : []),
      ].join('\n')
  const [note, setNote] = useState<string | null>(null) // transient verb feedback
  const [menuOpen, setMenuOpen] = useState(false)
  // Chosen export destination folder. Persisted in localStorage so it
  // sticks across sessions; re-asserted to the (session-global) server in
  // onExport, so it survives a cutd restart. null = default <project>/exports.
  const [outputDir, setOutputDir] = useState<string | null>(() => {
    return getStoredOutputDir()
  })
  const [renderOptsOpen, setRenderOptsOpen] = useState(false)
  const [preset, setPreset] = useState<Preset>('standard')
  const [profile, setProfile] = useState<Profile>('auto')
  const [aspect, setAspect] = useState<Aspect>('project')
  const [reframePreset, setReframePreset] = useState<ReframePreset>('talking_head')
  // render.final output file format (codec/container) + GPU-encoder toggle.
  // Defaults match the server (h264, hardware auto). Only sent on the render.final
  // branch — render.reframe takes neither arg, so its branch stays untouched.
  const [fileFormat, setFileFormat] = useState<FileFormat>('h264')
  const [useGpu, setUseGpu] = useState(true)
  // Loudness normalization target (render.final{normalize_loudness}); 'off' = omit.
  const [loudness, setLoudness] = useState<Loudness>('off')
  const [directorOpen, setDirectorOpen] = useState(false)
  // Batch-delivery (render.queue) surface — opened from the Export menu.
  const [queueOpen, setQueueOpen] = useState(false)
  const [otioPreview, setOtioPreview] = useState<OtioImportPreview | null>(null)
  const [otioBusy, setOtioBusy] = useState(false)
  const [otioError, setOtioError] = useState<string | null>(null)
  // Storyboard ("see the whole edit at a glance") overlay state. Display-only:
  // render.storyboard creates no op (zero-local-mutation contract) — this is a pure view affordance.
  const [sbOpen, setSbOpen] = useState(false)
  const [sbBusy, setSbBusy] = useState(false)
  const [sbResult, setSbResult] = useState<StoryboardResult | null>(null)
  const [sbError, setSbError] = useState<string | null>(null)
  const [preflight, setPreflight] = useState<{ report: PregateReport; actionLabel: string } | null>(null)
  const noteTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const pendingPreflight = useRef<(() => Promise<void>) | null>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const renderRef = useRef<HTMLDivElement>(null)
  const ffmpegMissing = isFfmpegMissing(doctor)

  useTopbarDismissibleMenu(menuRef, menuOpen, setMenuOpen)

  useEffect(() => {
    const onExportDir = (e: Event) => setOutputDir((e as CustomEvent<string | null>).detail ?? null)
    window.addEventListener('cut:export-output-dir', onExportDir)
    return () => window.removeEventListener('cut:export-output-dir', onExportDir)
  }, [])

  useTopbarDismissibleMenu(renderRef, renderOptsOpen, setRenderOptsOpen)

  /** Show a transient (5s) mono note next to the actions. */
  const flash = useCallback((text: string) => {
    setNote(text)
    if (noteTimer.current) clearTimeout(noteTimer.current)
    noteTimer.current = setTimeout(() => setNote(null), 5000)
  }, [])
  const openVideoToolsSetup = openVideoToolsSettings
  const exportNeedsFfmpeg = (id: string) =>
    id === 'video' || id === 'audio' || id === 'gif' || id.startsWith('pub_')
  const blockMissingFfmpeg = (action: string) => {
    flash(`Install FFmpeg before ${action}.`)
    return true
  }
  const openPreflight = (report: PregateReport, actionLabel: string, action: () => Promise<void>) => {
    pendingPreflight.current = action
    setPreflight({ report: { ...report, risks: report.risks ?? [] }, actionLabel })
  }
  const clearPreflight = () => {
    pendingPreflight.current = null
    setPreflight(null)
  }
  const continuePreflight = async () => {
    const action = pendingPreflight.current
    clearPreflight()
    if (action) await action()
  }
  const runVideoPreflight = async (actionLabel: string, action: () => Promise<void>) => {
    if (ffmpegMissing && blockMissingFfmpeg(actionLabel)) return
    try {
      const r = await callVerb('verify.pregate', {})
      if (r.ok && r.result) {
        const report = r.result
        const hasRisks = (report.risks ?? []).length > 0
        const hasUncheckedAssets = (report.uninstrumented_assets ?? []).length > 0
        if (report.pass === false || hasRisks || hasUncheckedAssets) {
          openPreflight(report, actionLabel, action)
          return
        }
      } else if (!r.ok) {
        flash(`preflight unavailable: ${r.error?.message ?? r.error?.code ?? 'continuing'}`)
      }
    } catch {
      flash('preflight unavailable; continuing')
    }
    await action()
  }

  const onRender = async (e: React.MouseEvent<HTMLButtonElement>) => {
    e.currentTarget.blur() // keep Space as the global play/pause key
    const startRender = async () => {
      try {
        // A non-project Format = SUBJECT-AWARE reframe (render.reframe): detect+track
        // the subject and follow it with a moving crop, project untouched. Otherwise
        // a normal full render (render.final). 'auto' profile omits the arg.
        const r =
          aspect !== 'project'
            ? await callVerb('render.reframe', {
                aspect,
                preset: reframePreset,
                rationale: `reframe to ${aspect} (${reframePreset})`,
              })
            : await callVerb('render.final', {
                preset,
                format: fileFormat,
                hardware: useGpu ? 'auto' : 'off',
                ...(profile !== 'auto' ? { profile } : {}),
                ...(loudness !== 'off' ? { normalize_loudness: Number(loudness) } : {}),
              })
        if (r.ok) {
          const jobId = (r.result as { job_id?: string })?.job_id
          flash(
            jobId
              ? `${aspect !== 'project' ? `reframe → ${aspect} · ` : 'render · '}${jobId}`
              : aspect !== 'project' ? 'reframe started' : 'render started',
          )
        } else {
          // Prefer the engine's message (e.g. "timeline is empty") over the bare
          // code — it tells the user what to actually do.
          flash(`${aspect !== 'project' ? 'reframe' : 'render'} failed: ${r.error?.message ?? r.error?.code ?? 'unknown'}`)
        }
      } catch {
        flash(`${aspect !== 'project' ? 'reframe' : 'render'} failed: server unreachable`)
      }
    }
    await runVideoPreflight(aspect !== 'project' ? 'reframing video' : 'rendering video', startRender)
  }

  // Push the chosen destination folder to the session-global engine.
  // Returns ok. Empty arg clears it server-side (back to <project>/exports).
  const applyOutputDir = async (dir: string | null): Promise<boolean> => {
    return applyExportOutputDir(dir)
  }
  // "Choose folder…" — native OS folder picker (desktop only) → set + remember.
  const chooseFolder = async () => {
    setMenuOpen(false)
    if (!isTauri()) { flash('Folder picker needs the desktop app'); return }
    const dir = await pickFolder()
    if (!dir) return
    if (await applyOutputDir(dir)) {
      setOutputDir(dir)
      setStoredOutputDir(dir)
      flash(`Exports → ${dir}`)
    } else {
      flash('Could not use that folder')
    }
  }
  // Revert to the default project/exports folder.
  const clearFolder = async () => {
    setMenuOpen(false)
    await applyOutputDir(null)
    setOutputDir(null)
    setStoredOutputDir(null)
    flash('Exports → project folder')
  }

  // Read-only preflight first. The modal binds confirmation to source_hash, so a
  // file changed between inspection and replacement is refused by the server.
  const importOtio = useCallback(async () => {
    setMenuOpen(false)
    if (!isTauri()) { flash('Importing an .otio needs the desktop app (it reads a file path)'); return }
    const path = await pickOtio()
    if (!path) return
    setOtioError(null)
    try {
      const r = await callVerb('import.otio', { path, mode: 'preview' })
      if (!r.ok) { flash(`import preview failed: ${r.error?.message ?? r.error?.code ?? 'unknown'}`); return }
      setOtioPreview(r.result as OtioImportPreview)
    } catch {
      flash('import preview failed: server unreachable')
    }
  }, [flash])

  useEffect(() => {
    const onImportOtio = () => void importOtio()
    document.addEventListener('cut:import-otio', onImportOtio)
    return () => document.removeEventListener('cut:import-otio', onImportOtio)
  }, [importOtio])

  const confirmOtio = async () => {
    if (!otioPreview || otioBusy) return
    setOtioBusy(true)
    setOtioError(null)
    try {
      const r = await callVerb('import.otio', {
        path: otioPreview.path,
        mode: 'replace',
        expected_hash: otioPreview.source_hash,
        rationale: 'confirmed OTIO preview in the desktop app',
      })
      if (!r.ok) {
        setOtioError([r.error?.message, r.error?.suggested_action].filter(Boolean).join(' · ') || 'Import failed')
        return
      }
      const res = r.result as { tracks_created?: number; clips_inserted?: number; missing_clips?: number }
      setOtioPreview(null)
      flash(`Imported timeline — ${res?.clips_inserted ?? 0} clips on ${res?.tracks_created ?? 0} tracks${res?.missing_clips ? ` · ${res.missing_clips} offline` : ''}`)
      onProjectChanged?.()
      document.dispatchEvent(new CustomEvent('cut:show-composed'))
    } catch {
      setOtioError('Import failed: server unreachable')
    } finally {
      setOtioBusy(false)
    }
  }

  const onExport = async (opt: (typeof EXPORT_OPTIONS)[number], explicitPath?: string) => {
    setMenuOpen(false)
    const startExport = async () => {
      try {
        // A native Save As selection temporarily authorizes its parent in the
        // engine's output fence. Default exports re-assert the stored folder.
        // Render-backed entries (platform publishes + the plain Video render)
        // carry the SHARED footage-profile choice (the same `profile` state the
        // Render menu's Footage select drives) so a silent screen-demo export
        // stops failing caption/loudness receipt checks. 'auto' maps to
        // undefined = omit the arg (engine default), exactly like onRender.
        const r = await withAuthorizedOutputPath(explicitPath, async () =>
          opt.id === 'frame'
            ? callVerb('export.frame', { at_ms: Math.max(0, Math.round(playheadMs ?? 0)), ...(explicitPath ? { path: explicitPath } : {}) })
            : opt.group === 'publish' || opt.id === 'video'
              ? opt.run(explicitPath, profile === 'auto' ? undefined : profile)
              : opt.run(explicitPath))
        if (r.ok) {
          // Async renders ('video' + platform publishes) return a job_id, not a
          // path → tell the user where the finished file + its Download button land.
          if (ASYNC_RENDER_IDS.has(opt.id)) {
            const res = r.result as { job_id?: string; publish?: { label?: string } }
            const jobId = res?.job_id
            // Publish entries echo the platform label (export.publish result.publish).
            const what = res?.publish?.label ? `publishing for ${res.publish.label}` : 'rendering video'
            flash(jobId ? `${what} · ${jobId} — Download from the Review tab when done` : `${what} — see Review tab`)
          } else {
            const path = (r.result as { path?: string })?.path
            flash(path ? `exported → ${path}` : 'exported')
          }
        } else {
          // The engine's message names the fix (e.g. "run captions.generate
          // first") — far more useful than the bare error code.
          flash(`export failed: ${r.error?.message ?? r.error?.code ?? 'unknown'}`)
        }
      } catch (error) {
        flash(`export failed: ${error instanceof Error ? error.message : 'server unreachable'}`)
      }
    }
    if (exportNeedsFfmpeg(opt.id)) {
      await runVideoPreflight(`exporting ${opt.label}`, startExport)
    } else {
      await startExport()
    }
  }

  const onExportSaveAs = async (opt: (typeof EXPORT_OPTIONS)[number]) => {
    if (ffmpegMissing && exportNeedsFfmpeg(opt.id) && blockMissingFfmpeg(`exporting ${opt.label}`)) return
    if (!isTauri()) { flash('Save As needs the desktop app'); return }
    const path = await pickExportOutput({
      title: `Save ${opt.label} — ShellX Cut`,
      defaultPath: opt.defaultPath,
      filters: opt.filters,
    })
    if (!path) return
    await onExport(opt, path)
  }

  // Storyboard: open the overlay immediately (busy state) and ask the engine for
  // a 12-frame inline contact sheet. inline:true → the JPEG comes back as base64
  // in the envelope, so the overlay renders it directly (no extra /api fetch).
  // Pure view — no op is created (zero-local-mutation contract); a verb error (e.g. empty timeline)
  // is surfaced honestly in the overlay, never thrown away.
  const onStoryboard = async (e: React.MouseEvent<HTMLButtonElement>) => {
    e.currentTarget.blur() // keep Space as the global play/pause key
    if (sbBusy) return
    setSbOpen(true)
    setSbBusy(true)
    setSbResult(null)
    setSbError(null)
    try {
      const r = await callVerb('render.storyboard', { count: 12, inline: true })
      if (r.ok && r.result) {
        setSbResult(r.result)
      } else {
        // The engine's message is the most useful thing we can show ("timeline
        // is empty", …); fall back to the code, then a generic line.
        setSbError(r.error?.message ?? r.error?.code ?? 'storyboard failed')
      }
    } catch {
      setSbError('server unreachable')
    } finally {
      setSbBusy(false)
    }
  }

  const closeStoryboard = () => {
    setSbOpen(false)
    // Drop the (potentially large) base64 sheet so it isn't retained while closed.
    setSbResult(null)
    setSbError(null)
  }

  return (
    <header className="tb" data-panel="topbar" data-cut-panel="topbar">
      <div className="tb-brand">
        <BrandMark />
        {/* Brand line: the fixed "ShellX CUT" wordmark + the (truncating) project
            name. .tb-title is a flex row so the wordmark stays put (flex:none) and
            ONLY the project name yields/ellipsizes under width pressure — it can
            never paint over the mode tabs (the FHD overlap bug). */}
        <span className="tb-title">
          <span className="tb-wordmark">ShellX CUT</span>
          {project ? (
            renaming ? (
              <span className="tb-proj tb-proj--editing">
                · <input
                  className="tb-proj-input"
                  data-cut-project-rename
                  autoFocus
                  value={renameVal}
                  onChange={(e) => setRenameVal(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') void commitRename()
                    else if (e.key === 'Escape') setRenaming(false)
                  }}
                  onBlur={() => void commitRename()}
                />.cutproj
              </span>
            ) : (
              <button
                type="button"
                className="tb-proj tb-proj--btn"
                data-cut-project
                title={`${project.name}.cutproj — click to rename`}
                onClick={() => { setRenameVal(project.name); setRenaming(true) }}
              >
                · {project.name}.cutproj
              </button>
            )
          ) : (
            <span className="tb-proj tb-proj--none" data-cut-project>· no project</span>
          )}
        </span>
      </div>

      {project && (
        <SequenceSwitcher
          project={project}
          onProjectChanged={onProjectChanged}
          onSequenceChanged={onSequenceChanged}
        />
      )}

      {/* Workspace mode switch (Edit, Record, Color, Audio,
          Export). Swaps the layout while the project persists; Record is the
          flagship capture surface. */}
      <div className="tb-modes" role="tablist" aria-label="Workspace mode" data-cut-modes={mode}>
        {WORKSPACE_MODES.map((m) => (
          <button
            key={m.id}
            type="button"
            role="tab"
            aria-selected={mode === m.id}
            className={`tb-mode${mode === m.id ? ' tb-mode--on' : ''}${m.id === 'record' ? ' tb-mode--record' : ''}`}
            data-cut-mode={m.id}
            title={m.hint}
            onClick={(e) => { e.currentTarget.blur(); onMode?.(m.id) }}
          >
            {m.id === 'record' && <span className="tb-mode-dot" aria-hidden="true" />}
            {m.label}
          </button>
        ))}
      </div>

      {/* Primary nav (top-left menu position): Projects + Library. "New"
          and "Import" were removed from the header — New lives in the Projects tab,
          Import lives in the Assets tray + Library. Projects opens the LEFT-SIDEBAR
          Projects tab; Library opens the dedicated Library workspace. */}
      <button
        className="tb-btn tb-btn--secondary tb-nav"
        data-cut-projects-btn
        aria-label="Projects"
        title="Projects — browse, reopen, or create a project (opens the left sidebar)"
        onClick={(e) => { e.currentTarget.blur(); onOpenProjects?.() }}
      >
        <Icon name="projectOpen" size={16} tone="brand" /> <span className="tb-nav-label">Projects</span>
      </button>
      <button
        className={`tb-btn tb-btn--secondary tb-nav${mode === 'library' ? ' tb-nav--active' : ''}`}
        data-cut-library-btn
        aria-label="Library"
        aria-pressed={mode === 'library'}
        title={mode === 'library'
          ? 'Return to Edit'
          : 'Library — browse reusable media across every project'}
        onClick={(e) => { e.currentTarget.blur(); onOpenLibrary?.() }}
      >
        <Icon name="library" size={16} tone="asset" /> <span className="tb-nav-label">Library</span>
      </button>
      {/* Settings hub (discoverability) — opens the Settings/Environment panel:
          export folder, recording destinations, installable tools, captions,
          services, render checks, appearance, version, and about. This is the
          always-on topbar entry point; the status-bar export-folder chip opens
          the same panel. A health DOT nudges proactively: amber when a card is
          degraded, red when an essential dep (ffmpeg) is missing. */}
      {(() => {
        const lvl = envHealthLevel(doctor)
        const dotColor =
          lvl === 'missing' ? 'var(--err)'
          : lvl === 'degraded' ? 'var(--amber)'
          : lvl === 'ok' ? 'var(--green)'
          : 'var(--ink-4)'
        const nudge = lvl === 'missing' || lvl === 'degraded'
        return (
          <button
            className="tb-btn tb-btn--secondary tb-nav"
            data-cut-setup-btn
            data-cut-settings-btn
            data-cut-setup-health={lvl}
            aria-label="Settings"
            title="Settings — export folder, recordings, installable tools, captions, app info"
            onClick={() => onOpenSetup?.()}
          >
            <Icon name="settings" size={16} tone="brand" /> <span className="tb-nav-label">Settings</span>
            <span
              className="tb-nav-status"
              data-cut-setup-dot={lvl}
              aria-hidden="true"
              style={{
                width: 7,
                height: 7,
                borderRadius: '50%',
                flex: 'none',
                marginLeft: 1,
                background: dotColor,
                boxShadow: nudge ? `0 0 6px ${dotColor}` : 'none',
              }}
            />
          </button>
        )
      })()}

      <button
        className="tb-btn tb-btn--secondary tb-nav"
        data-cut-manual-link
        aria-label="Manual"
        title="Manual — open the current ShellX Cut documentation"
        onClick={(e) => { e.currentTarget.blur(); openCutManual() }}
      >
        <Icon name="manual" size={16} tone="brand" /> <span className="tb-nav-label">Manual</span>
      </button>

      {/* Colour-theme switch (light/dark) — a compact icon toggle; the full
          labelled control also lives in the Setup/Environment hub. */}
      <ThemeToggle variant="icon" />

      <span className="tb-spacer" />

      {/* Quiet update nudge — renders ONLY while the desktop shell reports an
          available signed release (launch / 6-hourly / manual check). Full
          status + controls live in Settings > About; browser builds never
          render it (isTauri() false inside the component). */}
      <UpdateButton />

      {note && (
        <span className="tb-note" data-cut-topbar-note>
          {note}
        </span>
      )}

      {ffmpegMissing && (
        <span className="tb-ffmpeg-setup" data-cut-export-ffmpeg-setup role="status" aria-live="polite">
          <span className="tb-ffmpeg-label">FFmpeg needed</span>
          <button type="button" data-cut-export-install-ffmpeg onClick={openVideoToolsSetup} title="Open Settings and install video processing">
            Install
          </button>
          <button type="button" data-cut-export-ffmpeg-guide onClick={openVideoToolsGuide} title="Open the FFmpeg setup guide">
            Guide
          </button>
          <button type="button" data-cut-export-ffmpeg-recheck onClick={recheckVideoTools} title="Re-check video tools after installing FFmpeg">
            Re-check
          </button>
        </span>
      )}

      {/* jobs chip — quiet when idle, orange pulse + busiest job while running */}
      <span
        className={`tb-jobs ${jobList.length ? 'tb-jobs--active' : ''}`}
        data-cut-jobs-chip={jobList.length}
        title={jobsChipTitle}
      >
        <span className="tb-jobs-dot" />
        {jobList.length === 0
          ? 'idle'
          : jobList.length === 1
            ? `${activeJobLabel(jobList[0].kind)} · ${activeJobProgress(jobList[0])}`
            : `${jobList.length} jobs ${Math.round((jobList.reduce((s, j) => s + j.progress, 0) / jobList.length) * 100)}%`}
      </span>

      {/* ── tool launchers: icon-only + tooltip, GROUPED (project/assets | creative
          | review). Iconized so the bar stays uncrowded; data-cut hooks unchanged
          so the agent + e2e still drive each by name. */}
      <div className="tb-tools" data-cut-toolbar>
        <button
          className="tb-iconbtn"
          data-cut-title-btn
          aria-label="Title"
          disabled={!project}
          title="Add an animated lower-third or intro card"
          onClick={(e) => { e.currentTarget.blur(); onOpenTitle?.() }}
        ><Icon name="text" size={18} tone="brand" label="Title" /></button>
        <button
          className="tb-iconbtn"
          data-cut-shape-btn
          aria-label="Shape"
          disabled={!project}
          title="Add a rectangle, ellipse, line, arrow, or callout"
          onClick={(e) => { e.currentTarget.blur(); document.dispatchEvent(new CustomEvent('cut:open-shape')) }}
        ><Icon name="shape" size={18} tone="brand" label="Shape" /></button>
        <button
          className="tb-iconbtn"
          data-cut-mask-btn
          aria-label="Region mask"
          disabled={!project}
          title="Region mask — blur, pixelate, or cover an area of a selected clip"
          onClick={(e) => { e.currentTarget.blur(); onOpenMask?.() }}
        ><Icon name="mask" size={18} tone="brand" label="Region mask" /></button>
        <button
          className="tb-iconbtn"
          data-cut-music-btn
          aria-label="Music bed"
          disabled={!project}
          title="Add music under the edit and lower it beneath speech"
          onClick={(e) => { e.currentTarget.blur(); onOpenMusic?.() }}
        ><Icon name="music" size={18} tone="audio" label="Music bed" /></button>
        <button
          className="tb-iconbtn"
          data-cut-mixer-btn
          aria-label="Audio mixer"
          disabled={!project}
          title="Mix each track's level, pan, mute, and solo"
          onClick={(e) => { e.currentTarget.blur(); onOpenMixer?.() }}
        ><Icon name="mixer" size={18} tone="audio" label="Audio mixer" /></button>
        <button
          className="tb-iconbtn"
          data-cut-clips-btn
          aria-label="Repurpose into shorts"
          disabled={!project}
          title="Repurpose into shorts — best moments → render and validate a package per platform"
          onClick={(e) => { e.currentTarget.blur(); onOpenClips?.() }}
        ><Icon name="split" size={18} tone="brand" label="Repurpose into shorts" /></button>
        <button
          className="tb-iconbtn"
          data-cut-autopilot-btn
          aria-label="Autopilot"
          disabled={!project}
          title="Render, review quality, and apply safe automatic fixes"
          onClick={(e) => { e.currentTarget.blur(); onOpenAutopilot?.() }}
        ><Icon name="autopilot" size={18} tone="brand" label="Autopilot" /></button>
        <button
          className="tb-iconbtn"
          data-cut-recipes-btn
          aria-label="Recipes"
          /* NOT disabled without a project: recipe.list/describe are project-free
             reads, so a user can BROWSE the named workflows (and read each stage)
             before opening a project — the discoverability the recipe layer is for.
             Run/Preview inside the drawer require a project (engine fails fast). */
          title="Recipes — guided workflows for common edits; preview the steps before running"
          onClick={(e) => { e.currentTarget.blur(); onOpenRecipes?.() }}
        ><Icon name="bolt" size={18} tone="brand" label="Recipes" /></button>
        <button
          className="tb-iconbtn"
          data-cut-assemble-btn
          aria-label="Assemble with AI"
          disabled={!project}
          title="Find the best moments, match a script to footage, or fill a slot with b-roll"
          onClick={(e) => { e.currentTarget.blur(); onOpenAssemble?.() }}
        ><Icon name="effect" size={18} tone="brand" label="Assemble (AI)" /></button>
        <button
          className="tb-iconbtn"
          data-cut-storyboard-btn
          disabled={!project || sbBusy}
          aria-label="Storyboard"
          aria-busy={sbBusy}
          title="Create a contact sheet of the whole edit"
          onClick={onStoryboard}
        >{sbBusy ? <span className="tb-spinner" aria-hidden="true" /> : <Icon name="gridDense" size={18} label="Storyboard" />}</button>

        <span className="tb-divider" aria-hidden="true" />

        <button
          className={`tb-iconbtn ${commentsOpen ? 'tb-iconbtn--on' : ''}`}
          data-cut-comments-btn
          aria-pressed={!!commentsOpen}
          aria-label="Review comments"
          title="Review comments — leave a timecoded note, the agent drafts the edit (Ctrl/Cmd+Shift+C)"
          onClick={(e) => { e.currentTarget.blur(); onToggleComments?.() }}
        >
          <Icon name="comment" size={18} label="Review comments" />
          {openCommentCount > 0 && <span className="tb-badge" data-cut-comments-badge>{openCommentCount}</span>}
        </button>
      </div>

      {/* render zone: the primary Render action + a ▾ popover holding the output
          OPTIONS (format / quality / footage). These are settings, so they live in
          a menu — not as bare header selects, which read like
          settings, not header"). */}
      <div className="tb-render" ref={renderRef}>
        {/* GPU acceleration toggle — VISIBLE in the header so users can see
            it), bound to the same `useGpu` state as the render-options checkbox so
            the two stay in sync. ON = hardware encoder (NVENC/QSV/AMF/VideoToolbox)
            when present, ~much faster, probe-verified safe fallback to software;
            OFF = byte-deterministic software encode. Defaults ON. */}
        <button
          type="button"
          className={`tb-btn tb-gpu${useGpu ? ' tb-gpu--on' : ''}`}
          data-cut-gpu-toggle
          data-cut-gpu-on={useGpu || undefined}
          aria-pressed={useGpu}
          title={useGpu ? 'Faster exports are on. Turn off for repeatable software encoding.' : 'Faster exports are off. Turn on to use available video hardware.'}
          onClick={() => setUseGpu((g) => !g)}
        >
          <span className="tb-gpu-dot" aria-hidden="true" />
          <span className="tb-gpu-label">Faster {useGpu ? 'ON' : 'OFF'}</span>
        </button>
        <button
          className="tb-btn tb-btn--primary tb-render-go"
          data-cut-render-btn
          disabled={!project || renderRunning}
          title={renderRunning ? 'A render is already running' : 'Render the current timeline'}
          onClick={onRender}
        >
          {renderRunning ? 'Rendering…' : 'Render'}
        </button>
        <button
          className="tb-btn tb-btn--primary tb-render-opts"
          data-cut-render-opts
          disabled={!project || renderRunning}
          aria-expanded={renderOptsOpen}
          title="Render options — format, quality, footage profile"
          onClick={(e) => {
            e.currentTarget.blur()
            setMenuOpen(false)
            setRenderOptsOpen((o) => !o)
          }}
        >
          <Icon name="chevronDown" size={14} className="tb-caret" />
        </button>
        {renderOptsOpen && (
          <div className="tb-menu tb-render-menu" data-cut-render-menu role="menu">
            <p className="tb-render-note">
              This render uses the timeline size and frame rate. Aspect / reframe
              can create another output shape without changing the edit.
            </p>
            <label className="tb-render-field">
              <span>Aspect / reframe</span>
              <select
                className="tb-sel"
                data-cut-render-aspect
                value={aspect}
                title="Subject-aware reframe of THIS render to a new format — follows the subject with a moving crop; project geometry untouched."
                onChange={(e) => setAspect(selectedOption(ASPECTS, e.target.value, aspect))}
              >
                {ASPECTS.map((a) => (
                  <option key={a} value={a}>{a === 'project' ? 'Project size' : a === '9:16' ? '9:16 vertical' : a === '4:5' ? '4:5 portrait' : a === '1:1' ? '1:1 square' : '16:9 wide'}</option>
                ))}
              </select>
            </label>
            <label className="tb-render-field">
              <span>Quality</span>
              <select
                className="tb-sel"
                data-cut-render-preset
                value={preset}
                title="Quality tier"
                onChange={(e) => setPreset(selectedOption(PRESETS, e.target.value, preset))}
              >
                {PRESETS.map((p) => (
                  <option key={p} value={p}>{p === 'draft' ? 'Draft' : p === 'high' ? 'High' : 'Standard'}</option>
                ))}
              </select>
            </label>
            <label className="tb-render-field">
              <span>File format</span>
              <select
                className="tb-sel"
                data-cut-render-format
                value={fileFormat}
                title="Output file format (codec + container): H.264 universal mp4, HEVC smaller, WebM/VP9 web, ProRes pro .mov, AV1 best quality (software-slow on CPU — turn GPU on for speed)."
                onChange={(e) => setFileFormat(selectedOption(FORMATS, e.target.value, fileFormat))}
              >
                {FORMATS.map((f) => (
                  <option key={f} value={f}>{FORMAT_LABELS[f]}</option>
                ))}
              </select>
            </label>
            <label className="tb-render-field tb-render-field--check">
              <span>Use GPU when available</span>
              <input
                type="checkbox"
                data-cut-render-gpu
                checked={useGpu}
                title="Use the GPU/hardware encoder (NVENC/QSV/AMF/VideoToolbox) when present — much faster, with a probe-verified safe fallback to software. Uncheck to force byte-deterministic software encoding."
                onChange={(e) => setUseGpu(e.target.checked)}
              />
            </label>
            <label className="tb-render-field">
              <span>Footage</span>
              <select
                className="tb-sel"
                data-cut-render-profile
                value={profile}
                title="Footage profile for the check battery (auto = server default + auto-detect proposal)"
                onChange={(e) => setProfile(selectedOption(PROFILES, e.target.value, profile))}
              >
                {PROFILES.map((p) => (
                  <option key={p} value={p}>{p === 'silent_screen_demo' ? 'Screen demo' : p === 'talking_head' ? 'Talking head' : 'Auto-detect'}</option>
                ))}
              </select>
            </label>
            <label className="tb-render-field">
              <span>Loudness</span>
              <select
                className="tb-sel"
                data-cut-render-loudness
                value={loudness}
                title="Normalize the published audio to a target integrated loudness (LUFS). The render verifies it with the lufs receipt check. -14 social, -16 podcast, -23 broadcast. Off = no normalization."
                onChange={(e) => setLoudness(selectedOption(LOUDNESS, e.target.value, loudness))}
              >
                {LOUDNESS.map((l) => (
                  <option key={l} value={l}>{LOUDNESS_LABELS[l]}</option>
                ))}
              </select>
            </label>
            {/* Timeline composition format is an expert correction for mixed media
                or a required standard. New projects adopt the first video, so keep
                this project-wide mutation out of the normal delivery scan path. */}
            <details className="tb-render-timeline" data-cut-project-format-settings>
              <summary data-cut-project-format-toggle>
                <span>Advanced · timeline format</span>
                <small>
                  {project?.settings?.width}×{project?.settings?.height}
                  {' · '}{Math.round(project?.settings?.fps ?? 30)} fps
                </small>
              </summary>
              <div className="tb-render-timeline__body">
                <p className="tb-render-note">
                  Affects the whole editing canvas and frame timing. The first video
                  sets this automatically; change it only for a deliberate conform.
                </p>
                <label className="tb-render-field">
                  <span>Timeline size</span>
                  <select
                    className="tb-sel"
                    data-cut-project-resolution
                    value={resKey(project?.settings)}
                    disabled={!project}
                    title="Editing canvas size. The first video sets this automatically; change it only to reframe the whole timeline or meet a required standard."
                    onChange={(e) => setResolution(e.target.value)}
                  >
                    {RES_PRESETS.map((r) => (
                      <option key={r.label} value={r.label}>{r.label}</option>
                    ))}
                    {resKey(project?.settings) === 'custom' && (
                      <option value="custom" disabled>
                        {project?.settings?.width}×{project?.settings?.height} (custom)
                      </option>
                    )}
                  </select>
                </label>
                <label className="tb-render-field">
                  <span>Timeline frame rate</span>
                  <select
                    className="tb-sel"
                    data-cut-project-fps
                    value={String(Math.round(project?.settings?.fps ?? 30))}
                    disabled={!project}
                    title="How many timeline frames make one second. This is timing, not a quality slider; normally keep the rate adopted from the first video."
                    onChange={(e) => setFps(Number(e.target.value))}
                  >
                    {FPS_PRESETS.map((f) => (
                      <option key={f} value={String(f)}>{f} fps</option>
                    ))}
                  </select>
                </label>
              </div>
            </details>
            {aspect !== 'project' && (
              <>
                <label className="tb-render-field">
                  <span>Subject</span>
                  <select
                    className="tb-sel"
                    data-cut-reframe-preset
                      value={reframePreset}
                      title="Which subject to follow + the crop motion limits"
                      onChange={(e) => setReframePreset(selectedOption(REFRAME_PRESETS, e.target.value, reframePreset))}
                    >
                    {REFRAME_PRESETS.map((p) => (
                      <option key={p} value={p}>
                        {p === 'talking_head' ? 'Talking head' : p === 'general' ? 'General' : p[0].toUpperCase() + p.slice(1)}
                      </option>
                    ))}
                  </select>
                </label>
                <p className="tb-render-note">Subject-aware reframe: detects + follows the subject with a smoothed moving crop (not a static centre-crop). It is a LOSSY crop — the receipt reports how often the subject stayed in frame. Project geometry untouched.</p>
                <button
                  type="button"
                  className="tb-direct-btn"
                  data-cut-director-open
                  disabled={!project}
                  title="Direct the reframe: review each scene's subjects, pick who the shot follows, render, then QC — the foundation-model director loop"
                  onClick={() => { setRenderOptsOpen(false); setDirectorOpen(true) }}
                >
                  🎬 Direct… <span className="tb-direct-tag">choose the subject per scene</span>
                </button>
              </>
            )}
          </div>
        )}
      </div>

      <div className="tb-export" ref={menuRef}>
        <button
          className="tb-btn tb-btn--secondary"
          data-cut-export-btn
          disabled={!project}
          aria-expanded={menuOpen}
          title="Export a finished file, captions, audio, or timeline interchange"
          onClick={(e) => {
            e.currentTarget.blur()
            setMenuOpen((o) => !o)
          }}
        >
          Export <Icon name="chevronDown" size={14} className="tb-caret" />
        </button>
        {menuOpen && (
          <div className="tb-menu" data-cut-export-menu role="menu">
            {/* BATCH DELIVERY (render.queue) — stack ≥2 deliveries (each its own
                output + quality + format) and render them sequentially. Opens the
                queue modal rather than firing a single verb, so it lives here as its
                own entry, above the one-shot export options. */}
            <div className="tb-menu-dest" data-cut-render-queue-section>
              <button role="menuitem" data-cut-render-queue-open onClick={() => { setMenuOpen(false); setQueueOpen(true) }}>
                ⧉ Render queue / batch deliver…
              </button>
            </div>
            {/* Destination folder — exports drop here, not just the
                default project folder. Native picker on desktop. */}
            <div className="tb-menu-dest" data-cut-export-dest>
              <span className="tb-menu-dest-label" data-cut-output-dir={outputDir ?? ''}>
                Default export folder: <b title={outputDir ?? undefined}>{outputDir ? folderTail(outputDir) : 'project exports'}</b>
              </span>
              <button role="menuitem" data-cut-export-choose-folder onClick={() => void chooseFolder()}>
                Choose default export folder…
              </button>
              {outputDir && (
                <button role="menuitem" data-cut-export-clear-folder onClick={() => void clearFolder()}>
                  Use project exports folder
                </button>
              )}
            </div>
            {/* Footage QC profile — the SAME `profile` state as the Render
                menu's Footage select (one choice, visible in both menus).
                The render-backed entries (platform publishes + the Video
                render) forward it — export.publish{profile} / render.final
                {profile} — so the receipt's check battery matches the footage
                (silent screen demo ≠ talking head). 'auto' = omit the arg,
                engine default. */}
            <div className="tb-menu-dest" data-cut-export-profile-section>
              <label className="tb-render-field">
                <span>Footage</span>
                <select
                  className="tb-sel"
                  data-cut-export-profile
                  value={profile}
                  title="Footage profile for the publish check battery (auto = server default + auto-detect proposal). Shared with the Render menu — screen demo waives caption/loudness checks that don't apply to silent footage."
                  onChange={(e) => setProfile(selectedOption(PROFILES, e.target.value, profile))}
                >
                  {PROFILES.map((p) => (
                    <option key={p} value={p}>{p === 'silent_screen_demo' ? 'Screen demo' : p === 'talking_head' ? 'Talking head' : 'Auto-detect'}</option>
                  ))}
                </select>
              </label>
            </div>
            {EXPORT_GROUPS.map((group) => (
              <div
                className="tb-export-group"
                data-cut-export-group={group.id}
                key={group.id}
                role="group"
                aria-labelledby={`cut-export-group-${group.id}`}
              >
                <span className="tb-export-group-label" id={`cut-export-group-${group.id}`}>{group.label}</span>
                {EXPORT_OPTIONS.filter((option) => option.group === group.id).map((opt) => (
                  <div className="tb-export-option-row" data-cut-export-option-row={opt.id} key={opt.id}>
                    <button role="menuitem" data-cut-export-option={opt.id} onClick={() => void onExport(opt)}>
                      {opt.label}
                    </button>
                    <button
                      role="menuitem"
                      className="tb-export-saveas"
                      data-cut-export-saveas-option={opt.id}
                      title={`Choose a file for ${opt.label}`}
                      onClick={() => void onExportSaveAs(opt)}
                    >
                      <Icon name="save" size={14} label="Save As" />
                    </button>
                  </div>
                ))}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Storyboard contact-sheet overlay — fixed-position modal (its DOM
          location here doesn't affect layout). Mounted only while open. */}
      {sbOpen && (
        <StoryboardOverlay busy={sbBusy} result={sbResult} error={sbError} onClose={closeStoryboard} />
      )}
      {/* Director-model reframe surface — drives render.direct → pick → reframe → QC. */}
      {directorOpen && aspect !== 'project' && (
        <DirectorModal aspect={aspect} preset={reframePreset} onClose={() => setDirectorOpen(false)} />
      )}
      {/* Batch-delivery queue (render.queue) — opened from the Export menu. */}
      {queueOpen && <RenderQueueModal onClose={() => setQueueOpen(false)} />}
      {otioPreview && (
        <OtioImportModal
          preview={otioPreview}
          busy={otioBusy}
          error={otioError}
          onCancel={() => { if (!otioBusy) { setOtioPreview(null); setOtioError(null) } }}
          onConfirm={() => void confirmOtio()}
        />
      )}
      {preflight && (
        <PreflightWarning
          report={preflight.report}
          actionLabel={preflight.actionLabel}
          onCancel={clearPreflight}
          onContinue={() => void continuePreflight()}
        />
      )}
    </header>
  )
}
