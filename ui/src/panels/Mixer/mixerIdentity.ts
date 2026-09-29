import type { Project } from '../../lib/client'

/** Match projects by their canonical origin, not a user-editable display name. */
export function mixerProjectScope(project: Project | null): string {
  const identity = project?.project_identity
  return identity
    ? [identity.schema, identity.origin_path_sha256, identity.project_name].join('\u0000')
    : `unidentified\u0000${project?.name ?? ''}`
}

export function mixerMeasurementKey(project: Project | null, headOpId: string, trackIds: string[]): string {
  return [mixerProjectScope(project), project?.project_revision ?? '', headOpId, ...trackIds].join('\u0000')
}

export function mixerLoudnessKey(measurementKey: string, targetLufs: number): string {
  return `${measurementKey}\u0000${targetLufs}`
}

/** Reject a stem when the engine moved to another project or revision before readback. */
export function mixerSnapshotMatches(expected: Project | null, current: Project | null): boolean {
  return mixerProjectScope(expected) === mixerProjectScope(current)
    && expected?.project_revision === current?.project_revision
}
