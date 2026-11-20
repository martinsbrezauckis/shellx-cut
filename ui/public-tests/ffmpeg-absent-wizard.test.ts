import assert from 'node:assert/strict'

import { envHealthLevel, isFfmpegMissing, shouldAutoPopWizard, type DoctorCard, type DoctorReport } from '../src/lib/doctor'

const tool = (id: string, status: DoctorCard['status']): DoctorCard => ({ id, kind: 'tool', status, details: {} })
const report = (ffmpeg: DoctorCard['status'], ffprobe: DoctorCard['status'], essential_ok: boolean): DoctorReport => ({
  schema: 'shellx-cut/doctor/1',
  scanned_at: '2026-01-01T00:00:00Z',
  os: 'windows',
  arch: 'x86_64',
  app_version: '0',
  cards: [tool('ffmpeg', ffmpeg), tool('ffprobe', ffprobe)],
  essential_ok,
})

// No ffmpeg on the machine: the engine reports both cards missing and the
// essential gate off. The first-run wizard opens and the health chip is red.
const absent = report('missing', 'missing', false)
assert.equal(shouldAutoPopWizard(absent), true)
assert.equal(isFfmpegMissing(absent), true)
assert.equal(envHealthLevel(absent), 'missing')

// ffprobe alone missing beside a working ffmpeg: clips cannot be read, so the
// wizard opens too.
const noProbe = report('ok', 'missing', false)
assert.equal(shouldAutoPopWizard(noProbe), true)
assert.equal(isFfmpegMissing(noProbe), true)
assert.equal(envHealthLevel(noProbe), 'missing')

// An unverified (timed-out) probe never pops the wizard.
const slow = report('unknown', 'unknown', true)
assert.equal(shouldAutoPopWizard(slow), false)
assert.equal(isFfmpegMissing(slow), false)
assert.notEqual(envHealthLevel(slow), 'missing')

console.log('PASS absent ffmpeg opens the first-run wizard')
