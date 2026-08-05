import { useEffect, useState } from 'react'
import { callVerb, type JobRecord } from '../lib/client'
import { events } from '../lib/events'

interface JobView {
  job_id: string
  kind: string
  progress: number
}

export function isRenderBlockingJobKind(kind: string): boolean {
  return (
    kind === 'render' ||
    kind === 'reframe' ||
    kind.startsWith('reframe-') ||
    kind === 'render_queue'
  )
}

/** Tracks running jobs for the topbar chip and Render button disabled state. */
export function useTopbarJobs() {
  const [jobs, setJobs] = useState<Record<string, JobView>>({})

  useEffect(() => {
    const seed = async () => {
      try {
        const r = await callVerb('jobs.list', {})
        const list = (r.ok && (r.result as { jobs?: JobRecord[] })?.jobs) || []
        const running = list.filter((j) => j.state === 'queued' || j.state === 'running')
        setJobs(Object.fromEntries(running.map((j) => [j.job_id, { job_id: j.job_id, kind: j.kind, progress: j.progress }])))
      } catch {
        // Transport down: connection state is handled by the status bar.
      }
    }
    const offStatus = events.onStatus((s) => {
      if (s === 'open') void seed()
    })
    const offEvents = events.subscribe((ev) => {
      if (ev.type === 'job_progress') {
        setJobs((prev) => {
          const next = { ...prev }
          if (ev.progress >= 1) delete next[ev.job_id]
          else next[ev.job_id] = { job_id: ev.job_id, kind: ev.kind, progress: ev.progress }
          return next
        })
      } else if (ev.type === 'render_done') {
        setJobs((prev) => {
          const next = { ...prev }
          delete next[ev.job_id]
          return next
        })
      }
    })
    void seed()
    return () => {
      offStatus()
      offEvents()
    }
  }, [])

  const jobList = Object.values(jobs)
  const renderRunning = jobList.some((j) => isRenderBlockingJobKind(j.kind))
  return { jobList, renderRunning }
}
