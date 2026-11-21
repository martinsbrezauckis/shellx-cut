import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react'
import type { Project } from '../lib/client'
import { addRawRecordingToTimeline } from '../panels/Record/addRawRecordingToTimeline'
import { useRawRecordingCopy } from '../panels/Record/useRawRecordingCopy'
import { useRecordingExport } from '../panels/Record/useRecordingExport'
import { useAppRecordingSession } from './RecordingSessionContext'

function useRecordingDelivery(project: Project | null) {
  const session = useAppRecordingSession()
  const [format, setFormat] = useState<'mp4' | 'gif'>('mp4')
  const [exportNote, setExportNote] = useState('')
  const activeIdentity = project?.project_identity
  const digest = activeIdentity?.origin_path_sha256
  const projectKey = digest && digest.trim() === digest ? digest : null
  const result = session.state
  const ownsResult = Boolean(projectKey && result.resultProjectIdentity?.origin_path_sha256 === projectKey)
  const capture = ownsResult && result.source && result.plan
    ? { source: result.source, plan: result.plan } : null
  const rawPath = ownsResult ? result.rawPath : null
  const addOwnerKey = rawPath && result.raw && result.resultCaptureId
    && (result.phase === 'done' || result.phase === 'error')
    ? JSON.stringify([projectKey, result.resultCaptureId]) : null
  const addOwnerRef = useRef({ key: addOwnerKey })
  // A project round trip must also retire an add that is still completing.
  if (addOwnerRef.current.key !== addOwnerKey) addOwnerRef.current = { key: addOwnerKey }
  const addOwner = addOwnerRef.current
  const [addStatus, setAddStatus] = useState<{ owner: typeof addOwner; note: string } | null>(null)
  const addPendingRef = useRef<typeof addOwner | null>(null)
  const liveRef = useRef(true)
  useEffect(() => {
    liveRef.current = true
    return () => { liveRef.current = false }
  }, [])
  const addRawToTimeline = useCallback(async () => {
    if (!rawPath || !addOwner.key || addPendingRef.current === addOwner) return false
    const current = () => liveRef.current && addOwnerRef.current === addOwner
    if (!current()) return false
    addPendingRef.current = addOwner
    setAddStatus({ owner: addOwner, note: 'Adding to the timeline…' })
    try {
      const added = await addRawRecordingToTimeline(rawPath)
      if (!current()) return false
      setAddStatus({ owner: addOwner, note: added.ok ? 'Added to the timeline' : `add failed: ${added.reason}` })
      return added.ok
    } finally {
      if (addPendingRef.current === addOwner) addPendingRef.current = null
    }
  }, [addOwner, rawPath])
  const ownerKey = projectKey && result.resultCaptureId && capture
    ? JSON.stringify([projectKey, result.resultCaptureId, capture.source, capture.plan]) : null
  const exportDelivery = useRecordingExport({
    capture, projectKey, ownerKey, format, outputPath: null, setNote: setExportNote,
  })
  const rawCopy = useRawRecordingCopy(rawPath, projectKey, ownsResult ? result.resultCaptureId : null)
  const addNote = addStatus?.owner === addOwner ? addStatus.note : ''
  return { format, setFormat, exportNote: addNote || exportNote, setExportNote,
    addRawToTimeline, ownsResult, ...exportDelivery, rawCopy }
}

type RecordingDelivery = ReturnType<typeof useRecordingDelivery>
const Context = createContext<RecordingDelivery | null>(null)

/** Owns admitted recording deliveries for the app session, across workspace and project mounts. */
export function RecordingDeliveryProvider({ project, children }: { project: Project | null; children: ReactNode }) {
  const delivery = useRecordingDelivery(project)
  return <Context.Provider value={delivery}>{children}</Context.Provider>
}

export function useAppRecordingDelivery(): RecordingDelivery {
  const delivery = useContext(Context)
  if (!delivery) throw new Error('RecordingDeliveryProvider is missing')
  return delivery
}
