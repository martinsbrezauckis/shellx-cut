import assert from 'node:assert/strict'

import { envHealthLevel, isOptionalCard, optionalExtrasNote, type DoctorReport } from '../src/lib/doctor'

// What "Setup needs attention" counts: a REQUIRED card that is degraded or
// unknown. Optional extras (judges, background removal, GPU encoder, remote
// services) that are not ready never do. The cards below are the real Windows
// desktop run after FFmpeg and captions were installed.
function report(cards: Array<[string, string, string]>, essentialOk = true): DoctorReport {
  return {
    schema: 'x', scanned_at: 'x', os: 'windows', arch: 'x86_64', app_version: '0', essential_ok: essentialOk,
    cards: cards.map(([id, kind, status]) => ({ id, kind, status, source: null, version: null, hint: null, details: {} })),
  } as unknown as DoctorReport
}

const afterInstall = report([
  ['ffmpeg', 'tool', 'ok'], ['ffprobe', 'tool', 'ok'], ['gpu-encode', 'tool', 'ok'], ['perception', 'perception', 'ok'],
  ['matte', 'matte', 'missing'], ['matte_premium', 'matte', 'missing'], ['dub', 'service', 'unknown'], ['diarize', 'service', 'unknown'],
  ['judge.claude', 'judge', 'ok'], ['judge.codex', 'judge', 'degraded'], ['judge.antigravity', 'judge', 'degraded'], ['judge.grok', 'judge', 'degraded'],
  ['disk', 'disk', 'ok'],
])
assert.equal(envHealthLevel(afterInstall), 'ok', 'only optional extras are not ready: no warning')
assert.match(optionalExtrasNote(afterInstall) ?? '', /^Optional extras you can set up: matte, matte_premium, dub, diarize, judge\.codex/)

// A required card that is degraded or unknown still needs attention.
assert.equal(envHealthLevel(report([['ffmpeg', 'tool', 'degraded'], ['disk', 'disk', 'ok']])), 'degraded')
assert.equal(envHealthLevel(report([['ffmpeg', 'tool', 'ok'], ['perception', 'perception', 'degraded']])), 'degraded')
assert.equal(envHealthLevel(report([['ffmpeg', 'tool', 'ok'], ['disk', 'disk', 'degraded']])), 'degraded')
assert.equal(envHealthLevel(report([['ffmpeg', 'tool', 'ok'], ['perception', 'perception', 'unknown']])), 'degraded')
// A missing essential is the red rung.
assert.equal(envHealthLevel(report([['ffmpeg', 'tool', 'missing']], false)), 'missing')
assert.equal(isOptionalCard({ id: 'gpu-encode', kind: 'tool' }), true)
assert.equal(isOptionalCard({ id: 'ffmpeg', kind: 'tool' }), false)
assert.equal(optionalExtrasNote(report([['ffmpeg', 'tool', 'ok']])), null)
console.log('PASS setup health counts required cards only')
