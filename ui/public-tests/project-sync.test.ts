import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import type { OpRecord, Project } from '../src/lib/client'
import {
  applyProjectDelta,
  loadProjectOpsPages,
  mergeProjectOps,
  needsColdHistoryLoad,
  projectAfterUnavailableSync,
  projectDeltaChangesState,
  projectSyncFromVerbResult,
  revisionPull,
  type ProjectDelta,
} from '../src/lib/projectSync'
import { projectReconciliationDelay, ProjectSyncCoalescer } from '../src/lib/projectSyncQueue'

const project = {
  schema: 'shellx-cut/project@1',
  name: '10k clips',
  settings: { width: 1920, height: 1080, fps: 30, audio_rate: 48_000 },
  assets: {},
  tracks: Array.from({ length: 10_000 }, (_, index) => ({
    id: `v${index}`,
    kind: 'video',
    clips: [{ id: `c${index}`, kind: 'media' }],
  })),
  markers: [],
  caption_styles: {},
  checkpoints: [],
  project_revision: 'op_010000',
} as unknown as Project

const markerDelta: ProjectDelta = {
  mode: 'delta',
  from_revision: 'op_010000',
  project_revision: 'op_010001',
  ops: [],
  changes: [{ kind: 'marker_upsert', marker: { id: 'm1', at_ms: 20, label: 'reconnected' } }],
  affected: { markers: 1, assets: 0, project: 0 },
  encoded_bytes: 142,
}

const updated = applyProjectDelta(project, markerDelta)
assert.equal(updated.project_revision, 'op_010001')
assert.equal(updated.markers.length, 1)
assert.strictEqual(updated.tracks, project.tracks, 'a marker delta must not clone 10k timeline clip containers')
assert.equal(projectDeltaChangesState(markerDelta), false, 'an empty delta does not advance snapshot reconciliation')
assert.equal(needsColdHistoryLoad(project, false), true, 'the first project object needs one cold history transfer')
assert.equal(needsColdHistoryLoad({ ...project, markers: updated.markers }, true), false, 'an ordinary delta/snapshot project object replacement does not retrigger cold history')
assert.equal(needsColdHistoryLoad({ ...project }, false), true, 'the project-switch reset makes a new project eligible for one cold history transfer')
const closedProjectSync = projectSyncFromVerbResult({
  ok: false,
  error: { code: 'no_project', message: 'no project is open' },
})
assert.deepEqual(closedProjectSync, { mode: 'no_project' }, 'project.state exposes a confirmed closed project distinctly from a failed pull')
assert.equal(projectAfterUnavailableSync(updated, closedProjectSync), null, 'cached project plus reconnect no_project converges to Projects')
const transientProjectSync = projectSyncFromVerbResult({
  ok: false,
  error: { code: 'io', message: 'temporary state read failed' },
})
assert.equal(transientProjectSync, null, 'a transient project.state error is not mislabeled as a close')
assert.strictEqual(projectAfterUnavailableSync(updated, transientProjectSync), updated, 'a transient failed reconnect preserves the cached project until closure is confirmed')
assert.strictEqual(projectAfterUnavailableSync(updated, null), updated, 'an unavailable reconnect response also preserves cached state until a confirmed close')

assert.deepEqual(revisionPull('op_010000', 'op_010000'), {
  sinceRevision: 'op_010000',
  missedEventGap: false,
})
assert.deepEqual(revisionPull('op_010000', 'op_009997'), {
  sinceRevision: 'op_010000',
  missedEventGap: true,
})

