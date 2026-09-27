import {
  RecordingSourceControl,
  type MonitorInfo,
  type WindowInfo,
} from './RecordingSourceControl'
import type { RecordingSourceKind, RegionPickerCapability } from './regionPickerModel'

export interface RecordingSourceSetupSelection {
  readonly sourceKind: RecordingSourceKind
  readonly monitors: readonly MonitorInfo[]
  readonly monitorIdx: number | null
  readonly windows: readonly WindowInfo[]
  readonly windowTargetId: string | null
  readonly selectedWindowMissing: boolean
}

interface RecordingSourceSetupProps {
  readonly selection: RecordingSourceSetupSelection
  readonly disabled: boolean
  readonly pauseEnabled: boolean
  readonly windowCaptureSupported: boolean
  readonly regionCapability: RegionPickerCapability
  readonly onRefresh: () => void | Promise<unknown>
  readonly onSourceKindChange: (source: RecordingSourceKind) => void
  readonly onMonitorChange: (index: number) => void
  readonly onWindowChange: (id: string | null) => void
}

/** The one Screen and sound source selector. Preview starts in the Studio image. */
export function RecordingSourceSetup({
  selection, disabled, pauseEnabled, windowCaptureSupported, regionCapability, onRefresh, onSourceKindChange,
  onMonitorChange, onWindowChange,
}: RecordingSourceSetupProps) {
  return (
    <div className="rec__field rec__field--source">
      <span className="rec__label">Source</span>
      <RecordingSourceControl
        {...selection}
        disabled={disabled}
        allowWindow={windowCaptureSupported && !pauseEnabled}
        windowCaptureSupported={windowCaptureSupported}
        regionCapability={regionCapability}
        onRefresh={onRefresh}
        onSourceKindChange={onSourceKindChange}
        onMonitorChange={onMonitorChange}
        onWindowChange={onWindowChange}
      />
    </div>
  )
}
