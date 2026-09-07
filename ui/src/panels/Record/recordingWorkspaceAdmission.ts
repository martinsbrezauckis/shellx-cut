/**
 * App-shell admission for leaving Recording Studio. This represents only the
 * lifetime of the mounted recorder controls; capture ownership stays in Record.
 */
export type RecordingWorkspaceAdmissionPhase =
  | 'idle'
  | 'countdown'
  | 'starting'
  | 'recording'
  | 'finalizing'

export interface RecordingWorkspaceAdmission {
  phase: RecordingWorkspaceAdmissionPhase
  blocked: boolean
  reason: string | null
}

export const RECORDING_WORKSPACE_ADMISSION_IDLE: RecordingWorkspaceAdmission = {
  phase: 'idle',
  blocked: false,
  reason: null,
}

const EXIT_REASONS: Record<Exclude<RecordingWorkspaceAdmissionPhase, 'idle'>, string> = {
  countdown: 'Cancel the countdown before returning to Edit.',
  starting: 'Waiting for the recorder to confirm start. Stay in Recording Studio.',
  recording: 'Stop the recording before returning to Edit.',
  finalizing: 'Finishing the recording. Stay in Recording Studio until it completes.',
}

export function recordingWorkspaceAdmission(
  phase: RecordingWorkspaceAdmissionPhase,
): RecordingWorkspaceAdmission {
  if (phase === 'idle') return RECORDING_WORKSPACE_ADMISSION_IDLE
  return { phase, blocked: true, reason: EXIT_REASONS[phase] }
}

export function recordingWorkspaceExitBlocked(admission: RecordingWorkspaceAdmission): boolean {
  return admission.blocked
}

/** A guard applies only when a currently mounted Record workspace would unmount. */
export function recordingWorkspaceTransitionAllowed(
  currentWorkspace: string,
  nextWorkspace: string,
  admission: RecordingWorkspaceAdmission,
): boolean {
  return currentWorkspace !== 'record'
    || nextWorkspace === 'record'
    || !recordingWorkspaceExitBlocked(admission)
}
