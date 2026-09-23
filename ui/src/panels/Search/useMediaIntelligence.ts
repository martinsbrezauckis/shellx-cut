import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  callVerb,
  type JobRecord,
  type MediaEvidenceHit,
  type MediaIntelligenceStatusResult,
  type Project,
} from '../../lib/client'
import { MAX_CHAT_EVIDENCE_ATTACHMENTS } from '../../lib/evidenceAttachments'
import { evidenceKinds, projectIdentity, type EvidenceMode, type EvidenceScope } from './model'

interface RebuildState {
  jobId: string
  progress: number
  message: string
  cancelling: boolean
}

const errorMessage = (error: unknown): string => (
  error instanceof Error && error.message ? error.message : 'server unreachable'
)

export function useMediaIntelligence(project: Project | null) {
  const identity = useMemo(() => projectIdentity(project), [project])
  const hasProject = project !== null
  const [status, setStatus] = useState<MediaIntelligenceStatusResult | null>(null)
  const [statusBusy, setStatusBusy] = useState(false)
  const [query, setQuery] = useState('')
  const [mode, setModeState] = useState<EvidenceMode>('all')
  const [scope, setScopeState] = useState<EvidenceScope>('all_project_media')
  const [hits, setHits] = useState<MediaEvidenceHit[]>([])
  const [searchedQuery, setSearchedQuery] = useState('')
  const [nextCursor, setNextCursor] = useState<string | null>(null)
  const [resultIndexId, setResultIndexId] = useState<string | null>(null)
  const [searching, setSearching] = useState(false)
  const [rebuild, setRebuild] = useState<RebuildState | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [selected, setSelected] = useState<Set<string>>(() => new Set())

  const refreshStatus = useCallback(async (quiet = false) => {
    if (!hasProject) {
      setStatus(null)
      return
    }
    if (!quiet) setStatusBusy(true)
    try {
      const response = await callVerb('media.intelligence_status', {})
      if (!response.ok || !response.result) {
        setError(response.error?.message ?? 'Could not read search coverage.')
        return
      }
      setStatus(response.result)
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      if (!quiet) setStatusBusy(false)
    }
  }, [hasProject])

  useEffect(() => {
    setStatus(null)
    setHits([])
    setSearchedQuery('')
    setNextCursor(null)
    setSelected(new Set())
    setError(null)
    setNotice(null)
    setRebuild(null)
    if (hasProject) void refreshStatus()
  }, [identity, hasProject, refreshStatus])

  useEffect(() => {
    if (!rebuild) return
    let stopped = false
    const poll = async () => {
      const response = await callVerb('jobs.status', { job_id: rebuild.jobId })
      if (stopped) return
      if (!response.ok || !response.result) {
        setError(response.error?.message ?? 'Could not read search preparation progress.')
        setRebuild(null)
        return
      }
      const job: JobRecord = response.result
      if (job.state === 'done') {
        setRebuild(null)
        setNotice('Search evidence is ready.')
        await refreshStatus(true)
        return
      }
      if (job.state === 'failed') {
        const cancelled = job.outcome === 'cancelled'
        setRebuild(null)
        setNotice(cancelled ? 'Search preparation cancelled.' : null)
        if (!cancelled) setError(job.error?.message ?? 'Search preparation failed.')
        await refreshStatus(true)
        return
      }
      setRebuild((current) => current && current.jobId === job.job_id ? {
        ...current,
        progress: job.progress,
        message: job.message?.trim() || (job.state === 'queued' ? 'Waiting for analysis capacity…' : 'Preparing cited evidence…'),
      } : current)
    }
    void poll()
    const timer = window.setInterval(() => { void poll() }, 600)
    return () => {
      stopped = true
      window.clearInterval(timer)
    }
  }, [rebuild?.jobId, refreshStatus])

  const prepare = useCallback(async () => {
    if (!project || rebuild) return
    setError(null)
    setNotice(null)
    try {
      const response = await callVerb('media.intelligence_rebuild', {})
      if (!response.ok || !response.result?.job_id) {
        setError(response.error?.message ?? 'Could not start search preparation.')
        return
      }
      setRebuild({ jobId: response.result.job_id, progress: 0, message: 'Preparing cited evidence…', cancelling: false })
    } catch (reason) {
      setError(errorMessage(reason))
    }
  }, [project, rebuild])

  const cancelPrepare = useCallback(async () => {
    if (!rebuild || rebuild.cancelling) return
    setRebuild((current) => current ? { ...current, cancelling: true } : current)
    const response = await callVerb('jobs.cancel', { job_id: rebuild.jobId })
    if (!response.ok) {
      setError(response.error?.message ?? 'Cancellation is still pending.')
      setRebuild((current) => current ? { ...current, cancelling: false } : current)
    }
  }, [rebuild])

  const runSearch = useCallback(async (cursor?: string) => {
    const text = query.trim()
    if (!text) {
      setError('Describe a moment or type words that were spoken.')
      return
    }
    if (!status?.index_id) {
      setError('Prepare search evidence first.')
      return
    }
    setSearching(true)
    setError(null)
    setNotice(null)
    try {
      const response = await callVerb('media.intelligence_search', {
        query: text,
        kinds: evidenceKinds(mode),
        scope,
        limit: 30,
        cursor,
      })
      if (!response.ok || !response.result) {
        setError(response.error?.message ?? 'Search failed.')
        return
      }
      setHits((current) => cursor ? [...current, ...response.result!.hits] : response.result!.hits)
      setSearchedQuery(text)
      setNextCursor(response.result.next_cursor ?? null)
      setResultIndexId(response.result.index_id)
      if (!cursor) setSelected(new Set())
      const notices = [...response.result.warnings]
      if (response.result.stale_excluded > 0) notices.push('Changed evidence was excluded until refresh.')
      if (response.result.count === 0 && !cursor) notices.push('No cited moments matched this search.')
      setNotice(notices.join(' ' ) || null)
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setSearching(false)
    }
  }, [mode, query, scope, status?.index_id])

  const clearResults = useCallback(() => {
    setHits([])
    setSearchedQuery('')
    setNextCursor(null)
    setResultIndexId(null)
    setSelected(new Set())
    setNotice(null)
  }, [])

  const setMode = useCallback((value: EvidenceMode) => {
    setModeState(value)
    clearResults()
  }, [clearResults])

  const setScope = useCallback((value: EvidenceScope) => {
    setScopeState(value)
    clearResults()
  }, [clearResults])

  const toggleSelected = useCallback((evidenceId: string) => {
    setSelected((current) => {
      const next = new Set(current)
      if (next.has(evidenceId)) next.delete(evidenceId)
      else if (next.size < MAX_CHAT_EVIDENCE_ATTACHMENTS) next.add(evidenceId)
      return next
    })
  }, [])

  return {
    status, statusBusy, refreshStatus,
    query, setQuery, mode, setMode, scope, setScope,
    hits, searchedQuery, nextCursor, resultIndexId, searching, runSearch, clearResults,
    rebuild, prepare, cancelPrepare,
    error, notice, selected, toggleSelected,
  }
}
