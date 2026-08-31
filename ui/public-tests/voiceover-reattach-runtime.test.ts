import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

import {
  clearVoiceoverOwner,
  persistVoiceoverReattach,
  readVoiceoverReattach,
} from '../src/app/voiceoverOwnerRuntime'
import {
  selectVoiceoverStartIdentity,
  VOICEOVER_START_RETRY_REJECTED,
  voiceoverStartRetryWasRejected,
} from '../src/app/voiceoverStartIdentity'

const REATTACH_STORAGE_KEY = 'shellx-cut/voiceover-reattach/1'

test('voiceover persists a non-secret retry identity before start so a reload can reattach', () => {
  const savedWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')
  const values = new Map<string, string>()
  const sessionStorage = {
    getItem(key: string) { return values.get(key) ?? null },
    setItem(key: string, value: string) { values.set(key, value) },
    removeItem(key: string) { values.delete(key) },
  } as Storage
  Object.defineProperty(globalThis, 'window', {
    value: { sessionStorage },
    configurable: true,
  })

  const identity = {
    request_id: 'voiceover-retry-after-lost-response',
    expected_revision: 'project-revision-19',
    audio_track: 'audio-track-2',
    start_ms: 11_000,
    out_ms: 14_000,
  }
  try {
    clearVoiceoverOwner()
    persistVoiceoverReattach(identity)
    const persisted = JSON.parse(values.get(REATTACH_STORAGE_KEY) ?? '{}')
    assert.deepEqual(persisted, identity, 'the record sent before a response leaves a reload-safe identity')
    assert.equal('owner_session_id' in persisted, false)
    assert.equal('capability' in persisted, false)
    // A fresh control instance reads sessionStorage rather than volatile owner
    // memory, then reuses this exact start identity to recover the active claim.
    assert.deepEqual(readVoiceoverReattach(), identity)

    const control = readFileSync(new URL('../src/panels/Timeline/VoiceoverTrackControl.tsx', import.meta.url), 'utf8')
    const persistBeforeSend = control.indexOf('persistVoiceoverReattach(identity)')
    const startSend = control.indexOf("callVerb('voiceover.start'")
    assert.ok(persistBeforeSend >= 0 && persistBeforeSend < startSend, 'identity is persisted before the first voiceover.start send')
    assert.match(control, /useRef<VoiceoverReattachIdentity \| null>\(readVoiceoverReattach\(\)\)/)
    assert.match(control, /const reattach = retryIdentity\.current[\s\S]{0,240}const identity = reattach \?\? selectVoiceoverStartIdentity\(null/)
    assert.match(control, /if \(releaseTerminal\(next\)\) \{[\s\S]{0,120}clearVoiceoverOwner\(\)/)
    assert.match(control, /if \(!response\.ok\) \{[\s\S]{0,120}if \(voiceoverStartRetryWasRejected\(response\.error\)\) \{[\s\S]{0,120}retryIdentity\.current = null[\s\S]{0,120}clearVoiceoverOwner\(\)/)
    assert.match(control, /if \(!response\.result\) \{[\s\S]{0,180}retries the same request/)
  } finally {
    clearVoiceoverOwner()
    if (savedWindow) Object.defineProperty(globalThis, 'window', savedWindow)
    else delete (globalThis as { window?: unknown }).window
  }
})

test('a known refusal releases A for B while a transport-unknown reload retries exact A', () => {
  const savedWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')
  const values = new Map<string, string>()
  const sessionStorage = {
    getItem(key: string) { return values.get(key) ?? null },
    setItem(key: string, value: string) { values.set(key, value) },
    removeItem(key: string) { values.delete(key) },
  } as Storage
  Object.defineProperty(globalThis, 'window', {
    value: { sessionStorage },
    configurable: true,
  })
  const a = {
    request_id: 'voiceover-a',
    expected_revision: 'project-revision-19',
    audio_track: 'audio-track-2',
    start_ms: 11_000,
    out_ms: 14_000,
  }
  const b = {
    projectRevision: 'project-revision-20',
    audioTrack: 'audio-track-3',
    startMs: 22_000,
    outMs: 25_000,
  }
  try {
    clearVoiceoverOwner()
    persistVoiceoverReattach(a)
    const reloadA = readVoiceoverReattach()
    let minted = 0
    assert.deepEqual(
      selectVoiceoverStartIdentity(reloadA, b, () => { minted += 1; return 'voiceover-b' }),
      a,
      'a transport-unknown reload repeats exactly A even when the local range now points at B',
    )
    assert.equal(minted, 0, 'transport-unknown recovery does not mint a second request')

    assert.equal(voiceoverStartRetryWasRejected({ code: 'conflict' }), false)
    assert.equal(voiceoverStartRetryWasRejected(undefined), false)
    assert.equal(voiceoverStartRetryWasRejected({ code: VOICEOVER_START_RETRY_REJECTED }), true)
    assert.deepEqual(
      selectVoiceoverStartIdentity(readVoiceoverReattach(), b, () => { minted += 1; return 'voiceover-ignored' }),
      a,
      'a generic rejection cannot overwrite unresolved A with current B',
    )
    assert.equal(minted, 0)

    // Only the explicit pre-admission/no-owner disposition releases A. The
    // later Record click can then mint B at the current revision and range.
    clearVoiceoverOwner()
    const next = selectVoiceoverStartIdentity(readVoiceoverReattach(), b, () => { minted += 1; return 'voiceover-b' })
    assert.deepEqual(next, {
      request_id: 'voiceover-b',
      expected_revision: b.projectRevision,
      audio_track: b.audioTrack,
      start_ms: b.startMs,
      out_ms: b.outMs,
    })
    assert.equal(minted, 1, 'a known-rejected A cannot silently replay after moving to B')
  } finally {
    clearVoiceoverOwner()
    if (savedWindow) Object.defineProperty(globalThis, 'window', savedWindow)
    else delete (globalThis as { window?: unknown }).window
  }
})
