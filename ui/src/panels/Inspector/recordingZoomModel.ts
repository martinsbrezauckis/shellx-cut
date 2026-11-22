import type { RecordingZoomListResult } from '../../lib/client'
type Segment = RecordingZoomListResult['segments'][number]
export function zoomLabel(s: Pick<Segment, 'scale' | 'origin' | 'timeline'>): string {
 const time = s.timeline ? `${s.timeline.start_ms / 1000}–${s.timeline.end_ms / 1000} s` : 'Outside this clip'
 return `${time} · ${s.scale}× · ${s.origin === 'auto' ? 'Automatic' : 'Mine'}`
}
export function speedPatch(speed: string) {
 const factor = speed === 'Slow' ? 2 : speed === 'Fast' ? 0.5 : 1
 return { ramp_in_ms: 550 * factor, ramp_out_ms: 500 * factor }
}
export function timelineToSource(time: number, s: Pick<Segment, 'start_ms' | 'end_ms' | 'timeline'>): number | null {
 if (!s.timeline || s.timeline.end_ms === s.timeline.start_ms) return null
 return Math.round(s.start_ms + (time - s.timeline.start_ms) * (s.end_ms - s.start_ms) / (s.timeline.end_ms - s.timeline.start_ms))
}
