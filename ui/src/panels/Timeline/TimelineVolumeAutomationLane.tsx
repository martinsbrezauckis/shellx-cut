import { useEffect, useMemo, useRef, useState, type PointerEvent } from 'react'
import type { Keyframe } from '../../lib/client'
import { useVolumeAutomation } from '../../app/VolumeAutomationContext'
import {
  clampAutomationTime,
  formatAutomationTime,
} from '../Inspector/volumeAutomationModel'
import {
  VOLUME_AUTOMATION_MAX_DB,
  VOLUME_AUTOMATION_MIN_DB,
  addVolumeAutomationPoint,
  formatAutomationDb,
  linearVolumeToDb,
  moveVolumeAutomationPoint,
  playheadIsOverSelectedClip,
  volumeAutomationDragStillCurrent,
} from './volumeAutomationLaneModel'
import { msToPx, type LaidItem } from './layout'

interface TimelineVolumeAutomationLaneProps {
  item: LaidItem
  clip: { id: string; keyframes?: Keyframe[]; gain_db?: number; speed_ramp?: unknown }
  playheadMs: number
  projectRevision: string
  locked: boolean
  zoom: number
}

type LanePoint = { t_ms: number; value: number }

function pointStyle(point: LanePoint, durationMs: number) {
  const left = `${(100 * point.t_ms) / Math.max(1, durationMs)}%`
  const top = `${(100 * (VOLUME_AUTOMATION_MAX_DB - linearVolumeToDb(point.value))) / (VOLUME_AUTOMATION_MAX_DB - VOLUME_AUTOMATION_MIN_DB)}%`
  return { left, top }
}

function curvePoints(points: LanePoint[], durationMs: number) {
  return points.map((point) => {
    const x = (100 * point.t_ms) / Math.max(1, durationMs)
    const y = (100 * (VOLUME_AUTOMATION_MAX_DB - linearVolumeToDb(point.value))) / (VOLUME_AUTOMATION_MAX_DB - VOLUME_AUTOMATION_MIN_DB)
    return `${x},${y}`
  }).join(' ')
}

/** Selected-audio clip rubber band. Its pointer transaction stays local until
 * pointer-up; the shared coordinator then sends exactly one full sorted SET. */
