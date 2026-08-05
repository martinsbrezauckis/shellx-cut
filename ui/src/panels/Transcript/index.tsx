// panels/Transcript — flowing-word transcript and select-to-cut editing surface.
// Role: renders every transcribed asset as flowing word spans (data-word-idx);
// playhead word highlight + auto-scroll (paused on user scroll, 3s resume);
// click word = seek (ui.playhead); drag / shift-click range select → floating
// Cut toolbar → transcript.cut_words; removed spans struck-through with a
// Restore pill (edit.restore via data-op-id — receipt-linked to the review
// rail); filler words dotted-amber before removal; header buttons run the
// silence/filler passes (aggressiveness REQUIRED — the required-argument contract, enforced here
// by disabling the button until a preset is chosen).
//
// REEL MODE (highlight-reel authoring): a header toggle turns the floating
// selection toolbar into an "Add to reel" affordance. Added spans accumulate in
// an ordered, removable tray (the tray order IS the reel order); "Assemble reel"
// dispatches ONE transcript.assemble{asset, word_ranges} from the tray. The reel
// is scoped to a SINGLE asset (assemble takes one `asset`): the first span fixes
// the reel asset and adds from other assets are disabled until the tray clears.
// The tray is pure VIEW state (zero-local-mutation contract) — selecting/building accumulates nothing
// on the timeline; only the assemble dispatch does, and its clips arrive via
// op_applied like every other edit.
//
// Zero local mutation: every change is a verb; struck spans render only from
// confirmed ops (rule 8 — in-flight cut shows a pending tint until op_applied).
// Callers: App.tsx. Deps: lib/client, lib/events, Review/shared.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { callVerb, type OpRecord, type Project, type Transcript as TranscriptData } from '../../lib/client'
import { events } from '../../lib/events'
import { fetchDoctor, setupPerception, type DoctorCard } from '../../lib/doctor'
import AssetWords from './AssetWords'
import ReelTray from './ReelTray'
import TimelineView from './TimelineView'
import TranscriptSetupCard from './TranscriptSetupCard'
import type { TimelineWord } from '../../lib/client'
import { chaptersOf, isObject, numberField, reelSnippet, searchResultFrom, selRange, timelineEntriesFrom, type Aggressiveness, type ReelSpan, type Sel } from './model'
import { activeCutSpans, dispatchVerb, fmtDur, fmtTc, seekPlayhead, type CutSpan } from '../Review/shared'
import { sourceAtPlayhead } from '../Timeline/layout'
import { Icon } from '../../icons'
import { runUserVerb } from '../../lib/userActionFeedback'
import './transcript.css'

/** Typed props — contract between App.tsx and the transcript panel. */
export interface TranscriptProps {
  project: Project | null
  playheadMs: number
  /** The selected clip (Timeline) — drives the SELECTED-CLIP timeline view. */
  selectedClipId?: string | null
  /** Transcripts keyed by asset id (App loads via transcript.get). */
  transcripts: Record<string, TranscriptData>
  /** Cut request → App dispatches transcript.cut_words. */
  onCutWords: (asset: string, wordRange: [number, number], rationale?: string) => void
  /** Restore request → App dispatches edit.restore. */
  onRestore: (opId: string) => void
  /** Seek request (optional — integration wires; fallback = ui.playhead verb). */
  onSeek?: (atMs: number) => void
  /** Explicit snapshot refresh after transcript-only mutations. The normal
   * op_applied event remains primary, but a missed/reordered socket refresh
   * must not leave the source-word styling stale after a successful verb. */
  onProjectChanged?: () => void
}

/**
 * Default filler lexicon for the PRE-removal dotted-amber highlight only
 * so the agent's targets are visible before it acts. The default
 * until the perception sidecar's per-asset lexicon is wired; cutting itself
 * always goes through transcript.remove_fillers (server-side lexicon).
 */
const aggrFromInput = (value: string): '' | Aggressiveness =>
  value === 'calm' || value === 'natural' || value === 'jumpy' ? value : ''

