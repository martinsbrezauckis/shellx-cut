// Bounded project revision sync. The server only emits a delta when every
// change below is lossless to apply locally; any other edit arrives as a full
// snapshot instead of a guessed client-side timeline replay.

import { callVerb, type OpRecord, type Project, type VerbResult } from './client'

/** Ephemeral revision metadata is intentionally kept out of the durable
 * project model: only project.state response consumers need it. */
export type SyncedProject = Project & {
  project_revision?: string | null
  sync?: unknown
}

export type ProjectChange =
  | { kind: 'marker_upsert'; marker: { id: string; [key: string]: unknown } }
  | { kind: 'marker_remove'; id: string }
  | { kind: 'asset_upsert'; id: string; asset: unknown }
  | { kind: 'asset_remove'; id: string }
  | { kind: 'project_name'; name: string }

export interface ProjectDelta {
  mode: 'delta'
  from_revision: string
  project_revision?: string | null
  ops: OpRecord[]
  changes: ProjectChange[]
  affected: { markers: number; assets: number; project: number }
  encoded_bytes: number
}

export interface ProjectOpsPage {
  ops: OpRecord[]
  next_cursor?: string | null
  has_more: boolean
  project_revision?: string | null
}

/** App owns the one complete durable-history load for an open project. A
 * normal project object replacement (delta, snapshot, or job refresh) must
 * not trigger another cold replay; only the project-switch reset makes it
 * eligible again. */
export function needsColdHistoryLoad(project: Project | null | undefined, initialHistoryLoaded: boolean): boolean {
  return project != null && !initialHistoryLoaded
}

export type ProjectSync =
  | { mode: 'snapshot'; project: SyncedProject; reason: string; projectRevision?: string | null }
  | { mode: 'delta'; delta: ProjectDelta }
  /** A confirmed server close is different from a transient failed pull. */
  | { mode: 'no_project' }

/** Build the revision pull for an event or reconnect. A mismatched advertised
 * predecessor is a missed-event gap, but the durable cursor remains our last
 * applied revision: the server returns every missing bounded delta or a
 * snapshot fallback. */
