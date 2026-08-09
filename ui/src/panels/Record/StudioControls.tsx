import {
  backgroundLabel,
  cursorCorrelationLabel,
  type StudioBackground,
  type CursorCorrelation,
  type StudioRawStreams,
  type StudioState,
} from './studioTypes'

interface StudioControlsProps {
  studio: StudioState
  rawStreams: StudioRawStreams | null
  cursorCorrelation: CursorCorrelation | null
  onBackground: (background: StudioBackground) => void
}

export function StudioControls({
  studio,
  rawStreams,
  cursorCorrelation,
  onBackground,
}: StudioControlsProps) {
  const streamCount = rawStreams
    ? [rawStreams.screen, rawStreams.camera, rawStreams.mic, rawStreams.system, rawStreams.studio_events].filter(Boolean).length
    : 0

  return (
    <aside className="rec-studio-controls">
      <div
        className="rec-studio-controls__camera"
        data-cut-studio-camera-status
        data-cut-studio-camera-available="false"
      >
        <div className="rec-studio-controls__unavailable" data-cut-studio-camera-unavailable role="status">
          <strong>Camera capture</strong>
          <span>Not available in this release. Screen, microphone and system audio recording still work.</span>
        </div>
      </div>

      <label className="rec-studio-controls__group" data-cut-studio-background={studio.background}>
        <span className="rec-studio-controls__label">Background</span>
        <select
          className="rec__select rec-studio-controls__select"
          data-cut-studio-background-select
          value={studio.background}
          onChange={(event) => onBackground(event.target.value as StudioBackground)}
        >
          {(['gradient', 'solid', 'blur_screen', 'none'] as const).map((background) => (
            <option key={background} value={background}>{backgroundLabel(background)}</option>
          ))}
        </select>
      </label>

      <div
        className="rec-studio-controls__group rec-studio-controls__streams"
        data-cut-studio-raw-streams={streamCount}
      >
        <span className="rec-studio-controls__label">Raw streams</span>
        <div className="rec-studio-controls__chips">
          <span data-on={rawStreams?.screen ? 'true' : 'false'}>Screen</span>
          <span data-on={rawStreams?.camera ? 'true' : 'false'}>Camera</span>
          <span data-on={rawStreams?.mic ? 'true' : 'false'}>Mic</span>
          <span data-on={rawStreams?.system ? 'true' : 'false'}>System</span>
          <span data-on={rawStreams?.studio_events ? 'true' : 'false'}>Events</span>
        </div>
      </div>

      <div
        className="rec-studio-controls__group"
        data-cut-rec-cursor-correlation={cursorCorrelation?.state ?? 'unavailable'}
      >
        <span className="rec-studio-controls__label">Pointer positions</span>
        <span role="status">{cursorCorrelationLabel(cursorCorrelation)}</span>
        {cursorCorrelation?.detail && <small>{cursorCorrelation.detail}</small>}
      </div>

      <div
        className="rec-studio-controls__group rec-studio-controls__hotkeys"
        data-cut-studio-hotkey-status={studio.hotkeyStatus}
      >
        <span className="rec-studio-controls__label">Hotkeys</span>
        <div className="rec-studio-controls__chips">
          <span data-on="true">F9 Rec</span>
          <span data-on="true">F12 Mark</span>
        </div>
      </div>
    </aside>
  )
}
