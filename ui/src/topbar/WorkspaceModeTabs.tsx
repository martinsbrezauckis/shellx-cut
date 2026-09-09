import type { WorkspaceMode } from '../layout/useLayout'
import { WORKSPACE_MODES } from './model'

interface RecordingExitAdmission {
  readonly blocked: boolean
  readonly reason: string | null
}

interface WorkspaceModeTabsProps {
  readonly mode: WorkspaceMode
  readonly onMode?: (mode: WorkspaceMode) => void
  /** Present only in Recording Studio, where leaving Edit can unmount a capture owner. */
  readonly recordingExitAdmission?: RecordingExitAdmission
}

/**
 * Keeps workspace navigation in one header position. Recording supplies its
 * exit admission here instead of replacing the switch with a distant Back button.
 */
export function WorkspaceModeTabs({ mode, onMode, recordingExitAdmission }: WorkspaceModeTabsProps) {
  return (
    <div className="tb-modes" role="tablist" aria-label="Workspace mode" data-cut-modes={mode}>
      {WORKSPACE_MODES.map((workspace) => {
        const recordExitAdmission = mode === 'record' && workspace.id !== 'record'
          ? recordingExitAdmission
          : undefined
        const editExit = workspace.id === 'edit' ? recordExitAdmission : undefined
        const blocked = recordExitAdmission?.blocked ?? false
        const reason = blocked ? recordExitAdmission?.reason ?? null : null
        if (editExit) {
          return (
            <button
              key={workspace.id}
              type="button"
              role="tab"
              aria-selected={false}
              aria-describedby={reason ? 'cut-record-back-reason' : undefined}
              className="tb-mode"
              data-cut-mode="edit"
              data-cut-action="record-back-edit"
              data-cut-record-back-edit
              data-cut-record-back-blocked={blocked || undefined}
              disabled={blocked}
              title={reason ?? workspace.hint}
              onClick={(event) => { event.currentTarget.blur(); onMode?.('edit') }}
            >
              {workspace.label}
            </button>
          )
        }
        return (
          <button
            key={workspace.id}
            type="button"
            role="tab"
            aria-selected={mode === workspace.id}
            aria-describedby={reason ? 'cut-record-back-reason' : undefined}
            className={`tb-mode${mode === workspace.id ? ' tb-mode--on' : ''}${workspace.id === 'record' ? ' tb-mode--record' : ''}`}
            data-cut-mode={workspace.id}
            data-cut-manual-id={workspace.id === 'record' ? 'cut.record.open' : undefined}
            disabled={blocked}
            title={reason ?? workspace.hint}
            onClick={(event) => { event.currentTarget.blur(); if (!blocked) onMode?.(workspace.id) }}
          >
            {workspace.id === 'record' && <span className="tb-mode-dot" aria-hidden="true" />}
            {workspace.label}
          </button>
        )
      })}
    </div>
  )
}