const op = (op_id: string): OpRecord => ({
  op_id,
  ts: '2026-08-08T00:00:00.000Z',
  actor: { kind: 'system', name: 'test', via: 'test' },
  verb: 'edit.add_marker',
  args: {},
  status: 'applied',
})
assert.equal(projectDeltaChangesState({ ...markerDelta, ops: [op('op_010001')] }), true, 'a durable operation advances bounded reconciliation')
assert.deepEqual(
  mergeProjectOps([op('op_010000')], [op('op_010000'), op('op_010001')]).map((entry) => entry.op_id),
  ['op_010000', 'op_010001'],
  'reconnect delta must deduplicate an event that arrived before its pull',
)
assert.deepEqual(
  mergeProjectOps(
    [op('op_1000000'), op('op_999999'), op('legacy-not-an-op')],
    [op('op_999999'), op('op_1000001')],
  ).map((entry) => entry.op_id),
  ['legacy-not-an-op', 'op_999999', 'op_1000000', 'op_1000001'],
  'canonical operation ids retain numeric order across the 999999 to 1000000 width boundary while duplicate and malformed ids fall back safely',
)

const tenThousandOps = Array.from({ length: 10_000 }, (_, index) => op(`op_${String(index + 1).padStart(6, '0')}`))
const mergedHistory = mergeProjectOps(tenThousandOps.slice(0, 9_999), [tenThousandOps[9_998], tenThousandOps[9_999]])
assert.equal(mergedHistory.length, 10_000, 'a reconnect page must retain 10k unique history rows while deduplicating overlap')

const requestedCursors: Array<string | undefined> = []
const completeHistory = await loadProjectOpsPages(
  async (cursor) => {
    requestedCursors.push(cursor)
    const start = cursor ? Number(cursor.slice(3)) : 0
    const ops = tenThousandOps.slice(start, start + 128)
    const hasMore = start + ops.length < tenThousandOps.length
    return {
      ops,
      has_more: hasMore,
      next_cursor: hasMore ? ops.at(-1)?.op_id : null,
      project_revision: tenThousandOps.at(-1)?.op_id,
    }
  },
  () => true,
)
assert.equal(completeHistory?.ops.length, 10_000, 'cold history follows every page through the final tip')
assert.equal(completeHistory?.ops[4_999].op_id, 'op_005000', 'cold history retains a middle operation beyond page one')
assert.equal(completeHistory?.ops.at(-1)?.op_id, 'op_010000', 'cold history retains the final tip')
assert.equal(completeHistory?.projectRevision, 'op_010000', 'a final page anchors reconnect at project_revision when next_cursor is absent')
assert.equal(requestedCursors.length, 79, '10k records transfer as 79 bounded 128-op pages')
assert.equal(requestedCursors[0], undefined)

let stillCurrent = true
const discardedForSwitch = await loadProjectOpsPages(
  async () => {
    stillCurrent = false
    return { ops: tenThousandOps.slice(0, 128), has_more: true, next_cursor: 'op_000128' }
  },
  () => stillCurrent,
)
assert.equal(discardedForSwitch, null, 'a project switch during a page fetch discards partial old-project history')

let releaseDeferredFinalPage: () => void = () => undefined
let markFinalPageRequested: () => void = () => undefined
const deferredFinalPage = new Promise<void>((resolve) => { releaseDeferredFinalPage = resolve })
const finalPageRequested = new Promise<void>((resolve) => { markFinalPageRequested = resolve })
let projectHistoryGeneration = 21
const discardedForConfirmedClose = loadProjectOpsPages(
  async (cursor) => {
    if (!cursor) {
      const first = tenThousandOps.slice(0, 128)
      return { ops: first, has_more: true, next_cursor: first.at(-1)?.op_id }
    }
    markFinalPageRequested()
    await deferredFinalPage
    return { ops: tenThousandOps.slice(128, 256), has_more: false, next_cursor: null }
  },
  () => projectHistoryGeneration === 21,
)
await finalPageRequested
projectHistoryGeneration += 1 // confirmed no_project invalidates the old history generation
releaseDeferredFinalPage()
assert.equal(await discardedForConfirmedClose, null, 'a confirmed close before a deferred final page resolves discards the old project history')

