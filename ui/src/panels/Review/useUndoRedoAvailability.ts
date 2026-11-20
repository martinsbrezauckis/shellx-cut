import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb, type OpRecord, type Project } from '../../lib/client'

type Availability = { undo: boolean; redo: boolean; unavailable: boolean }
type OwnedAvailability = Availability & { projectKey: string | null }
const EMPTY: Availability = { undo: false, redo: false, unavailable: false }

export function useUndoRedoAvailability(project: Project | null, ops: OpRecord[], tab: string) {
  const projectKey = project
    ? project.project_identity?.origin_path_sha256 || `name:${project.name}`
    : null
  const [owned, setOwned] = useState<OwnedAvailability>({ ...EMPTY, projectKey })
  const scheduleRef = useRef<() => void>(() => {})

  useEffect(() => {
    let alive = true
    let requestId = 0
    let timer: number | null = null
    setOwned(previous => previous.projectKey === projectKey ? previous : { ...EMPTY, projectKey })

    const refresh = async () => {
      const id = ++requestId
      try {
        const response = await callVerb('project.ops', {})
        const result = response.result
        if (!response.ok || typeof result?.undo_available !== 'boolean' || typeof result.redo_available !== 'boolean') {
          throw new Error('History availability response is unavailable')
        }
        if (alive && id === requestId) {
          setOwned({ projectKey, undo: result.undo_available, redo: result.redo_available, unavailable: false })
        }
      } catch {
        if (alive && id === requestId) setOwned({ projectKey, undo: false, redo: false, unavailable: true })
      }
    }

    const schedule = () => {
      // An action invalidates reads started before it and any earlier delayed read.
      ++requestId
      if (timer !== null) window.clearTimeout(timer)
      setOwned({ projectKey, undo: false, redo: false, unavailable: false })
      timer = window.setTimeout(() => { timer = null; void refresh() }, 350)
    }
    scheduleRef.current = schedule
    if (projectKey !== null) void refresh()
    return () => {
      alive = false
      ++requestId
      if (timer !== null) window.clearTimeout(timer)
      if (scheduleRef.current === schedule) scheduleRef.current = () => {}
    }
  }, [projectKey, ops, tab])

  const scheduleRefresh = useCallback(() => scheduleRef.current(), [])
  return {
    availability: owned.projectKey === projectKey ? owned : { ...EMPTY, projectKey },
    scheduleRefresh,
  }
}
