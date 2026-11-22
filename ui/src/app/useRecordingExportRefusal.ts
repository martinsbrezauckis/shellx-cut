import { useEffect } from 'react'
import type { Project } from '../lib/client'
import { RECORDING_EXPORT_REFUSED, setDeliveryFeedbackSession } from '../topbar/exportRefusal'

export function useRecordingExportRefusal(project: Project | null, session: number, reveal: (id: string) => void): void {
  setDeliveryFeedbackSession(session)
  useEffect(() => {
    const onRefused = () => {
      const clip = project?.tracks.flatMap((track) => track.clips)
        .find((item) => 'recording' in item && item.recording?.stale && !item.recording.camera)
      if (clip && 'id' in clip) reveal(clip.id)
    }
    document.addEventListener(RECORDING_EXPORT_REFUSED, onRefused)
    return () => document.removeEventListener(RECORDING_EXPORT_REFUSED, onRefused)
  }, [project, reveal])
}
