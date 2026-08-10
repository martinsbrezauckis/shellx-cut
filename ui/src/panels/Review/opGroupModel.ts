import type { OpRecord } from '../../lib/client'

export interface IndexedOperation {
  op: OpRecord
  index: number
}

export interface OperationGroup {
  key: string
  groupId: string | null
  entries: IndexedOperation[]
}

/** The durable group tag is a flattened effect detail, not a top-level field. */
export function operationGroupId(op: OpRecord): string | null {
  for (const effect of op.effects ?? []) {
    const value = effect.group_id
    if (typeof value === 'string' && value.trim()) return value
  }
  return null
}

/** Collapse only adjacent records with the same durable group tag. Reusing a
 * tag later must never reorder history or merge across intervening actions. */
export function groupOperations(ops: OpRecord[]): OperationGroup[] {
  const groups: OperationGroup[] = []
  for (const [index, op] of ops.entries()) {
    const groupId = operationGroupId(op)
    const previous = groups.at(-1)
    if (groupId && previous?.groupId === groupId) {
      previous.entries.push({ op, index })
      continue
    }
    groups.push({
      key: groupId ? `${groupId}:${op.op_id}` : op.op_id,
      groupId,
      entries: [{ op, index }],
    })
  }
  return groups
}

export function operationGroupHeading(group: OperationGroup): string {
  const first = group.entries[0]?.op
  if (!first) return 'Grouped action'
  const actor = first.actor?.name?.trim() || first.actor?.kind || 'Cut'
  const rationale = first.rationale?.trim()
  if (rationale) return `${actor}: ${rationale}`
  const verbs = [...new Set(group.entries.map((entry) => entry.op.verb))]
  return `${actor}: ${verbs.join(' + ')}`
}
