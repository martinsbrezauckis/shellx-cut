// Receipt-bound output verification for Review > Receipts.
// A completed job is accepted only when its ids, hash, and profile agree with
// this immutable source receipt. A stale result is a visible failure, never a
// green recheck.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  callVerb,
  type JobRecord,
  type OutputCheckRerun,
  type RenderReceipt,
} from '../../lib/client'
import {
  isRerunHandle,
  isRerunResult,
  profileFromReceipt,
  type ReceiptRerunIdentity,
  type ReceiptRerunProfile,
} from './receiptRerunModel'

type RerunState = 'idle' | 'running' | 'success' | 'failed' | 'cancelled'
interface StoredRerun {
  job_id: string
  render_id: string
  output_hash: string
  output_path: string
  at_op: string
  profile: ReceiptRerunProfile
}

const storageKey = (renderId: string) => `shellx-cut:review:receipt-rerun:${renderId}`

function sameIdentity(stored: StoredRerun, identity: ReceiptRerunIdentity): boolean {
  return stored.render_id === identity.renderId
    && stored.output_hash === identity.outputHash
    && stored.output_path === identity.outputPath
    && stored.at_op === identity.atOp
    && stored.profile === identity.profile
}

function readStoredJob(identity: ReceiptRerunIdentity): StoredRerun | null {
  try {
    const raw = localStorage.getItem(storageKey(identity.renderId))
    if (!raw || !identity.profile) return null
    const value = JSON.parse(raw) as Partial<StoredRerun>
    if (typeof value.job_id !== 'string' || typeof value.render_id !== 'string'
      || typeof value.output_hash !== 'string'
      || typeof value.output_path !== 'string' || typeof value.at_op !== 'string'
      || (value.profile !== 'talking_head' && value.profile !== 'silent_screen_demo')) return null
    const stored = value as StoredRerun
    return sameIdentity(stored, identity) ? stored : null
  } catch { return null }
}

function storeJob(identity: ReceiptRerunIdentity, jobId: string | null): void {
  try {
    const key = storageKey(identity.renderId)
    if (jobId && identity.profile) {
      localStorage.setItem(key, JSON.stringify({
        job_id: jobId,
        render_id: identity.renderId,
        output_hash: identity.outputHash,
        output_path: identity.outputPath,
        at_op: identity.atOp,
        profile: identity.profile,
      } satisfies StoredRerun))
    } else localStorage.removeItem(key)
  } catch { /* resume support is optional; the durable job record is not */ }
}

function errorText(message?: string, action?: string): string {
  return [message, action].filter(Boolean).join(' — ') || 'Output checks could not be re-run.'
}

function isCancelled(record: JobRecord): boolean {
  return record.outcome === 'cancelled'
    || record.outcome_reason === 'user_cancelled'
    || record.outcome_reason === 'project_switch_cancelled'
    || record.error?.code === 'render_cancelled'
    || record.error?.code === 'job_cancelled'
}

