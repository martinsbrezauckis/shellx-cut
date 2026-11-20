import { strict as assert } from 'node:assert'

import { mockMotionManagementStatus } from '../src/panels/Review/mock'

const status = mockMotionManagementStatus()

assert.equal(status.schema, 'shellx-cut/motion-management-status@1')
assert.equal(status.readOnly, true)
assert.equal(status.runtime.status, 'not-discovered')
assert.equal(status.runtime.engine, null)
assert.equal(status.connector.execution, 'not-executed')
assert.equal(status.distribution.status, 'blocked')
assert.equal(status.distribution.blocker.id, 'MOTION-DIST-01')
for (const action of ['install', 'repair', 'update', 'remove'] as const) {
  assert.deepEqual(status.distribution.actions[action], { available: false, prerequisite: 'MOTION-DIST-01' })
}

console.log('PASS offline Motion management status contract')
