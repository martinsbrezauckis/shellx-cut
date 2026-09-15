import { strict as assert } from 'node:assert'
import { acceptAppliedPlan, acceptPlanBinding } from '../src/panels/Assemble/assemblePlanAcceptance'
import { AssembleRequestEpoch } from '../src/panels/Assemble/useAssembleRequestEpoch'

const binding = {
  schema: 'shellx-cut/assemble-plan-binding/1',
  project_identity: {
    schema: 'shellx-cut/project-identity/1',
    origin_path_sha256: `sha256:${'a'.repeat(64)}`,
    project_name: 'Reviewed plan',
  },
  project_revision: 'op_000001',
  verb: 'assemble.repurpose',
  asset: 'a1',
  selected_ranges: [[0, 2]],
  transcript_sha256: `sha256:${'b'.repeat(64)}`,
  materialization: { kind: 'reel' },
}

assert.deepEqual(acceptPlanBinding(binding, 'assemble.repurpose'), binding)
assert.equal(acceptPlanBinding({ ...binding, transcript_sha256: 'not-a-hash' }, 'assemble.repurpose'), null)
assert.deepEqual(acceptPlanBinding({ ...binding, verb: 'assemble.from_script', selected_ranges: [] }, 'assemble.from_script')?.selected_ranges, [])

const receipt = {
  materialized: true,
  kind: 'reel',
  asset: 'a1',
  spans_placed: 1,
  video_clip_ids: ['v1'],
  audio_clip_ids: ['a1'],
  caption_track: null,
  caption_clip_ids: [],
  total_ms: 1200,
  undo: { verb: 'project.undo', op_id: 'op_000002' },
}
assert.deepEqual(acceptAppliedPlan({ ok: true, result: receipt, op_ids: ['op_000002'], project_revision: 'op_000002' }, 'assemble.repurpose', 'a1'), receipt)
assert.equal(acceptAppliedPlan({ ok: true, result: {}, op_ids: [] }, 'assemble.repurpose', 'a1'), null)
assert.equal(acceptAppliedPlan({ ok: true, result: receipt, op_ids: ['other'] }, 'assemble.repurpose', 'a1'), null)
assert.equal(acceptAppliedPlan({ ok: true, result: { ...receipt, kind: 'shorts' }, op_ids: ['op_000002'] }, 'assemble.repurpose', 'a1'), null)
assert.equal(acceptAppliedPlan({ ok: true, result: { ...receipt, asset: 'other' }, op_ids: ['op_000002'] }, 'assemble.repurpose', 'a1'), null)
assert.equal(acceptAppliedPlan({ ok: true, result: receipt, op_ids: ['op_000002'], project_revision: 'op_000003' }, 'assemble.repurpose', 'a1'), null)

const epoch = new AssembleRequestEpoch('project-a', 'op_000001')
const planning = epoch.start()
assert.equal(epoch.current(planning), true)
assert.equal(epoch.reconcileProject('project-b', 'op_000001'), 'foreign-project')
assert.equal(epoch.current(planning), false, 'project changes invalidate deferred plans')
const applying = epoch.start()
epoch.invalidate()
assert.equal(epoch.current(applying), false, 'parameter and mode changes invalidate deferred apply results')
const unmounted = epoch.start()
epoch.dispose()
assert.equal(epoch.current(unmounted), false, 'unmount prevents late state writes')

const eventBeforeResponse = new AssembleRequestEpoch('project-a', 'op_000001')
const applyToken = eventBeforeResponse.startApply('op_000001')
assert.notEqual(applyToken, null)
assert.equal(eventBeforeResponse.reconcileProject('project-a', 'op_000002'), 'apply-pending')
assert.equal(eventBeforeResponse.acceptApply(applyToken!, 'op_000002'), 'accepted')
assert.equal(eventBeforeResponse.current(applyToken!), true, 'the operation event may arrive before its HTTP receipt')

const responseBeforeEvent = new AssembleRequestEpoch('project-a', 'op_000001')
const laterToken = responseBeforeEvent.startApply('op_000001')
assert.notEqual(laterToken, null)
assert.equal(responseBeforeEvent.acceptApply(laterToken!, 'op_000002'), 'accepted')
assert.equal(responseBeforeEvent.reconcileProject('project-a', 'op_000001'), 'awaiting-project-refresh')
assert.equal(responseBeforeEvent.reconcileProject('project-a', 'op_000002'), 'apply-published')

