import { createContext, useContext, useState, type ReactNode } from 'react'
import type { Project } from '../lib/client'
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
  const ownerKey = projectKey && result.resultCaptureId && capture
    ? JSON.stringify([projectKey, result.resultCaptureId, capture.source, capture.plan]) : null
  const exportDelivery = useRecordingExport({
    capture, projectKey, ownerKey, format, outputPath: null, setNote: setExportNote,
  })
  const rawCopy = useRawRecordingCopy(rawPath, projectKey, ownsResult ? result.resultCaptureId : null)
  return { format, setFormat, exportNote, setExportNote, ownsResult, ...exportDelivery, rawCopy }
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
