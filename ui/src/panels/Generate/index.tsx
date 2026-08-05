// panels/Generate — the "Generate (AI)" surface for assets.generate.
//
// Role: the human UI for the agent-only assets.generate verb — generate an
// image or short video from a text prompt via the USER'S OWN codex/grok CLI, and
// import the result straight into the open project as a normal asset (it lands in
// the Assets tray). Makes the agent-only verb a discoverable user feature in the
// LEFT sidebar beside Assets/Library.
//
// PAID-GEN GUARD (critical — this spends real money): assets.generate runs the
// user's OWN generation CLI, which can incur provider cost / need sign-in. So the
// Generate button is a TWO-STEP arm-then-confirm control: the first click ARMS it
// (the button becomes an explicit "Confirm — run my CLI" with a cost warning), the
// second click actually dispatches the verb. The warning copy states plainly that
// it generates via the user's own CLI/credits. No generation can fire on a single
// stray click.
//
// HONEST degradation mirrors the verb: an empty prompt, unsupported kind, or an
// absent/un-signed-in CLI fails the queued job with a structured reason (NEVER a
// fake asset). We surface that reason; success names the imported asset id.
//
// Provider work runs through the persisted job queue. The panel polls the active
// job, survives a remount through localStorage, and exposes explicit cancellation.
//
// Caller: GenerateTemplatesWorkspace in LeftPanel (mounted as the Generate tab).
// This surface is deliberately embedded; the former direct-mount modal branch had
// no product caller and exposed a Close action that users and agents could never
// reach. Deps: lib/client (callVerb), ../drawer.css (shared cd-* styles).

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  callVerb,
  mediaClipTimelineDurationMs,
  type AssetsGenerateJobResult,
  type AssetsGenerateResult,
  type GeneratedAssetRecord,
  type GeneratedAssetPlacement,
  type Project,
  type VerbArgs,
} from '../../lib/client'
import { baseVideoTrackId } from '../../lib/layerStack'
import { Icon } from '../../icons'
import GenerationHistory, { GenerationReferences } from './GenerationHistory'
import '../drawer.css'
import './generate.css'

export interface GenerateDrawerProps {
  project: Project | null
  playheadMs?: number
  selectedClipId?: string | null
  /** Refresh the project snapshot after a successful import (so the new asset shows
   *  in the Assets tray immediately). Wired to App.resync. */
  onGenerated?: () => void
}

/** assets.generate provider: codex = gpt-image (image only); grok = grok-imagine
 *  (image + video). The kind toggle is constrained to what the provider supports. */
type Provider = 'codex' | 'grok'
type Kind = 'image' | 'video'
type PlacementMode = 'asset' | 'insert' | 'replace'
type RetryPlacement = { mode: 'replace'; target_clip: string }

function secondsLabel(ms: number): string {
  const seconds = ms / 1000
  return `${Number.isInteger(seconds) ? seconds : seconds.toFixed(1)} s`
}

interface StoredGenerationJob {
  job_id: string
  retry_placement?: RetryPlacement
}

const PROVIDERS: { id: Provider; label: string; kinds: Kind[] }[] = [
  { id: 'codex', label: 'Codex — gpt-image (images)', kinds: ['image'] },
  { id: 'grok', label: 'Grok — grok-imagine (images + video)', kinds: ['image', 'video'] },
]

const GENERATION_JOB_STORAGE_KEY = 'cut.generate.active-job'

function selectedVideoMedia(project: Project | null, clipId?: string | null) {
  if (!project || !clipId) return null
  for (const track of project.tracks) {
    if (track.kind !== 'video') continue
    const clip = track.clips.find((item) => 'id' in item && item.id === clipId)
    if (!clip || !('asset' in clip)) continue
    return {
      clipId,
      trackId: track.id,
      locked: track.locked === true,
      durationMs: mediaClipTimelineDurationMs(clip),
    }
  }
  return null
}