const staleRevision = new AssembleRequestEpoch('project-a', 'op_000001')
const staleToken = staleRevision.start()
assert.equal(staleRevision.reconcileProject('project-a', 'op_000002'), 'stale-plan')
assert.equal(staleRevision.reconcileProject('project-a', 'op_000002'), 'stale-plan', 'StrictMode second render must retain the invalidation')
staleRevision.acknowledgeProjectChange('stale-plan')
assert.equal(staleRevision.reconcileProject('project-a', 'op_000002'), 'unchanged')
assert.equal(staleRevision.current(staleToken), false, 'external revisions invalidate a reviewed plan')

const broll = new AssembleRequestEpoch('project-a', 'op_000001')
const brollToken = broll.startIdentityRequest()
assert.notEqual(brollToken, null)
assert.equal(broll.reconcileProject('project-a', 'op_000002'), 'broll-pending', 'the automatic checkpoint is owned by this b-roll request')
assert.equal(broll.reconcileProject('project-a', 'op_000003'), 'broll-pending', 'child operations stay within the request lifetime')
assert.equal(broll.completeIdentityRequest(brollToken!), true, 'the HTTP response may update b-roll results after its child operations')
assert.equal(broll.current(brollToken!), true)
assert.equal(broll.reconcileProject('project-a', 'op_000002'), 'broll-published', 'a delayed state refresh may still report an owned intermediate operation')
assert.equal(broll.reconcileProject('project-a', 'op_000003'), 'broll-published')

const brollResponseBeforeFinalEvent = new AssembleRequestEpoch('project-a', 'op_000001')
const responseFirstBrollToken = brollResponseBeforeFinalEvent.startIdentityRequest()
assert.notEqual(responseFirstBrollToken, null)
assert.equal(brollResponseBeforeFinalEvent.completeIdentityRequest(responseFirstBrollToken!), true)
assert.equal(brollResponseBeforeFinalEvent.reconcileProject('project-a', 'op_000002'), 'broll-published', 'a final b-roll child operation may publish after its HTTP response')
assert.equal(brollResponseBeforeFinalEvent.current(responseFirstBrollToken!), true, 'a same-project final event retains b-roll feedback')
const laterPlannerToken = brollResponseBeforeFinalEvent.start()
assert.equal(brollResponseBeforeFinalEvent.reconcileProject('project-a', 'op_000003'), 'stale-plan', 'the next request ends b-roll revision tolerance')
assert.equal(brollResponseBeforeFinalEvent.current(laterPlannerToken), false)

const brollThenApply = new AssembleRequestEpoch('project-a', 'op_000001')
const brollThenApplyToken = brollThenApply.startIdentityRequest()
assert.notEqual(brollThenApplyToken, null)
assert.equal(brollThenApply.completeIdentityRequest(brollThenApplyToken!), true)
const applyAfterBroll = brollThenApply.startApply('op_000001')
assert.notEqual(applyAfterBroll, null)
assert.equal(brollThenApply.reconcileProject('project-a', 'op_000002'), 'apply-pending', 'a new Apply restores strict revision binding after b-roll')

const cancelledBroll = new AssembleRequestEpoch('project-a', 'op_000001')
const cancelledBrollToken = cancelledBroll.startIdentityRequest()
assert.notEqual(cancelledBrollToken, null)
cancelledBroll.invalidate()
assert.equal(cancelledBroll.completeIdentityRequest(cancelledBrollToken!), false, 'parameter changes discard a deferred b-roll result')
const foreignBroll = new AssembleRequestEpoch('project-a', 'op_000001')
const foreignBrollToken = foreignBroll.startIdentityRequest()
assert.notEqual(foreignBrollToken, null)
assert.equal(foreignBroll.reconcileProject('project-b', 'op_000001'), 'foreign-project')
assert.equal(foreignBroll.completeIdentityRequest(foreignBrollToken!), false, 'a project switch discards a deferred b-roll result')
const unmountedBroll = new AssembleRequestEpoch('project-a', 'op_000001')
const unmountedBrollToken = unmountedBroll.startIdentityRequest()
assert.notEqual(unmountedBrollToken, null)
unmountedBroll.dispose()
assert.equal(unmountedBroll.completeIdentityRequest(unmountedBrollToken!), false, 'unmount discards a deferred b-roll result')

console.log('PASS Assemble accepts only bound one-operation receipts and rejects stale completion epochs')
