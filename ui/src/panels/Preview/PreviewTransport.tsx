import { Icon } from '../../icons'
import { formatClock, type TimeDisplayMode } from '../Timeline/layout'
import { MasterMeter } from './MasterMeter'
import type { ActiveVideo } from './model'
import { guideLabel, type GuideMode } from './usePreviewViewOptions'

/** Playback rate: 0 = paused; +/-1/+/-2/+/-4 = J/K/L shuttle ladder. */
export type Rate = -4 | -2 | -1 | 0 | 1 | 2 | 4

interface PreviewTransportProps {
  playheadMs: number
  durationMs: number
  fps: number
  timeMode: TimeDisplayMode
  rate: Rate
  playing: boolean
  snapNote: string | null
  snapBusy: boolean
  hasProject: boolean
  exactBusy: boolean
  hasSection: boolean
  audioOn: boolean
  mixBusy: boolean
  meterAnalyser: AnalyserNode | null
  composed: boolean
  video: ActiveVideo | null
  guides: GuideMode
  isFullscreen: boolean
  onSeekTo: (ms: number) => void
  onShuttle: (dir: -1 | 1) => void
  onPlayPause: () => void
  onSnapFrame: () => void
  onRenderSection: () => void
  onAudioToggle: () => void
  onComposedToggle: () => void
  onCycleGuides: () => void
  onFullscreenToggle: () => void
}

