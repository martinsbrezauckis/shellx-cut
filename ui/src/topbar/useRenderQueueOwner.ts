import { useCallback, useEffect, useRef, useState } from 'react'
import type { Project, VerbArgs } from '../lib/client'
import { callVerb } from '../lib/client'
import { withAuthorizedOutputPath } from '../lib/exportDestination'
import { renderQueueTerminalError, type RenderQueueTerminalResult } from '../lib/renderQueueTerminal'

export interface RenderQueueRow {
  output: string
  preset: 'draft' | 'standard' | 'high'
  aspect: 'project' | '16:9' | '9:16' | '1:1' | '4:5'
}
export const newRenderQueueRow = (): RenderQueueRow => ({ output: '', preset: 'standard', aspect: 'project' })
export interface RenderQueueResult extends RenderQueueTerminalResult {
  queue_id?: string
  jobs?: Array<{ idx?: number; output?: string | null; job_id?: string; state?: string; ok?: boolean; error?: { code?: string; message?: string } }>
}
type Phase = 'form' | 'submitting' | 'submit_unknown' | 'running' | 'status_unknown' | 'done' | 'error'
interface AdmittedQueue { id: string; projectKey: string }
interface QueueState {
  phase: Phase
  originKey: string | null
  admitted: AdmittedQueue | null
  rows: RenderQueueRow[]
  progress: number
  result: RenderQueueResult | null
  error: string | null
}
const initialRows = () => [newRenderQueueRow(), { ...newRenderQueueRow(), aspect: '9:16' as const }]
const INITIAL: QueueState = { phase: 'form', originKey: null, admitted: null, rows: initialRows(), progress: 0, result: null, error: null }
const ADMISSION_UNKNOWN = 'Submission result unknown. The queue may be running. Check Jobs, Review, and output files before starting another batch; another attempt may duplicate deliveries.'

