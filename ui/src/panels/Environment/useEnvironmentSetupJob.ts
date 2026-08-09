import { useEffect, useRef, useState } from 'react'
import { callVerb, type JobRecord } from '../../lib/client'
import { events } from '../../lib/events'

const SETUP_JOB_POLL_MS = 750
export const SETUP_JOB_MAX_POLLS = 960
export const SETUP_JOB_MAX_READ_FAILURES = 5

interface JobUi {
  busy: boolean
  pct: number | null
  msg: string | null
  err: string | null
}

const IDLE_JOB: JobUi = { busy: false, pct: null, msg: null, err: null }
const STATUS_READ_ERROR = 'could not read setup progress — re-scan and try again'

export function useEnvironmentSetupJob(onChanged: () => void) {
  const [job, setJob] = useState<JobUi>(IDLE_JOB)
  const activeJobRef = useRef<string | null>(null)
  const mountedRef = useRef(true)

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
      activeJobRef.current = null
    }
  }, [])

  useEffect(() => events.onEvent((event) => {
    if (event.type !== 'job_progress') return
    if (!mountedRef.current || event.job_id !== activeJobRef.current) return
    setJob((current) => current.busy
      ? { ...current, pct: Math.round(event.progress * 100), msg: event.message ?? current.msg }
      : current)
  }), [])

  const failStatusRead = (message = STATUS_READ_ERROR) => {
    activeJobRef.current = null
    if (mountedRef.current) {
      setJob({ busy: false, pct: null, msg: null, err: message })
    }
  }

  const runJob = async (
    start: () => Promise<{ job_id: string } | null>,
    startError: string,
  ) => {
    setJob({ busy: true, pct: 0, msg: null, err: null })
    let started: { job_id: string } | null
    try {
      started = await start()
    } catch {
      if (mountedRef.current) {
        setJob({ busy: false, pct: null, msg: null, err: `${startError}: server unreachable` })
      }
      return
    }
    if (!mountedRef.current) return
    if (!started) {
      setJob({ busy: false, pct: null, msg: null, err: startError })
      return
    }

    const jobId = started.job_id
    activeJobRef.current = jobId
    let readFailures = 0
    for (let poll = 0; poll < SETUP_JOB_MAX_POLLS; poll += 1) {
      await new Promise((resolve) => setTimeout(resolve, SETUP_JOB_POLL_MS))
      if (!mountedRef.current || activeJobRef.current !== jobId) return

      let response
      try {
        response = await callVerb('jobs.status', { job_id: jobId })
      } catch {
        readFailures += 1
        if (readFailures < SETUP_JOB_MAX_READ_FAILURES) continue
        failStatusRead()
        return
      }
      if (!response.ok) {
        readFailures += 1
        if (readFailures < SETUP_JOB_MAX_READ_FAILURES) continue
        failStatusRead(response.error?.message)
        return
      }

      readFailures = 0
      if (!mountedRef.current || activeJobRef.current !== jobId) return
      const record = response.result as JobRecord
      const recordMessage = (record as JobRecord & { message?: string }).message
      const message = typeof recordMessage === 'string' ? recordMessage : null
      setJob((current) => ({
        ...current,
        pct: Math.round(record.progress * 100),
        msg: message ?? current.msg,
      }))
      if (record.state === 'done') {
        activeJobRef.current = null
        if (!mountedRef.current) return
        setJob({ busy: false, pct: 100, msg: 'done', err: null })
        onChanged()
        return
      }
      if (record.state === 'failed') {
        failStatusRead(record.error?.message ?? 'setup failed')
        return
      }
    }

    failStatusRead('setup timed out — re-scan before trying again')
  }

  return { job, runJob }
}
