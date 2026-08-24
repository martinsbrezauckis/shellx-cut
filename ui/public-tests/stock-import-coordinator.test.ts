import assert from 'node:assert/strict'
import {
  createStockImportCoordinator,
  stockImportKey,
} from '../src/panels/Stock/importCoordinator'

const coordinator = createStockImportCoordinator()
let notifications = 0
const off = coordinator.subscribe(() => { notifications += 1 })

// This is the same controller instance App passes through AppWorkspace and
// LeftPanel. Reading it as a fresh subscriber models a Stock remount after the
// user switches Find media → Find moment/Sequence while the first response is
// still held.
const projectA = 41
const projectB = 42
coordinator.setProjectScope(projectA)
const first = coordinator.begin(projectA, 'arrow_right')
assert.ok(first, 'the first import is synchronously admitted')
assert.equal(coordinator.getSnapshot().request.fetchingId, 'arrow_right')

const remountedSnapshot = coordinator.getSnapshot()
assert.equal(remountedSnapshot.request.fetchingId, 'arrow_right', 'a remounted panel observes the app-owned active import')
assert.equal(coordinator.begin(projectA, 'circle'), null, 'a remounted different-hit handler is refused before assets.fetch dispatch')

coordinator.finish(projectA, first.fetchEpoch, {
  key: stockImportKey('stickers', 'arrow_right'),
  assetId: 'a3',
})
const completedSnapshot = coordinator.getSnapshot()
assert.equal(completedSnapshot.request.fetchingId, null, 'the admitted first import releases the shared busy state')
assert.equal(completedSnapshot.fetched[stockImportKey('stickers', 'arrow_right')], 'a3', 'the completed import is durable for the remounted Added state')
assert.ok(notifications >= 2, 'mount subscribers receive both active and completed coordinator states')

// A project switch clears all presentation data at once, but cannot reopen the
// admission gate while an old server request is in flight. That old completion
// releases the lock without projecting A's hit or Added state into B.
const oldProjectRequest = coordinator.begin(projectA, 'circle')
assert.ok(oldProjectRequest)
coordinator.rememberSearch(projectA, {
  provider: 'stickers', kind: 'image', dir: '', q: '',
  session: { provider: 'stickers', dir: null },
  hits: [],
})
coordinator.setProjectScope(projectB)
assert.equal(coordinator.getSnapshot().request.fetchingId, 'circle', 'project B keeps the old request locked until it settles')
assert.equal(coordinator.getSnapshot().search, null, 'project A search hits cannot render in project B')
assert.deepEqual(coordinator.getSnapshot().fetched, {}, 'project A Added state cannot render in project B')
assert.equal(coordinator.begin(projectB, 'triangle'), null, 'project B cannot dispatch a duplicate while A remains active')
coordinator.finish(projectA, oldProjectRequest.fetchEpoch, {
  key: stockImportKey('stickers', 'circle'), assetId: 'a4',
})
assert.equal(coordinator.getSnapshot().request.fetchingId, null, 'the old completion releases the cross-project lock')
assert.deepEqual(coordinator.getSnapshot().fetched, {}, 'the old completion cannot project an Added state into B')
assert.ok(coordinator.begin(projectB, 'triangle'), 'project B admits its own import only after A has settled')
off()

console.log('PASS stock import coordinator remount continuity')
