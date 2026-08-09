import type { ReactNode } from 'react'
import './track-lock-gate.css'

export interface TrackLockGateProps {
  locked: boolean
  children: ReactNode
}

export default function TrackLockGate({ locked, children }: TrackLockGateProps) {
  return (
    <>
      {locked && (
        <p className="insp__hint insp__lock-note" data-cut-inspector-locked-note>
          This track is locked. Unlock it in the timeline before changing this clip.
        </p>
      )}
      <fieldset className="insp__edit-fieldset" data-cut-inspector-edit-fieldset disabled={locked}>
        {children}
      </fieldset>
    </>
  )
}