export default function Transcript({ project, playheadMs, selectedClipId, transcripts, onCutWords, onRestore, onSeek, onProjectChanged }: TranscriptProps) {
  // --- transcripts: prop wins; panel self-loads the rest via transcript.get --
  const [loaded, setLoaded] = useState<Record<string, TranscriptData>>({})
  // Hold a successful ignore response until the authoritative App snapshot
  // catches up. A concurrent op_applied refresh can briefly deliver the
  // pre-mutation project and must not erase the confirmed ignored-word styling.
  const [ignoreOverride, setIgnoreOverride] = useState<NonNullable<Project['transcript_ignores']> | null>(null)
  // --- ops: needed to derive struck spans; self-subscribes (App owns the
  //     canonical copy for the rail; an optional `ops` prop is integration's
  //     call — self-loading keeps this panel correct standalone).
  const [ops, setOps] = useState<OpRecord[]>([])
  const [sel, setSel] = useState<Sel | null>(null)
  /** In-flight cut (verb sent, op_applied not yet seen) — pending tint, rule 8. */
  const [pendingCut, setPendingCut] = useState<Sel | null>(null)
  const [aggr, setAggr] = useState<'' | Aggressiveness>('')
  const [passBusy, setPassBusy] = useState<'' | 'silence' | 'fillers' | 'captions' | 'retakes' | 'chapters'>('')
  const [passNote, setPassNote] = useState('')
  // --- reel authoring (highlight reel) — pure VIEW state, zero-local-mutation contract ------------
  /** Reel mode toggle: when on, the selection toolbar offers "Add to reel"
   *  instead of building a one-shot cut. */
  const [reelMode, setReelMode] = useState(false)
  /** Ordered tray of queued spans (order = reel order). Single-asset scoped:
   *  the first add fixes the reel's asset; cross-asset adds are blocked. */
  const [reel, setReel] = useState<ReelSpan[]>([])
  /** Busy + outcome note for the assemble dispatch (honest success/error). */
  const [reelBusy, setReelBusy] = useState(false)
  const [reelNote, setReelNote] = useState('')
  /** Confirmed seek echo when App doesn't pass onSeek (set AFTER verb ok). */
  const [seekEcho, setSeekEcho] = useState<number | null>(null)
  // --- Perception (transcription engine) readiness ---------------------------
  // On a COLD install the Python/Whisper sidecar isn't set up, so media.import
  // never produces words and this panel would stay blank forever with a
  // misleading "pending" note. We probe the doctor's `perception` card: when it
  // is NOT ok (tier none / instruments-capable = no word-level STT) the empty
  // state explains it and offers a one-click `system.setup_perception` (the
  // matte-requirements pattern), rather than implying transcription is coming.
  /** null = probing/unknown, true = STT engine (onnx-asr/whisperX) ready, false = not set up. */
  const [perceptionReady, setPerceptionReady] = useState<boolean | null>(null)
  const [perceptionHint, setPerceptionHint] = useState<string | null>(null)
  /** Setup job in flight + its live progress message (WS job_progress). */
  const [setupBusy, setSetupBusy] = useState(false)
  const [setupMsg, setSetupMsg] = useState('')
  const [setupErr, setSetupErr] = useState<string | null>(null)
  // --- EDL-aware transcript views -------------------------------------
  /** 'timeline' = words mapped to the timeline (default); 'source' = legacy
   *  raw per-asset blobs (kept as a fallback for whole-source review). */
  const [txView, setTxView] = useState<'timeline' | 'source'>('timeline')
  /** Timeline scope: SELECTED-CLIP (default) or the whole PROGRAM output line. */
  const [txScope, setTxScope] = useState<'clip' | 'program'>('clip')
  /** transcript.timeline entries for the active scope. */
  const [tlEntries, setTlEntries] = useState<TimelineWord[]>([])

  // Read the perception card from the doctor → ready/hint. Cheap (cached read).
  const probePerception = useCallback(async () => {
    const report = await fetchDoctor(false)
    if (!report) return
    const card: DoctorCard | undefined = report.cards.find((c) => c.id === 'perception')
    // Word-level transcription needs the STT runtime specifically. Gate on the card's
    // `stt_ready` detail — NOT `status === 'ok'`: the card can be 'ok' (perception
    // instruments installed: silence/scenes/beats) while word-level STT is still absent,
    // which made the panel show "pending…" forever instead of the setup card (the
    // honest-empty regression). This matches the release check's `details.stt_ready`.
    setPerceptionReady(card ? card.details?.stt_ready === true : false)
    setPerceptionHint(card?.hint ?? null)
  }, [])

  // Probe on mount + whenever a project loads; live-update on doctor_updated
  // (system.setup_perception finishes → doctor_rescan → push) and surface the
  // setup job's progress message via job_progress.
  useEffect(() => {
    void probePerception()
    const off = events.subscribe((ev) => {
      if (ev.type === 'doctor_updated') {
        const card = ev.report.cards.find((c) => c.id === 'perception')
        const ready = card ? card.details?.stt_ready === true : false
        setPerceptionReady(ready)
        setPerceptionHint(card?.hint ?? null)
        if (ready) {
          setSetupBusy(false)
          setSetupMsg('')
        }
      } else if (ev.type === 'job_progress' && ev.kind === 'setup_perception') {
        setSetupMsg(ev.message ?? `setting up… ${Math.round(ev.progress * 100)}%`)
      }
    })
    return off
  }, [probePerception, project?.name])

  // One-click: provision the Python/Whisper sidecar. Long job (minutes) — the
  // WS job_progress drives the message; doctor_updated flips ready on finish.
  const startPerceptionSetup = useCallback(async () => {
    setSetupBusy(true)
    setSetupErr(null)
    setSetupMsg('starting…')
    const r = await setupPerception()
    if (!r) {
      setSetupBusy(false)
      setSetupErr('Could not start captions install (server unreachable?).')
    }
  }, [])

  // Fetch the EDL-mapped transcript for the active scope. Re-runs on project /
  // edit (ops) / selection / scope changes. SELECTED-CLIP with no selection →
  // empty (the view shows a "select a clip" hint). PROGRAM → whole output line.
  useEffect(() => {
    if (txView !== 'timeline' || !project) { setTlEntries([]); return }
    if (txScope === 'clip' && !selectedClipId) { setTlEntries([]); return }
    let cancelled = false
    const run = async () => {
      const args = txScope === 'clip' && selectedClipId ? { clip: selectedClipId } : {}
      const r = await callVerb('transcript.timeline', args)
      const entries = r.ok ? timelineEntriesFrom(r.result) : null
      if (!cancelled && entries) setTlEntries(entries)
      else if (!cancelled && !r.ok) setTlEntries([])
    }
    void run()
    return () => { cancelled = true }
    // `ops` re-fetches after every edit; project name covers project switches.
  }, [txView, txScope, selectedClipId, project?.name, ops])

  // Clip-scoped cut from the timeline view → transcript.cut_words{clip}.
  const cutTimelineWords = useCallback((asset: string, wordRange: [number, number], clipId: string | null) => {
    void runUserVerb('transcript.cut_words', {
      asset,
      word_range: wordRange,
      ...(clipId ? { clip: clipId } : {}),
      rationale: 'transcript (timeline view) cut',
    }, 'Could not remove the selected transcript words.')
  }, [])

  const seekTimeline = useCallback((atMs: number) => {
    if (onSeek) onSeek(atMs)
    else void callVerb('ui.playhead', { at_ms: atMs })
  }, [onSeek])

  const bodyRef = useRef<HTMLDivElement>(null)
  const mouseDownRef = useRef(false)
  const draggedRef = useRef(false)
  /** Last clicked word — the anchor a later shift-click extends from,
      surviving the plain-click path that clears the visual selection. */
  const lastDownRef = useRef<{ asset: string; idx: number } | null>(null)
  /** Auto-scroll pause: user wheel/drag pauses follow until this timestamp. */
  const pausedUntilRef = useRef(0)

  // PRUNE cached transcripts when the project changes — drop any whose asset is
  // no longer in the open project. Without this, creating/opening a NEW project
  // left the previous project's words rendered in the sidebar after a project
  // switch,
  // because `loaded` is component-local and survives the project switch.
  useEffect(() => {
    setLoaded((prev) => {
      const keep = Object.fromEntries(Object.entries(prev).filter(([id]) => project?.assets?.[id]))
      return Object.keys(keep).length === Object.keys(prev).length ? prev : keep
    })
  }, [project])

  // Load transcripts for assets that have one and aren't provided via props.
  useEffect(() => {
    if (!project) return
    for (const [assetId, asset] of Object.entries(project.assets)) {
      if (!asset.transcript || transcripts[assetId] || loaded[assetId]) continue
      void callVerb('transcript.get', { asset: assetId }).then((r) => {
        if (r.ok && r.result) {
          const transcript = r.result
          setLoaded((prev) => ({ ...prev, [assetId]: transcript }))
        }
      })
    }
  }, [project, transcripts, loaded])

  // Seed the op log + fold in live op_applied events (clears pending ghosts).
  useEffect(() => {
    void callVerb('project.ops', {}).then((r) => {
      if (r.ok && r.result) setOps(r.result.ops)
    })
    return events.subscribe((ev) => {
      if (ev.type === 'op_applied') {
        setOps((prev) => [...prev, ev.op])
        setPendingCut(null)
        if (ev.op.verb === 'project.undo' || ev.op.verb === 'project.redo') {
          setIgnoreOverride(null)
        }
      }
    })
  }, [])

  const all: Record<string, TranscriptData> = useMemo(
    () => ({ ...loaded, ...transcripts }),
    [loaded, transcripts],
  )
  // Only render transcripts for assets that exist in the CURRENT project — a
  // second guard (besides the prune above) so a stale cache entry can never leak
  // a previous project's words into a new one.
  const assetIds = useMemo(
    () => Object.keys(all).filter((id) => project?.assets?.[id]).sort(),
    [all, project],
  )

  // op-id → removed word ranges, per asset (Review/shared owns the rules).
  const cutsByAsset = useMemo(() => {
    const map: Record<string, CutSpan[]> = {}
    for (const span of activeCutSpans(ops)) (map[span.asset] ??= []).push(span)
    return map
  }, [ops])

  // --- playhead → active word (+ auto-scroll) -------------------------------
  // Map the timeline playhead back to the asset's source milliseconds by walking the
  // EDL (sourceAtPlayhead) BEFORE the word lookup. Transcript words are in
  // SOURCE time; after any cut the timeline and source clocks diverge, so
  // treating the playhead as source ms drifts by the removed duration before it.
  // sourceAtPlayhead resolves which clip covers the playhead and converts to
  // that clip's source time; we then look the word up only within that asset.
  const effPlayhead = onSeek ? playheadMs : (seekEcho ?? playheadMs)
  const active = useMemo(() => {
    const at = sourceAtPlayhead(project, effPlayhead)
    if (!at) return null
    const tr = all[at.asset]
    if (!tr) return null
    const w = tr.words.find((x) => x.start_ms <= at.srcMs && at.srcMs < x.end_ms)
    return w ? { asset: at.asset, idx: w.idx } : null
  }, [project, all, effPlayhead])

  useEffect(() => {
    if (!active || Date.now() < pausedUntilRef.current) return
    bodyRef.current
      ?.querySelector(`[data-asset="${CSS.escape(active.asset)}"][data-word-idx="${active.idx}"]`)
      ?.scrollIntoView({ block: 'center', behavior: 'smooth' })
  }, [active])

  const pauseFollow = useCallback(() => {
    pausedUntilRef.current = Date.now() + 3000 // resume after 3s idle
  }, [])

  // --- selection state machine: click=seek, drag/shift-click=range ----------
  const seek = useCallback(
    (atMs: number) => {
      if (onSeek) onSeek(atMs)
      else void seekPlayhead(atMs).then((ms) => ms !== null && setSeekEcho(ms))
    },
    [onSeek],
  )

  // transcript.search — find a phrase, jump to the first match. Searches the
  // first asset (the talking-head case); the word ranges it returns are also
  // what feed cut_words/assemble.
  const [searchQuery, setSearchQuery] = useState('')
  const [searchNote, setSearchNote] = useState<string | null>(null)
  // Secondary pass/reel controls live under a "Tools" menu so the
  // header stops overflowing/hiding buttons when the sidebar is narrow. Search
  // stays inline (the primary action); everything else is one click away.
  const [toolsOpen, setToolsOpen] = useState(false)
  const toolsRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!toolsOpen) return
    const onDown = (e: MouseEvent) => {
      if (!(e.target instanceof Node) || !toolsRef.current?.contains(e.target)) setToolsOpen(false)
    }
    const onEsc = (e: KeyboardEvent) => { if (e.key === 'Escape') setToolsOpen(false) }
    window.addEventListener('mousedown', onDown)
    window.addEventListener('keydown', onEsc)
    return () => {
      window.removeEventListener('mousedown', onDown)
      window.removeEventListener('keydown', onEsc)
    }
  }, [toolsOpen])
  const onSearch = useCallback(async () => {
    const q = searchQuery.trim()
    if (!q) return
    const asset = Object.keys(all)[0]
    if (!asset) { setSearchNote('no transcript'); return }
    const r = await callVerb('transcript.search', { asset, query: q })
    const result = r.ok ? searchResultFrom(r.result) : null
    if (result) {
      setSearchNote(`${result.match_count} match${result.match_count === 1 ? '' : 'es'}`)
      if (result.matches[0]) seek(result.matches[0].at_ms)
    } else setSearchNote(r.error?.code ?? 'no matches')
    setTimeout(() => setSearchNote(null), 4000)
  }, [searchQuery, all, seek])

  const onWordDown = useCallback(
    (asset: string, idx: number, ev: React.MouseEvent) => {
      ev.preventDefault() // suppress native text selection — spans are the unit
      mouseDownRef.current = true
      draggedRef.current = false
      // shift-click extends from the live selection OR the last clicked word
      const anchor = sel?.asset === asset ? sel.anchor : lastDownRef.current?.asset === asset ? lastDownRef.current.idx : null
      if (ev.shiftKey && anchor !== null) {
        setSel({ asset, anchor, head: idx }) // shift-click = range select
        draggedRef.current = true // not a seek-click
      } else {
        lastDownRef.current = { asset, idx }
        setSel({ asset, anchor: idx, head: idx })
      }
    },
    [sel],
  )

  const onWordEnter = useCallback((asset: string, idx: number) => {
    if (!mouseDownRef.current) return
    setSel((prev) => {
      if (!prev || prev.asset !== asset || prev.head === idx) return prev
      draggedRef.current = true
      return { ...prev, head: idx }
    })
  }, [])

  // Mouseup anywhere ends the drag; a no-drag release on a word = seek-click.
  useEffect(() => {
    const up = () => {
      if (!mouseDownRef.current) return
      mouseDownRef.current = false
      if (!draggedRef.current) {
        setSel((prev) => {
          if (prev && prev.anchor === prev.head) {
            const w = all[prev.asset]?.words[prev.anchor]
            if (w) seek(w.start_ms)
            return null // plain click: seek, no selection left behind
          }
          return prev
        })
      }
    }
    window.addEventListener('mouseup', up)
    return () => window.removeEventListener('mouseup', up)
  }, [all, seek])

  useEffect(() => {
    const esc = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setSel(null)
    }
    window.addEventListener('keydown', esc)
    return () => window.removeEventListener('keydown', esc)
  }, [])

  const doCut = useCallback(() => {
    if (!sel) return
    const range = selRange(sel)
    onCutWords(sel.asset, range) // human cut: rationale stays empty (the rationale-preservation contract — optional)
    setPendingCut(sel) // ghost tint until op_applied confirms (rule 8)
    setSel(null)
  }, [sel, onCutWords])

  // --- Non-destructive mute-word ---------------------------------------------
  // Per-asset UNION of mute ranges across audio-track clips (SOURCE ms) — the
  // muted-word styling source. Per-clip differences collapse to "muted
  // somewhere on the timeline" in this source view, which is the honest
  // summary a flowing transcript can show.
  const mutedByAsset = useMemo(() => {
    const m = new Map<string, Array<[number, number]>>()
    for (const t of project?.tracks ?? []) {
      if (t.kind !== 'audio') continue
      for (const c of t.clips ?? []) {
        const mc = c as { asset?: string; mute_ranges?: Array<[number, number]> }
        if (!mc.asset || !mc.mute_ranges?.length) continue
        m.set(mc.asset, [...(m.get(mc.asset) ?? []), ...mc.mute_ranges])
      }
    }
    return m
  }, [project])

  const ignoredByAsset = useMemo(() => {
    const m = new Map<string, Array<[number, number]>>()
    for (const r of ignoreOverride ?? project?.transcript_ignores ?? []) {
      if (!r.asset || !r.word_range) continue
      m.set(r.asset, [...(m.get(r.asset) ?? []), r.word_range])
    }
    return m
  }, [ignoreOverride, project?.transcript_ignores])

  useEffect(() => {
    if (ignoreOverride === null) return
    if (JSON.stringify(project?.transcript_ignores ?? []) === JSON.stringify(ignoreOverride)) {
      setIgnoreOverride(null)
    }
  }, [ignoreOverride, project?.transcript_ignores])

  useEffect(() => {
    setIgnoreOverride(null)
  }, [project?.active_sequence, project?.name])

  /** Mute the selected words in place (transcript.mute_words) — keeps timing. */
  const doMute = useCallback(() => {
    if (!sel) return
    void runUserVerb('transcript.mute_words', {
      asset: sel.asset,
      word_range: selRange(sel),
      rationale: 'transcript word mute (non-destructive)',
    }, 'Could not mute the selected words.').then((result) => {
      if (result?.ok) setSel(null)
    })
  }, [sel])

  /** Ignore selected words for transcript-derived outputs without cutting/muting. */
  const doIgnore = useCallback(() => {
    if (!sel) return
    void runUserVerb('transcript.ignore_words', {
      asset: sel.asset,
      word_range: selRange(sel),
      rationale: 'transcript word ignore (non-destructive)',
    }, 'Could not ignore the selected words.').then((r) => {
      if (!r?.ok) return
      setIgnoreOverride(r.result?.transcript_ignores ?? [])
      onProjectChanged?.()
      setSel(null)
    })
  }, [onProjectChanged, sel])

  /** Remove ignore state for the selected word span. */
  const doUnignore = useCallback(() => {
    if (!sel) return
    void runUserVerb('transcript.ignore_words', {
      asset: sel.asset,
      word_range: selRange(sel),
      remove: true,
      rationale: 'transcript word unignore',
    }, 'Could not restore the selected transcript words.').then((r) => {
      if (!r?.ok) return
      setIgnoreOverride(r.result?.transcript_ignores ?? [])
      onProjectChanged?.()
      setSel(null)
    })
  }, [onProjectChanged, sel])

  /** Surgically unmute the selected span (edit.mute_range remove_ms) on every
   *  audio clip of the asset whose mutes intersect it — other mutes untouched. */
  const doUnmute = useCallback(() => {
    if (!sel || !project) return
    const [lo, hi] = selRange(sel)
    const words = all[sel.asset]?.words ?? []
    const from = words[lo]
    const to = words[hi]
    if (!from || !to) return
    // Same ±40ms word-edge padding the mute used, so the whole gate lifts.
    const span: [number, number] = [Math.max(0, from.start_ms - 40), to.end_ms + 40]
    const requests: Array<ReturnType<typeof runUserVerb>> = []
    for (const t of project.tracks ?? []) {
      if (t.kind !== 'audio') continue
      for (const c of t.clips ?? []) {
        const mc = c as { id?: string; asset?: string; mute_ranges?: Array<[number, number]> }
        if (mc.asset !== sel.asset || !mc.id || !mc.mute_ranges?.length) continue
        if (!mc.mute_ranges.some((r) => r[0] < span[1] && r[1] > span[0])) continue
        requests.push(runUserVerb(
          'edit.mute_range',
          { clip: mc.id, remove_ms: span, rationale: 'transcript word unmute' },
          'Could not unmute the selected words.',
        ))
      }
    }
    void Promise.all(requests).then((results) => {
      if (results.every((result) => result?.ok)) setSel(null)
    })
  }, [sel, project, all])

  // --- reel: add the live selection to the tray ------------------------------
  /** The asset the reel is locked to (first span's asset; null = empty tray). */
  const reelAsset = reel[0]?.asset ?? null
  /** True when the current selection can be added: there IS one, and it matches
   *  the reel's locked asset (or the tray is empty). Single-asset scope guard. */
  const canAddSel = !!sel && (reelAsset === null || sel.asset === reelAsset)

  const addSelToReel = useCallback(() => {
    if (!sel) return
    // Single-asset scope: silently no-op + note if it's a foreign asset (the
    // button is disabled in that case, but guard the keyboard/agent path too).
    if (reelAsset !== null && sel.asset !== reelAsset) {
      setReelNote(`reel is scoped to ${reelAsset} — clear it to start one for ${sel.asset}`)
      return
    }
    const [lo, hi] = selRange(sel)
    const words = all[sel.asset]?.words ?? []
    setReel((prev) => [...prev, { asset: sel.asset, range: [lo, hi], snippet: reelSnippet(words, lo, hi) }])
    setReelNote('')
    setSel(null) // clear so the next span is a fresh selection
  }, [sel, reelAsset, all])

  const removeFromReel = useCallback((i: number) => {
    setReel((prev) => prev.filter((_, j) => j !== i))
  }, [])

  const clearReel = useCallback(() => {
    setReel([])
    setReelNote('')
  }, [])

  // --- reel: assemble the queued spans into a highlight reel ------------------
  // ONE transcript.assemble dispatch carries the tray's [lo,hi] list IN ORDER;
  // the engine appends one edit.insert op per span (visible in the review rail)
  // and the clips arrive on the timeline via op_applied — no local mutation.
  const assembleReel = useCallback(async () => {
    if (reel.length === 0 || reelBusy) return
    const asset = reel[0].asset
    const word_ranges = reel.map((s) => s.range)
    setReelBusy(true)
    setReelNote('')
    const r = await dispatchVerb('transcript.assemble', { asset, word_ranges, rationale: 'highlight reel' })
    setReelBusy(false)
    if (r.ok) {
      const result = isObject(r.result) ? r.result : null
      const placed = result ? numberField(result, 'spans_placed') ?? reel.length : reel.length
      const totalMs = result ? numberField(result, 'total_ms') ?? 0 : 0
      setReel([]) // clear the tray on success; clips render via op_applied
      setReelNote(`Reel: ${placed} span${placed === 1 ? '' : 's'}, ${fmtDur(totalMs)}`)
    } else {
      // Surface the engine's message verbatim (rule: never fake a result).
      setReelNote(`reel failed: ${r.error?.message ?? 'error'}`)
    }
  }, [reel, reelBusy])

  // --- silence / filler passes (header controls) -----------------------------
  const runSilencePass = useCallback(async () => {
    if (!aggr) return // the required-argument contract: aggressiveness REQUIRED — button stays disabled
    setPassBusy('silence')
    setPassNote('')
    const r = await dispatchVerb('transcript.remove_silences', { aggressiveness: aggr })
    setPassBusy('')
    setPassNote(r.ok ? `silence pass: ${r.op_ids?.length ?? 0} cuts` : `silence pass failed: ${r.error?.message ?? 'error'}`)
  }, [aggr])

  const runFillerPass = useCallback(async () => {
    setPassBusy('fillers')
    setPassNote('')
    const r = await dispatchVerb('transcript.remove_fillers', {})
    setPassBusy('')
    setPassNote(r.ok ? `filler pass: ${r.op_ids?.length ?? 0} cuts` : `filler pass failed: ${r.error?.message ?? 'error'}`)
  }, [])

  // "Remove retakes" — auto-remove repeated line attempts (retakes / do-overs),
  // keeping the best take (transcript.remove_retakes), a common talking-head chore;
  // mirrors the filler-pass pattern (dispatchVerb → ripple cuts via op_applied).
  // removed_takes:0 (no retakes found) is a clean no-op, reported honestly.
  const runRetakesPass = useCallback(async () => {
    setPassBusy('retakes')
    setPassNote('')
    const r = await dispatchVerb('transcript.remove_retakes', {})
    setPassBusy('')
    setPassNote(r.ok ? `retakes pass: ${r.op_ids?.length ?? 0} cuts` : `retakes pass failed: ${r.error?.message ?? 'error'}`)
  }, [])

  // "Generate chapters" — auto-segment the transcript into topic chapters
  // (transcript.chapters, NON-MUTATING) and DROP one timeline marker per chapter
  // start (edit.add_marker), so the chapters are visible + navigable on the ruler.
  // Uses the first transcribed asset (the talking-head case). Always ≥1 chapter
  // for a non-empty transcript; an un-transcribed asset → the verb's error.
  const runChapters = useCallback(async () => {
    const asset = Object.keys(all)[0]
    if (!asset) { setPassNote('chapters: no transcript yet'); return }
    setPassBusy('chapters')
    setPassNote('')
    const r = await callVerb('transcript.chapters', { asset })
    if (r.ok && r.result) {
      const chapters = chaptersOf(r.result)
      let dropped = 0
      for (const ch of chapters) {
        const m = await callVerb('edit.add_marker', { at_ms: Math.round(ch.start_ms), label: (ch.title || `Chapter ${dropped + 1}`).slice(0, 80), rationale: 'transcript: chapter marker' })
        if (m.ok) dropped++
      }
      setPassNote(`chapters: ${chapters.length} found, ${dropped} marker${dropped === 1 ? '' : 's'} added`)
    } else {
      setPassNote(`chapters failed: ${r.error?.message ?? r.error?.code ?? 'error'}`)
    }
    setPassBusy('')
  }, [all])

  // "Generate captions" builds a caption track from the transcript.
  // One verb, no args (server picks a default style);
  // the new caption clips render on the timeline's caption lane via op_applied
  // (zero local mutation — invariant 1). Note shows the op outcome honestly.
  const runGenerateCaptions = useCallback(async () => {
    setPassBusy('captions')
    setPassNote('')
    const r = await dispatchVerb('captions.generate', {})
    setPassBusy('')
    setPassNote(r.ok ? 'captions generated' : `captions failed: ${r.error?.message ?? 'error'}`)
  }, [])

  // --- floating Cut toolbar geometry (above the selection start word) -------
  const toolbar = useMemo(() => {
    if (!sel) return null
    const [a, b] = selRange(sel)
    const words = all[sel.asset]?.words
    if (!words) return null
    const from = words[a]
    const to = words[b]
    if (!from || !to) return null
    const range: [number, number] = [a, b]
    return { asset: sel.asset, range, count: b - a + 1, fromMs: from.start_ms, toMs: to.end_ms }
  }, [sel, all])
  const [toolbarXY, setToolbarXY] = useState<{ x: number; y: number } | null>(null)
  useEffect(() => {
    if (!toolbar || !bodyRef.current) {
      setToolbarXY(null)
      return
    }
    const el = bodyRef.current.querySelector<HTMLElement>(
      `[data-asset="${CSS.escape(toolbar.asset)}"][data-word-idx="${toolbar.range[0]}"]`,
    )
    if (!el) return
    // offsetTop is relative to the positioned scroller — toolbar floats above
    // the first selected word, clamped into the panel.
    setToolbarXY({ x: Math.max(8, Math.min(el.offsetLeft, bodyRef.current.clientWidth - 360)), y: Math.max(4, el.offsetTop - 44) })
  }, [toolbar])

  // --- render ----------------------------------------------------------------
  const hasProject = !!project
  const hasAssets = hasProject && Object.keys(project.assets).length > 0

  return (
    <section className="panel tx" data-panel="transcript" data-cut-panel="transcript">
      <div className="panel__header tx__header">
        {/* transcript.search — find a phrase, jump to the first match. The
            PRIMARY control, kept inline; the rest moves under Tools. */}
        <span className="tx__search">
          <input
            className="tx__search-input"
            data-cut-transcript-search
            placeholder="find phrase…"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') void onSearch() }}
          />
          {searchNote && <span className="tx__search-note" data-cut-search-note>{searchNote}</span>}
        </span>
        {passNote && <span className="tx__pass-note" title={passNote}>{passNote}</span>}
        {/* view toggle: SELECTED-CLIP / PROGRAM (timeline-mapped), or the
            legacy SOURCE blobs. Timeline views show only what's on the timeline. */}
        <span className="tx__viewtoggle" role="group" aria-label="Transcript view" data-cut-transcript-view={txView === 'timeline' ? txScope : 'source'}>
          <button
            type="button"
            className={`tx__viewbtn${txView === 'timeline' && txScope === 'clip' ? ' tx__viewbtn--on' : ''}`}
            data-cut-action="view-clip"
            onClick={() => { setTxView('timeline'); setTxScope('clip') }}
            title="The selected clip's words (cut trims that clip)"
          >Clip</button>
          <button
            type="button"
            className={`tx__viewbtn${txView === 'timeline' && txScope === 'program' ? ' tx__viewbtn--on' : ''}`}
            data-cut-action="view-program"
            onClick={() => { setTxView('timeline'); setTxScope('program') }}
            title="The whole output line in timeline order"
          >Program</button>
          <button
            type="button"
            className={`tx__viewbtn${txView === 'source' ? ' tx__viewbtn--on' : ''}`}
            data-cut-action="view-source"
            onClick={() => setTxView('source')}
            title="Raw per-source transcript (legacy whole-asset view)"
          >Source</button>
        </span>
        <span className="tx__header-spacer" />
        {/* Tools menu — the secondary pass/reel/caption actions, one click away.
            Reel-mode "on" tints the trigger so the active state stays visible
            even when the menu is closed. */}
        <div className="tx__tools" ref={toolsRef}>
          <button
            className={`tx__pass-btn tx__tools-btn${reelMode ? ' tx__pass-btn--on' : ''}`}
            data-cut-action="tools-menu"
            aria-haspopup="menu"
            aria-expanded={toolsOpen}
            onClick={() => setToolsOpen((v) => !v)}
            title="Transcript tools — silence/filler passes, captions, reel mode"
          >
            {reelMode ? 'Tools · Reel on' : 'Tools'}
            <Icon name="chevronDown" size={14} className="tx__tools-caret" />
          </button>
          {toolsOpen && (
            <div className="tx__tools-menu" role="menu" data-cut-tools-menu>
              {/* Reel mode toggle: turns the selection toolbar into "Add to reel". */}
              <button
                className={`tx__tools-item${reelMode ? ' tx__tools-item--on' : ''}`}
                role="menuitemcheckbox"
                aria-checked={reelMode}
                data-cut-action="reel-mode"
                disabled={passBusy !== ''}
                onClick={() => setReelMode((v) => !v)}
                title="Select moments, add them to the reel, then assemble the highlight"
              >
                <span className="tx__tools-check">{reelMode ? <Icon name="check" size={14} tone="success" /> : null}</span>
                Reel mode
              </button>
              <div className="tx__tools-sep" />
              {/* Silence pass + its REQUIRED aggressiveness (the required-argument contract). */}
              <div className="tx__tools-row">
                <button
                  className="tx__tools-item tx__tools-item--inline"
                  role="menuitem"
                  data-cut-action="silence-pass"
                  disabled={!aggr || passBusy !== ''}
                  onClick={() => void runSilencePass()}
                  title="Remove long pauses using the selected intensity"
                >
                  {passBusy === 'silence' ? 'cutting…' : 'Silence pass'}
                </button>
                <select
                  className="tx__aggr"
                  data-cut-aggressiveness=""
                  value={aggr}
                  onChange={(e) => setAggr(aggrFromInput(e.target.value))}
                  title="Silence-pass aggressiveness (required)"
                >
                  <option value="">aggr…</option>
                  <option value="calm">calm</option>
                  <option value="natural">natural</option>
                  <option value="jumpy">jumpy</option>
                </select>
              </div>
              <button
                className="tx__tools-item"
                role="menuitem"
                data-cut-action="filler-pass"
                disabled={passBusy !== ''}
                onClick={() => void runFillerPass()}
                title="Remove filler words such as um and uh"
              >
                {passBusy === 'fillers' ? 'cutting…' : 'Filler pass'}
              </button>
              {/* Remove retakes — auto-cut repeated line attempts, keep the best
                  take (transcript.remove_retakes). */}
              <button
                className="tx__tools-item"
                role="menuitem"
                data-cut-action="retakes-pass"
                disabled={passBusy !== ''}
                onClick={() => void runRetakesPass()}
                title="Remove repeated line attempts while keeping the best take"
              >
                {passBusy === 'retakes' ? 'cutting…' : 'Remove retakes'}
              </button>
              <div className="tx__tools-sep" />
              {/* Generate chapters — topic chapters as ruler markers
                  (transcript.chapters → edit.add_marker per chapter). */}
              <button
                className="tx__tools-item"
                role="menuitem"
                data-cut-action="generate-chapters"
                disabled={passBusy !== ''}
                onClick={() => void runChapters()}
                title="Find topic changes and add chapter markers"
              >
                {passBusy === 'chapters' ? 'segmenting…' : 'Generate chapters'}
              </button>
              <div className="tx__tools-sep" />
              {/* Generate captions through captions.generate. */}
              <button
                className="tx__tools-item"
                role="menuitem"
                data-cut-action="generate-captions"
                disabled={passBusy !== ''}
                onClick={() => void runGenerateCaptions()}
                title="Build a caption track from the transcript"
              >
                {passBusy === 'captions' ? 'generating…' : 'Generate captions'}
              </button>
              {/* 0.5.0: Animate — opens the kinetic-captions drawer (captions.kinetic). */}
              <button
                className="tx__tools-item"
                role="menuitem"
                data-cut-action="open-kinetic"
                disabled={passBusy !== ''}
                onClick={() => { setToolsOpen(false); document.dispatchEvent(new CustomEvent('cut:open-kinetic')) }}
                title="Animate the captions so each line pops in and fades out with speech"
              >
                Animate captions
              </button>
            </div>
          )}
        </div>
      </div>
      {/* Reel tray — ordered, removable spans (order = reel order). Pure VIEW
          state; the reel is built ONLY by the Assemble dispatch (zero-local-mutation contract). */}
      {reelMode && (
        <ReelTray
          reel={reel}
          reelAsset={reelAsset}
          reelBusy={reelBusy}
          reelNote={reelNote}
          onClear={clearReel}
          onAssemble={() => void assembleReel()}
          onRemove={removeFromReel}
        />
      )}
      <div className="panel__body tx__body" ref={bodyRef} onWheel={pauseFollow} onMouseDown={pauseFollow}>
        {!hasAssets && (
          hasProject ? (
            <button
              type="button"
              className="tx__empty tx__empty--cta"
              data-cut-import-cta
              onClick={() => document.dispatchEvent(new CustomEvent('cut:open-import'))}
            >
              ⬑ Import media to begin
            </button>
          ) : (
            <div className="tx__empty">Create a project in Projects to begin</div>
          )
        )}
        {hasAssets && assetIds.length === 0 && (
          perceptionReady === false ? (
            <TranscriptSetupCard
              hint={perceptionHint}
              error={setupErr}
              busy={setupBusy}
              message={setupMsg}
              onInstall={() => void startPerceptionSetup()}
            />
          ) : (
            // Engine ready (or still probing) → transcripts are genuinely coming.
            <div className="tx__empty" data-cut-transcribe-pending>
              Transcribing… words appear here live
            </div>
          )
        )}
        {/* : timeline-mapped views (Clip / Program) are the default; the
            legacy raw per-source view is kept behind the Source toggle. */}
        {hasAssets && assetIds.length > 0 && txView === 'timeline' && (
          <TimelineView
            entries={tlEntries}
            scope={txScope}
            playheadMs={playheadMs}
            onSeek={seekTimeline}
            onCut={cutTimelineWords}
          />
        )}
        {hasAssets && assetIds.length > 0 && txView === 'source' && assetIds.map((assetId) => (
          <AssetWords
            key={assetId}
            assetId={assetId}
            words={all[assetId].words}
            cuts={cutsByAsset[assetId] ?? []}
            muted={mutedByAsset.get(assetId) ?? []}
            ignored={ignoredByAsset.get(assetId) ?? []}
            activeIdx={active?.asset === assetId ? active.idx : -1}
            sel={sel?.asset === assetId ? selRange(sel) : null}
            pending={pendingCut?.asset === assetId ? selRange(pendingCut) : null}
            onWordDown={onWordDown}
            onWordEnter={onWordEnter}
            onRestore={onRestore}
          />
        ))}
        {toolbar && toolbarXY && (
          <div className="tx__cut-toolbar" style={{ left: toolbarXY.x, top: toolbarXY.y }} data-cut-toolbar="">
            {/* Reel mode → primary action is "Add to reel"; Cut stays available
                as a secondary action. canAddSel enforces single-asset scope. */}
            {reelMode ? (
              <button
                className="tx__cut-btn"
                data-cut-action="add-to-reel"
                disabled={!canAddSel}
                onMouseDown={(e) => e.preventDefault()}
                onClick={addSelToReel}
                title={canAddSel ? 'Add this span to the reel' : `reel is scoped to ${reelAsset} — clear it first`}
              >
                Add to reel
              </button>
            ) : (
              <>
                <button className="tx__cut-btn" data-cut-action="cut-words" onMouseDown={(e) => e.preventDefault()} onClick={doCut}>
                  Cut {toolbar.count} word{toolbar.count === 1 ? '' : 's'}
                </button>
                {(ignoredByAsset.get(toolbar.asset) ?? []).some((r) => r[0] <= toolbar.range[1] && r[1] >= toolbar.range[0]) ? (
                  <button
                    className="tx__cut-btn tx__ignore-btn"
                    data-cut-action="unignore-words"
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={doUnignore}
                    title="Use these words again in generated captions and reels"
                  >
                    Unignore
                  </button>
                ) : (
                  <button
                    className="tx__cut-btn tx__ignore-btn"
                    data-cut-action="ignore-words"
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={doIgnore}
                    title="Hide these words from generated captions and reels without cutting or muting"
                  >
                    Ignore
                  </button>
                )}
                {(mutedByAsset.get(toolbar.asset) ?? []).some((r) => r[0] < toolbar.toMs && r[1] > toolbar.fromMs) ? (
                  <button
                    className="tx__cut-btn tx__mute-btn"
                    data-cut-action="unmute-words"
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={doUnmute}
                    title="Remove the silence over these words; other muted sections stay"
                  >
                    Unmute
                  </button>
                ) : (
                  <button
                    className="tx__cut-btn tx__mute-btn"
                    data-cut-action="mute-words"
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={doMute}
                    title="Silence these words in place; timing and sync stay unchanged"
                  >
                    Mute
                  </button>
                )}
              </>
            )}
            <span className="tx__cut-tc">
              {fmtTc(toolbar.fromMs)} – {fmtTc(toolbar.toMs)}
            </span>
          </div>
        )}
      </div>
    </section>
  )
}
