import assert from 'node:assert/strict'
import { mixerLoudnessKey, mixerMeasurementKey, mixerProjectScope, mixerSnapshotMatches } from '../src/panels/Mixer/mixerIdentity'
import type { Project } from '../src/lib/client'

const makeProject = (origin: string): Project => ({
  schema: 'shellx-cut/project/1',
  name: 'Same name',
  project_revision: 'op_000001',
  project_identity: { schema: 'shellx-cut/project-identity/1', origin_path_sha256: origin, project_name: 'Same name' },
  settings: {} as Project['settings'],
  assets: {}, tracks: [], markers: [], caption_styles: {}, checkpoints: [],
})
const first = makeProject('sha256:aaa')
const second = makeProject('sha256:bbb')
assert.notEqual(mixerProjectScope(first), mixerProjectScope(second))
const firstKey = mixerMeasurementKey(first, 'op_000001', ['a1t'])
const secondKey = mixerMeasurementKey(second, 'op_000001', ['a1t'])
assert.notEqual(firstKey, secondKey, 'same name, revision, and track still differ by origin')
assert.equal(mixerSnapshotMatches(first, second), false, 'server readback cannot attach another project stem')
assert.equal(mixerSnapshotMatches(first, { ...first, project_revision: 'op_000002' }), false, 'server edit before prop refresh cannot attach a newer stem to an older revision')
assert.equal(mixerSnapshotMatches(first, { ...first }), true)
assert.notEqual(mixerLoudnessKey(firstKey, -14), mixerLoudnessKey(firstKey, -16), 'target switch hides stale source reading')
let activeKey = firstKey
let visibleStem: { key: string; sample: number } | null = null
let visibleReading: { key: string; lufs: number } | null = null
const oldResponse = Promise.resolve().then(() => {
  if (activeKey === firstKey) {
    visibleStem = { key: firstKey, sample: 1 }
    visibleReading = { key: firstKey, lufs: -14 }
  }
})
activeKey = secondKey
await oldResponse
assert.equal(visibleStem, null, 'late stem from foreign project is discarded')
assert.equal(visibleReading, null, 'late loudness from foreign project is discarded')
console.log('mixer identity: same-name project switch rejects old async results')
