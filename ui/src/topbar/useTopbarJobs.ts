import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb, type JobRecord } from '../lib/client'
import { events } from '../lib/events'

export interface JobView {
  job_id: string
  kind: string
  state: 'queued' | 'running'
  progress: number
  message?: string
  queue?: { resource: string; max_running: number; position?: number; waiting?: number }
  waiting_on?: { job_id: string; kind: string }
}

export function isRenderBlockingJobKind(kind: string): boolean {
  return (
    kind === 'render' ||
    kind === 'reframe' ||
    kind.startsWith('reframe-') ||
    kind === 'render_queue'
  )
}

/** Stable active projection from the unordered server map. Queue positions are
 * per resource and use durable creation order; they are presentation facts,
 * not persisted scheduler promises. */
export function activeJobViews(records: JobRecord[]): JobView[] {
  const active = records
    .filter((job) => job.state === 'queued' || job.state === 'running')
    .sort((left, right) => left.created_ts.localeCompare(right.created_ts) || left.job_id.localeCompare(right.job_id))
  const waiting = new Map<string, number>()
  for (const job of active) {
    if (job.state === 'queued' && job.queue) {
      waiting.set(job.queue.resource, (waiting.get(job.queue.resource) ?? 0) + 1)
    }
  }
  const positions = new Map<string, number>()
  return active.map((job) => {
    const queue = job.state === 'queued' ? job.queue : undefined
    let queueView: JobView['queue']
    if (queue) {
      const position = (positions.get(queue.resource) ?? 0) + 1
      positions.set(queue.resource, position)
      queueView = { ...queue, position, waiting: waiting.get(queue.resource) ?? 1 }
    }
    return {
      job_id: job.job_id,
      kind: job.kind,
      state: job.state === 'queued' ? 'queued' : 'running',
      progress: job.progress,
      ...(job.message ? { message: job.message } : {}),
      ...(queueView ? { queue: queueView } : {}),
      ...(job.waiting_on ? { waiting_on: job.waiting_on } : {}),
    }
  })
}

/** Tracks running jobs for the topbar chip and Render button disabled state. */
export function useTopbarJobs() {
  const [jobs, setJobs] = useState<Record<string, JobView>>({})
  const seedRequest = useRef(0)

  useEffect(() => {
    const seed = async () => {
      const request = ++seedRequest.current
      try {
        const r = await callVerb('jobs.list', {})
        if (request !== seedRequest.current) return
        const list = (r.ok && (r.result as { jobs?: JobRecord[] })?.jobs) || []
        setJobs(Object.fromEntries(activeJobViews(list).map((job) => [job.job_id, job])))
      } catch {
        // Transport down: connection state is handled by the status bar.
      }
    }
    const offStatus = events.onStatus((s) => {
      if (s === 'open') void seed()
    })
    const offEvents = events.subscribe((ev) => {
      if (ev.type === 'job_progress') {
        seedRequest.current += 1
        setJobs((prev) => {
          const next = { ...prev }
          if (ev.progress >= 1) delete next[ev.job_id]
          else next[ev.job_id] = {
            job_id: ev.job_id,
            kind: ev.kind,
            state: 'running',
            progress: ev.progress,
            ...(ev.message ? { message: ev.message } : {}),
          }
          return next
        })
      } else if (ev.type === 'render_done') {
        seedRequest.current += 1
        setJobs((prev) => {
          const next = { ...prev }
          delete next[ev.job_id]
          return next
        })
      }
    })
    void seed()
    const poll = window.setInterval(() => void seed(), 3_000)
    return () => {
      seedRequest.current += 1
      window.clearInterval(poll)
      offStatus()
      offEvents()
    }
  }, [])

  const removeJob = useCallback((jobId: string) => {
    setJobs((previous) => {
      const next = { ...previous }
      delete next[jobId]
      return next
    })
  }, [])
  const jobList = Object.values(jobs)
  const renderRunning = jobList.some((j) => isRenderBlockingJobKind(j.kind))
  return { jobList, renderRunning, removeJob }
}
