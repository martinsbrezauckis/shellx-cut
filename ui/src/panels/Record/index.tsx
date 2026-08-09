// panels/Record — the recording workspace.
//
// Role: a full-surface capture mode (not a drawer) for "record your screen → a
// polished clip on the timeline, no editing". Drives the in-process screen
// recorder verbs (screen_record.*): doctor → capability cards; start → a capture
// with a live HUD; on finish → stop (autoedit) → polish → the baked clip lands on
// the timeline.
//
// OPEN-ENDED: the default is "Record until I stop" — `screen_record.start`
// with NO `duration_ms`, ended by a manual Stop button or the keyboard shortcut
// (F9 toggles Start ⇄ Stop). The duration presets remain as an
// OPTIONAL upper-bound cap (the first option, "No limit", is the default); when a
// cap is chosen the surface also counts down and auto-finalizes at the bound.
//
// SOURCE: full-screen capture. On a MULTI-MONITOR setup, screen_record.doctor
// returns an enumerated `monitors` list and we render a real <select> so the user
// picks which display; the chosen 1-based index is passed through as
// screen_record.start{monitor}. On a single display (or Linux, where the doctor
// list is empty because the XDG portal shows its OWN source picker at capture
// time) we keep the single "Full screen" button — no regression. Per-window
// capture needs engine-side window enumeration (not yet wired) — surfaced honestly
// as "coming", never a dead control.
//
// Zero hidden state: every result is read from the verb envelope. Callers: App
// (workspaceMode === 'record'). Deps: lib/client (verbs), lib/doctor (cards).

import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb, type Project } from '../../lib/client'
import { folderTail, withAuthorizedOutputPath } from '../../lib/exportDestination'
import { isBlockingOverlayActive, shouldIgnoreGlobalShortcut } from '../../lib/dom'
import { matchesFixedAction } from '../../lib/keymap'
import { isTauri, onRecordHotkey, pickExportOutput } from '../../lib/tauri'
import { runUserVerb } from '../../lib/userActionFeedback'
import { StudioControls } from './StudioControls'
import { StudioPreview } from './StudioPreview'
import { useRecordingExport } from './useRecordingExport'
import {
  defaultStudioState,
  type StudioBackground,
  type CursorCorrelation,
  type StudioEventPayload,
  type StudioRawStreams,
  type StudioState,
} from './studioTypes'
import './record.css'

export interface RecordProps {
  project: Project | null
  /** Re-sync App state after a polish drops a clip on the timeline. */
  onClipAdded?: () => void
  /** Open Settings at the shared default export folder row. */
  onOpenOutputSettings?: () => void
}

/** What to tell the user when a one-off Save As target is what broke the action. */
const OUTPUT_PATH_HINT =
  'pick another file with "Choose file", or Clear it to use the default export folder'

/**
 * Human reason for a rejected promise from the verb/authorization chain.
 *
 * `fetch` rejects with a TypeError when the connection itself fails, and its
 * message is engine-flavoured browser text ("Failed to fetch" / "Load failed")
 * that means nothing to a user — so that case keeps the plain transport wording.
 * Everything else is one of OUR thrown Errors (today: withAuthorizedOutputPath
 * refusing the chosen output folder), whose message is already the useful thing
 * to show, so it is passed through verbatim.
 */
function failureReason(error: unknown): string {
  if (error instanceof TypeError) return 'server unreachable'
  return error instanceof Error && error.message ? error.message : 'server unreachable'
}

interface RecordCard { name: string; status: string; detail: string }
const RECORD_CARD_LABELS: Record<string, string> = {
  ffmpeg: 'Media tools',
  screen_capture: 'Screen capture',
  input_hook: 'Pointer and keys',
  gstreamer: 'Linux capture',
  wayland_input: 'Linux input',
}
function recordCardLabel(name: string): string {
  return RECORD_CARD_LABELS[name] ?? name.replaceAll('_', ' ')
}
// One display the doctor enumerated for the monitor PICKER. `index` is 1-based —
// exactly what screen_record.start{monitor} expects. Empty list ⇒ no in-app picker
// (single display, or Linux where the OS portal owns source choice).
interface MonitorInfo { index: number; name: string; width: number; height: number; primary: boolean }
// One application window the doctor enumerated for the window picker (record one
// app, not the whole screen). `title` is passed back as screen_record.start{window}.
// Empty list ⇒ no in-app window picker (Linux/macOS — the OS portal owns it).
interface WindowInfo { id: number; title: string; app: string }
// Result of screen_record.doctor{warm_mic:true}: whether the default mic went
// live (prompt answered + stream up), the device name, and whether this build has a
// mic backend at all. Drives the "mic ready" indicator so the user knows the first
// recording will actually capture their voice.
interface MicWarm { live: boolean; device?: string; supported: boolean }

type Phase = 'idle' | 'recording' | 'finalizing' | 'done' | 'error'

// Length options. `ms: null` = OPEN-ENDED ("record until I stop", the default);
// the rest are OPTIONAL upper-bound caps. Listed first so it's the default choice.
const DUR_PRESETS: { label: string; ms: number | null }[] = [
  { label: 'No limit', ms: null },
  { label: '10s', ms: 10_000 },
  { label: '30s', ms: 30_000 },
  { label: '1 min', ms: 60_000 },
  { label: '2 min', ms: 120_000 },
]

// The start/stop toggle binding uses one key rather than a multi-key chord. The
// desktop shell registers F9 as a GLOBAL OS hotkey (lib.rs) so it toggles even
// while another app is focused — essential for a recorder, whose own window is
// backgrounded the whole time it captures other apps. The same F9 also works as
// an in-page keydown FALLBACK here, covering the focused-window case and the
// plain web/dev build (where there is no shell to register the OS-level hotkey).
const SHORTCUT_LABEL = 'F9'

