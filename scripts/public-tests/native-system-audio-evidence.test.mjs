import test from 'node:test'
import assert from 'node:assert/strict'

import {
  NATIVE_SYSTEM_AUDIO_SCHEMA,
  nativeSystemAudioClaim,
} from '../lib/native-system-audio-evidence.mjs'

const source = {
  gitCommit: 'a'.repeat(40),
  version: '0.6.109',
}
const contentManifestSha256 = 'b'.repeat(64)
const sha = (char) => char.repeat(64)

function receipt(surface = 'linux-control') {
  const mac = surface === 'macos-installed'
  const artifact = mac
    ? { kind: 'tree', bytes: 321, files: 3, sha256: sha('c'), version: source.version, integrityVerified: true }
    : { kind: 'file', bytes: 123, sha256: sha('c'), version: source.version, integrityVerified: true }
  return {
    schema: NATIVE_SYSTEM_AUDIO_SCHEMA,
    generatedAt: '2026-08-12T06:00:00.000Z',
    status: 'pass',
    installedApp: mac,
    surface,
    host: { platform: mac ? 'darwin' : 'linux', arch: 'arm64' },
    source: { ...source, gitDirty: false, contentManifestSha256 },
    artifact,
    probe: {
      backend: mac ? 'core-audio-process-tap' : 'pipewire-default-sink-monitor',
      windowMs: 5_000,
      live: true,
      signalDetected: true,
      firstPacketOffsetMs: 750,
      sampleFrames: 120_000,
    },
    capture: {
      durationMs: 5_000,
      captureId: 'cap_audio_20260812_0001',
      systemWav: {
        sha256: sha('d'), bytes: 960_044, codec: 'pcm_s16le', sampleRate: 48_000, channels: 2, durationMs: 5_000,
      },
      ...(mac ? {} : { firstPacketOffsetMs: 750 }),
    },
    signal: {
      marker: 'controlled-tone', playbackStartedAfterCaptureMs: 750, peakDb: -6.1, meanDb: -19.4, detected: true,
    },
    logs: [
      { name: 'probe-stdout', kind: 'file', bytes: 81, sha256: sha('e') },
      { name: 'wav-metadata', kind: 'file', bytes: 93, sha256: sha('f') },
    ],
    checks: [
      { id: 'controlled-tone-playback', pass: true },
      { id: 'native-first-packet', pass: true },
      { id: 'pcm-wav-metadata', pass: true },
      { id: 'non-silent-signal', pass: true },
      { id: 'source-identity-stable', pass: true },
    ],
  }
}

function claim(parsed, surface = parsed.surface) {
  return nativeSystemAudioClaim(parsed, {
    source,
    sourceContentManifestSha256: contentManifestSha256,
    surface,
    artifacts: [parsed.artifact],
    evidenceName: 'native-system-audio',
  })
}

test('Linux source-native PipeWire receipt binds a retained real WAV and first packet', () => {
  const parsed = receipt()
  const result = claim(parsed)
  assert.equal(result.installedApp, false)
  assert.equal(result.backend, 'pipewire-default-sink-monitor')
  assert.equal(result.captureFirstPacketOffsetMs, 750)
  assert.equal(result.systemWavSha256, sha('d'))
})

test('macOS installed Core Audio receipt may use physical WAV padding instead of a capture sidecar', () => {
  const parsed = receipt('macos-installed')
  const result = claim(parsed)
  assert.equal(result.installedApp, true)
  assert.equal(result.backend, 'core-audio-process-tap')
  assert.equal(result.captureFirstPacketOffsetMs, null)
  assert.equal(result.artifactKind, 'tree')
})

test('native system-audio evidence fails closed on replay, silence, wrong host, or unbound artifacts', () => {
  const replay = receipt()
  replay.probe.firstPacketOffsetMs = null
  assert.throws(() => claim(replay), /first-packet/)

  const silent = receipt()
  silent.signal.meanDb = -70
  assert.throws(() => claim(silent), /controlled non-silent signal/)

  const host = receipt('macos-installed')
  host.host.platform = 'linux'
  assert.throws(() => claim(host), /host does not match/)

  const unbound = receipt()
  assert.throws(() => nativeSystemAudioClaim(unbound, {
    source,
    sourceContentManifestSha256: contentManifestSha256,
    surface: 'linux-control',
    artifacts: [],
    evidenceName: 'native-system-audio',
  }), /not bound/)
})

test('native system-audio evidence refuses incomplete checks and Linux records a capture first packet', () => {
  const missing = receipt()
  missing.checks.pop()
  assert.throws(() => claim(missing), /checks are incomplete/)

  const missingLinuxTiming = receipt()
  delete missingLinuxTiming.capture.firstPacketOffsetMs
  assert.throws(() => claim(missingLinuxTiming), /Linux PipeWire capture/)
})