let releaseBurst: () => void = () => undefined
const burstGate = new Promise<void>((resolve) => { releaseBurst = resolve })
let burstPulls = 0
let burstSnapshots = 0
const burstSync = new ProjectSyncCoalescer<{ mode: 'snapshot' }>(
  async (request) => {
    burstPulls += 1
    burstSnapshots += 1
    await burstGate
    return {
      value: { mode: 'snapshot' },
      generation: request.generation,
      projectRevision: 'op_010000',
    }
  },
  (request, outcome) => (
    request.generation === outcome.generation
    && !request.forceSnapshot
    && request.targetRevision === outcome.projectRevision
  ),
)
const burstComplete = burstSync.request({
  generation: 4,
  forceSnapshot: false,
  advertisedPrevious: 'op_000000',
  targetRevision: 'op_000001',
})
for (let index = 2; index <= 10_000; index += 1) {
  void burstSync.request({
    generation: 4,
    forceSnapshot: false,
    advertisedPrevious: `op_${String(index - 1).padStart(6, '0')}`,
    targetRevision: `op_${String(index).padStart(6, '0')}`,
  })
}
releaseBurst()
await burstComplete
const burstMetrics = burstSync.metrics()
assert.equal(burstPulls, 1, 'a 10k WebSocket burst performs one project.state pull when its first snapshot reaches the final revision')
assert.equal(burstSnapshots, 1, 'a 10k WebSocket burst performs no repeated full snapshots')
assert.deepEqual(burstMetrics, {
  pullsStarted: 1,
  requestsCoalesced: 9_999,
  pendingPullsSkippedAsCurrent: 1,
}, 'the coalescer exposes deterministic burst request and snapshot evidence')

let releaseOldProject: () => void = () => undefined
const oldProjectGate = new Promise<void>((resolve) => { releaseOldProject = resolve })
const pullGenerations: number[] = []
const switchedProjectSync = new ProjectSyncCoalescer<{ mode: 'snapshot' }>(
  async (request) => {
    pullGenerations.push(request.generation)
    if (request.generation === 8) await oldProjectGate
    return {
      value: { mode: 'snapshot' },
      generation: request.generation,
      projectRevision: request.generation === 8 ? 'op_000128' : 'op_000003',
    }
  },
  (request, outcome) => request.generation === outcome.generation && !request.forceSnapshot && request.targetRevision === outcome.projectRevision,
)
const switchComplete = switchedProjectSync.request({
  generation: 8,
  forceSnapshot: false,
  targetRevision: 'op_000128',
})
void switchedProjectSync.request({ generation: 9, forceSnapshot: true })
releaseOldProject()
await switchComplete
assert.deepEqual(pullGenerations, [8, 9], 'a project switch starts its own forced pull after the old generation, never merging their revisions')
assert.equal(switchedProjectSync.metrics().pullsStarted, 2, 'a project switch clears and reloads exactly once after the stale pull drains')

let releaseJobBurst: () => void = () => undefined
const jobBurstGate = new Promise<void>((resolve) => { releaseJobBurst = resolve })
let jobProgressSnapshots = 0
const jobProgressSync = new ProjectSyncCoalescer<{ mode: 'snapshot' }>(
  async (request) => {
    jobProgressSnapshots += 1
    if (jobProgressSnapshots === 1) await jobBurstGate
    return {
      value: { mode: 'snapshot' },
      generation: request.generation,
      projectRevision: 'op_010000',
    }
  },
  () => false, // Force snapshots need a trailing pull: job metadata has no op revision target.
)
const jobBurstComplete = jobProgressSync.request({ generation: 11, forceSnapshot: true })
for (let index = 0; index < 10_000; index += 1) {
  void jobProgressSync.request({ generation: 11, forceSnapshot: true })
}
releaseJobBurst()
await jobBurstComplete
const jobBurstMetrics = jobProgressSync.metrics()
assert.equal(jobProgressSnapshots, 2, 'a large job-progress burst performs one in-flight and one trailing full snapshot')
assert.deepEqual(jobBurstMetrics, {
  pullsStarted: 2,
  requestsCoalesced: 10_000,
  pendingPullsSkippedAsCurrent: 0,
}, 'job-progress snapshots use the same bounded coalescing queue')

