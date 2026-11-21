import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { callVerb, type Project, type Transcript as TranscriptData } from '../../lib/client'

type Cached = { locator: string; transcript: TranscriptData }
type SourceState = { key: string | null; items: Record<string, Cached>; failed: Record<string, string> }
type Request = { locator: string; settled: boolean }
type Owner = { key: string | null; active: boolean; requests: Map<string, Request> }

/** Source reads belong to an origin and an asset's transcript locator. */
export function useTranscriptSources(
  project: Project | null,
  projectKey: string | null,
  supplied: Record<string, TranscriptData>,
) {
  const [state, setState] = useState<SourceState>({ key: projectKey, items: {}, failed: {} })
  const ownerRef = useRef<Owner | null>(null)
  const loaded = useMemo(() => {
    if (state.key !== projectKey || !project) return {}
    return Object.fromEntries(Object.entries(state.items)
      .filter(([id, item]) => project.assets[id]?.transcript === item.locator)
      .map(([id, item]) => [id, item.transcript])) as Record<string, TranscriptData>
  }, [state, project, projectKey])
  const failures = state.key === projectKey && project
    ? Object.entries(state.failed).filter(([id, locator]) => project.assets[id]?.transcript === locator).map(([id]) => id)
    : []

  // Project switches and unmounts invalidate every outstanding request. A
  // sibling completion only updates state and never tears down this owner.
  useEffect(() => {
    const owner: Owner = { key: projectKey, active: true, requests: new Map() }
    ownerRef.current = owner
    return () => {
      owner.active = false
      owner.requests.clear()
      if (ownerRef.current === owner) ownerRef.current = null
    }
  }, [projectKey])

  useEffect(() => {
    setState((previous) => {
      if (previous.key !== projectKey) return { key: projectKey, items: {}, failed: {} }
      const items = Object.fromEntries(Object.entries(previous.items)
        .filter(([id, item]) => project?.assets[id]?.transcript === item.locator)) as Record<string, Cached>
      const failed = Object.fromEntries(Object.entries(previous.failed)
        .filter(([id, locator]) => project?.assets[id]?.transcript === locator)) as Record<string, string>
      return Object.keys(items).length === Object.keys(previous.items).length
        && Object.keys(failed).length === Object.keys(previous.failed).length
        ? previous : { ...previous, items, failed }
    })
  }, [project, projectKey])

  useEffect(() => {
    const owner = ownerRef.current
    if (!project || state.key !== projectKey || !owner?.active || owner.key !== projectKey) return
    for (const [id, request] of owner.requests) {
      if (project.assets[id]?.transcript !== request.locator || supplied[id]) owner.requests.delete(id)
    }
    for (const [assetId, asset] of Object.entries(project.assets)) {
      const locator = asset.transcript
      if (!locator || supplied[assetId] || state.items[assetId]?.locator === locator
        || state.failed[assetId] === locator || owner.requests.has(assetId)) continue
      const request: Request = { locator, settled: false }
      owner.requests.set(assetId, request)
      void (async () => {
        try {
          const response = await callVerb('transcript.get', { asset: assetId })
          const transcript = response.result
          if (!response.ok || !transcript || transcript.asset !== assetId || !Array.isArray(transcript.words)) {
            throw new Error('Source transcript unavailable')
          }
          if (owner.active && owner.requests.get(assetId) === request) {
            request.settled = true
            setState((previous) => previous.key === projectKey
              ? { ...previous, items: { ...previous.items, [assetId]: { locator, transcript } } } : previous)
          }
        } catch {
          if (owner.active && owner.requests.get(assetId) === request) {
            request.settled = true
            setState((previous) => previous.key === projectKey
              ? { ...previous, failed: { ...previous.failed, [assetId]: locator } } : previous)
          }
        }
      })()
    }
  }, [project, projectKey, supplied, state])

  const retry = useCallback(() => {
    const owner = ownerRef.current
    if (owner?.key === projectKey) {
      for (const [id, request] of owner.requests) if (request.settled) owner.requests.delete(id)
    }
    setState((previous) => previous.key === projectKey ? { ...previous, failed: {} } : previous)
  }, [projectKey])

  // A replacement writes the same receipt path, so locator-based cache
  // pruning cannot detect it. Discard just that asset after the job finishes.
  const refreshSource = useCallback((assetId: string) => {
    const owner = ownerRef.current
    if (owner?.key === projectKey) owner.requests.delete(assetId)
    setState((previous) => {
      if (previous.key !== projectKey) return previous
      const items = { ...previous.items }
      const failed = { ...previous.failed }
      delete items[assetId]
      delete failed[assetId]
      return { ...previous, items, failed }
    })
  }, [projectKey])

  return { loaded, loadFailures: failures, retrySources: retry, refreshSource }
}
