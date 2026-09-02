import type { ScreenRecordStatusResult } from '../../lib/clientResults'
import { recordingCaptureSafetyPresentation } from './recordingCaptureSafetyModel'
import './recordingCaptureSafetyStatus.css'

type SourceLifecycle = ScreenRecordStatusResult['source_lifecycle']
type ControllerPlacement = ScreenRecordStatusResult['controller_placement']

interface RecordingCaptureSafetyStatusProps {
  sourceLifecycle: SourceLifecycle | null
  controllerPlacement: ControllerPlacement | null
}

/**
 * Compact, read-only proof displayed beside the live transport. It exposes no
 * native source or controller identity and never attempts to hide a window.
 */
export function RecordingCaptureSafetyStatus({
  sourceLifecycle,
  controllerPlacement,
}: RecordingCaptureSafetyStatusProps) {
  if (!sourceLifecycle || !controllerPlacement) return null
  const presentation = recordingCaptureSafetyPresentation(sourceLifecycle, controllerPlacement)
  return (
    <div className="rec-capture-safety" data-cut-rec-capture-safety>
      <span data-cut-rec-source-lifecycle title={sourceLifecycle.reason}>
        {presentation.sourceLabel}
      </span>
      <span
        data-cut-rec-controller-placement={controllerPlacement.state}
        className={`rec-capture-safety__controller rec-capture-safety__controller--${presentation.controllerTone}`}
        title={controllerPlacement.reason}
      >
        {presentation.controllerLabel}
      </span>
    </div>
  )
}
