import type { ScreenRecordSourcePreviewStatus } from '../../lib/clientResults'
import {
  RecordingSourceControl,
  type MonitorInfo,
  type WindowInfo,
} from './RecordingSourceControl'
import { RecordingSourcePreview } from './RecordingSourcePreview'
import type { RecordingSourcePreviewPresentation, RecordingSourcePreviewTarget } from './recordingNativeSourcePreview'
import type { RecordingSourceKind, RegionPickerCapability } from './regionPickerModel'

export interface RecordingSourceSetupSelection {
  readonly sourceKind: RecordingSourceKind
  readonly monitors: readonly MonitorInfo[]
  readonly monitorIdx: number | null
  readonly windows: readonly WindowInfo[]
  readonly windowTargetId: string | null
  readonly selectedWindowMissing: boolean
}

interface RecordingSourcePreviewController {
  readonly presentation: RecordingSourcePreviewPresentation
  readonly status: ScreenRecordSourcePreviewStatus
  readonly target: RecordingSourcePreviewTarget | null
  readonly statusError: string | null
  readonly busy: boolean
  start(): void
  pause(): void
  resume(): void
  hide(): void
  stop(): void
}

interface RecordingSourceSetupProps {
  readonly selection: RecordingSourceSetupSelection
  readonly disabled: boolean
  readonly pauseEnabled: boolean
  readonly regionCapability: RegionPickerCapability
  readonly onRefresh: () => void | Promise<unknown>
  readonly onSourceKindChange: (source: RecordingSourceKind) => void
  readonly onMonitorChange: (index: number) => void
  readonly onWindowChange: (id: string | null) => void
  readonly preview: RecordingSourcePreviewController
}

/** The one Screen & sound source selector and its separately-owned preview lease. */
export function RecordingSourceSetup({
  selection, disabled, pauseEnabled, regionCapability, onRefresh, onSourceKindChange,
  onMonitorChange, onWindowChange, preview,
}: RecordingSourceSetupProps) {
  return (
    <div className="rec__field rec__field--source">
      <span className="rec__label">Source</span>
      <RecordingSourceControl
        {...selection}
        disabled={disabled}
        allowWindow={!pauseEnabled}
        regionCapability={regionCapability}
        onRefresh={onRefresh}
        onSourceKindChange={onSourceKindChange}
        onMonitorChange={onMonitorChange}
        onWindowChange={onWindowChange}
      />
      <RecordingSourcePreview
        preview={preview.presentation}
        status={preview.status}
        target={preview.target}
        statusError={preview.statusError}
        busy={disabled || preview.busy}
        onStart={() => { void preview.start() }}
        onPause={() => { void preview.pause() }}
        onResume={() => { void preview.resume() }}
        onHide={() => { void preview.hide() }}
        onStop={() => { void preview.stop() }}
      />
    </div>
  )
}
