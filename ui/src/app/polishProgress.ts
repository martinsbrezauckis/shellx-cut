// While screen_record.polish runs, its bake is a `screen_record_polish` job in
// jobs.list (engine: screen_record/polish_job.rs). The Record panel reads its
// progress once a second so "Finishing your recording" shows how far it is.

import { callVerb } from '../lib/client'

type JobRow = { kind?: string; state?: string; progress?: number }

/** The percent of the running polish job in a jobs.list reply, or null. */
export function polishPercent(reply: { ok: boolean; result?: unknown }): number | null {
  if (!reply.ok) return null
  const jobs = (reply.result as { jobs?: JobRow[] } | undefined)?.jobs ?? []
  const job = jobs.find((row) => row.kind === 'screen_record_polish' && (row.state === 'running' || row.state === 'queued'))
  return typeof job?.progress === 'number' && Number.isFinite(job.progress)
    ? Math.max(0, Math.min(100, Math.round(job.progress * 100)))
    : null
}

/** Report the polish job's percent every second until the returned stop is called. */
export function watchPolishProgress(onPercent: (percent: number) => void): () => void {
  let stopped = false
  const timer = window.setInterval(() => {
    void callVerb('jobs.list', {}).then((reply) => {
      const percent = polishPercent(reply)
      if (!stopped && percent !== null) onPercent(percent)
    }).catch(() => undefined)
  }, 1_000)
  return () => {
    stopped = true
    window.clearInterval(timer)
  }
}
