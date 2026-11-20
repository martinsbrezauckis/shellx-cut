import assert from 'node:assert/strict'
import { addRawRecordingToTimeline } from '../src/panels/Record/addRawRecordingToTimeline'
import type { Project } from '../src/lib/client'

const rawPath = '/owned/capture/raw.mp4'

function runScenario(existingClip: boolean, enrichmentRunning = false) {
  const calls: string[] = []
  const project = {
    assets: { ...(existingClip ? { prior: { path: '/owned/prior.mp4' } } : {}) },
    tracks: [{ id: 'v1', kind: 'video', clips: existingClip
      ? [{ id: 'old', asset: 'prior', src_in_ms: 0, src_out_ms: 2000 }] : [] }],
  } as unknown as Project
  const call = async (verb: string, args?: { proxy?: boolean }) => {
    calls.push(verb)
    if (verb === 'media.import') {
      assert.equal(args?.proxy, false, 'raw import should skip a heavy editing proxy')
      return { ok: true, result: { asset_id: 'raw-asset', job_id: 'raw-job' } }
    }
    if (verb === 'jobs.status') {
      project.assets['raw-asset'] = { path: '/canonical/owned/capture/raw.mp4',
        probe: { kind: 'video', duration_ms: 2400, has_audio: true } } as never
      if (!existingClip) project.tracks[0].clips.push({ id: 'auto-raw', asset: 'raw-asset' } as never)
      return { ok: true, result: { state: enrichmentRunning ? 'running' : 'done' } }
    }
    if (verb === 'project.state') return { ok: true, result: project }
    throw new Error(`unexpected ${verb}`)
  }
  const place = async (args: { asset: string; at_ms: number; kind: string }) => {
    assert.equal(args.asset, 'raw-asset')
    assert.equal(args.kind, 'video')
    assert.equal(args.at_ms, 2000)
    calls.push('edit.insert_linked')
    project.tracks[0].clips.push({ id: 'placed-raw', asset: 'raw-asset' } as never)
    return { ok: true }
  }
  return { calls, run: () => addRawRecordingToTimeline(rawPath, {
    call: call as never, place: place as never, wait: async () => {},
  }) }
}

const empty = runScenario(false)
assert.deepEqual(await empty.run(), { ok: true, assetId: 'raw-asset', placedClipId: 'auto-raw' })
assert.equal(empty.calls.includes('edit.insert_linked'), false, 'first import auto-placement must not be duplicated')

const populated = runScenario(true, true)
assert.deepEqual(await populated.run(), { ok: true, assetId: 'raw-asset', placedClipId: 'placed-raw' })
assert.equal(populated.calls.filter((verb) => verb === 'edit.insert_linked').length, 1,
  'a nonempty timeline needs one explicit linked placement')
assert.equal(populated.calls.filter((verb) => verb === 'jobs.status').length, 1,
  'placement can complete while unrelated enrichment still runs')