export default function PreviewTransport({
  playheadMs,
  durationMs,
  fps,
  timeMode,
  rate,
  playing,
  snapNote,
  snapBusy,
  hasProject,
  exactBusy,
  hasSection,
  audioOn,
  mixBusy,
  meterAnalyser,
  composed,
  video,
  guides,
  isFullscreen,
  onSeekTo,
  onShuttle,
  onPlayPause,
  onSnapFrame,
  onRenderSection,
  onAudioToggle,
  onComposedToggle,
  onCycleGuides,
  onFullscreenToggle,
}: PreviewTransportProps) {
  return (
    <div className="pv-transport" data-cut-transport>
      <span className="pv-tc" data-cut-tc>
        {formatClock(playheadMs, fps, timeMode)} <span className="pv-dur">/ {formatClock(durationMs, fps, timeMode)}</span>
      </span>
      <div className="pv-buttons">
        <button className="pv-btn" title="To start (Home)" data-cut-transport-btn="start" onClick={() => onSeekTo(0)}>
          <Icon name="skipBack" size={14} />
        </button>
        <button className={`pv-btn ${rate < 0 ? 'pv-btn--active' : ''}`} title="Shuttle back (J)" data-cut-transport-btn="back" onClick={() => onShuttle(-1)}>
          <Icon name="rewind" size={14} />
          {rate < -1 && <span className="pv-rate">{Math.abs(rate)}×</span>}
        </button>
        <button className="pv-btn" title="Play/Pause (Space)" data-cut-transport-btn="play" onClick={onPlayPause}>
          {playing ? (
            <Icon name="pause" size={14} />
          ) : (
            <Icon name="play" size={14} />
          )}
        </button>
        <button className={`pv-btn ${rate > 1 ? 'pv-btn--active' : rate === 1 ? 'pv-btn--active' : ''}`} title="Shuttle forward (L)" data-cut-transport-btn="fwd" onClick={() => onShuttle(1)}>
          <Icon name="fastForward" size={14} />
          {rate > 1 && <span className="pv-rate">{rate}×</span>}
        </button>
        <button className="pv-btn" title="To end (End)" data-cut-transport-btn="end" onClick={() => onSeekTo(durationMs)}>
          <Icon name="skipForward" size={14} />
        </button>
      </div>
      <div className="pv-chips">
        {snapNote && <span className="pv-snap-note" data-cut-snap-note>{snapNote}</span>}
        <button
          className="pv-toggle pv-icon-toggle pv-snap"
          data-cut-action="snapshot-frame"
          disabled={snapBusy || !hasProject}
          aria-label={snapBusy ? 'Saving current frame' : 'Save current frame'}
          title="Save the current frame as an image asset"
          onClick={onSnapFrame}
        >
          <Icon name="exportFrame" size={14} />
          <span className="pv-control-label">{snapBusy ? '…' : 'Frame'}</span>
        </button>
        <button
          className="pv-toggle pv-icon-toggle pv-section"
          data-cut-action="render-section"
          disabled={exactBusy || !hasProject || !hasSection}
          aria-label={exactBusy ? 'Rendering selected range' : 'Render selected range'}
          title={hasSection
            ? 'Render the selected range to the EXACT composite — verify the final look with full audio, then Save to Assets'
            : 'Drag on the ruler to select a span (or select clips), then render it before saving to Assets'}
          onClick={onRenderSection}
        >
          <Icon name="render" size={14} />
          <span className="pv-control-label">{exactBusy ? 'Rendering…' : 'Render selection'}</span>
        </button>
        <button
          className={`pv-toggle pv-icon-toggle pv-audio ${audioOn ? 'pv-toggle--on' : ''}`}
          data-cut-audio-toggle
          data-cut-audio-on={audioOn ? 'true' : 'false'}
          aria-pressed={audioOn}
          aria-label={audioOn ? 'Mute timeline audio monitoring' : 'Enable timeline audio monitoring'}
          title={
            audioOn
              ? 'Timeline audio monitoring ON — play to hear the full mix (all tracks, gains, fades; matches the export). Click to mute the preview.'
              : 'Timeline audio monitoring OFF — the preview is silent. Click to hear the mix on play.'
          }
          onClick={onAudioToggle}
        >
          {mixBusy
            ? <><Icon name="volume" size={14} tone="audio" /><span className="pv-control-label">…</span></>
            : audioOn
              ? <><Icon name="volume" size={14} tone="audio" /><span className="pv-control-label">Audio</span></>
              : <><Icon name="mute" size={14} /><span className="pv-control-label">Audio</span></>}
        </button>
        <MasterMeter analyser={meterAnalyser} active={audioOn && rate === 1} />
        <button
          className={`pv-toggle pv-icon-toggle pv-composed ${composed ? 'pv-toggle--on' : ''}`}
          data-cut-quality-toggle
          data-cut-composed={composed ? 'true' : 'false'}
          aria-pressed={composed}
          aria-label={composed ? 'Show raw source preview' : 'Show composed preview'}
          title={
            composed
              ? 'COMPOSED preview ON — responsive live composite during playback; exact engine frame while paused. Click for the fastest source view.'
              : `COMPOSED preview OFF — fast ${video?.kind ?? 'source'} playback. Click to inspect the exact composed frame when paused.`
          }
          onClick={onComposedToggle}
        >
          <Icon name="layers" size={14} className="pv-composed-icon" />
          <span className="pv-composed-label">COMPOSED</span>
          <span className="pv-composed-state">{composed ? 'ON' : 'OFF'}</span>
        </button>
        <button
          className={`pv-btn ${guides !== 'off' ? 'pv-btn--active' : ''}`}
          data-cut-action="cycle-guides"
          data-cut-guides={guides}
          aria-pressed={guides !== 'off'}
          title={`${guideLabel(guides)} — click or press G to cycle (off → thirds → safe → both)`}
          onClick={onCycleGuides}
        >
          <Icon name="gridDense" size={14} />
        </button>
        <button
          className={`pv-btn ${isFullscreen ? 'pv-btn--active' : ''}`}
          data-cut-action="fullscreen-toggle"
          data-cut-fullscreen-on={isFullscreen ? 'true' : 'false'}
          aria-pressed={isFullscreen}
          title={isFullscreen ? 'Exit full screen (F or Esc)' : 'Full-screen preview (F)'}
          onClick={onFullscreenToggle}
        >
          <Icon name={isFullscreen ? 'fullscreenExit' : 'fullscreen'} size={14} />
        </button>
      </div>
    </div>
  )
}
