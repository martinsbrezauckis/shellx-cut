import type { DoctorReport } from '../../lib/doctor'
import { clipEffectsOf, type InspectorMediaClip } from './model'

export interface InspectorTaskSummary {
  label: string
  tone: 'neutral' | 'active' | 'warning'
}

function joined(parts: string[], fallback: string): InspectorTaskSummary {
  return {
    label: parts.length > 0 ? parts.join(' · ') : fallback,
    tone: parts.length > 0 ? 'active' : 'neutral',
  }
}

export function videoMotionSummary(clip: InspectorMediaClip): InspectorTaskSummary {
  const parts: string[] = []
  if (clip.stabilize) parts.push('Stabilized')
  const scaleKeyframes = (clip.keyframes ?? []).filter((keyframe) => keyframe.param === 'scale').length
  if (scaleKeyframes > 0) parts.push(`${scaleKeyframes} zoom keyframe${scaleKeyframes === 1 ? '' : 's'}`)
  return joined(parts, 'Stabilization and auto zoom')
}

export function videoColorSummary(clip: InspectorMediaClip): InspectorTaskSummary {
  const parts: string[] = []
  if (clip.grade) parts.push('Grade applied')
  const layers = clip.grade_stack?.length ?? 0
  const windows = clip.grade_windows?.length ?? 0
  if (layers > 0) parts.push(`${layers} layer${layers === 1 ? '' : 's'}`)
  if (windows > 0) parts.push(`${windows} window${windows === 1 ? '' : 's'}`)
  if (clip.input_color_space) parts.push(clip.input_color_space.toUpperCase())
  return joined(parts, 'No color changes')
}

export function videoEffectsSummary(clip: InspectorMediaClip, blendMode: string): InspectorTaskSummary {
  const parts: string[] = []
  const effects = clipEffectsOf(clip.effects).length
  if (effects > 0) parts.push(`${effects} effect${effects === 1 ? '' : 's'}`)
  if (blendMode !== 'normal') parts.push(`${blendMode} blend`)
  return joined(parts, 'No clip effects')
}

export function videoPrivacySummary(clip: InspectorMediaClip): InspectorTaskSummary {
  const parts: string[] = []
  if (clip.mask) parts.push('Redaction applied')
  if (clip.matte) parts.push('Background removed')
  return joined(parts, 'No privacy effect')
}

export function audioCleanupSummary(clip: InspectorMediaClip): InspectorTaskSummary {
  const parts: string[] = []
  const effects = clipEffectsOf(clip.effects).length
  if (effects > 0) parts.push(`${effects} effect${effects === 1 ? '' : 's'}`)
  if (clip.eq) parts.push('EQ applied')
  return joined(parts, 'No cleanup chain')
}

export function duckingSummary(speechTrackId: string | null): InspectorTaskSummary {
  return speechTrackId
    ? { label: 'Speech reference ready', tone: 'active' }
    : { label: 'Needs a second audio track', tone: 'warning' }
}

export function stabilizationReadiness(report: DoctorReport | null): {
  ready: boolean
  reason: string | null
} {
  if (!report) return { ready: false, reason: 'Checking the installed video tools…' }
  const ffmpeg = report.cards.find((card) => card.id === 'ffmpeg')
  if (!ffmpeg || ffmpeg.status === 'unknown') {
    return { ready: false, reason: 'Video tools could not be verified. Re-scan in Settings.' }
  }
  if (ffmpeg.status === 'missing') {
    return { ready: false, reason: 'Install video processing in Settings before stabilizing.' }
  }
  if (ffmpeg.details.can_stabilize !== true) {
    return { ready: false, reason: 'The selected FFmpeg build does not include stabilization.' }
  }
  return { ready: true, reason: null }
}
