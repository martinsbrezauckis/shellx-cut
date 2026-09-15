import type { AssemblePlanBinding, VerbResult } from '../../lib/client'

type PlanVerb = 'assemble.repurpose' | 'assemble.shorts' | 'assemble.from_script'
type ApplyKind = 'reel' | 'shorts'

export interface AppliedAssemblePlan {
  materialized: true
  kind: ApplyKind
  asset: string
  spans_placed: number
  video_clip_ids: string[]
  audio_clip_ids: string[]
  caption_track: string | null
  caption_clip_ids: string[]
  total_ms: number
  undo: { verb: 'project.undo'; op_id: string }
}

function record(value: unknown): Record<string, unknown> | null {
  return value != null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

function stringArray(value: unknown): string[] | null {
  return Array.isArray(value) && value.every((item) => typeof item === 'string' && item.length > 0)
    ? value as string[]
    : null
}

function ranges(value: unknown): Array<[number, number]> | null {
  if (!Array.isArray(value)) return null
  const parsed: Array<[number, number]> = []
  for (const range of value) {
    if (!Array.isArray(range) || range.length !== 2 || !range.every(Number.isInteger)) return null
    parsed.push([range[0] as number, range[1] as number])
  }
  return parsed
}

/** Refuses malformed planner output before it can become an Apply capability. */
export function acceptPlanBinding(value: unknown, verb: PlanVerb): AssemblePlanBinding | null {
  const binding = record(value)
  const identity = binding && record(binding.project_identity)
  const selectedRanges = binding && ranges(binding.selected_ranges)
  const materialization = binding && record(binding.materialization)
  if (!binding || !identity || !selectedRanges || !materialization
    || binding.schema !== 'shellx-cut/assemble-plan-binding/1'
    || binding.verb !== verb
    || typeof binding.project_revision !== 'string' || !binding.project_revision
    || typeof binding.asset !== 'string' || !binding.asset
    || typeof binding.transcript_sha256 !== 'string' || !/^sha256:[0-9a-f]{64}$/.test(binding.transcript_sha256)
    || identity.schema !== 'shellx-cut/project-identity/1'
    || typeof identity.origin_path_sha256 !== 'string' || !identity.origin_path_sha256
    || typeof identity.project_name !== 'string' || !identity.project_name) return null
  if (verb !== 'assemble.from_script' && selectedRanges.length === 0) return null
  if (verb === 'assemble.shorts') {
    if (materialization.kind !== 'shorts' || typeof materialization.aspect !== 'string'
      || !Object.hasOwn(materialization, 'crop')) return null
  } else if (materialization.kind !== 'reel') return null
  return binding as unknown as AssemblePlanBinding
}

/** Binds UI success to the actual one-op receipt, never a truthy response object. */
export function acceptAppliedPlan(
  response: VerbResult<unknown>,
  verb: PlanVerb,
  expectedAsset: string,
): AppliedAssemblePlan | null {
  const value = response.ok ? record(response.result) : null
  const undo = value && record(value.undo)
  const videoClipIds = value && stringArray(value.video_clip_ids)
  const audioClipIds = value && stringArray(value.audio_clip_ids)
  const captionClipIds = value && stringArray(value.caption_clip_ids)
  const expectedKind: ApplyKind = verb === 'assemble.shorts' ? 'shorts' : 'reel'
  if (!value || !undo || !videoClipIds || !audioClipIds || !captionClipIds
    || value.materialized !== true || value.kind !== expectedKind
    || value.asset !== expectedAsset
    || !Number.isInteger(value.spans_placed) || (value.spans_placed as number) < 1
    || videoClipIds.length !== value.spans_placed
    || (value.caption_track !== null && typeof value.caption_track !== 'string')
    || !Number.isFinite(value.total_ms) || (value.total_ms as number) <= 0
    || undo.verb !== 'project.undo' || typeof undo.op_id !== 'string' || !undo.op_id
    || !Array.isArray(response.op_ids) || response.op_ids.length !== 1 || response.op_ids[0] !== undo.op_id
    || response.project_revision !== undo.op_id) return null
  if (verb === 'assemble.shorts' && (value.caption_track !== 'asmcap1' || captionClipIds.length === 0)) return null
  return value as unknown as AppliedAssemblePlan
}
