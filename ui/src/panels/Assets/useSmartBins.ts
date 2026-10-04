import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb } from '../../lib/client'

export interface SmartBinRow {
  name: string
  kind?: string
  text?: string
  unused?: boolean
  min_width?: number
  min_height?: number
  offline?: boolean
  modified_after_ms?: number
  modified_before_ms?: number
  matches?: string[]
  match_count: number
}

type ReadStatus = 'idle' | 'loading' | 'ready' | 'unavailable'
interface BinState { owner: string; bins: SmartBinRow[]; status: ReadStatus }

/** Owns only smart-bin reads; mutations remain in Assets. */
export function useSmartBins(projectScope: number, originDigest: string | null, hasProject: boolean) {
  const owner = hasProject ? `${projectScope}:${originDigest ?? ''}` : ''
  const currentRef = useRef({ owner, generation: 0 })
  if (currentRef.current.owner !== owner) {
    currentRef.current = { owner, generation: currentRef.current.generation + 1 }
  }
  const liveRef = useRef(false)
  const [state, setState] = useState<BinState>({ owner, bins: [], status: 'idle' })
  const visible = state.owner === owner ? state : { owner, bins: [], status: 'idle' as const }

  useEffect(() => {
    liveRef.current = true
    return () => { liveRef.current = false; currentRef.current.generation += 1 }
  }, [])

  const reload = useCallback(async (): Promise<boolean> => {
    if (!owner || !liveRef.current || currentRef.current.owner !== owner) return false
    const generation = ++currentRef.current.generation
    const current = () => liveRef.current && currentRef.current.owner === owner
      && currentRef.current.generation === generation
    setState(previous => ({ owner, bins: previous.owner === owner ? previous.bins : [], status: 'loading' }))
    try {
      const reply = await callVerb('media.bin_list', {})
      if (!current()) return false
      const listed = (reply.result as { bins?: unknown } | undefined)?.bins
      if (!reply.ok || !Array.isArray(listed)
        || !listed.every(bin => bin && typeof bin.name === 'string' && typeof bin.match_count === 'number')) {
        setState(previous => previous.owner === owner ? { ...previous, status: 'unavailable' } : previous)
        return false
      }
      setState({ owner, bins: listed as SmartBinRow[], status: 'ready' })
      return true
    } catch {
      if (current()) setState(previous => previous.owner === owner ? { ...previous, status: 'unavailable' } : previous)
      return false
    }
  }, [owner])

  useEffect(() => {
    setState({ owner, bins: [], status: 'idle' })
    if (owner) void reload()
  }, [owner, reload])

  const removeLocal = useCallback((name: string) => {
    if (currentRef.current.owner !== owner) return
    setState(previous => previous.owner === owner
      ? { ...previous, bins: previous.bins.filter(bin => bin.name !== name) } : previous)
  }, [owner])

  return { bins: visible.bins, status: visible.status, reload, removeLocal }
}
