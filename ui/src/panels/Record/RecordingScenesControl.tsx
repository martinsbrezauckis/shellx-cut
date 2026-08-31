import {
  RECORDING_SCENE_PRESETS,
  recordingSceneTimerLabel,
  sceneUsesCamera,
  type RecordingScenePreset,
  type RecordingSceneTimer,
  type RecordingSceneTimerAction,
} from './recordingScenes'
import type {
  RecordingRecoveryState,
  RecordingSceneStatus,
  RecordingSceneTimerStatus,
} from './useRecordingScenes'

interface RecordingScenesControlProps {
  selectedScene: RecordingScenePreset
  status: RecordingSceneStatus
  recovery: RecordingRecoveryState
  timer: RecordingSceneTimer
  timerStatus: RecordingSceneTimerStatus
  recording: boolean
  disabled: boolean
  cameraReason: string | null
  unavailableReason?: string | null
  liveSupported: boolean
  onSelect: (sceneId: string) => void
  onTimerChange: (timer: RecordingSceneTimer) => void
  onTimerControl: (action: RecordingSceneTimerAction) => void
  onRefreshRecovery: () => void
}

function sceneDescription(scene: RecordingScenePreset): string {
  if (scene.layout.kind === 'screen') return 'Your screen fills the recording with no camera bubble.'
  return scene.layout.corner === 'bottom_right'
    ? 'Keep your camera in the lower-right while the screen stays central.'
    : 'A compact rounded camera in the upper-right for walkthroughs.'
}

/** Compact named scenes for the exact layouts the current Studio preview can render. */
export function RecordingScenesControl({
  selectedScene,
  status,
  recovery,
  timer,
  timerStatus,
  recording,
  disabled,
  cameraReason,
  unavailableReason,
  liveSupported,
  onSelect,
  onTimerChange,
  onTimerControl,
  onRefreshRecovery,
}: RecordingScenesControlProps) {
  const liveUnavailableReason = recording && !liveSupported
    ? 'Live scene switching is unavailable because this recorder has not advertised the scene API.'
    : null
  const timerDisabledReason = recording && !liveSupported
    ? 'Live timer controls are unavailable because this recorder has not advertised the scene API.'
    : null
  const timerActionDisabled = disabled || Boolean(unavailableReason) || Boolean(timerDisabledReason) || timerStatus.state === 'switching'

  return (
    <section className="rec-scenes" data-cut-rec-scenes data-cut-rec-scene-state={status.state}>
      <div className="rec-scenes__head">
        <div>
          <span className="rec-studio-controls__label">Scenes</span>
          <strong data-cut-rec-scene-name>{selectedScene.name}</strong>
        </div>
        <span className="rec-scenes__mode" data-cut-rec-scene-live={recording ? 'true' : 'false'}>
          {recording ? 'Live controls' : 'Next recording'}
        </span>
      </div>
      <div className="rec-scenes__choices" role="group" aria-label="Recording scene">
        {RECORDING_SCENE_PRESETS.map((scene) => {
          const disabledReason = unavailableReason ?? (sceneUsesCamera(scene) && cameraReason ? cameraReason : liveUnavailableReason)
          const isDisabled = disabled || Boolean(disabledReason)
          return (
            <button
              key={scene.id}
              type="button"
              className={`rec-scenes__choice${selectedScene.id === scene.id ? ' rec-scenes__choice--selected' : ''}`}
              data-cut-rec-scene={scene.id}
              data-cut-rec-scene-layout={scene.layout.kind}
              aria-pressed={selectedScene.id === scene.id}
              disabled={isDisabled}
              title={disabledReason ?? sceneDescription(scene)}
              onClick={() => onSelect(scene.id)}
            >
              <span className={`rec-scenes__thumbnail rec-scenes__thumbnail--${scene.layout.kind}`} aria-hidden="true">
                <i />
                {sceneUsesCamera(scene) && <b />}
              </span>
              <span>
                <strong>{scene.name}</strong>
                <small>{sceneDescription(scene)}</small>
                {disabledReason && <em data-cut-rec-scene-disabled-reason>{disabledReason}</em>}
              </span>
            </button>
          )
        })}
      </div>
      <p className="rec-scenes__status" data-cut-rec-scene-status={status.state} role="status">{status.message}</p>
      <div className="rec-scenes__timer" data-cut-rec-scene-timer={timer.kind}>
        <span className="rec-studio-controls__label">Recording timer</span>
        {!recording ? (
          <div className="rec-scenes__timer-choices" role="group" aria-label="Recording timer">
            <button
              type="button"
              data-cut-rec-scene-timer-choice="off"
              aria-pressed={timer.kind === 'off'}
              disabled={disabled}
              onClick={() => onTimerChange({ kind: 'off' })}
            >
              Off
            </button>
            <button
              type="button"
              data-cut-rec-scene-timer-choice="elapsed"
              aria-pressed={timer.kind === 'elapsed'}
              disabled={disabled}
              onClick={() => onTimerChange({ kind: 'elapsed' })}
            >
              Count up
            </button>
            <button
              type="button"
              data-cut-rec-scene-timer-choice="countdown"
              aria-pressed={timer.kind === 'countdown'}
              disabled={disabled}
              onClick={() => onTimerChange({ kind: 'countdown', duration_ms: 300_000 })}
            >
              5:00 down
            </button>
          </div>
        ) : (
          <div className="rec-scenes__timer-choices rec-scenes__timer-choices--live" role="group" aria-label="Live recording timer">
            {(['pause', 'resume', 'reset', 'restart', 'end'] as const).map((action) => (
              <button
                key={action}
                type="button"
                data-cut-rec-scene-timer-action={action}
                disabled={timerActionDisabled}
                title={timerDisabledReason ?? `Ask the recorder to ${action} the timer.`}
                onClick={() => onTimerControl(action)}
              >
                {action === 'end' ? 'End timer' : action[0].toUpperCase() + action.slice(1)}
              </button>
            ))}
          </div>
        )}
        <p data-cut-rec-scene-timer-state={timerStatus.state} role="status">
          {recording ? timerStatus.message : recordingSceneTimerLabel(timer)}
        </p>
      </div>
      <div className="rec-scenes__recovery" data-cut-rec-scene-recovery={recovery.state}>
        <span>Recovery</span>
        <p role="status">{recovery.message}</p>
        {recovery.state === 'error' && (
          <button type="button" className="rec__export-btn rec__export-btn--small" data-cut-action="record-scene-recovery-refresh" onClick={onRefreshRecovery}>
            Check again
          </button>
        )}
      </div>
    </section>
  )
}
