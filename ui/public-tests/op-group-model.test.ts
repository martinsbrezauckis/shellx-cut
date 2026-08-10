import { strict as assert } from 'node:assert'
import type { OpRecord } from '../src/lib/client'
import {
  groupOperations,
  operationGroupHeading,
  operationGroupId,
} from '../src/panels/Review/opGroupModel'

function op(id: string, verb: string, groupId?: string, rationale?: string): OpRecord {
  return {
    op_id: id,
    ts: '2026-08-09T00:00:00.000Z',
    actor: { kind: 'agent', name: 'CALI', via: 'mcp' },
    verb,
    args: {},
    rationale,
    effects: groupId ? [{ group_id: groupId }] : [],
    status: 'applied',
  }
}

const ops = [
  op('op_1', 'edit.split', 'grp-a', 'Split linked picture and sound'),
  op('op_2', 'edit.split', 'grp-a', 'Split linked picture and sound'),
  op('op_3', 'edit.grade'),
  op('op_4', 'edit.move', 'grp-a'),
  op('op_5', 'edit.move', 'grp-a'),
]

assert.equal(operationGroupId(ops[0]), 'grp-a')
assert.equal(operationGroupId(ops[2]), null)
const groups = groupOperations(ops)
assert.deepEqual(groups.map((group) => group.entries.map((entry) => entry.index)), [[0, 1], [2], [3, 4]])
assert.equal(groups[0].groupId, 'grp-a')
assert.notEqual(groups[0].key, groups[2].key, 'a later reuse of the tag stays a separate history unit')
assert.equal(operationGroupHeading(groups[0]), 'CALI: Split linked picture and sound')
assert.equal(operationGroupHeading(groups[2]), 'CALI: edit.move')

console.log('PASS adjacent durable operation groups preserve history order and human summaries')
