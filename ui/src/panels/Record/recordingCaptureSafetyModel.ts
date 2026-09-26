import type { ScreenRecordStatusResult } from '../../lib/clientResults'

type SourceLifecycle = ScreenRecordStatusResult['source_lifecycle']
type ControllerPlacement = ScreenRecordStatusResult['controller_placement']

export interface RecordingCaptureSafetyPresentation {
  sourceLabel: string
  controllerLabel: string
  controllerTone: 'safe' | 'warning' | 'muted'
}

/** Pure, fail-closed presentation for observed recorder status—not a capability guess. */
export function recordingCaptureSafetyPresentation(
  sourceLifecycle: SourceLifecycle,
  controllerPlacement: ControllerPlacement,
): RecordingCaptureSafetyPresentation {
  const sourceLabel = sourceLifecycle.state === 'active'
    ? 'Native source active'
    : sourceLifecycle.state === 'awaiting_first_frame'
      ? 'Waiting for native source'
      : sourceLifecycle.state === 'source_lost'
        ? 'Selected source closed'
        : sourceLifecycle.state === 'unavailable'
          ? 'Source-loss monitoring unavailable'
          : 'Native capture ended'
  switch (controllerPlacement.state) {
    case 'not_excluded':
      return { sourceLabel, controllerLabel: 'Cut appears when visible on the selected display', controllerTone: 'muted' }
    case 'excluded':
      return { sourceLabel, controllerLabel: 'Controls excluded from capture', controllerTone: 'safe' }
    case 'auto_hidden':
      return { sourceLabel, controllerLabel: 'Controls auto-hidden while recording', controllerTone: 'safe' }
    case 'refused':
      return { sourceLabel, controllerLabel: 'Controller exclusion was refused', controllerTone: 'warning' }
    case 'unavailable':
      return { sourceLabel, controllerLabel: 'Controller exclusion is unverified', controllerTone: 'muted' }
  }
}