export function revisionPull(cachedRevision?: string | null, advertisedPrevious?: string): {
  sinceRevision?: string
  missedEventGap: boolean
} {
  return {
    sinceRevision: cachedRevision ?? undefined,
    missedEventGap: Boolean(cachedRevision && advertisedPrevious && cachedRevision !== advertisedPrevious),
  }
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

function string(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

function deltaFrom(value: unknown): ProjectDelta | null {
  const outer = record(value)
  const sync = outer ? record(outer.sync) : null
  if (!sync || sync.mode !== 'delta') return null
  const from = string(sync.from_revision)
  const ops = Array.isArray(sync.ops) ? sync.ops as OpRecord[] : null
  const changes = Array.isArray(sync.changes) ? sync.changes as ProjectChange[] : null
  const affected = record(sync.affected)
  const bytes = typeof sync.encoded_bytes === 'number' ? sync.encoded_bytes : null
  if (!from || !ops || !changes || !affected || bytes == null) return null
  return {
    mode: 'delta',
    from_revision: from,
    project_revision: string(sync.project_revision) ?? null,
    ops,
    changes,
    affected: {
      markers: typeof affected.markers === 'number' ? affected.markers : 0,
      assets: typeof affected.assets === 'number' ? affected.assets : 0,
      project: typeof affected.project === 'number' ? affected.project : 0,
    },
    encoded_bytes: bytes,
  }
}

/** Fetch a small applicable delta after a known revision, or the server's
 * explicit full-state fallback for cold, invalid, stale, or broad changes. */
export async function fetchProjectSync(sinceRevision?: string): Promise<ProjectSync | null> {
  const result = await callVerb(
    'project.state',
    sinceRevision ? { since_revision: sinceRevision } : {},
  )
  return projectSyncFromVerbResult(result)
}

/** Decode the server envelope without collapsing a confirmed close into a
 * transient/unknown failure. The latter deliberately preserves cached UI state
 * until a later pull can establish the truth. */
export function projectSyncFromVerbResult(result: VerbResult<unknown>): ProjectSync | null {
  if (!result.ok) return result.error?.code === 'no_project' ? { mode: 'no_project' } : null
  if (!result.result) return null
  const delta = deltaFrom(result.result)
  if (delta) return { mode: 'delta', delta }
  const project = result.result as SyncedProject
  const sync = record(project.sync)
  return {
    mode: 'snapshot',
    project,
    reason: string(sync?.reason) ?? 'cold',
    projectRevision: string(sync?.project_revision) ?? project.project_revision,
  }
}

/** Resolve a failed reconnect without guessing. Only a confirmed `no_project`
 * clears an existing workspace; all other failed/unavailable responses retain
 * the cached project until a later pull succeeds. */
export function projectAfterUnavailableSync(
  cached: SyncedProject | null,
  response: ProjectSync | null,
): SyncedProject | null {
  return response?.mode === 'no_project' ? null : cached
}

/** Apply only server-certified changes. This never attempts a local timeline
 * reducer: unsupported edits are represented by a snapshot response. */
export function applyProjectDelta(project: SyncedProject, delta: ProjectDelta): SyncedProject {
  let markers = project.markers
  let assets = project.assets
  let name = project.name
  for (const change of delta.changes) {
    switch (change.kind) {
      case 'marker_upsert': {
        const marker = change.marker as unknown as Project['markers'][number]
        const at = markers.findIndex((existing) => existing.id === marker.id)
        markers = at < 0
          ? [...markers, marker]
          : markers.map((existing, index) => index === at ? marker : existing)
        break
      }
      case 'marker_remove':
        markers = markers.filter((marker) => marker.id !== change.id)
        break
      case 'asset_upsert':
        assets = { ...assets, [change.id]: change.asset as Project['assets'][string] }
        break
      case 'asset_remove': {
        const { [change.id]: _removed, ...rest } = assets
        assets = rest
        break
      }
      case 'project_name':
        name = change.name
        break
    }
  }
  return {
    ...project,
    markers,
    assets,
    name,
    project_revision: delta.project_revision ?? project.project_revision,
    sync: { mode: 'delta', project_revision: delta.project_revision, affected: delta.affected },
  }
}

/** Empty bounded deltas only acknowledge a revision already held by the UI;
 * they must not advance snapshot-reconciliation counters. */
export function projectDeltaChangesState(delta: ProjectDelta): boolean {
  return delta.ops.length > 0
}

/** Deduplicate durable history while preserving canonical op-id order. */
export function mergeProjectOps(existing: OpRecord[], incoming: OpRecord[]): OpRecord[] {
  const byId = new Map(existing.map((op) => [op.op_id, op]))
  for (const op of incoming) byId.set(op.op_id, op)
  return [...byId.values()].sort(compareOpRecords)
}

/** `format_id` widens beyond six digits (`op_1000000`), so lexicographic sort
 * ceases to be chronological after `op_999999`. Canonical ids sort by arbitrary
 * precision sequence; malformed legacy ids retain the prior lexical fallback. */
function compareOpRecords(left: OpRecord, right: OpRecord): number {
  const leftSequence = canonicalOpSequence(left.op_id)
  const rightSequence = canonicalOpSequence(right.op_id)
  if (leftSequence != null && rightSequence != null && leftSequence !== rightSequence) {
    return leftSequence < rightSequence ? -1 : 1
  }
  return left.op_id.localeCompare(right.op_id)
}

function canonicalOpSequence(opId: string): bigint | null {
  const match = /^op_(\d+)$/.exec(opId)
  if (!match) return null
  try {
    return BigInt(match[1])
  } catch {
    return null
  }
}

/** Read the complete durable history through small, ordered pages. Callers own
 * the current-project guard; a false guard discards all partial results, so a
 * switch can never merge records from another project. `next_cursor` is only
 * required while has_more is true; a final page is anchored by its
 * project_revision. */
export async function loadProjectOpsPages(
  fetchPage: (cursor?: string) => Promise<ProjectOpsPage | null>,
  isCurrent: () => boolean,
  cursor?: string,
): Promise<{ ops: OpRecord[]; projectRevision?: string | null } | null> {
  const ops: OpRecord[] = []
  const seen = new Set<string>()
  let next = cursor
  for (;;) {
    if (!isCurrent()) return null
    const page = await fetchPage(next)
    if (!page || !isCurrent()) return null
    for (const op of page.ops) {
      if (!seen.has(op.op_id)) {
        seen.add(op.op_id)
        ops.push(op)
      }
    }
    if (!page.has_more) return { ops, projectRevision: page.project_revision }
    const pageCursor = page.next_cursor ?? undefined
    const last = page.ops.at(-1)?.op_id
    if (!pageCursor || pageCursor === next || pageCursor !== last) return null
    next = pageCursor
  }
}