/** App-lifetime owner of one admitted render queue and its exact project job identity. */
export function useRenderQueueOwner(project: Project | null, projectSession: number) {
  const digest = project?.project_identity?.origin_path_sha256
  const projectKey = digest && digest.trim() === digest ? digest : null
  const [state, setState] = useState<QueueState>(INITIAL)
  const pendingRef = useRef(false)
  const admittedRef = useRef<AdmittedQueue | null>(null)
  const statusInFlightRef = useRef<Promise<unknown> | null>(null)
  const liveRef = useRef(true)
  const projectRef = useRef(projectKey)
  const sessionRef = useRef(projectSession)
  const epochRef = useRef({ projectKey, projectSession, value: 0 })
  if (epochRef.current.projectKey !== projectKey || epochRef.current.projectSession !== projectSession) {
    epochRef.current = { projectKey, projectSession, value: epochRef.current.value + 1 }
  }
  projectRef.current = projectKey
  sessionRef.current = projectSession

  useEffect(() => {
    liveRef.current = true
    return () => { liveRef.current = false }
  }, [])

  // An idle form belongs to no job. Never carry an explicit output choice into
  // another project's submission after the active project changes.
  useEffect(() => {
    setState(current => current.phase === 'form' && current.originKey && current.originKey !== projectKey
      ? { ...INITIAL, rows: initialRows() } : current)
  }, [projectKey, state.phase, state.originKey])

  const admitted = state.admitted
  const terminal = state.phase === 'done' || state.phase === 'error'
  useEffect(() => {
    if (!admitted || admitted.projectKey !== projectKey || terminal) return
    let stale = false
    let timer: number | null = null
    let inFlight = false
    let failures = 0
    const epoch = epochRef.current.value
    const current = () => !stale && liveRef.current && admittedRef.current === admitted
      && projectRef.current === admitted.projectKey && sessionRef.current === projectSession
      && epochRef.current.value === epoch
    const unknown = () => {
      failures += 1
      setState(previous => previous.admitted === admitted
        ? { ...previous, phase: 'status_unknown', error: 'Queue status unknown; checking this same queue again…' } : previous)
    }
    const poll = async () => {
      if (!current() || inFlight) return
      const prior = statusInFlightRef.current
      if (prior) {
        try { await prior } catch { /* the prior effect handles its own failure */ }
        if (!current() || inFlight) return
      }
      inFlight = true
      let request: ReturnType<typeof callVerb<'jobs.status'>> | null = null
      try {
        request = callVerb('jobs.status', { job_id: admitted.id, expected_origin_path_sha256: admitted.projectKey })
        statusInFlightRef.current = request
        const reply = await request
        if (!current()) return
        const record = reply.result
        if (!reply.ok || !record || record.job_id !== admitted.id || record.kind !== 'render_queue'
          || !['queued', 'running', 'done', 'failed'].includes(record.state)) unknown()
        else {
          failures = 0
          const result = record.result && typeof record.result === 'object' ? record.result as RenderQueueResult : null
          const progress = typeof record.progress === 'number' && Number.isFinite(record.progress)
            ? Math.max(0, Math.min(1, record.progress)) : null
          if (record.state === 'done' || record.state === 'failed') {
            const error = record.state === 'failed'
              ? record.error?.message ?? record.error?.code ?? 'A queued delivery failed.'
              : renderQueueTerminalError(result ?? undefined)
            setState(previous => previous.admitted === admitted
              ? { ...previous, phase: error ? 'error' : 'done', error,
                progress: progress ?? previous.progress, result: result ?? previous.result } : previous)
            return
          }
          setState(previous => previous.admitted === admitted
            ? { ...previous, phase: 'running', error: null,
              progress: progress ?? previous.progress, result: result ?? previous.result } : previous)
        }
      } catch {
        if (current()) unknown()
      } finally {
        if (statusInFlightRef.current === request) statusInFlightRef.current = null
        inFlight = false
        if (current()) timer = window.setTimeout(() => { void poll() }, failures ? Math.min(3_000, 500 * 2 ** failures) : 700)
      }
    }
    void poll()
    return () => { stale = true; if (timer !== null) window.clearTimeout(timer) }
  }, [admitted, projectKey, projectSession, terminal])

  const intentEpoch = useCallback(() => epochRef.current.value, [])
  const intentCurrent = useCallback((epoch: number) => liveRef.current && epochRef.current.value === epoch
    && Boolean(projectRef.current), [])

  const submit = useCallback(async (jobs: VerbArgs['render.queue']['jobs'], explicitPath: string | undefined, epoch: number) => {
    if (pendingRef.current || admittedRef.current || !intentCurrent(epoch)) return
    pendingRef.current = true
    const originKey = projectRef.current!
    setState(previous => ({ ...previous, phase: 'submitting', originKey, error: null, progress: 0, result: null }))
    let dispatched = false
    try {
      const reply = await withAuthorizedOutputPath(explicitPath, () => {
        if (!intentCurrent(epoch) || projectRef.current !== originKey) throw new Error('project changed before queue submission')
        dispatched = true
        return callVerb('render.queue', { jobs, rationale: `batch deliver ${jobs.length} renders`, expected_origin_path_sha256: originKey })
      })
      if (!liveRef.current) return
      if (!reply.ok) {
        setState(previous => ({ ...previous, phase: 'form', error: reply.error?.message ?? reply.error?.code ?? 'Render queue was refused.' }))
        return
      }
      const result = reply.result as RenderQueueResult | undefined
      const id = result?.queue_id
      if (typeof id !== 'string' || !id.trim() || id !== id.trim()) {
        setState(previous => ({ ...previous, phase: 'submit_unknown', error: ADMISSION_UNKNOWN }))
        return
      }
      const owner = { id, projectKey: originKey }
      admittedRef.current = owner
      setState(previous => ({ ...previous, phase: 'running', originKey, admitted: owner, result: result ?? null, error: null }))
    } catch (error) {
      if (!liveRef.current) return
      setState(previous => ({ ...previous, phase: dispatched ? 'submit_unknown' : 'form',
        error: dispatched ? ADMISSION_UNKNOWN : error instanceof Error && error.message === 'project changed before queue submission'
          ? 'Project changed before queue submission. Review the batch and start again.'
          : error instanceof Error ? error.message : 'Could not authorize the queue output.' }))
    } finally {
      pendingRef.current = false
    }
  }, [intentCurrent])

  const setRows = useCallback((update: (rows: RenderQueueRow[]) => RenderQueueRow[]) => {
    if (pendingRef.current || admittedRef.current) return
    setState(previous => previous.phase === 'form' ? { ...previous, rows: update(previous.rows), originKey: projectRef.current, error: null } : previous)
  }, [])
  const setFormError = useCallback((error: string | null) => {
    setState(previous => previous.phase === 'form' ? { ...previous, error } : previous)
  }, [])
  const acknowledge = useCallback(() => {
    if (pendingRef.current || state.phase === 'running' || state.phase === 'status_unknown' || state.phase === 'submitting') return
    if (admittedRef.current && state.originKey && state.originKey !== projectRef.current) return
    admittedRef.current = null
    setState({ ...INITIAL, rows: initialRows() })
  }, [state.phase, state.originKey])
  return { state, projectKey,
    foreign: Boolean(state.originKey && state.originKey !== projectKey
      && (state.admitted || state.phase === 'submitting')),
    intentEpoch, intentCurrent, submit, setRows, setFormError, acknowledge }
}

export type RenderQueueOwner = ReturnType<typeof useRenderQueueOwner>
