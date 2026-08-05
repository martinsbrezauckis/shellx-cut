import type { TimeDisplayMode, LaidItem } from './layout'
import { formatClock, ZOOM_KEY_FACTOR } from './layout'
import { TIME_DISPLAY_LABEL, TIME_DISPLAY_TITLE } from '../../lib/timedisplay'
import { Icon } from '../../icons'
import SpeedControl from './SpeedControl'
import TimelineSaveActions from './TimelineSaveActions'
import TimelineAutomationMenu from './TimelineAutomationMenu'
import type { Project } from '../../lib/client'

interface TimelineToolbarProps {
  playheadMs: number
  project: Project | null
  fps: number
  timeMode: TimeDisplayMode
  zoom: number
  razorMode: boolean
  /** Trim tool: select (normal) | slip | slide | roll — `t` cycles. */
  trimTool: 'select' | 'slip' | 'slide' | 'roll'
  snapEnabled: boolean
  selectedClipCount: number
  selectedMedia: LaidItem[]
  selectionSpeed: number | undefined
  hasBeatMarkers: boolean
  canMulticam: boolean
  syncNote: string | null
  savingRange: boolean
  savingGif: boolean
  saveNote: string | null
  onCycleTimeDisplay: () => void
  onZoom: (nextZoom: number, anchorMs: number) => void
  onToggleRazor: () => void
  onCycleTrimTool: () => void
  onToggleSnap: () => void
  onAddTrack: (kind: 'video' | 'audio') => void | Promise<void>
  onRippleTrim: (side: 'start' | 'end') => void | Promise<void>
  onDeleteSelection: (ripple: boolean) => void
  onSetSpeed: (factor: number) => void
  onSyncByAudio: () => void | Promise<void>
  onMulticamSwitch: () => void | Promise<void>
  onCutToBeat: () => void | Promise<void>
  onSaveRange: () => void
  onSaveGif: () => void
}

