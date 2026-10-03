import assert from 'node:assert/strict'
import test from 'node:test'
import { handleVoiceoverPlaybackCommand } from '../src/app/voiceoverPlaybackCommand'
import { clearVoiceoverOwner, rememberVoiceoverOwner } from '../src/app/voiceoverOwnerRuntime'

type CommandArgs = Parameters<typeof handleVoiceoverPlaybackCommand>[0]

async function exercise(options: {
  playhead?: number
  commit?: boolean
  playing?: boolean
  wrongOwner?: boolean
  staleProject?: boolean
  locked?: boolean
} = {}) {
  const saved = ['window', 'document'].map((key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const)
  const stateRef = { current: { state_revision: 7, playhead_ms: options.playhead ?? 0 } } as CommandArgs['stateRef']
  const calls: string[] = []
  const answers: unknown[][] = []
  const errors: Array<{ code: string; message: string }> = []
  const identity = {
    request_id: 'voiceover-owned-take', expected_revision: 'op_000001',
    audio_track: 'a1t', start_ms: 0, out_ms: null,
  }
  Object.defineProperty(globalThis, 'window', { configurable: true, value: {
    requestAnimationFrame: (callback: FrameRequestCallback) => setTimeout(() => callback(0), 0),
    cancelAnimationFrame: clearTimeout, setTimeout, clearTimeout,
  } })
  Object.defineProperty(globalThis, 'document', { configurable: true, value: {
    dispatchEvent(event: CustomEvent) {
      calls.push('playback')
      assert.equal(stateRef.current.playhead_ms, 0)
      assert.equal(event.type, 'cut:voiceover-playback')
      assert.equal(event.detail.request_id, identity.request_id)
      assert.equal(event.detail.bridge_epoch, 1)
      assert.equal(event.detail.owner_session_id, 'session-owned')
      return true
    },
    querySelector(selector: string) {
      assert.equal(selector, '[data-cut-panel="preview"][data-cut-playing="true"]')
      return options.playing === false ? null : {}
    },
  } })
  clearVoiceoverOwner()
  rememberVoiceoverOwner({ ...identity, request_id: options.wrongOwner ? 'other-take' : identity.request_id },
    { session_id: 'session-owned', capability: 'capability-owned' })
  let commitTimer: ReturnType<typeof setTimeout> | undefined
  try {
    await handleVoiceoverPlaybackCommand({
      command: { request_id: 11, args: {
        request_id: identity.request_id, request_fingerprint: 'c'.repeat(64), bridge_epoch: 1,
        accepted_revision: identity.expected_revision, audio_track: identity.audio_track,
        start_ms: 0, out_ms: null,
      } },
      project: { project_revision: options.staleProject ? 'op_stale' : identity.expected_revision,
        tracks: [{ id: 'a1t', kind: 'audio', locked: options.locked ?? false }] } as CommandArgs['project'],
      beforeRevision: 7, stateRef,
      setPlayheadMs(value) {
        assert.equal(value, 0)
        calls.push('seek')
        if (options.commit) commitTimer = setTimeout(() => {
          calls.push('commit')
          stateRef.current = { ...stateRef.current, state_revision: 8, playhead_ms: 0 }
        }, 20)
      },
      answer: (...args) => { calls.push('answer'); answers.push(args) },
      reject: (_requested, error) => { errors.push(error) },
    })
    return { calls, answers, errors, revision: stateRef.current.state_revision }
  } finally {
    if (commitTimer !== undefined) clearTimeout(commitTimer)
    clearVoiceoverOwner()
    for (const [key, descriptor] of saved) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor)
      else Reflect.deleteProperty(globalThis, key)
    }
  }
}

test('already committed Voiceover start position acknowledges playback without an unrelated revision', async () => {
  const result = await exercise()
  assert.deepEqual(result.errors, [])
  assert.equal(result.answers.length, 1)
  assert.equal(result.revision, 7)
  assert.deepEqual(result.calls, ['playback', 'answer'])
})

test('changed Voiceover position waits for the real committed seek before playback', async () => {
  const result = await exercise({ playhead: 900, commit: true })
  assert.deepEqual(result.errors, [])
  assert.deepEqual(result.calls, ['seek', 'commit', 'playback', 'answer'])
  assert.equal(result.revision, 8)
})

test('a changed position that never commits refuses playback', async () => {
  const result = await exercise({ playhead: 900 })
  assert.deepEqual(result.calls, ['seek'])
  assert.equal(result.answers.length, 0)
  assert.match(result.errors[0].message, /playhead did not commit/)
})

for (const [name, options, error] of [
  ['wrong owner', { wrongOwner: true }, /does not own/],
  ['stale project', { staleProject: true }, /stale/],
  ['locked track', { locked: true }, /unlocked audio track/],
] as const) test(`${name} refuses before seeking or playback`, async () => {
  const result = await exercise(options)
  assert.deepEqual(result.calls, [])
  assert.equal(result.answers.length, 0)
  assert.match(result.errors[0].message, error)
})

test('committed no-op seek still refuses when Preview did not actually begin playback', async () => {
  const result = await exercise({ playing: false })
  assert.equal(result.answers.length, 0)
  assert.match(result.errors[0].message, /Preview did not begin/)
  assert.ok(result.calls.includes('playback'))
})