// mm:ss for the open-ended elapsed clock.
function fmtElapsed(totalSec: number): string {
  const m = Math.floor(totalSec / 60)
  const s = totalSec % 60
  return `${m}:${s.toString().padStart(2, '0')}`
}

export default function Record({ project, onClipAdded, onOpenOutputSettings }: RecordProps) {
  const [cards, setCards] = useState<RecordCard[]>([])
  const [ready, setReady] = useState<boolean | null>(null)
  // `start_allowed` is deliberately narrower than Doctor `ready`: on Linux,
  // Start is the user-initiated portal picker while Doctor stays unknown/non-green.
  const [startAllowed, setStartAllowed] = useState<boolean | null>(null)
  // Monitor PICKER: the doctor's enumerated displays (empty on single-display /
  // Linux), and the chosen 1-based monitor index (null = primary / engine default).
  const [monitors, setMonitors] = useState<MonitorInfo[]>([])
  const [monitorIdx, setMonitorIdx] = useState<number | null>(null)
  // Window picker: the doctor's enumerated app windows (empty on Linux/macOS), and
  // the chosen window title (null = capture the whole screen/monitor, not one window).
  const [windows, setWindows] = useState<WindowInfo[]>([])
  const [windowTitle, setWindowTitle] = useState<string | null>(null)
  // `capMs === null` = open-ended (the default). Otherwise it is the cap in ms.
  const [capMs, setCapMs] = useState<number | null>(null)
  const [fps, setFps] = useState(30)
  const [audio, setAudio] = useState(true)
  // Capture DESKTOP/SYSTEM audio (game/app sound) as a SEPARATE mixable track.
  // Defaults OFF for safety: desktop audio capture should be an explicit opt-in.
  // Linux uses the PulseAudio-compatible monitor source.
  const [systemAudio, setSystemAudio] = useState(false)
  const [keys, setKeys] = useState(false)
  // When ON (default), stop → auto-polish (auto-zoom, cursor, framing) — the
  // slower re-render. When OFF, stop → a FAST stream-copy (raw:true) so the clip lands
  // on the timeline almost immediately; the user can polish later if they want.
  // (Only meaningful in AUTO-EDIT mode; ignored in RAW capture.)
  const [autoPolish, setAutoPolish] = useState(true)
  // RAW CAPTURE mode. false = AUTO-EDIT (the flagship: record → autoedit
  // → polish → a clip on the timeline). true = RAW: keep ALL the same capture options
  // (source, fps, mic + system-audio sources) but on stop SKIP autoedit AND polish —
  // the engine just folds the streams into one raw.mp4 (screen_record.stop{mux_raw}).
  // We surface the file and OFFER to add it as-is; nothing is auto-edited or auto-placed.
  // Default AUTO-EDIT so existing behaviour is unchanged; raw is the explicit opt-in.
  const [rawCapture, setRawCapture] = useState(false)
  const [studio, setStudio] = useState<StudioState>(() => defaultStudioState())
  const [lastRawStreams, setLastRawStreams] = useState<StudioRawStreams | null>(null)
  const [lastCursorCorrelation, setLastCursorCorrelation] = useState<CursorCorrelation | null>(null)
  // Mic-warm result from the on-mount warm; null until probed / when mic is off.
  const [micWarm, setMicWarm] = useState<MicWarm | null>(null)
  // The last finalized capture's source+plan, retained so "Export clip" can render it
  // to a file (screen_record.export) without re-recording. (AUTO-EDIT mode only.)
  const [lastCapture, setLastCapture] = useState<{ source: string; plan: string } | null>(null)
  // RAW mode: the last finalized raw recording — its file path + which sound sources
  // it folded in. Drives the "Raw recording saved → …" done-state + "Add to timeline".
  const [lastRaw, setLastRaw] = useState<{ path: string; hasMic: boolean; hasSystem: boolean } | null>(null)
  const [exportFmt, setExportFmt] = useState<'mp4' | 'gif'>('mp4')
  const [recordOutputPath, setRecordOutputPath] = useState<string | null>(null)
  const [recordOutputNote, setRecordOutputNote] = useState('')
  const [exportNote, setExportNote] = useState('')
  const { exportJob, exportClip, cancelExport } = useRecordingExport({
    capture: lastCapture,
    format: exportFmt,
    outputPath: recordOutputPath,
    setNote: setExportNote,
  })
  // Seconds elapsed in the finalize/bake phase, so the wait is not opaque.
  const [finalizeSec, setFinalizeSec] = useState(0)
  const [phase, setPhase] = useState<Phase>('idle')
  // `remaining` is the cap countdown (only meaningful when capMs != null);
  // `elapsed` is the wall-clock since start (the open-ended clock).
  const [remaining, setRemaining] = useState(0)
  const [elapsed, setElapsed] = useState(0)
  const [note, setNote] = useState('')
  const [err, setErr] = useState<string | null>(null)
  const tickRef = useRef<number | null>(null)
  // The capture_id of the in-flight recording — drives the manual Stop + shortcut.
  const captureRef = useRef<string | null>(null)
  const recordStartedAtRef = useRef<number | null>(null)
  // Timestamp of the last F9 toggle. When the Cut window is FOCUSED, one physical
  // F9 press can reach BOTH the global OS hotkey (→ cut:record-hotkey event) AND
  // the in-page keydown — the OS does not swallow a global shortcut for the
  // focused window on most platforms — which would toggle twice and cancel out.
  // Collapsing toggles within a short window to one keeps a single press = a
  // single start/stop, whichever path(s) fire.
  const lastToggleRef = useRef(0)

  const emitStudioEvent = useCallback(async (payload: StudioEventPayload): Promise<boolean> => {
    const captureId = captureRef.current
    const startedAt = recordStartedAtRef.current ?? Date.now()
    if (!captureId || startedAt === null) return false
    const tMs = Math.max(0, Date.now() - startedAt)
    const result = await runUserVerb('screen_record.studio_event', {
      capture_id: captureId,
      event: { t_ms: tMs, ...payload },
    }, 'Could not save the live recording change.')
    if (!result?.ok && captureRef.current === captureId) {
      setNote('Live Studio changes may not replay in polish.')
    }
    return Boolean(result?.ok)
  }, [])

  const setStudioBackground = useCallback((background: StudioBackground) => {
    setStudio((prev) => ({ ...prev, background }))
    void emitStudioEvent({ source: 'background', kind: 'style', background })
  }, [emitStudioEvent])

  const addRecordingMarker = useCallback(() => {
    if (phase !== 'recording' || !captureRef.current) {
      setNote('Start recording before adding a marker.')
      return
    }
    const at = fmtElapsed(elapsed)
    void emitStudioEvent({ source: 'recording', kind: 'marker', label: `Marker ${at}` })
      .then((ok) => { if (ok) setNote(`Marker added at ${at}`) })
  }, [elapsed, emitStudioEvent, phase])

  // Probe the recorder's capability cards (in-process screen_record.doctor).
  const probe = useCallback(async () => {
    const r = await callVerb('screen_record.doctor', {})
    if (r.ok && r.result) {
      const res = r.result as { cards: RecordCard[]; ready: boolean; start_allowed?: boolean; monitors?: MonitorInfo[]; windows?: WindowInfo[] }
      setCards(res.cards)
      setReady(res.ready)
      // A pre-start server predates this field, so its strict `ready` result is
      // the safe fallback; never infer permission from an arbitrary unknown card.
      setStartAllowed(res.start_allowed ?? res.ready)
      const mons = res.monitors ?? []
      setMonitors(mons)
      // App windows for the window picker. Re-probing (re-entering Record)
      // refreshes the list. Drop a stale selection if its window is gone.
      const wins = res.windows ?? []
      setWindows(wins)
      setWindowTitle((prev) => (prev && wins.some((w) => w.title === prev) ? prev : null))
      // Default the picker to the primary display (else the first), so the chosen
      // index is explicit once there's a list. Empty list ⇒ null (engine primary).
      setMonitorIdx((prev) => {
        if (mons.length === 0) return null
        if (prev !== null && mons.some((m) => m.index === prev)) return prev
        return (mons.find((m) => m.primary) ?? mons[0]).index
      })
    } else {
      setReady(false)
      setStartAllowed(false)
    }
  }, [])
  useEffect(() => { void probe() }, [probe])

  // Warm the default mic on entering Record (and whenever mic capture is turned
  // on) so the OS permission prompt is answered + the cpal stream is spun up BEFORE the
  // user hits record — a short FIRST recording otherwise finishes before the just-
  // granted mic starts flowing. screen_record.doctor{warm_mic:true} opens it briefly.
  useEffect(() => {
    if (!audio) { setMicWarm(null); return }
    let cancelled = false
    void callVerb('screen_record.doctor', { warm_mic: true }).then((r) => {
      if (cancelled || !r.ok || !r.result) return
      const mw = (r.result as { mic_warm?: MicWarm }).mic_warm
      if (mw) setMicWarm(mw)
    })
    return () => { cancelled = true }
  }, [audio])

  useEffect(() => {
    if (!audio || !navigator.mediaDevices?.addEventListener) return
    let cancelled = false
    const refreshMic = () => {
      setMicWarm(null)
      void probe()
      void callVerb('screen_record.doctor', { warm_mic: true }).then((r) => {
        if (cancelled || !r.ok || !r.result) return
        const mw = (r.result as { mic_warm?: MicWarm }).mic_warm
        if (mw) setMicWarm(mw)
      })
    }
    navigator.mediaDevices.addEventListener('devicechange', refreshMic)
    return () => {
      cancelled = true
      navigator.mediaDevices.removeEventListener('devicechange', refreshMic)
    }
  }, [audio, probe])

  const clearTick = () => {
    if (tickRef.current) window.clearInterval(tickRef.current)
    tickRef.current = null
  }

  // Finalize: stop (with autoedit) → polish → clip on the timeline. Idempotent on
  // captureRef so a double-trigger (button + shortcut, or cap + manual) is harmless.
  const finalize = useCallback(async (captureId: string, source: string | null) => {
    clearTick()
    if (captureRef.current !== captureId) return // already finalized / superseded
    captureRef.current = null
    recordStartedAtRef.current = null
    const rawMode = rawCapture
    const polishing = autoPolish
    setPhase('finalizing')
    setExportNote('')
    setFinalizeSec(0)
    setNote(rawMode ? 'Saving the raw recording…' : polishing ? 'Polishing — auto-zoom, cursor, framing…' : 'Preparing your clip…')
    // A visible elapsed clock so the bake is not an opaque wait (it scales with
    // recording length on the polish path; the raw path is near-instant).
    const finalizeStart = Date.now()
    tickRef.current = window.setInterval(() => {
      setFinalizeSec(Math.floor((Date.now() - finalizeStart) / 1000))
    }, 500)
    try {
      // RAW CAPTURE: stop WITHOUT autoedit and ask the engine to fold the captured
      // streams into ONE raw.mp4 (mux_raw). NO autoedit, NO polish, NO auto-place —
      // just the recording, exactly as captured. We surface the file path and offer
      // to add it to the timeline as-is (addRawToTimeline); nothing is post-processed.
      if (rawMode) {
        const rawOutputPath = recordOutputPath ?? undefined
        const stop = await withAuthorizedOutputPath(rawOutputPath, () =>
          callVerb('screen_record.stop', { capture_id: captureId, autoedit: false, mux_raw: true, raw_path: rawOutputPath }))
        if (!stop.ok) { setErr(`stop failed: ${stop.error?.message ?? 'error'}`); setPhase('error'); return }
        const sr = stop.result as { raw_path?: string; raw_has_mic?: boolean; raw_has_system?: boolean; source?: string; raw_streams?: StudioRawStreams; cursor_correlation?: CursorCorrelation }
        setLastRawStreams(sr.raw_streams ?? null)
        setLastCursorCorrelation(sr.cursor_correlation ?? null)
        const rawPath = sr.raw_path ?? sr.source ?? null
        if (!rawPath) { setErr('capture produced no raw recording'); setPhase('error'); return }
        setLastRaw({ path: rawPath, hasMic: !!sr.raw_has_mic, hasSystem: !!sr.raw_has_system })
        setNote('Raw recording saved')
        setPhase('done')
        return
      }
      const stop = await callVerb('screen_record.stop', { capture_id: captureId, autoedit: true })
      if (!stop.ok) { setErr(`stop failed: ${stop.error?.message ?? 'error'}`); setPhase('error'); return }
      const sr = stop.result as { source?: string; plan?: string; raw_streams?: StudioRawStreams; cursor_correlation?: CursorCorrelation }
      setLastRawStreams(sr.raw_streams ?? null)
      setLastCursorCorrelation(sr.cursor_correlation ?? null)
      const src = sr.source ?? source
      if (!src || !sr.plan) { setErr('capture produced no source/plan'); setPhase('error'); return }
      // Retain source+plan so "Export clip" can render a file later without re-recording.
      setLastCapture({ source: src, plan: sr.plan })
      // raw:true takes the FAST stream-copy path (no zoom/cursor re-render).
      const pol = await callVerb('screen_record.polish', { source: src, plan: sr.plan, raw: !polishing })
      if (!pol.ok) { setErr(`${polishing ? 'polish' : 'preparing clip'} failed: ${pol.error?.message ?? 'error'}`); setPhase('error'); return }
      const pr = pol.result as { clip_id?: string }
      setNote(`Done — clip ${pr.clip_id ?? ''} added to the timeline`)
      setPhase('done')
      onClipAdded?.()
    } catch (error) {
      // Same class as exportClip's catch: the RAW path authorizes the chosen Save As
      // folder through withAuthorizedOutputPath, which THROWS when the engine refuses
      // it. Reporting that as "server unreachable" sent the user to check the engine
      // when the actual fix is to pick another recording file — so name the real
      // reason, and keep the transport wording for the case that really is one
      // (fetch rejects with a TypeError when the connection fails).
      const reason = failureReason(error)
      setErr(rawMode && recordOutputPath
        ? `finalize failed: ${reason} — ${OUTPUT_PATH_HINT} (${recordOutputPath})`
        : `finalize failed: ${reason}`)
      setPhase('error')
    } finally {
      clearTick() // stop the finalize elapsed clock on every exit path
    }
  }, [onClipAdded, autoPolish, rawCapture, recordOutputPath])

  // RAW mode: add the saved raw recording to the timeline AS-IS. `media.import`
  // auto-places the first clip into an empty timeline (the common fresh-recording
  // case) — no autoedit, no polish. The raw.mp4 already carries the combined audio
  // (mic + system folded by mux_raw), so a single import brings picture + sound.
  const addRawToTimeline = useCallback(async () => {
    if (!lastRaw) return
    setExportNote('Adding to the timeline…')
    const imp = await callVerb('media.import', { path: lastRaw.path })
    if (!imp.ok) { setExportNote(`add failed: ${imp.error?.message ?? 'error'}`); return }
    setExportNote('Added to the timeline')
    onClipAdded?.()
  }, [lastRaw, onClipAdded])

  // Start a capture. Open-ended by default (no duration_ms); a chosen cap is passed
  // as an upper bound AND drives a local countdown that auto-finalizes at the bound.
  const start = useCallback(async () => {
    setErr(null)
    setNote('')
    // Record without a project: if none is open, create one on the fly so
    // you can record straight from the Record surface — the capture lands in a fresh
    // auto-named project. project.create OPENS it server-side; onClipAdded re-syncs App.
    if (!project) {
      const stamp = new Date().toISOString().slice(0, 16).replace(/[:T]/g, '-')
      const pc = await callVerb('project.create', { name: `Recording ${stamp}` })
      if (!pc.ok) { setErr(`could not create a project: ${pc.error?.message ?? 'error'}`); setPhase('error'); return }
      onClipAdded?.()
    }
    // RAW mode never renders the key-cast overlay (that's a polish pass), so don't
    // capture keystrokes in raw mode — a small privacy win (keys can reveal secrets).
    setLastRawStreams(null)
    setLastCursorCorrelation(null)
    setLastCapture(null)
    setLastRaw(null)
    const startArgs: {
      fps: number
      audio: boolean
      system_audio: boolean
      keys: boolean
      duration_ms?: number
      monitor?: number
      window?: string
      studio?: unknown
    } = {
      fps,
      audio,
      system_audio: systemAudio,
      keys: rawCapture ? false : keys,
      studio: {
        background: studio.background,
      },
    }
    if (capMs !== null) startArgs.duration_ms = capMs // omitted entirely = open-ended
    // Source selection: a chosen window (by title) wins; otherwise a chosen monitor on a
    // multi-monitor setup; else neither = engine default (primary full screen).
    if (windowTitle) startArgs.window = windowTitle
    else if (monitorIdx !== null && monitors.length >= 2) startArgs.monitor = monitorIdx
    const r = await callVerb('screen_record.start', startArgs)
    if (!r.ok) {
      setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'could not start capture'}`)
      setPhase('error')
      return
    }
    const res = r.result as { capture_id: string; out_dir?: string }
    captureRef.current = res.capture_id
    recordStartedAtRef.current = Date.now()
    setPhase('recording')
    setElapsed(0)
    setRemaining(capMs !== null ? Math.ceil(capMs / 1000) : 0)
    // HUD tick: count elapsed up (open-ended clock) and, when a cap is set, the
    // remaining down — auto-finalizing when the cap elapses. Manual Stop / the
    // shortcut can end it earlier at any time.
    const startedAt = recordStartedAtRef.current
    clearTick()
    tickRef.current = window.setInterval(() => {
      const elapsedSec = Math.floor((Date.now() - startedAt) / 1000)
      setElapsed(elapsedSec)
      if (capMs !== null) {
        const left = Math.max(0, Math.ceil((capMs - (Date.now() - startedAt)) / 1000))
        setRemaining(left)
        if (left <= 0) void finalize(res.capture_id, null)
      }
    }, 250)
  }, [capMs, fps, audio, systemAudio, keys, rawCapture, monitorIdx, monitors, windowTitle, finalize, project, onClipAdded, studio])

  // Manual STOP — ends an open-ended (or capped) capture right now.
  const stop = useCallback(() => {
    const id = captureRef.current
    if (id) void finalize(id, null)
  }, [finalize])

  const chooseOutputPath = useCallback(async () => {
    if (!isTauri()) {
      setRecordOutputNote('Choose file needs the desktop app.')
      return
    }
    const ext = rawCapture ? 'mp4' : exportFmt
    const path = await pickExportOutput({
      title: 'Choose recording output file — ShellX Cut',
      defaultPath: `recording.${ext}`,
      filters: ext === 'gif'
        ? [{ name: 'GIF image', extensions: ['gif'] }]
        : [{ name: 'MP4 video', extensions: ['mp4'] }],
    })
    if (!path) return
    setRecordOutputPath(path)
    setRecordOutputNote(rawCapture ? 'Raw recording will save to this file.' : 'Polished export will use this file when you export.')
    setExportNote('')
  }, [rawCapture, exportFmt])

  // The ONE start⇄stop toggle, shared by the global OS hotkey (F9) and the
  // in-page F9 keydown fallback — so every trigger does exactly the same thing.
  // Coalesced (lastToggleRef) so a single F9 press that reaches BOTH paths (Cut
  // window focused) toggles once, not twice. No-op while finalizing, or when a
  // project isn't open / capture cannot start (matches the button's disabled state).
  const toggle = useCallback(() => {
    const now = Date.now()
    if (now - lastToggleRef.current < 400) return // de-dupe the global+in-page double-fire
    lastToggleRef.current = now
    if (phase === 'recording') {
      stop()
    } else if (phase === 'idle' || phase === 'done' || phase === 'error') {
      if (startAllowed !== false) void start() // start() auto-creates a project if none is open
    }
  }, [phase, project, startAllowed, start, stop])

  // A SINGLE key (F9) toggles Start ⇄ Stop — no 3-key chord.
  //
  // GLOBAL path (desktop): the shell registers F9 as an OS-level global hotkey
  // and emits `cut:record-hotkey`; we listen for it here so F9 STOPS a capture
  // even while the recorded app is focused (the whole reason a recorder needs a
  // global key). No-op outside Tauri.
  useEffect(() => onRecordHotkey(() => {
    if (!isBlockingOverlayActive()) toggle()
  }), [toggle])

  // FOCUSED-WINDOW fallback: the same F9 as a plain in-page keydown. Covers the
  // case where the Cut window itself is focused, and is the SOLE path in the
  // plain web/dev build (no shell ⇒ no global registration). Listener lives HERE
  // (not App.tsx) so it doesn't touch the layout agent's files; active only while
  // the Record surface is mounted.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (shouldIgnoreGlobalShortcut(e)) return
      if (matchesFixedAction(e, 'recording.toggle')) {
        e.preventDefault()
        toggle()
        return
      }
      if (matchesFixedAction(e, 'recording.marker')) {
        e.preventDefault()
        addRecordingMarker()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [addRecordingMarker, toggle])

  // Clean up the tick on unmount.
  useEffect(() => () => { clearTick() }, [])

  const busy = phase === 'recording' || phase === 'finalizing'

  // Staleness guard: the user's open windows change while the Record tab sits open
  // (they alt-tab, open Chrome, close an app), but the picker only probed on mount —
  // so it showed a stale list with only an old entry while Chrome and a terminal
  // were open). Re-probe every 3s while idle (NOT recording) so the window/monitor list
  // tracks the live desktop. Cheap enough (the same doctor call) and it stops during a
  // capture. The select's onMouseDown also re-probes for an instant refresh on click.
  useEffect(() => {
    if (busy) return
    const id = setInterval(() => { void probe() }, 3000)
    return () => clearInterval(id)
  }, [busy, probe])
  const cardStatus = (c: RecordCard) => (
    c.status === 'ok' ? 'ok' : c.status === 'degraded' ? 'degraded' : c.status === 'unknown' ? 'unknown' : 'missing'
  )
  const studioElapsed = phase === 'recording'
    ? fmtElapsed(elapsed)
    : phase === 'finalizing'
      ? `${finalizeSec}s`
      : '0:00'

  return (
    <section className="rec" data-cut-panel="record" data-cut-record-mode={rawCapture ? 'quick' : 'studio'}>
      <header className="rec__head">
        <div className="rec__title-wrap">
          <span className="rec__dot" data-cut-rec-phase={phase} aria-hidden="true" />
          <h1 className="rec__title">Record</h1>
        </div>
        <p className="rec__sub">
          {rawCapture
            ? 'Raw capture — record your screen and save it exactly as captured, no auto-edit.'
            : 'Capture your screen → a polished clip on the timeline, no editing.'}
        </p>
      </header>

      {!project && <p className="rec__source-note" data-cut-rec-no-project>No project open yet — Start creates one automatically and the recording lands inside it.</p>}

      {(
        <div className="rec__body">
          {/* Capability cards (screen_record.doctor) */}
          <div className="rec__cards" data-cut-rec-cards data-cut-rec-readiness>
            {cards.filter((c) => c.name !== 'webcam').map((c) => (
              <div key={c.name} className={`rec__card rec__card--${cardStatus(c)}`} data-cut-rec-card={c.name} data-cut-rec-card-status={cardStatus(c)}>
                <span className="rec__card-name">{recordCardLabel(c.name)}</span>
                <span className="rec__card-detail">{c.detail}</span>
              </div>
            ))}
            {ready === false && startAllowed === true && (
              <p className="rec__not-ready" data-cut-rec-portal-prompt>
                Screen capture is deliberately unverified until the Linux source picker runs.
                Start recording opens that picker; Doctor stays non-green until a later delivery proof.
              </p>
            )}
            {ready === false && startAllowed !== true && (
              <p className="rec__not-ready" data-cut-rec-not-ready>
                Screen capture isn’t verified on this machine — an unknown status is not ready.
                Recording needs a desktop session (Linux XDG portal / Windows / macOS) with ffmpeg. Core editing still works.
              </p>
            )}
          </div>

          <div className="rec__studio" data-cut-rec-studio>
            <StudioPreview
              background={studio.background}
              phase={phase}
              elapsed={studioElapsed}
            />
            <StudioControls
              studio={studio}
              rawStreams={lastRawStreams}
              cursorCorrelation={lastCursorCorrelation}
              onBackground={setStudioBackground}
            />
          </div>

          {/* Settings */}
          <div className="rec__settings" data-cut-rec-settings>
            {/* MODE: AUTO-EDIT (record → polished clip on the timeline) vs RAW CAPTURE
                (save the recording exactly as captured — no autoedit, no polish). The
                same source/length/fps/mic/system-audio options apply to BOTH modes. */}
            <div className="rec__field rec__field--mode">
              <span className="rec__label">Mode</span>
              <div className="rec__seg" role="radiogroup" aria-label="Recording mode">
                <button
                  type="button"
                  className={`rec__seg-btn${!rawCapture ? ' rec__seg-btn--on' : ''}`}
                  data-cut-rec-mode="auto"
                  aria-pressed={!rawCapture}
                  disabled={busy}
                  onClick={() => setRawCapture(false)}
                  title="Record → an auto-edited, polished clip on the timeline (zoom-to-cursor, framing)."
                >
                  Auto-edit
                </button>
                <button
                  type="button"
                  className={`rec__seg-btn${rawCapture ? ' rec__seg-btn--on' : ''}`}
                  data-cut-rec-mode="raw"
                  aria-pressed={rawCapture}
                  disabled={busy}
                  onClick={() => setRawCapture(true)}
                  title="Raw capture — save the recording exactly as captured (your sound sources included), with no auto-edit or polish."
                >
                  Raw capture
                </button>
              </div>
              <p className="rec__source-note" data-cut-rec-mode-note>
                {rawCapture
                  ? 'Raw capture saves the recording as-is — no auto-edit, no polish. Your mic and system/desktop audio are folded into one file; you choose whether to add it to the timeline.'
                  : 'Auto-edit records, then polishes (zoom-to-cursor, framing) and drops the finished clip on the timeline.'}
              </p>
            </div>
            <div className="rec__field rec__field--output" data-cut-rec-output-path={recordOutputPath ?? ''}>
              <span className="rec__label">Recording file</span>
              <div className="rec__output-row">
                <code className="rec__output-path" title={recordOutputPath ?? undefined}>
                  {recordOutputPath ? folderTail(recordOutputPath) : 'Uses default export folder'}
                </code>
                <button
                  type="button"
                  className="rec__export-btn rec__export-btn--ghost rec__export-btn--small"
                  data-cut-action="record-output-default-folder"
                  disabled={busy || !onOpenOutputSettings}
                  onClick={() => onOpenOutputSettings?.()}
                >
                  Default folder
                </button>
                <button
                  type="button"
                  className="rec__export-btn rec__export-btn--small"
                  data-cut-action="record-output-pick"
                  disabled={busy}
                  onClick={() => void chooseOutputPath()}
                >
                  Choose file
                </button>
                {recordOutputPath && (
                  <button
                    type="button"
                    className="rec__export-btn rec__export-btn--ghost rec__export-btn--small"
                    data-cut-action="record-output-clear"
                    disabled={busy}
                    onClick={() => {
                      setRecordOutputPath(null)
                      setRecordOutputNote('Using the default export folder.')
                    }}
                  >
                    Clear
                  </button>
                )}
              </div>
              {recordOutputNote && <p className="rec__source-note" data-cut-rec-output-note>{recordOutputNote}</p>}
            </div>
            <div className="rec__field rec__field--source">
              <span className="rec__label">Source</span>
              <div className="rec__seg">
                {/* SOURCE picker. Windows: the doctor enumerates `monitors` (displays)
                    AND `windows` (app windows), so a real <select> offers Full-screen /
                    per-monitor or per-window capture (record one app, not the whole
                    screen). Linux / single display with no window list (empty arrays —
                    the XDG portal shows its OWN source picker at capture time): the
                    single "Full screen" button. The data-cut-rec-source hook stays on
                    whichever control renders so agent/UI tests still find the source. */}
                {/* Always render the source <select>, even on a single display with no
                    other app windows open — so the source choice is explicit + discoverable.
                    Opening it re-probes (onMouseDown) and reveals app windows as they appear. */ (
                  <select
                    className="rec__select"
                    data-cut-rec-source={windowTitle ? 'window' : 'screen'}
                    data-cut-rec-monitor={windowTitle ? '' : (monitorIdx ?? '')}
                    data-cut-rec-window={windowTitle ?? ''}
                    disabled={busy}
                    // Re-enumerate the live windows the moment the user opens the picker,
                    // so a window opened or closed since mount shows up immediately.
                    onMouseDown={() => { void probe() }}
                    value={windowTitle ? `win:${windowTitle}` : `mon:${monitorIdx ?? (monitors.find((m) => m.primary)?.index ?? monitors[0]?.index ?? 1)}`}
                    onChange={(e) => {
                      const v = e.target.value
                      if (v.startsWith('win:')) setWindowTitle(v.slice(4))
                      else { setWindowTitle(null); setMonitorIdx(Number(v.slice(4))) }
                    }}
                    aria-label="What to record — a screen or one application window"
                  >
                    <optgroup label={monitors.length >= 2 ? 'Displays' : 'Screen'}>
                      {(monitors.length >= 1 ? monitors : [{ index: 1, name: '', width: 0, height: 0, primary: true }]).map((m) => (
                        <option key={`mon-${m.index}`} value={`mon:${m.index}`}>
                          {`${monitors.length >= 2 ? `Monitor ${m.index}` : 'Full screen'}${m.name ? ` — ${m.name}` : ''}${m.width && m.height ? ` (${m.width}×${m.height})` : ''}${m.primary && monitors.length >= 2 ? ' (primary)' : ''}`}
                        </option>
                      ))}
                    </optgroup>
                    {windows.length >= 1 && (
                      <optgroup label="Windows — record one app">
                        {windows.map((w) => (
                          <option key={`win-${w.id}`} value={`win:${w.title}`}>
                            {`${w.title}${w.app ? ` — ${w.app}` : ''}`}
                          </option>
                        ))}
                      </optgroup>
                    )}
                  </select>
                )}
              </div>
              {monitors.length < 2 && windows.length < 1 && ready !== false && (
                <p className="rec__source-note" data-cut-rec-source-note>
                  {/Mac/i.test(navigator.platform) || /Mac OS X/.test(navigator.userAgent)
                    ? 'On macOS, capture records your main display. The first capture asks for Screen Recording permission (and Microphone, if audio is on).'
                    : 'On Linux the OS screen-share dialog lets you pick which display/window at the start of capture.'}
                </p>
              )}
            </div>
            <div className="rec__field rec__field--length">
              <span className="rec__label">Length</span>
              <div className="rec__seg">
                {DUR_PRESETS.map((d) => {
                  const on = capMs === d.ms
                  return (
                    <button
                      key={d.label}
                      type="button"
                      className={`rec__seg-btn${on ? ' rec__seg-btn--on' : ''}`}
                      data-cut-rec-dur={d.ms === null ? 'none' : d.ms}
                      disabled={busy}
                      onClick={() => setCapMs(d.ms)}
                    >
                      {d.label}
                    </button>
                  )
                })}
              </div>
            </div>
            <div className="rec__field rec__field--fps">
              <span className="rec__label">Frame rate</span>
              <div className="rec__seg">
                {[24, 30, 60].map((f) => (
                  <button key={f} type="button" className={`rec__seg-btn${fps === f ? ' rec__seg-btn--on' : ''}`} data-cut-rec-fps={f} disabled={busy} onClick={() => setFps(f)}>{f}</button>
                ))}
              </div>
            </div>
            <label className="rec__toggle rec__toggle--mic" data-cut-rec-audio-toggle>
              <input type="checkbox" data-cut-rec-audio-toggle-input checked={audio} disabled={busy} onChange={(e) => setAudio(e.target.checked)} /> Capture microphone audio
            </label>
            {audio && micWarm && (
              <p className="rec__mic-warm" data-cut-rec-mic-warm={micWarm.live ? 'ready' : micWarm.supported ? 'pending' : 'none'}>
                {micWarm.live
                  ? `✓ Mic ready${micWarm.device ? ` — ${micWarm.device}` : ''}`
                  : micWarm.supported
                    ? 'Grant microphone access when prompted — warming it now so your first recording isn’t silent.'
                    : 'No microphone detected on this machine.'}
              </p>
            )}
            <label className="rec__toggle rec__toggle--system" data-cut-rec-system-audio-toggle title="Capture the desktop/app sound (e.g. a game) onto its OWN audio track, separate from the mic">
              <input type="checkbox" data-cut-rec-system-audio-toggle-input checked={systemAudio} disabled={busy} onChange={(e) => setSystemAudio(e.target.checked)} /> Capture system / desktop audio (game sound)
            </label>
            {/* Key-cast + auto-polish are POLISH-pass features (a burned-in overlay / a
                zoom-cursor-framing re-render). RAW capture skips polish entirely, so
                these controls are hidden in raw mode rather than left as dead toggles. */}
            {!rawCapture && (
              <label className="rec__toggle rec__toggle--keys" data-cut-rec-keys-toggle title="Keystrokes can reveal passwords — off by default">
                <input type="checkbox" data-cut-rec-keys-toggle-input checked={keys} disabled={busy} onChange={(e) => setKeys(e.target.checked)} /> Show keystrokes (key-cast)
              </label>
            )}
            {!rawCapture && (
              <label className="rec__toggle rec__toggle--polish" data-cut-rec-autopolish-toggle title="Auto-polish adds zoom-to-cursor, cursor smoothing and a framed background after you stop — a short render that scales with length. Turn off to drop the RAW recording onto the timeline instantly and polish later.">
                <input type="checkbox" data-cut-rec-autopolish-toggle-input checked={autoPolish} disabled={busy} onChange={(e) => setAutoPolish(e.target.checked)} /> Auto-polish after recording (zoom, cursor, framing)
              </label>
            )}
          </div>

          {/* Transport / HUD */}
          <div className="rec__transport" data-cut-studio-result={phase} data-cut-rec-primary-transport>
            {phase === 'recording' ? (
              <div className="rec__hud-wrap">
                <div className="rec__hud" data-cut-rec-hud>
                  <span className="rec__hud-dot" aria-hidden="true" />
                  <span className="rec__hud-text" data-cut-rec-elapsed={elapsed}>
                    {capMs !== null
                      ? `Recording… ${remaining}s left`
                      : `Recording… ${fmtElapsed(elapsed)}`}
                  </span>
                </div>
                <button
                  type="button"
                  className="rec__stop"
                  data-cut-action="record-stop"
                  onClick={() => stop()}
                >
                  ■ Stop ({SHORTCUT_LABEL})
                </button>
              </div>
            ) : phase === 'finalizing' ? (
              <div className="rec__hud rec__hud--finalize" data-cut-rec-finalizing data-cut-rec-finalize-sec={finalizeSec}>
                {note || 'finalizing…'}{finalizeSec > 0 ? ` (${finalizeSec}s)` : ''}
              </div>
            ) : (
              <button
                type="button"
                className="rec__start"
                data-cut-action="record-start"
                disabled={busy || startAllowed === false}
                onClick={() => void start()}
              >
                ● Start recording ({SHORTCUT_LABEL})
              </button>
            )}
            {/* RAW-mode done: surface the saved raw file + offer to add it as-is. No
                autoedit/polish ran; "Add to timeline" imports the raw.mp4 (which already
                carries the combined mic+system audio) straight onto the timeline. */}
            {phase === 'done' && rawCapture && lastRaw && (
              <div className="rec__export" data-cut-rec-raw-done>
                <p className="rec__done" data-cut-rec-done data-cut-rec-raw-path={lastRaw.path}>
                  Raw recording saved → {lastRaw.path}
                </p>
                <p className="rec__source-note" data-cut-rec-raw-audio={lastRaw.hasMic && lastRaw.hasSystem ? 'mic+system' : lastRaw.hasMic ? 'mic' : lastRaw.hasSystem ? 'system' : 'none'}>
                  {lastRaw.hasMic && lastRaw.hasSystem
                    ? 'Includes your microphone + system/desktop audio (folded into one track).'
                    : lastRaw.hasMic
                      ? 'Includes your microphone audio.'
                      : lastRaw.hasSystem
                        ? 'Includes system/desktop audio.'
                        : 'Video only — no audio sources were captured.'}
                </p>
                <button type="button" className="rec__export-btn" data-cut-action="record-add-raw" onClick={() => void addRawToTimeline()}>
                  Add to timeline
                </button>
                {exportNote && <p className="rec__export-note" data-cut-rec-raw-note>{exportNote}</p>}
              </div>
            )}
            {phase === 'done' && !rawCapture && <p className="rec__done" data-cut-rec-done>{note}</p>}
            {phase === 'done' && !rawCapture && lastCapture && (
              <div className="rec__export" data-cut-rec-export>
                <span className="rec__label">Export this recording as a file</span>
                <div className="rec__seg">
                  {(['mp4', 'gif'] as const).map((f) => (
                    <button
                      key={f}
                      type="button"
                      className={`rec__seg-btn${exportFmt === f ? ' rec__seg-btn--on' : ''}`}
                      data-cut-rec-export-fmt={f}
                      disabled={!!exportJob}
                      onClick={() => setExportFmt(f)}
                    >
                      {f.toUpperCase()}
                    </button>
                  ))}
                  <button type="button" className="rec__export-btn" data-cut-action="record-export" disabled={!!exportJob} onClick={() => void exportClip()}>
                    Export clip…
                  </button>
                  {exportJob && (
                    <button type="button" className="rec__export-btn" data-cut-action="record-export-cancel" onClick={() => void cancelExport()}>
                      Cancel export
                    </button>
                  )}
                </div>
                {exportNote && <p className="rec__export-note" data-cut-rec-export-note data-cut-rec-export-progress={exportJob ? 'active' : 'idle'}>{exportNote}</p>}
              </div>
            )}
            {err && <p className="rec__err" data-cut-rec-error>{err}</p>}
            {phase === 'recording' && (
              <p className="rec__hint">
                {capMs !== null
                  ? `Stops automatically at the limit, or press Stop / ${SHORTCUT_LABEL} any time.`
                  : `Recording until you stop — press Stop or ${SHORTCUT_LABEL}.`}
                {' '}{SHORTCUT_LABEL} works globally — even when another app is focused.
                {' '}The first capture on this machine pops a one-time screen-share consent dialog.
              </p>
            )}
          </div>
        </div>
      )}
    </section>
  )
}
