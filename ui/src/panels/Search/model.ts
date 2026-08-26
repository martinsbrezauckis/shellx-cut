import type {
  MediaEvidenceHit,
  MediaEvidenceKind,
  MediaEvidenceOccurrence,
  MediaIntelligenceStatusResult,
  Project,
} from '../../lib/client'
import type { IconName } from '../../icons'

export type EvidenceMode = 'all' | 'spoken' | 'visual' | 'rhythm' | 'notes'
export type EvidenceScope = 'all_project_media' | 'this_sequence'

export const EVIDENCE_MODES: ReadonlyArray<{ value: EvidenceMode; label: string }> = [
  { value: 'all', label: 'All evidence' },
  { value: 'spoken', label: 'Spoken words' },
  { value: 'visual', label: 'Visuals' },
  { value: 'rhythm', label: 'Scenes & beats' },
  { value: 'notes', label: 'Notes & metadata' },
]

const MODE_KINDS: Record<EvidenceMode, MediaEvidenceKind[]> = {
  all: ['transcript', 'visual', 'scene', 'beat', 'marker', 'metadata'],
  spoken: ['transcript'],
  visual: ['visual'],
  rhythm: ['scene', 'beat'],
  notes: ['marker', 'metadata'],
}

export const KIND_PRESENTATION: Record<MediaEvidenceKind, { label: string; icon: IconName }> = {
  transcript: { label: 'Transcript', icon: 'transcript' },
  visual: { label: 'Visual', icon: 'videoClip' },
  scene: { label: 'Scene', icon: 'film' },
  beat: { label: 'Beat', icon: 'music' },
  marker: { label: 'Marker', icon: 'marker' },
  metadata: { label: 'Metadata', icon: 'file' },
}

export function evidenceKinds(mode: EvidenceMode): MediaEvidenceKind[] {
  return MODE_KINDS[mode]
}

export function projectIdentity(project: Project | null): string {
  if (!project) return 'closed'
  const assets = Object.entries(project.assets ?? {})
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([id, asset]) => `${id}:${asset.hash}`)
    .join('|')
  return `${project.name}:${project.active_sequence ?? ''}:${assets}`
}

export function assetLabel(project: Project | null, assetId: string): string {
  const path = project?.assets?.[assetId]?.path
  return path?.split(/[\\/]/).pop() || assetId
}

export function formatEvidenceTime(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000))
  const seconds = total % 60
  const minutes = Math.floor(total / 60) % 60
  const hours = Math.floor(total / 3600)
  return hours > 0
    ? `${hours}:${minutes.toString().padStart(2, '0')}:${seconds.toString().padStart(2, '0')}`
    : `${minutes}:${seconds.toString().padStart(2, '0')}`
}

export function nearestActiveOccurrence(
  hit: MediaEvidenceHit,
  project: Project | null,
  playheadMs: number,
): MediaEvidenceOccurrence | null {
  const active = project?.active_sequence
  const candidates = hit.occurrences.filter((occurrence) => !active || occurrence.sequence_id === active)
  return candidates.reduce<MediaEvidenceOccurrence | null>((nearest, occurrence) => {
    if (!nearest) return occurrence
    return Math.abs(occurrence.timeline_start_ms - playheadMs)
      < Math.abs(nearest.timeline_start_ms - playheadMs) ? occurrence : nearest
  }, null)
}

export function coverageSummary(status: MediaIntelligenceStatusResult | null): string {
  if (!status) return 'Checking searchable evidence…'
  if (!status.index_id) return 'Search evidence has not been prepared yet.'
  if (status.stale) return `${status.entry_count} cited moments ready; changed analysis is excluded until refresh.`
  if (!status.complete) return `${status.entry_count} cited moments ready from available analysis.`
  return `${status.entry_count} cited moments ready.`
}

export function selectedEvidencePrompt(hits: MediaEvidenceHit[], project: Project | null): string {
  const evidence = hits.slice(0, 12).map((hit, index) => {
    const range = `${formatEvidenceTime(hit.source_start_ms)}–${formatEvidenceTime(hit.source_end_ms)}`
    return `${index + 1}. ${assetLabel(project, hit.asset_id)} @ ${range} [${hit.kind}; ${hit.evidence_id}]\n${hit.excerpt}`
  }).join('\n\n')
  return [
    'Help me work with these cited moments from Find > Moment.',
    'Treat them as evidence, inspect the current project before proposing or applying any edit, and preserve source/timeline time distinctions.',
    '',
    evidence,
  ].join('\n')
}