assert.equal(projectReconciliationDelay(31), null, 'safety reconciliation waits until 32 real delta applications')
assert.equal(projectReconciliationDelay(32), 2_000, 'supported deltas reconcile after an idle delay, not inline')
assert.equal(projectReconciliationDelay(4_096), 0, 'the large hard cap still guarantees periodic reconciliation during a continuous stream')
let deltasSinceSnapshot = 0
let sustainedHardSnapshots = 0
for (let index = 0; index < 10_000; index += 1) {
  deltasSinceSnapshot += 1
  if (projectReconciliationDelay(deltasSinceSnapshot) === 0) {
    sustainedHardSnapshots += 1
    deltasSinceSnapshot = 0
  }
}
const sustainedIdleSnapshots = projectReconciliationDelay(deltasSinceSnapshot) == null ? 0 : 1
assert.equal(sustainedHardSnapshots + sustainedIdleSnapshots, 3, '10k sustained supported deltas have two hard-cap snapshots plus one eventual idle reconciliation, never hundreds')

const transcriptSource = readFileSync(new URL('../src/panels/Transcript/index.tsx', import.meta.url), 'utf8')
const appSource = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8')
const workspaceSource = readFileSync(new URL('../src/app/AppWorkspace.tsx', import.meta.url), 'utf8')
const leftPanelSource = readFileSync(new URL('../src/panels/LeftPanel/index.tsx', import.meta.url), 'utf8')
assert.match(transcriptSource, /ops: OpRecord\[\]/, 'Transcript accepts the App-owned durable history prop')
assert.doesNotMatch(transcriptSource, /callVerb\(\s*['"]project\.ops['"]/, 'Transcript never starts a duplicate cold history fetch on project updates')
assert.doesNotMatch(transcriptSource, /setOps\(/, 'Transcript does not maintain a competing history cache')
assert.match(workspaceSource, /<LeftPanel[\s\S]*ops=\{ops\}/, 'AppWorkspace forwards the authoritative history to LeftPanel')
assert.match(leftPanelSource, /<Transcript[\s\S]*ops=\{ops\}/, 'LeftPanel forwards the authoritative history to Transcript')
assert.match(appSource, /if \(ev\.kind === 'import_chain' \|\| ev\.kind === 'enrich' \|\| ev\.progress >= 1\) \{\s*void syncProject\(true\)/, 'job-progress state refreshes enter the shared snapshot coalescer')
assert.match(appSource, /response\?\.mode === 'no_project'[\s\S]*setProject\(null\)/, 'a confirmed no_project reconnect clears the visible stale workspace')
assert.match(appSource, /const resetProjectScopedUi[\s\S]*historyLoadGeneration\.current \+= 1[\s\S]*fullHistoryLoad\.current = null/, 'the shared hard reset invalidates in-flight paged history before it can merge')
assert.match(appSource, /const resetProjectScopedUi[\s\S]*setSelectedClipIds\(\[\]\)[\s\S]*setExportRange\(null\)[\s\S]*setPlayheadMs\(0\)[\s\S]*setCommentsOpen\(false\)[\s\S]*setFocusComment\(null\)[\s\S]*clearClipboard\(\)/, 'the confirmed-close reset clears selection, export, playhead, comment, and clipboard state')
assert.match(appSource, /response\?\.mode === 'no_project'[\s\S]*resetProjectScopedUi\(true\)/, 'confirmed no_project uses the same project-scoped UI reset as a switch')

console.log(`PASS project-sync clips=10000 history_ops=${mergedHistory.length} pages=${requestedCursors.length} burst_pulls=${burstPulls} burst_snapshots=${burstSnapshots} job_snapshots=${jobProgressSnapshots} sustained_delta_snapshots=${sustainedHardSnapshots + sustainedIdleSnapshots} coalesced=${burstMetrics.requestsCoalesced} missed-event cursor, reconnect dedupe`)
