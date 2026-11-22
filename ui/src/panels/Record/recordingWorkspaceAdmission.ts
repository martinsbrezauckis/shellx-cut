/**
 * App-shell admission for leaving Recording Studio. This represents only the
 * whether the mounted Record controls may safely unmount. Capture ownership
 * stays in the app-level recording session.
 */
export type RecordingWorkspaceAdmissionPhase =
  | 'idle'
  | 'countdown'
  | 'starting'
  | 'recording'
  | 'finalizing'
  | 'recovery'

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

/**
 * The Recording Studio top bar shows the reason in one line beside its buttons.
 * Keep every reason within this many characters so that line is never cut off;
 * the full text is also the element's tooltip.
 */
export const RECORDING_EXIT_REASON_MAX_CHARS = 52

const EXIT_REASONS: Record<Exclude<RecordingWorkspaceAdmissionPhase, 'idle'>, string> = {
  countdown: 'Cancel the countdown before returning to Edit.',
  starting: 'Starting the recording. Stay on the Record page.',
  recording: 'Stop the recording before returning to Edit.',
  finalizing: 'Finishing the recording. Stay on the Record page.',
  recovery: 'Check this recording before returning to Edit.',
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