function retryFromPlacement(placement?: GeneratedAssetPlacement | null): RetryPlacement | null {
  return placement?.target_clip ? { mode: 'replace', target_clip: placement.target_clip } : null
}

function readStoredGenerationJob(value: string | null): StoredGenerationJob | null {
  if (!value) return null
  try {
    const parsed = JSON.parse(value) as Partial<StoredGenerationJob>
    if (typeof parsed.job_id === 'string') {
      return {
        job_id: parsed.job_id,
        retry_placement: parsed.retry_placement?.mode === 'replace'
          && typeof parsed.retry_placement.target_clip === 'string'
          ? parsed.retry_placement
          : undefined,
      }
    }
  } catch {
    // Legacy builds stored the bare job id.
  }
  return { job_id: value }
}

function providerFromInput(value: string, fallback: Provider): Provider {
  for (const provider of PROVIDERS) {
    if (provider.id === value) return provider.id
  }
  return fallback
}

export default function GenerateDrawer({
  project,
  playheadMs = 0,
  selectedClipId,
  onGenerated,
}: GenerateDrawerProps) {
  const [provider, setProvider] = useState<Provider>('codex')
  const [kind, setKind] = useState<Kind>('image')
  const [prompt, setPrompt] = useState('')
  const [model, setModel] = useState('')
  const [references, setReferences] = useState<string[]>([])
  const [variation, setVariation] = useState<string | null>(null)
  const [placementMode, setPlacementMode] = useState<PlacementMode>('asset')
  const [insertTrack, setInsertTrack] = useState('')
  const [insertDurationMs, setInsertDurationMs] = useState(5000)
  const [retryPlacement, setRetryPlacement] = useState<RetryPlacement | null>(null)
  const [chosenAssetId, setChosenAssetId] = useState<string | null>(null)
  const [historyBusy, setHistoryBusy] = useState(false)
  const [history, setHistory] = useState<GeneratedAssetRecord[]>([])
  const [historyLoading, setHistoryLoading] = useState(false)
  const [busy, setBusy] = useState(false)
  // Two-step paid-gen guard: false = idle, true = ARMED (next click actually runs
  // the user's CLI). Any edit to the prompt/provider/kind disarms it again, so the
  // confirm always reflects exactly what is about to be generated.
  const [armed, setArmed] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [jobId, setJobId] = useState<string | null>(null)
  const [phase, setPhase] = useState('')
  const activeJobRef = useRef<string | null>(null)
  const activeRetryRef = useRef<RetryPlacement | null>(null)
  const pollTimer = useRef<number | null>(null)
  const hasProject = !!project
  const videoTracks = useMemo(
    () => project?.tracks.filter((track) => track.kind === 'video') ?? [],
    [project],
  )
  const selectedMedia = useMemo(
    () => selectedVideoMedia(project, selectedClipId),
    [project, selectedClipId],
  )
  const selectedReplaceReady = !!selectedMedia && !selectedMedia.locked
  const preferredInsertTrack = project
    ? baseVideoTrackId(project.tracks) ?? videoTracks[0]?.id ?? ''
    : ''
  const insertTrackReady = videoTracks.some((track) => track.id === insertTrack && track.locked !== true)
  const historyScope = project
    ? `${project.name}\u0000${Object.entries(project.assets ?? {}).map(([id, asset]) => `${id}:${asset.hash}`).sort().join('\u0000')}`
    : ''

  const meta = PROVIDERS.find((p) => p.id === provider)!

  // Keep the kind valid for the selected provider (codex = image only).
  useEffect(() => {
    if (!meta.kinds.includes(kind)) setKind(meta.kinds[0])
  }, [provider]) // eslint-disable-line react-hooks/exhaustive-deps

  // Editing any generation input disarms the confirm — you must re-confirm the exact
  // thing you're about to pay to generate.
  const disarm = () => {
    setArmed(false)
    setRetryPlacement(null)
  }

  useEffect(() => {
    if (videoTracks.some((track) => track.id === insertTrack && track.locked !== true)) return
    const preferred = videoTracks.find((track) => track.id === preferredInsertTrack && track.locked !== true)
      ?? videoTracks.find((track) => track.locked !== true)
    setInsertTrack(preferred?.id ?? '')
  }, [project, insertTrack, preferredInsertTrack, videoTracks])

  const refreshHistory = useCallback(async () => {
    if (!hasProject) {
      setHistory([])
      setHistoryLoading(false)
      return
    }
    setHistoryLoading(true)
    try {
      const response = await callVerb('assets.generated_list', { limit: 100 })
      setHistory(response.ok ? response.result?.items ?? [] : [])
    } catch {
      setHistory([])
    } finally {
      setHistoryLoading(false)
    }
  }, [hasProject])

  useEffect(() => { void refreshHistory() }, [historyScope, refreshHistory])

  useEffect(() => {
    const available = new Set(Object.keys(project?.assets ?? {}))
    setReferences((current) => {
      const next = current.filter((id) => available.has(id))
      return next.length === current.length ? current : next
    })
  }, [project])

  const finishGeneration = useCallback((res: AssetsGenerateJobResult, retry: RetryPlacement | null) => {
    const action = res.generated.reused ? 'Reused' : 'Generated'
    const id = res.generated.generation_id ? ` · ${res.generated.generation_id}` : ''
    if (res.placement?.state === 'failed') {
      const reason = res.placement.error?.message ?? 'the timeline target changed before placement'
      setErr(`Generated asset ${res.asset_id} is ready, but placement failed: ${reason}`)
      setRetryPlacement(retry)
      setNote(`${action} → ${res.asset_id}${id}. The imported asset was kept; retry can reuse it without another provider run.`)
    } else {
      setRetryPlacement(null)
      setNote(`${action} → ${res.asset_id}${id}${res.placement?.state === 'applied' ? ' · placed on timeline' : ''}. ${res.generated.cost_note}`)
    }
    onGenerated?.()
    void refreshHistory()
  }, [onGenerated, refreshHistory])

  const toggleReference = (assetId: string) => {
    setReferences((current) => {
      if (current.includes(assetId)) return current.filter((id) => id !== assetId)
      if (current.length >= 4) {
        setErr('Generation accepts at most four visual references.')
        return current
      }
      setErr(null)
      return [...current, assetId]
    })
    disarm()
  }

  const prepareVariation = (record: GeneratedAssetRecord) => {
    if (!record.provider || !record.kind) return
    setProvider(record.provider)
    setKind(record.kind)
    setPrompt(record.prompt)
    setModel(record.model ?? '')
    setReferences(record.reference_asset_ids.filter((id) => project?.assets?.[id]).slice(0, 4))
    setVariation(`take-${Date.now().toString(36)}`)
    setArmed(false)
    setRetryPlacement(null)
    setErr(null)
    setNote('New variation prepared. Review the inputs, then use the two-step generation confirmation.')
  }

  const pollGeneration = useCallback((id: string, delayMs = 600) => {
    const poll = async () => {
      if (activeJobRef.current !== id) return
      let response
      try {
        response = await callVerb('jobs.status', { job_id: id })
      } catch {
        if (activeJobRef.current !== id) return
        setPhase('Waiting for server…')
        pollTimer.current = window.setTimeout(() => void poll(), 1200)
        return
      }
      if (activeJobRef.current !== id) return
      const job = response.ok ? response.result : undefined
      if (job?.state === 'done') {
        const retry = activeRetryRef.current
        activeJobRef.current = null
        activeRetryRef.current = null
        localStorage.removeItem(GENERATION_JOB_STORAGE_KEY)
        setJobId(null); setBusy(false); setPhase('')
        finishGeneration(job.result as AssetsGenerateJobResult, retry)
        return
      }
      if (job?.state === 'failed') {
        const retry = activeRetryRef.current
        activeJobRef.current = null
        activeRetryRef.current = null
        localStorage.removeItem(GENERATION_JOB_STORAGE_KEY)
        setJobId(null); setBusy(false); setPhase('')
        setRetryPlacement(retry)
        setErr(job.error?.code === 'job_cancelled'
          ? 'Generation cancelled.'
          : `${job.error?.code ?? 'generation_failed'}: ${job.error?.message ?? 'generation failed'}`)
        return
      }
      if (!response.ok) {
        const retry = activeRetryRef.current
        activeJobRef.current = null
        activeRetryRef.current = null
        localStorage.removeItem(GENERATION_JOB_STORAGE_KEY)
        setJobId(null); setBusy(false); setPhase('')
        setRetryPlacement(retry)
        setErr(`${response.error?.code ?? 'failed'}: ${response.error?.message ?? 'generation job was not found'}`)
        return
      }
      const pct = Math.round((job?.progress ?? 0) * 100)
      setPhase(job?.state === 'queued' ? 'Queued…' : `Generating… ${pct}%`)
      pollTimer.current = window.setTimeout(() => void poll(), 600)
    }
    pollTimer.current = window.setTimeout(() => void poll(), delayMs)
  }, [finishGeneration])

  useEffect(() => {
    const active = readStoredGenerationJob(localStorage.getItem(GENERATION_JOB_STORAGE_KEY))
    if (active) {
      activeJobRef.current = active.job_id
      activeRetryRef.current = active.retry_placement ?? null
      setJobId(active.job_id); setBusy(true); setPhase('Resuming…')
      pollGeneration(active.job_id, 0)
    }
    return () => {
      if (pollTimer.current) window.clearTimeout(pollTimer.current)
    }
  }, [pollGeneration])

  const canGenerate = !!prompt.trim() && !busy

  const requestedPlacement = (): VerbArgs['assets.generate']['placement'] => {
    if (retryPlacement) return retryPlacement
    if (placementMode === 'replace' && selectedReplaceReady && selectedMedia) {
      return { mode: 'replace', target_clip: selectedMedia.clipId }
    }
    if (placementMode === 'insert' && insertTrackReady) {
      return {
        mode: 'insert',
        track: insertTrack,
        at_ms: Math.max(0, Math.round(playheadMs)),
        duration_ms: Math.max(1, Math.round(insertDurationMs)),
      }
    }
    return undefined
  }

  // First click ARMS (shows the cost confirm); second click DISPATCHES. This is the
  // money gate — a real generation only fires on the explicit confirm.
  const onGenerateClick = async () => {
    if (!prompt.trim()) { setErr('Describe what to generate first.'); return }
    if (!project) { setErr('Create or open a project first — the generated media imports into it.'); return }
    if (!retryPlacement && placementMode === 'insert' && !insertTrackReady) {
      setErr('Choose an unlocked video track for the pending timeline slot.')
      return
    }
    if (!retryPlacement && placementMode === 'replace' && !selectedReplaceReady) {
      setErr('Select an unlocked video media clip to replace.')
      return
    }
    if (!armed) { setErr(null); setNote(null); setArmed(true); return }

    setArmed(false)
    setBusy(true); setErr(null); setNote(null)
    try {
      // Only the schema-required + chosen args: prompt + provider + kind. model and
      // timeout_ms are left to the server default (the verb clamps timeout 10s–30m).
      const placement = requestedPlacement()
      const r = await callVerb('assets.generate', {
        prompt: prompt.trim(),
        provider,
        kind,
        model: model.trim() || undefined,
        references: references.length > 0 ? references : undefined,
        variation: variation ?? undefined,
        placement,
        rationale: `human: generate ${kind} via ${provider} CLI`,
      })
      const res: AssetsGenerateResult | undefined = r.ok ? r.result : undefined
      if (r.ok && res?.job_id) {
        const retry = retryFromPlacement(res.placement)
        activeJobRef.current = res.job_id
        activeRetryRef.current = retry
        localStorage.setItem(GENERATION_JOB_STORAGE_KEY, JSON.stringify({
          job_id: res.job_id,
          retry_placement: retry ?? undefined,
        } satisfies StoredGenerationJob))
        setRetryPlacement(null)
        setJobId(res.job_id); setPhase('Queued…')
        if (res.placement?.mode === 'insert') onGenerated?.()
        pollGeneration(res.job_id)
      } else {
        setBusy(false)
        setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'generation failed'}`)
      }
    } catch {
      setErr('server unreachable')
      setBusy(false)
    }
  }

  const cancelGeneration = async () => {
    const active = activeJobRef.current
    if (!active) return
    setPhase('Cancelling…')
    let response
    try {
      response = await callVerb('jobs.cancel', { job_id: active })
    } catch {
      setPhase('Waiting for server…')
      setErr('Server unreachable; the generation job is still being tracked.')
      return
    }
    if (!response.ok) {
      setErr(`${response.error?.code ?? 'failed'}: ${response.error?.message ?? 'could not cancel generation'}`)
      return
    }
    const retry = activeRetryRef.current
    activeJobRef.current = null
    activeRetryRef.current = null
    localStorage.removeItem(GENERATION_JOB_STORAGE_KEY)
    if (pollTimer.current) window.clearTimeout(pollTimer.current)
    setRetryPlacement(retry)
    setJobId(null); setBusy(false); setPhase('')
    setErr(retry
      ? 'Generation cancelled. The pending timeline slot was kept for retry.'
      : 'Generation cancelled.')
  }

  const prepareRetry = () => {
    if (!retryPlacement) return
    setErr(null)
    setNote(`Retry will replace pending clip ${retryPlacement.target_clip}. Confirming the request remains a separate paid-provider action; an unchanged completed take is reused.`)
    setArmed(true)
  }

  const placeExisting = async (record: GeneratedAssetRecord, mode: 'insert' | 'replace') => {
    if (!project || record.integrity !== 'verified') return
    setHistoryBusy(true); setErr(null); setNote(null); setChosenAssetId(record.asset_id)
    try {
      const response = mode === 'insert'
        ? await callVerb('edit.insert', {
          asset: record.asset_id,
          track: insertTrack,
          at_ms: Math.max(0, Math.round(playheadMs)),
          duration_ms: record.kind === 'image' ? Math.max(1, Math.round(insertDurationMs)) : undefined,
          rationale: `human: insert generated take ${record.generation_id}`,
        })
        : await callVerb('edit.replace', {
          target_clip: selectedMedia?.clipId ?? '',
          asset: record.asset_id,
          source_out_ms: record.kind === 'image' ? selectedMedia?.durationMs : undefined,
          link_audio: false,
          rationale: `human: replace with generated take ${record.generation_id}`,
        })
      if (!response.ok) {
        setErr(`${response.error?.code ?? 'failed'}: ${response.error?.message ?? `could not ${mode} generated media`}`)
        return
      }
      setNote(`${mode === 'insert' ? 'Inserted' : 'Replaced with'} ${record.generation_id}.`)
      onGenerated?.()
    } catch {
      setErr(`server unreachable while trying to ${mode} generated media`)
    } finally {
      setHistoryBusy(false)
    }
  }

  const body = (
    <div className="cd-body" data-cut-generate-body>
      {/* provider */}
      <label className="cd-field">
        <span className="cd-field-label">Generator (your own CLI)</span>
        <select
          className="cd-sel"
          data-cut-generate-provider
          value={provider}
          onChange={(e) => { setProvider(providerFromInput(e.target.value, provider)); disarm() }}
        >
          {PROVIDERS.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
        </select>
      </label>

      {/* kind toggle (image | video) — constrained to what the provider supports */}
      <div className="cd-field">
        <span className="cd-field-label">Kind</span>
        <div className="cd-seg" role="tablist" data-cut-generate-kind>
          {meta.kinds.map((k) => (
            <button
              key={k} role="tab" aria-selected={kind === k}
              className={`cd-seg-btn ${kind === k ? 'cd-seg-btn--on' : ''}`}
              data-cut-generate-kind-opt={k}
              onClick={() => { setKind(k); disarm() }}
            >
              <Icon name={k === 'video' ? 'videoClip' : 'eye'} size={14} tone={k === 'video' ? 'media' : 'asset'} /> {k}
            </button>
          ))}
        </div>
      </div>

      {/* prompt */}
      <label className="cd-field">
        <span className="cd-field-label">Describe the {kind}</span>
        <textarea
          className="cd-input cd-textarea"
          data-cut-generate-prompt
          autoFocus
          rows={4}
          placeholder={kind === 'video'
            ? 'e.g. a slow drone shot over a misty pine forest at dawn'
            : 'e.g. a flat-design icon of a rocket on a navy background'}
          value={prompt}
          onChange={(e) => { setPrompt(e.target.value); disarm() }}
        />
      </label>

      <GenerationReferences
        project={project}
        selected={references}
        disabled={busy}
        onToggle={toggleReference}
      />

      <details className="cd-advanced" data-cut-generate-advanced>
        <summary data-cut-generate-advanced-toggle>Advanced</summary>
        <input
          className="cd-input cd-input--mono"
          data-cut-generate-model
          value={model}
          disabled={busy}
          placeholder="Provider default model"
          aria-label="Generation model override"
          onChange={(event) => { setModel(event.target.value); disarm() }}
        />
        <p className="cd-note">Leave blank to use the provider default.</p>
      </details>

      {variation && (
        <div className="gen-variant" data-cut-generate-variation={variation}>
          <span>Variation</span>
          <code>{variation}</code>
          <button
            className="cd-btn cd-btn--ghost cd-btn--sm"
            data-cut-generate-variation-clear
            title="Return to the reusable base take"
            disabled={busy}
            onClick={() => { setVariation(null); disarm() }}
          >
            Clear
          </button>
        </div>
      )}

      <section className="gen-placement" data-cut-generate-placement>
        <span className="cd-field-label">Destination</span>
        <div className="cd-seg" role="tablist" aria-label="Generated media destination">
          {([
            ['asset', 'Assets'],
            ['insert', 'Insert'],
            ['replace', 'Replace'],
          ] as const).map(([mode, label]) => (
            <button
              key={mode}
              role="tab"
              aria-selected={placementMode === mode}
              className={`cd-seg-btn ${placementMode === mode ? 'cd-seg-btn--on' : ''}`}
              data-cut-generate-placement-mode={mode}
              disabled={busy || (mode === 'replace' && !selectedReplaceReady)}
              title={mode === 'asset'
                ? 'Import the generated media without changing the timeline'
                : mode === 'insert'
                  ? 'Reserve a visible slot at the playhead, then fill it on success'
                  : selectedReplaceReady
                    ? 'Keep the selected clip slot and replace its source on success'
                    : 'Select an unlocked video media clip first'}
              onClick={() => { setPlacementMode(mode); disarm() }}
            >
              <Icon name={mode === 'asset' ? 'assets' : mode === 'insert' ? 'plus' : 'reset'} size={14} /> {label}
            </button>
          ))}
        </div>
        {placementMode === 'insert' && (
          <div className="gen-placement__row" data-cut-generate-placement-insert>
            <label>
              Video track
              <select
                className="cd-sel"
                value={insertTrack}
                disabled={busy}
                data-cut-generate-placement-track
                onChange={(event) => { setInsertTrack(event.target.value); disarm() }}
              >
                {videoTracks.map((track) => (
                  <option key={track.id} value={track.id} disabled={track.locked === true}>
                    {track.id}{track.locked ? ' (locked)' : ''}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Slot seconds
              <input
                className="cd-input cd-input--mono"
                type="number"
                min={0.1}
                max={3600}
                step={0.1}
                value={insertDurationMs / 1000}
                disabled={busy}
                data-cut-generate-placement-duration
                onChange={(event) => {
                  setInsertDurationMs(Math.max(100, Math.min(3600000, Math.round((Number(event.target.value) || 0.1) * 1000))))
                  disarm()
                }}
              />
            </label>
          </div>
        )}
        {placementMode === 'insert' && (
          <p className="gen-placement__target">
            Playhead <code>{secondsLabel(Math.max(0, Math.round(playheadMs)))}</code>. The pending slot stays on failure or cancellation.
          </p>
        )}
        {placementMode === 'replace' && (
          <p className="gen-placement__target" data-cut-generate-placement-target={selectedMedia?.clipId}>
            Selected clip <code>{selectedMedia?.clipId ?? 'none'}</code>{selectedMedia ? ` · ${secondsLabel(selectedMedia.durationMs)}` : ''}.
          </p>
        )}
        {retryPlacement && (
          <p className="gen-placement__target" data-cut-generate-retry-target={retryPlacement.target_clip}>
            Retry target <code>{retryPlacement.target_clip}</code> overrides the destination until inputs change.
          </p>
        )}
      </section>

      {/* PAID-GEN GUARD — always visible cost notice + a two-step confirm button. */}
      <p className="cd-note cd-note--warn" data-cut-generate-cost-notice>
        <Icon name="warning" size={14} tone="warn" /> Uses your signed-in <b>{provider} CLI</b> and
        may consume provider credits.
      </p>

      <button
        className={`cd-btn ${armed ? 'cd-btn--danger' : 'cd-btn--primary'}`}
        data-cut-generate-run
        data-cut-generate-armed={armed || undefined}
        disabled={!canGenerate}
        onClick={() => void onGenerateClick()}
      >
        {busy
          ? phase || 'Generating…'
          : armed
            ? `Confirm — run my ${provider} CLI to generate this ${kind}`
            : <><Icon name="effect" size={14} tone="brand" /> Generate {kind} (AI)</>}
      </button>
      {busy && jobId && (
        <button
          className="cd-btn cd-btn--ghost cd-btn--sm"
          data-cut-generate-job-cancel={jobId}
          disabled={phase === 'Cancelling…'}
          onClick={() => void cancelGeneration()}
        >
          <Icon name="close" size={14} label="Cancel generation" /> Cancel generation
        </button>
      )}
      {busy && phase && <p className="cd-note" data-cut-generate-job-state>{phase}</p>}
      {armed && !busy && (
        <button
          className="cd-btn cd-btn--ghost cd-btn--sm"
          data-cut-generate-cancel
          onClick={() => setArmed(false)}
        >
          Cancel
        </button>
      )}

      {err && <div className="cd-err" data-cut-generate-error role="alert">{err}</div>}
      {retryPlacement && !busy && !armed && (
        <div className="gen-retry" data-cut-generate-retry={retryPlacement.target_clip}>
          <span>The pending timeline slot is still available.</span>
          <button className="cd-btn cd-btn--ghost cd-btn--sm" data-cut-generate-retry-prepare onClick={prepareRetry}>
            <Icon name="reset" size={14} /> Prepare retry
          </button>
        </div>
      )}
      {note && <p className="cd-note" data-cut-generate-note>{note}</p>}

      <p className="cd-note">
        Generated files are validated, rejected if empty or fake, and imported into Assets with the normal receipts.
      </p>

      <GenerationHistory
        items={history}
        loading={historyLoading}
        selectedReferences={references}
        chosenAssetId={chosenAssetId}
        canInsert={insertTrackReady && !historyBusy}
        canReplace={selectedReplaceReady && !historyBusy}
        onToggleReference={toggleReference}
        onPrepareVariation={prepareVariation}
        onChoose={(record) => {
          setChosenAssetId(record.asset_id)
          setNote(`Chosen ${record.generation_id} for timeline placement.`)
          setErr(null)
        }}
        onInsert={(record) => void placeExisting(record, 'insert')}
        onReplace={(record) => void placeExisting(record, 'replace')}
      />
    </div>
  )

  return (
    <section className="cd-embed" data-cut-generate data-cut-generate-open="true" data-cut-generate-embed aria-label="Generate (AI)">
      {body}
    </section>
  )
}