export default function ReceiptRerunControl({ receipt }: { receipt: RenderReceipt }) {
  const profile = profileFromReceipt(receipt)
  const identity = useMemo<ReceiptRerunIdentity>(() => ({
    renderId: receipt.render_id,
    outputHash: receipt.output_hash,
    outputPath: receipt.output_path,
    atOp: receipt.at_op,
    profile,
  }), [profile, receipt.at_op, receipt.output_hash, receipt.output_path, receipt.render_id])
  const [state, setState] = useState<RerunState>('idle')
  const [result, setResult] = useState<OutputCheckRerun | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [cancelNotice, setCancelNotice] = useState<string | null>(null)
  const activeJob = useRef<string | null>(null)
  const timer = useRef<number | null>(null)

  const clearTimer = useCallback(() => {
    if (timer.current !== null) window.clearTimeout(timer.current)
    timer.current = null
  }, [])
  const terminalFailure = useCallback((next: 'failed' | 'cancelled', message: string) => {
    activeJob.current = null
    clearTimer()
    storeJob(identity, null)
    setState(next)
    setResult(null)
    setCancelNotice(null)
    setError(message)
  }, [clearTimer, identity])

  const poll = useCallback((jobId: string, delayMs = 800) => {
    clearTimer()
    const run = async () => {
      if (activeJob.current !== jobId) return
      try {
        const status = await callVerb('jobs.status', { job_id: jobId })
        if (!status.ok || !status.result) {
          terminalFailure('failed', errorText(status.error?.message, status.error?.suggested_action))
          return
        }
        const record = status.result
        if (record.job_id !== jobId) {
          terminalFailure('failed', 'Received status for a different output-check job; the result was discarded.')
          return
        }
        if (record.state === 'done') {
          activeJob.current = null
          clearTimer()
          storeJob(identity, null)
          if (isRerunResult(record.result, identity, jobId)) {
            setResult(record.result); setError(null); setCancelNotice(null); setState('success')
          } else {
            setResult(null); setCancelNotice(null)
            setError('Output-check job completed with a different receipt, hash, or profile; it was discarded.')
            setState('failed')
          }
          return
        }
        if (record.state === 'failed') {
          terminalFailure(isCancelled(record) ? 'cancelled' : 'failed', errorText(record.error?.message, record.error?.suggested_action))
          return
        }
      } catch {
        // A reconnect must not turn a durable running job into a fake terminal state.
      }
      if (activeJob.current === jobId) timer.current = window.setTimeout(() => void run(), 1_200)
    }
    timer.current = window.setTimeout(() => void run(), delayMs)
  }, [clearTimer, identity, terminalFailure])

  useEffect(() => {
    clearTimer(); activeJob.current = null
    setResult(null); setError(null); setCancelNotice(null); setState('idle')
    const stored = readStoredJob(identity)
    if (stored) {
      activeJob.current = stored.job_id
      setState('running')
      poll(stored.job_id, 0)
    } else storeJob(identity, null)
    return () => { activeJob.current = null; clearTimer() }
  }, [clearTimer, identity, poll])

  const start = useCallback(async () => {
    if (state === 'running') return
    if (!identity.profile) {
      terminalFailure('failed', 'This render receipt has incomplete footage-profile evidence. Render again before re-running output checks.')
      return
    }
    clearTimer(); activeJob.current = null
    setResult(null); setError(null); setCancelNotice(null); setState('running')
    try {
      const started = await callVerb('verify.rerun', { render_id: identity.renderId })
      if (!started.ok || !isRerunHandle(started.result, identity)) {
        terminalFailure('failed', started.ok
          ? 'The engine returned a recheck handle for a different render or output hash; it was discarded.'
          : errorText(started.error?.message, started.error?.suggested_action))
        return
      }
      activeJob.current = started.result.job_id
      storeJob(identity, started.result.job_id)
      poll(started.result.job_id)
    } catch {
      terminalFailure('failed', 'Could not start output checks. Check the Cut engine connection and try again.')
    }
  }, [clearTimer, identity, poll, state, terminalFailure])

  const cancel = useCallback(async () => {
    const jobId = activeJob.current
    if (!jobId || state !== 'running') return
    setCancelNotice('Cancellation requested; waiting for the job to stop safely.')
    try {
      const response = await callVerb('jobs.cancel', { job_id: jobId })
      if (!response.ok || response.result?.cancelled !== true) {
        setCancelNotice(errorText(response.error?.message, response.error?.suggested_action))
        return
      }
      poll(jobId, 0)
    } catch { setCancelNotice('Could not request cancellation. The job is still being monitored.') }
  }, [poll, state])

  const resultLabel = result?.pass ? 'OUTPUT CHECKS PASSED' : 'OUTPUT CHECKS NEED REVIEW'
  return <div className="rr-rc__profile" data-cut-receipt-rerun-state={state}>
    {state === 'running' ? (
      <button type="button" className="rr-rc__download" data-cut-action="receipt-rerun-cancel"
        data-cut-receipt-rerun-cancel={identity.renderId} onClick={() => void cancel()}
        title="Cancel this output-check job; its owned sidecar and probe will stop safely">Cancel output checks</button>
    ) : (
      <button type="button" className="rr-rc__download" data-cut-action="receipt-rerun"
        data-cut-receipt-rerun={identity.renderId} onClick={() => void start()}
        title="Run output checks against this exact rendered file; does not re-render or replace the receipt">Re-run output checks</button>
    )}
    <span> checks this exact render; no re-render</span>
    <span data-cut-receipt-rerun-scope=""> · loudness, black/frozen, border, edge-silence, and receipt-duration only; no source, caption, word-cut, or current-timeline checks</span>
    {state === 'running' && <span data-cut-receipt-rerun-progress=""> · {cancelNotice ?? 'checking artifact identity and output'}</span>}
    {state === 'success' && <span data-cut-receipt-rerun-result={result?.pass ? 'pass' : 'fail'}> · {resultLabel}</span>}
    {state === 'failed' && <span data-cut-receipt-rerun-error=""> · {error}</span>}
    {state === 'cancelled' && <span data-cut-receipt-rerun-cancelled=""> · Output checks cancelled — {error}</span>}
  </div>
}