export default function TimelineToolbar({
  playheadMs,
  project,
  fps,
  timeMode,
  zoom,
  razorMode,
  trimTool,
  snapEnabled,
  selectedClipCount,
  selectedMedia,
  selectionSpeed,
  hasBeatMarkers,
  canMulticam,
  syncNote,
  savingRange,
  savingGif,
  saveNote,
  onCycleTimeDisplay,
  onZoom,
  onToggleRazor,
  onCycleTrimTool,
  onToggleSnap,
  onAddTrack,
  onRippleTrim,
  onDeleteSelection,
  onSetSpeed,
  onSyncByAudio,
  onMulticamSwitch,
  onCutToBeat,
  onSaveRange,
  onSaveGif,
}: TimelineToolbarProps) {
  const hasSelectedVideo = selectedMedia.some((i) => i.kind === 'video')
  const zoomLabel = `${zoom >= 1 ? zoom.toFixed(1) : zoom.toFixed(2)}\u00d7`
  return (
    <div className="tl-toolbar" data-cut-timeline-toolbar>
      <button
        className="tl-tc-chip"
        data-cut-tc-readout
        data-cut-time-display={timeMode}
        title={TIME_DISPLAY_TITLE[timeMode]}
        onClick={onCycleTimeDisplay}
      >
        {formatClock(playheadMs, fps, timeMode)}
        <span className="tl-tc-mode" data-cut-time-mode={timeMode}>{TIME_DISPLAY_LABEL[timeMode]}</span>
      </button>
      <div className="tl-zoom-chip">
        <button data-cut-zoom-out title="Zoom out (-)" onClick={() => onZoom(zoom / ZOOM_KEY_FACTOR, playheadMs)}>&minus;</button>
        <span>{zoomLabel}</span>
        <button data-cut-zoom-in title="Zoom in (+)" onClick={() => onZoom(zoom * ZOOM_KEY_FACTOR, playheadMs)}>+</button>
      </div>

      <div className="tl-tools" data-cut-tools>
        <button
          className={`tl-tool ${razorMode ? 'tl-tool--on' : ''}`}
          data-cut-tool="razor"
          data-cut-on={razorMode || undefined}
          title="Razor — click a clip to split it at the cursor. S splits at the playhead."
          aria-pressed={razorMode}
          onClick={onToggleRazor}
        >
          <Icon name="split" size={14} />
          Razor
        </button>
        <button
          className={`tl-tool ${trimTool !== 'select' ? 'tl-tool--on' : ''}`}
          data-cut-tool="trim"
          data-cut-trim-tool={trimTool}
          title="Trim tool (T cycles) — slip shifts the source, slide moves a clip between neighbours, and roll moves a shared cut. Esc returns to Select."
          aria-pressed={trimTool !== 'select'}
          onClick={onCycleTrimTool}
        >
          <Icon name="sliders" size={14} />
          {trimTool === 'select' ? 'Trim' : `Trim: ${trimTool}`}
        </button>
        <button
          className={`tl-tool ${snapEnabled ? 'tl-tool--on' : ''}`}
          data-cut-tool="snap"
          data-cut-on={snapEnabled || undefined}
          title="Snap magnet (N) - snap to clip edges, markers, playhead. Hold Shift to bypass per move."
          aria-pressed={snapEnabled}
          onClick={onToggleSnap}
        >
          <Icon name="snap" size={14} />
          Snap
        </button>
        <span className="tl-tool-sep" aria-hidden="true" />
        <button
          type="button"
          className="tl-tool"
          data-cut-action="add-video-track"
          disabled={!project}
          title="Add a video track for another angle, overlay, or picture-in-picture layer"
          onClick={() => void onAddTrack('video')}
        >
          <Icon name="video" size={14} />
          Video track
        </button>
        <button
          type="button"
          className="tl-tool"
          data-cut-action="add-audio-track"
          disabled={!project}
          title="Add an audio track for dialogue, music, ambience, or effects"
          onClick={() => void onAddTrack('audio')}
        >
          <Icon name="waveform" size={14} />
          Audio track
        </button>
        <span className="tl-tool-sep" aria-hidden="true" />
        <button
          type="button"
          className="tl-tool"
          data-cut-action="ripple-trim-start"
          disabled={!project}
          title="Ripple trim the active clip start to the playhead (Q)"
          onClick={() => void onRippleTrim('start')}
        >
          <Icon name="skipBack" size={14} />
          Trim start
        </button>
        <button
          type="button"
          className="tl-tool"
          data-cut-action="ripple-trim-end"
          disabled={!project}
          title="Ripple trim the active clip end to the playhead (W)"
          onClick={() => void onRippleTrim('end')}
        >
          <Icon name="skipForward" size={14} />
          Trim end
        </button>
        <button
          className="tl-tool"
          data-cut-tool="ripple-del"
          disabled={!selectedClipCount}
          title="Ripple delete (Del) - remove the selection and close the gap."
          onClick={() => onDeleteSelection(true)}
        >
          <Icon name="rippleDelete" size={14} />
          Ripple del
        </button>
        <button
          className="tl-tool"
          data-cut-tool="lift-del"
          disabled={!selectedClipCount}
          title="Lift delete (Alt+Del) - remove the selection and leave a gap of equal length."
          onClick={() => onDeleteSelection(false)}
        >
          <Icon name="liftDelete" size={14} />
          Lift del
        </button>
        <span className="tl-tool-sep" aria-hidden="true" />
        <SpeedControl
          disabled={!selectedMedia.length}
          current={selectionSpeed}
          onSet={onSetSpeed}
        />
        <span className="tl-tool-sep" aria-hidden="true" />
        <button
          type="button"
          className="tl-tool"
          data-cut-action="open-grade"
          disabled={!hasSelectedVideo}
          title="Adjust contrast, brightness, saturation, white balance, and looks"
          onClick={() => document.dispatchEvent(new CustomEvent('cut:open-grade'))}
        >
          Grade
        </button>
        <button
          type="button"
          className="tl-tool"
          data-cut-action="open-layer"
          disabled={!hasSelectedVideo}
          title="Position, scale, crop, and reorder the selected overlay"
          onClick={() => document.dispatchEvent(new CustomEvent('cut:open-layer'))}
        >
          Layer
        </button>
        <button
          type="button"
          className="tl-tool"
          data-cut-action="open-matte"
          disabled={!hasSelectedVideo}
          title="Cut the subject out of the selected clip without a green screen"
          onClick={() => document.dispatchEvent(new CustomEvent('cut:open-matte'))}
        >
          Matte
        </button>
        <TimelineSaveActions
          canSaveRange={selectedMedia.length > 0}
          canSaveGif={hasSelectedVideo}
          savingRange={savingRange}
          savingGif={savingGif}
          saveNote={saveNote}
          onSaveRange={onSaveRange}
          onSaveGif={onSaveGif}
        />
      </div>
      {syncNote && (
        <span className="tl-action-note" data-cut-timeline-action-note role="status" title={syncNote}>
          {syncNote}
        </span>
      )}
      <TimelineAutomationMenu
        project={project}
        selectedMediaCount={selectedMedia.length}
        hasBeatMarkers={hasBeatMarkers}
        canMulticam={canMulticam}
        syncNote={syncNote}
        onSyncByAudio={onSyncByAudio}
        onMulticamSwitch={onMulticamSwitch}
        onCutToBeat={onCutToBeat}
      />
    </div>
  )
}
