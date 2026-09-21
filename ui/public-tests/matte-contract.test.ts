import assert from 'node:assert/strict'
import type { Project } from '../src/lib/clientModel'
import { matteResultFromReceipt, matteSubjectSeed } from '../src/panels/Matte/contract'

const project = {
  assets: { a1: { path: 'subject.mp4', hash: 'sha256:test', probe: { kind: 'video', width: 1920, height: 1080 } } },
  tracks: [{ id: 'v1', kind: 'video', clips: [
    { kind: 'gap', duration_ms: 2000 },
    { id: 'c1', asset: 'a1', src_in_ms: 5000, src_out_ms: 9000, speed: 2 },
  ] }],
} as unknown as Project
assert.deepEqual(matteSubjectSeed(project, 'c1', 2500, 0.5, 0.5), { at_ms: 6000, point: [960, 540] })
assert.deepEqual(matteSubjectSeed(project, 'c1', 2500, 1, 0), { at_ms: 6000, point: [1919, 0] })
assert.throws(() => matteSubjectSeed(project, 'c1', 1000, 0.5, 0.5), /playhead/)
assert.throws(() => matteSubjectSeed(project, 'c1', 2500, Number.NaN, 0.5), /between 0 and 1/)
assert.throws(() => matteSubjectSeed(project, 'c1', 2500, -0.1, 1), /between 0 and 1/)
const noDimensions = structuredClone(project)
noDimensions.assets.a1.probe = { kind: 'video' }
assert.throws(() => matteSubjectSeed(noDimensions, 'c1', 2500, 0.5, 0.5), /dimensions/)
const reverse = structuredClone(project)
Object.assign(reverse.tracks[0].clips[1], { reverse: true })
assert.equal(matteSubjectSeed(reverse, 'c1', 2500, 0.5, 0.5).at_ms, 8000)
const freeze = structuredClone(project)
Object.assign(freeze.tracks[0].clips[1], { freeze: { at_ms: 750 } })
assert.equal(matteSubjectSeed(freeze, 'c1', 2500, 0.5, 0.5).at_ms, 5750)

function receipt(enabled: boolean) {
  return { op: { op_id: 'op_123', verb: 'edit.matte', status: 'applied', args: { clip: 'c1', enabled },
    effects: [{ track: 'v1', clip: 'c1', old_matte: null, new_matte: enabled ? { model: 'rvm', mode: 'replace' } : null }],
  }, ...(enabled ? { matte: { frames: 30, fps: 30, width: 1920, height: 1080 } } : {}) }
}
assert.deepEqual(matteResultFromReceipt(receipt(true), 'c1', true), { clip: 'c1', enabled: true })
assert.deepEqual(matteResultFromReceipt(receipt(false), 'c1', false), { clip: 'c1', enabled: false })
assert.throws(() => matteResultFromReceipt(receipt(true), 'c2', true), /matching/)
assert.throws(() => matteResultFromReceipt(receipt(true), 'c1', false), /matching/)
assert.throws(() => matteResultFromReceipt({ clip: 'c1', enabled: false }, 'c1', false), /matching/)
const missingEffect = receipt(false)
missingEffect.op.effects = []
assert.throws(() => matteResultFromReceipt(missingEffect, 'c1', false), /matching/)
console.log('PASS Matte source-pixel seed, source-time mapping, and committed apply/clear feedback')
