import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import {
  createVolumeAutomationMutationController,
  volumeAutomationProjectLeaseAdmits,
} from '../src/panels/Inspector/volumeAutomationModel'

const complete = (controller: ReturnType<typeof createVolumeAutomationMutationController>, requestId: string, revision: string) =>
  controller.complete(requestId, { ok: true, projectRevision: revision })

// A project-wide lease holds static Gain through its response. Because a Gain
// response has no complete volume track to project, another automation SET must
// wait for current project props instead of reusing that returned revision.
const gainProjectLease = createVolumeAutomationMutationController('op_000050')
const gainClipLease = createVolumeAutomationMutationController('op_000050')
assert.equal(
  volumeAutomationProjectLeaseAdmits(gainProjectLease.state(), gainClipLease.state(), 'op_000050', null),
  true,
  'a current zero-point clip may acquire the project volume lease for static Gain',
)
assert.ok(gainProjectLease.begin('volume-gain-lease-0001'))
assert.ok(gainClipLease.begin('volume-gain-lease-0001'))
assert.equal(
  gainProjectLease.begin('volume-automation-after-gain-0001'),
  null,
  'rapid Gain then automation cannot dispatch a second request against the same project revision',
)
assert.equal(complete(gainClipLease, 'volume-gain-lease-0001', 'op_000051').status, 'saved')
assert.equal(complete(gainProjectLease, 'volume-gain-lease-0001', 'op_000051').status, 'saved')
assert.equal(
  volumeAutomationProjectLeaseAdmits(gainProjectLease.state(), gainClipLease.state(), 'op_000050', null),
  false,
  'static Gain cannot authorize automation from a response-only revision without a complete projected track',
)
gainProjectLease.requireAuthoritativeAfter('op_000050')
gainClipLease.requireAuthoritativeAfter('op_000050')
gainProjectLease.observeAuthoritative('op_000051')
gainClipLease.observeAuthoritative('op_000051')
assert.equal(
  volumeAutomationProjectLeaseAdmits(gainProjectLease.state(), gainClipLease.state(), 'op_000051', null),
  true,
  'a refreshed project revision reopens automation after static Gain',
)

// A returned complete SET may safely feed only the clip that owns its matching
// optimistic track. A different clip waits until project.state carries P61.
const crossClipProjectLease = createVolumeAutomationMutationController('op_000060')
const clipALease = createVolumeAutomationMutationController('op_000060')
const clipBLease = createVolumeAutomationMutationController('op_000060')
assert.ok(crossClipProjectLease.begin('volume-clip-a-0001'))
assert.ok(clipALease.begin('volume-clip-a-0001'))
assert.equal(complete(clipALease, 'volume-clip-a-0001', 'op_000061').status, 'saved')
assert.equal(complete(crossClipProjectLease, 'volume-clip-a-0001', 'op_000061').status, 'saved')
assert.equal(
  volumeAutomationProjectLeaseAdmits(crossClipProjectLease.state(), clipALease.state(), 'op_000060', 'op_000061'),
  true,
  'clip A may follow its own returned revision only with its complete projected track',
)
assert.equal(
  volumeAutomationProjectLeaseAdmits(crossClipProjectLease.state(), clipBLease.state(), 'op_000060', null),
  false,
  'clip B cannot start from clip A’s response-only project revision before an authoritative refresh',
)
clipBLease.observeAuthoritative('op_000061')
crossClipProjectLease.observeAuthoritative('op_000061')
assert.equal(
  volumeAutomationProjectLeaseAdmits(crossClipProjectLease.state(), clipBLease.state(), 'op_000061', null),
  true,
  'clip B becomes eligible only after project props carry the new lease base',
)

const uiRoot = resolve(import.meta.dirname, '..')
const source = (relative: string) => readFileSync(resolve(uiRoot, relative), 'utf8')
const coordinator = source('src/app/VolumeAutomationContext.tsx')
const volumeSection = source('src/panels/Inspector/VolumeSection.tsx')
const timelineLane = source('src/panels/Timeline/TimelineVolumeAutomationLane.tsx')
const timelineCss = source('src/panels/Timeline/timeline.css')
const editor = source('src/panels/Inspector/VolumeAutomationEditor.tsx')

assert.ok(coordinator.includes('private projectLease'), 'shared coordinator owns one project-scoped volume mutation lease')
assert.ok(coordinator.includes('volumeAutomationProjectLeaseAdmits'), 'coordinator admits a write only from its lease base')
assert.ok(coordinator.includes('await action(controls)'), 'coordinator awaits static Gain before releasing its lease')
assert.ok(volumeSection.includes('=> runUserVerb('), 'Volume returns edit.gain promises to the coordinator lease')
assert.ok(!volumeSection.includes('void runUserVerb'), 'Volume does not release the lease before edit.gain settles')
assert.ok(timelineLane.includes('onMouseDown={(event) => event.stopPropagation()}'), 'novice Add does not become a timeline seek before its click')
assert.ok(timelineLane.includes("cut:edit-volume-automation-point"), 'keyboard activation routes a lane point to exact Inspector editing')
assert.ok(editor.includes("addEventListener('cut:edit-volume-automation-point'"), 'Inspector owns the keyboard point-edit destination')
assert.match(timelineCss, /tl-volume-lane__point[^}]*width: 24px; height: 24px/s, 'point controls retain a usable transparent hit target')
console.log('PASS volume automation project lease contracts')
