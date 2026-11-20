import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb, type JobRecord } from '../lib/client'
import { events } from '../lib/events'
import { failedJobNotices, type FailedJobNotices } from '../lib/failedJobNotices'
import {
  interruptedJobNotices,
  interruptedKey,
  readInterruptedJobs,
  type InterruptedJob,
} from '../lib/interruptedJobNotices'

const NO_FAILED_JOBS: FailedJobNotices = { shown: [], hidden: 0 }

export interface JobView {
  job_id: string
  kind: string
  state: 'queued' | 'running'
  progress: number
  message?: string
  queue?: { resource: string; max_running: number; position?: number; waiting?: number }
  waiting_on?: { job_id: string; kind: string }
}

export interface RetryableJobView {
  job_id: string
  kind: string
  attempt: number
  root_job_id: string
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

/** Failed-job retry controls are entirely driven by the durable engine
 * projection. Clients do not reverse-engineer eligibility from kind/error. */
export function retryableJobViews(records: JobRecord[]): RetryableJobView[] {
  return records
    .filter((job) => (
      job.state === 'failed'
      && job.retry?.eligible === true
      && job.outcome !== 'cancelled'
      && job.outcome !== 'superseded'
      && job.outcome_reason !== 'user_cancelled'
      && job.outcome_reason !== 'project_switch_cancelled'
      && job.outcome_reason !== 'superseded'
    ))
    .sort((left, right) => left.created_ts.localeCompare(right.created_ts) || left.job_id.localeCompare(right.job_id))
    .map((job) => ({
      job_id: job.job_id,
      kind: job.kind,
      attempt: job.retry!.attempt,
      root_job_id: job.retry!.root_job_id,
    }))
}

/** Tracks running jobs for the topbar chip and Render button disabled state,
 * plus failed jobs the person has not dismissed. A project change forgets the
 * dismissals and ignores failures from before it. */
export function useTopbarJobs() {
  const [jobs, setJobs] = useState<Record<string, JobView>>({})
  const [retryJobs, setRetryJobs] = useState<Record<string, RetryableJobView>>({})
  const [failed, setFailed] = useState<FailedJobNotices>(NO_FAILED_JOBS)
  const seedRequest = useRef(0)
  const lastRecords = useRef<JobRecord[]>([])
  const dismissed = useRef(new Set<string>())
  const watchSince = useRef(Date.now())
  const [interrupted, setInterrupted] = useState<InterruptedJob[]>([])

  const refreshFailed = useCallback(() => {
    setFailed(failedJobNotices(lastRecords.current, watchSince.current, dismissed.current))
  }, [])

  const forgetFailures = useCallback(() => {
    seedRequest.current += 1
    lastRecords.current = []
    dismissed.current = new Set()
    watchSince.current = Date.now()
    setFailed(NO_FAILED_JOBS)
  }, [])

  useEffect(() => {
    const seed = async () => {
      const request = ++seedRequest.current
      try {
        const r = await callVerb('jobs.list', {})
        if (request !== seedRequest.current) return
        const list = (r.ok && (r.result as { jobs?: JobRecord[] })?.jobs) || []
        setJobs(Object.fromEntries(activeJobViews(list).map((job) => [job.job_id, job])))
        setRetryJobs(Object.fromEntries(retryableJobViews(list).map((job) => [job.job_id, job])))
        lastRecords.current = list
        refreshFailed()
      } catch {
        // Transport down: connection state is handled by the status bar.
      }
    }
    // Work a kill, crash or power cut stopped: told at start, project or not.
    const seedInterrupted = async () => {
      try {
        const r = await callVerb('jobs.interrupted', {})
        if (r.ok) setInterrupted(readInterruptedJobs(r.result))
      } catch {
        // Transport down: connection state is handled by the status bar.
      }
    }
    const offStatus = events.onStatus((s) => {
      if (s === 'open') {
        void seed()
        void seedInterrupted()
      }
    })
    const offEvents = events.subscribe((ev) => {
      if (ev.type === 'job_progress') {
        // The job ENDED (done, failed or cancelled): learn it from the engine's
        // job state. Progress alone says nothing about an ending, an ended job
        // keeps the progress it reached.
        if (ev.terminal) {
          void seed()
          return
        }
        seedRequest.current += 1
        setJobs((prev) => {
          const next = { ...prev }
          next[ev.job_id] = {
            job_id: ev.job_id,
            kind: ev.kind,
            state: 'running',
            progress: ev.progress,
            ...(ev.message ? { message: ev.message } : {}),
          }
          return next
        })
      } else if (ev.type === 'render_done') {
        void seed()
      } else if (ev.type === 'project_changed') {
        forgetFailures()
        void seed()
      }
    })
    void seed()
    void seedInterrupted()
    const poll = window.setInterval(() => void seed(), 3_000)
    return () => {
      seedRequest.current += 1
      window.clearInterval(poll)
      offStatus()
      offEvents()
    }
  }, [refreshFailed, forgetFailures])

  const removeJob = useCallback((jobId: string) => {
    setJobs((previous) => {
      const next = { ...previous }
      delete next[jobId]
      return next
    })
  }, [])
  const removeRetryJob = useCallback((jobId: string) => {
    setRetryJobs((previous) => {
      const next = { ...previous }
      delete next[jobId]
      return next
    })
  }, [])
  const dismissFailedJob = useCallback((jobId: string) => {
    const key = interruptedKey(jobId)
    if (key !== null) {
      setInterrupted((previous) => previous.filter((job) => job.key !== key))
      void callVerb('jobs.interrupted_dismiss', { key }).catch(() => {})
      return
    }
    dismissed.current.add(jobId)
    refreshFailed()
  }, [refreshFailed])
  const jobList = Object.values(jobs)
  const retryJobList = Object.values(retryJobs)
  const renderRunning = jobList.some((j) => isRenderBlockingJobKind(j.kind))
  const failedJobs: FailedJobNotices = interrupted.length > 0
    ? { shown: [...interruptedJobNotices(interrupted), ...failed.shown], hidden: failed.hidden }
    : failed
  return { jobList, retryJobList, failedJobs, renderRunning, removeJob, removeRetryJob, dismissFailedJob }
}
