// Typed wire models for cited media intelligence. Kept outside the broad
// compatibility registries so this feature can evolve without growing them.

export type MediaEvidenceKind =
  | 'transcript'
  | 'visual'
  | 'scene'
  | 'beat'
  | 'marker'
  | 'metadata'

export interface MediaIntelligenceStatusArgs { asset_ids?: string[] }
export interface MediaIntelligenceRebuildArgs { asset_ids?: string[]; kinds?: MediaEvidenceKind[] }
export interface MediaIntelligenceSearchArgs {
  query: string
  asset_ids?: string[]
  kinds?: MediaEvidenceKind[]
  scope?: 'all_project_media' | 'this_sequence'
  limit?: number
  cursor?: string
}
export interface InspectMediaArgs { asset_ids?: string[] }
export interface InspectRangeArgs { evidence_ids: string[]; index_id?: string }

export interface MediaEvidenceCoverage {
  ready: number
  total: number
  missing: number
  stale?: number
}

export interface MediaIntelligenceAssetStatus {
  asset_id: string
  available: boolean
  kinds: Partial<Record<MediaEvidenceKind, {
    source: boolean
    indexed: boolean
    state: 'ready' | 'stale' | 'missing'
  }>>
}

export interface MediaIntelligenceStatusResult {
  schema: 'shellx-cut/media-intelligence-status/1'
  index_id?: string | null
  project_revision?: string | null
  complete: boolean
  stale: boolean
  entry_count: number
  coverage: Record<MediaEvidenceKind, MediaEvidenceCoverage>
  assets: MediaIntelligenceAssetStatus[]
}

export interface MediaEvidenceOccurrence {
  sequence_id: string
  clip_id: string
  track_id: string
  timeline_start_ms: number
  timeline_end_ms: number
}

export interface MediaEvidenceHit {
  schema: 'shellx-cut/evidence-hit/1'
  evidence_id: string
  asset_id: string
  source_start_ms: number
  source_end_ms: number
  anchor_ms: number
  kind: MediaEvidenceKind
  excerpt: string
  speaker?: string | null
  match: 'exact' | 'semantic' | 'contains' | 'inspect'
  relevance?: number | null
  provenance: { kind: MediaEvidenceKind; sha256: string }
  available: boolean
  occurrence_count: number
  occurrences: MediaEvidenceOccurrence[]
}

export interface MediaIntelligenceSearchResult {
  schema: 'shellx-cut/evidence-search-result/1'
  query: string
  index_id: string
  complete: boolean
  stale_excluded: number
  warnings: string[]
  count: number
  hits: MediaEvidenceHit[]
  next_cursor?: string | null
}

export interface MediaInspectionAsset {
  asset_id: string
  content_hash: string
  available: boolean
  media_kind: string
  duration_ms?: number | null
  has_video: boolean
  has_audio: boolean
  evidence: MediaIntelligenceAssetStatus['kinds']
}

export interface MediaInspectionResult {
  schema: 'shellx-cut/media-inspection/1'
  project_revision?: string | null
  index_id?: string | null
  total: number
  count: number
  truncated: boolean
  assets: MediaInspectionAsset[]
}

export interface EvidenceInspectionResult {
  schema: 'shellx-cut/evidence-inspection/1'
  index_id: string
  count: number
  hits: MediaEvidenceHit[]
}
