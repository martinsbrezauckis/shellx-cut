import { useCallback, useRef, useState, type Dispatch, type SetStateAction } from 'react'
import type { LayoutState } from '../layout/useLayout'
import {
  RECORDING_WORKSPACE_ADMISSION_IDLE,
  recordingWorkspaceTransitionAllowed,
  type RecordingWorkspaceAdmission,
} from '../panels/Record/recordingWorkspaceAdmission'

export type RequestLayout = (update: SetStateAction<LayoutState>) => boolean

function sameAdmission(a: RecordingWorkspaceAdmission, b: RecordingWorkspaceAdmission): boolean {
  return a.phase === b.phase && a.blocked === b.blocked && a.reason === b.reason
}

/**
 * Keeps recorder ownership inside Record while making every app-shell layout
 * transition refuse to unmount it during an admitted capture lifecycle.
 */
export function useRecordingWorkspaceNavigation(
  layout: LayoutState,
  setLayout: Dispatch<SetStateAction<LayoutState>>,
) {
  const layoutRef = useRef(layout)
  layoutRef.current = layout
  const admissionRef = useRef<RecordingWorkspaceAdmission>(RECORDING_WORKSPACE_ADMISSION_IDLE)
  const [admission, setAdmission] = useState<RecordingWorkspaceAdmission>(RECORDING_WORKSPACE_ADMISSION_IDLE)

  const reportAdmission = useCallback((next: RecordingWorkspaceAdmission) => {
    if (sameAdmission(admissionRef.current, next)) return
    admissionRef.current = next
    setAdmission(next)
  }, [])

  const requestLayout = useCallback<RequestLayout>((update) => {
    const current = layoutRef.current
    const next = typeof update === 'function' ? update(current) : update
    if (!recordingWorkspaceTransitionAllowed(
      current.workspaceMode,
      next.workspaceMode,
      admissionRef.current,
    )) {
      return false
    }

    setLayout((previous) => {
      const candidate = typeof update === 'function' ? update(previous) : update
      if (!recordingWorkspaceTransitionAllowed(
        previous.workspaceMode,
        candidate.workspaceMode,
        admissionRef.current,
      )) {
        return previous
      }
      return candidate
    })
    return true
  }, [setLayout])

  return { admission, reportAdmission, requestLayout }
}
