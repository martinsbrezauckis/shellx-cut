import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import type { KeyboardEvent } from 'react'
import { recordingRehearsalPlaybackKey } from '../src/panels/Record/RecordingRehearsal'

function fixture({ paused = true, rejectPlay = false } = {}) {
  let plays = 0
  let pauses = 0
  let prevented = 0
  let stopped = 0
  const video = {
    paused,
    ownerDocument: { activeElement: null as unknown },
    play() { plays++; this.paused = false; return rejectPlay ? Promise.reject(new Error('play rejected')) : Promise.resolve() },
    pause() { pauses++; this.paused = true },
  }
  video.ownerDocument.activeElement = video
  const event = {
    type: 'keydown', key: ' ', repeat: false,
    altKey: false, ctrlKey: false, metaKey: false, shiftKey: false,
    defaultPrevented: false, target: video, currentTarget: video,
    preventDefault() { prevented++; this.defaultPrevented = true },
    stopPropagation() { stopped++ },
  }
  return { video, event, send: () => recordingRehearsalPlaybackKey(event as unknown as KeyboardEvent<HTMLVideoElement>),
    counts: () => ({ plays, pauses, prevented, stopped }) }
}

test('focused paused video starts once and suppresses browser toggle/scroll', () => {
  const f = fixture()
  f.send()
  // Browser default must be cancelled before its own playback action runs.
  if (!f.event.defaultPrevented) f.video.pause()
  assert.equal(f.video.paused, false)
  assert.deepEqual(f.counts(), { plays: 1, pauses: 0, prevented: 1, stopped: 1 })
})

test('focused playing video pauses once', () => {
  const f = fixture({ paused: false })
  f.send()
  assert.equal(f.video.paused, true)
  assert.deepEqual(f.counts(), { plays: 0, pauses: 1, prevented: 1, stopped: 1 })
})

for (const modifier of ['altKey', 'ctrlKey', 'metaKey', 'shiftKey'] as const) {
  test(`${modifier} Space is untouched`, () => {
    const f = fixture(); f.event[modifier] = true; f.send()
    assert.deepEqual(f.counts(), { plays: 0, pauses: 0, prevented: 0, stopped: 0 })
  })
}

for (const mismatch of ['target', 'focus', 'key', 'cancelled'] as const) {
  test(`${mismatch} does not control playback`, () => {
    const f = fixture()
    if (mismatch === 'target') f.event.target = {} as typeof f.video
    if (mismatch === 'focus') f.video.ownerDocument.activeElement = {}
    if (mismatch === 'key') f.event.key = 'Enter'
    if (mismatch === 'cancelled') f.event.defaultPrevented = true
    f.send()
    assert.deepEqual(f.counts(), { plays: 0, pauses: 0, prevented: 0, stopped: 0 })
  })
}

test('held Space and keyup suppress defaults without an extra toggle', () => {
  const f = fixture()
  f.event.repeat = true; f.send()
  f.event.defaultPrevented = false; f.event.repeat = false; f.event.type = 'keyup'; f.send()
  assert.deepEqual(f.counts(), { plays: 0, pauses: 0, prevented: 2, stopped: 2 })
})

test('rejected play is handled without an unhandled rejection', async () => {
  const f = fixture({ rejectPlay: true }); f.send()
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(f.counts().plays, 1)
})

test('production video owns capture-phase key handlers and retains native media errors', () => {
  const source = readFileSync(new URL('../src/panels/Record/RecordingRehearsal.tsx', import.meta.url), 'utf8')
  assert.match(source, /<video[\s\S]*onKeyDownCapture=\{recordingRehearsalPlaybackKey\}[\s\S]*onKeyUpCapture=\{recordingRehearsalPlaybackKey\}/)
  assert.match(source, /controls\s+autoPlay\s+playsInline/)
  assert.match(source, /onError=\{\(\) => setState\(\{ kind: 'error'/)
})
