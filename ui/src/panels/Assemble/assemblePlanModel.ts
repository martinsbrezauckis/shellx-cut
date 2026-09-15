import type { AssemblePlanBinding, Project, VerbArgs } from '../../lib/client'

export type AssembleMode = 'shorts' | 'repurpose' | 'from_script' | 'broll'
export type EditableAssembleMode = Exclude<AssembleMode, 'broll'>
export type ApplyState = 'idle' | 'applying' | 'applied'

export interface RepurposeClip {
  rank: number
  range_ms: [number, number]
  duration_ms: number
  text: string
  score: number
  reason?: string
}

export interface ScriptSegment {
  line_idx: number
  script_line: string
  matched: boolean
  score: number
  range_ms: [number, number] | null
  text: string
}

export interface BrollPlaced {
  query: string
  at_ms: number
  duration_ms: number
}

export interface ShortsItem {
  rank: number
  range_ms: [number, number]
  duration_ms: number
  score: number
  reason?: string
  title: string
  factors?: Record<string, number>
  reframe?: { aspect: string; crop: { x: number; y: number; w: number; h: number } | null }
  has_captions?: boolean
}

export type RepurposeRequest = Omit<VerbArgs['assemble.repurpose'], 'apply'>
export type ShortsRequest = Omit<VerbArgs['assemble.shorts'], 'apply'>
export type ScriptRequest = Omit<VerbArgs['assemble.from_script'], 'apply'>

export interface ReviewedPlan<T> {
  request: T
  binding: AssemblePlanBinding
}

export interface ShortsMaterialization {
  eligible: boolean
  required_aspect: string
  project_aspect: string
  reason?: string
}

export function assembleRequestId(mode: string): string {
  const suffix = typeof globalThis.crypto?.randomUUID === 'function'
    ? globalThis.crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(36).slice(2)}`
  return `assemble-${mode}-${suffix}`
}

/** Project identity owns deferred request lifetime; revision owns plan freshness. */
export function assembleProjectIdentityScope(project: Project | null): string {
  const identity = project?.project_identity
  return [
    identity?.schema ?? '',
    identity?.origin_path_sha256 ?? '',
    identity?.project_name ?? '',
  ].join('\u0000')
}

export function assembleProjectRevision(project: Project | null): string {
  return project?.project_revision ?? ''
}