export default function TimelineVolumeAutomationLane({ item, clip, playheadMs, projectRevision, locked, zoom }: TimelineVolumeAutomationLaneProps) {
  const durationMs = Math.max(0, Math.round(item.durMs))
  const gainDb = typeof clip.gain_db === 'number' ? clip.gain_db : 0
  const automation = useVolumeAutomation({
    clipId: clip.id,
    durationMs,
    keyframes: clip.keyframes,
    projectRevision,
    hasSpeedRamp: clip.speed_ramp != null,
    staticGainDb: gainDb,
  })
  const [preview, setPreview] = useState<LanePoint[] | null>(null)
  const [dragging, setDragging] = useState(false)
  const [laneNotice, setLaneNotice] = useState('')
  const dragRef = useRef<{
    originalTimeMs: number
    initial: LanePoint[]
    revision: string | null
    trackFingerprint: string
  } | null>(null)
  const previewRef = useRef<LanePoint[] | null>(null)
  const dragCleanupRef = useRef<(() => void) | null>(null)
  previewRef.current = preview
  const points = preview ?? automation.track.points
  const currentRevisionRef = useRef<string | null>(automation.mutationState.projectRevision)
  const currentTrackFingerprintRef = useRef(JSON.stringify(automation.track))
  currentRevisionRef.current = automation.mutationState.projectRevision
  currentTrackFingerprintRef.current = JSON.stringify(automation.track)
  const clipPlayheadMs = clampAutomationTime(playheadMs - item.startMs, durationMs)
  const playheadIsOnClip = playheadIsOverSelectedClip(playheadMs, item.startMs, durationMs)
  const unavailableReason = locked
    ? 'Unlock this audio track before editing clip volume.'
    : automation.unavailableReason
  const inFlightReason = automation.mutationState.inFlight
    ? 'Volume automation is updating. Wait for the current save to finish.'
    : null
  const enabled = unavailableReason === null && !automation.mutationState.inFlight
  const addAtPlayheadReason = unavailableReason ?? inFlightReason
    ?? (!playheadIsOnClip ? 'Move playhead over the selected clip to add a volume point.' : null)
  const addAtPlayheadEnabled = addAtPlayheadReason === null && !automation.mutationState.inFlight
  const laneTitle = addAtPlayheadReason ?? 'Clip volume. Drag a point, use Add point at playhead, or Ctrl/Cmd-click the curve.'
  const polyline = useMemo(() => curvePoints(points, durationMs), [points, durationMs])

  const cancelDrag = () => {
    dragCleanupRef.current?.()
    dragCleanupRef.current = null
    dragRef.current = null
    previewRef.current = null
    setPreview(null)
    setDragging(false)
  }

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && dragRef.current) {
        event.preventDefault()
        cancelDrag()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])

  useEffect(() => () => {
    dragCleanupRef.current?.()
    dragCleanupRef.current = null
    dragRef.current = null
    previewRef.current = null
  }, [])

  const addAtPlayhead = () => {
    if (!addAtPlayheadEnabled) return
    const next = addVolumeAutomationPoint(automation.track.points, durationMs, clipPlayheadMs, gainDb)
    void automation.commit(next, automation.track.interp, `timeline: add volume point at ${clipPlayheadMs}ms`)
  }

  const onCurvePointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (!enabled || (!event.ctrlKey && !event.metaKey)) return
    event.preventDefault()
    event.stopPropagation()
    const rect = event.currentTarget.getBoundingClientRect()
    const time = clampAutomationTime(((event.clientX - rect.left) / Math.max(1, rect.width)) * durationMs, durationMs)
    const next = addVolumeAutomationPoint(automation.track.points, durationMs, time, gainDb)
    void automation.commit(next, automation.track.interp, `timeline: add volume point at ${time}ms (accelerator)`)
  }

  const startDrag = (event: PointerEvent<HTMLButtonElement>, point: LanePoint) => {
    if (!enabled) return
    event.preventDefault()
    event.stopPropagation()
    const initial = automation.track.points.slice()
    const lane = event.currentTarget.closest<HTMLElement>('[data-cut-volume-lane]')
    setLaneNotice('')
    dragRef.current = {
      originalTimeMs: point.t_ms,
      initial,
      revision: automation.mutationState.projectRevision,
      trackFingerprint: JSON.stringify(automation.track),
    }
    previewRef.current = initial
    setPreview(initial)
    setDragging(true)
    const onMove = (move: globalThis.PointerEvent) => {
      const current = dragRef.current
      if (!current) return
      const rect = lane?.getBoundingClientRect()
      if (!rect) return
      const time = clampAutomationTime(((move.clientX - rect.left) / Math.max(1, rect.width)) * durationMs, durationMs)
      const db = VOLUME_AUTOMATION_MAX_DB - ((move.clientY - rect.top) / Math.max(1, rect.height)) * (VOLUME_AUTOMATION_MAX_DB - VOLUME_AUTOMATION_MIN_DB)
      const next = moveVolumeAutomationPoint(current.initial, current.originalTimeMs, time, db, durationMs)
      previewRef.current = next
      setPreview(next)
    }
    const cleanup = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onEnd)
      window.removeEventListener('pointercancel', onCancel)
    }
    const onEnd = (up: globalThis.PointerEvent) => {
      cleanup()
      dragCleanupRef.current = null
      const current = dragRef.current
      const next = previewRef.current
      dragRef.current = null
      previewRef.current = null
      setPreview(null)
      setDragging(false)
      if (!current || !next || up.type !== 'pointerup') return
      if (!volumeAutomationDragStillCurrent(
        current.revision,
        current.trackFingerprint,
        currentRevisionRef.current,
        currentTrackFingerprintRef.current,
      )) {
        setLaneNotice('Project changed while dragging. Review the current volume points and try again.')
        return
      }
      void automation.commit(next, automation.track.interp, `timeline: move volume point from ${current.originalTimeMs}ms`)
    }
    const onCancel = () => {
      cleanup()
      dragCleanupRef.current = null
      cancelDrag()
    }
    dragCleanupRef.current = cleanup
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onEnd)
    window.addEventListener('pointercancel', onCancel)
  }

  return (
    <div
      className={`tl-volume-lane${dragging ? ' tl-volume-lane--dragging' : ''}${enabled ? '' : ' tl-volume-lane--disabled'}`}
      style={{ left: msToPx(item.startMs, zoom), width: Math.max(2, msToPx(item.durMs, zoom)) }}
      data-cut-volume-lane={clip.id}
      data-cut-volume-lane-disabled={unavailableReason ?? undefined}
      aria-label={`Clip volume for ${item.label}`}
      title={laneTitle}
      onPointerDown={onCurvePointerDown}
    >
      <div className="tl-volume-lane__clip">
        <svg className="tl-volume-lane__curve" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
          {points.length > 0 && <polyline points={polyline} />}
        </svg>
        <button
          type="button"
          className="tl-volume-lane__add"
          data-cut-action="timeline-volume-add"
          aria-label={`Add volume point at playhead ${formatAutomationTime(clipPlayheadMs)}`}
          disabled={!addAtPlayheadEnabled}
          onPointerDown={(event) => event.stopPropagation()}
          onMouseDown={(event) => event.stopPropagation()}
          onClick={(event) => { event.stopPropagation(); addAtPlayhead() }}
        >Add point</button>
        {points.map((point) => (
          <button
            type="button"
            key={`${point.t_ms}:${point.value}`}
            className="tl-volume-lane__point"
            style={pointStyle(point, durationMs)}
            data-cut-action="timeline-volume-point"
            data-cut-volume-point={point.t_ms}
            aria-label={`Volume point at ${formatAutomationTime(point.t_ms)}, ${formatAutomationDb(point.value)}. Drag to adjust; use Inspector for exact values.`}
            disabled={!enabled}
            onPointerDown={(event) => startDrag(event, point)}
            onKeyDown={(event) => {
              if (event.key !== 'Enter' && event.key !== ' ') return
              event.preventDefault()
              event.stopPropagation()
              document.dispatchEvent(new CustomEvent('cut:edit-volume-automation-point', {
                detail: { clipId: clip.id, tMs: point.t_ms },
              }))
            }}
          />
        ))}
        {addAtPlayheadReason && <span className="tl-volume-lane__reason" data-cut-volume-lane-unavailable>{addAtPlayheadReason}</span>}
        {(laneNotice || automation.message) && <span className="tl-volume-lane__status" role="status" aria-live="polite">{laneNotice || automation.message}</span>}
      </div>
    </div>
  )
}
