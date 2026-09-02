export const NATIVE_SYSTEM_AUDIO_SCHEMA = 'shellx-cut/native-system-audio@1'

const COMMIT_RX = /^[a-f0-9]{40}$/
const SHA256_RX = /^[a-f0-9]{64}$/
const CAPTURE_ID_RX = /^[A-Za-z0-9][A-Za-z0-9_-]{7,127}$/
const REQUIRED_CHECKS = new Set([
  'controlled-tone-playback',
  'native-first-packet',
  'pcm-wav-metadata',
  'non-silent-signal',
  'source-identity-stable',
])

function digest(value) {
  return SHA256_RX.test(String(value || ''))
}

function integer(value, { min = 0, max = Number.MAX_SAFE_INTEGER } = {}) {
  return Number.isSafeInteger(value) && value >= min && value <= max
}

function fail(evidenceName, errors) {
  if (errors.length) {
    throw new Error(`native system-audio evidence '${evidenceName}' is invalid: ${errors.join('; ')}`)
  }
}

function platformContract(surface) {
  if (surface === 'linux-control') {
    return {
      hostPlatform: 'linux',
      installedApp: false,
      backend: 'pipewire-default-sink-monitor',
      artifactKinds: new Set(['file']),
    }
  }
  if (surface === 'macos-installed') {
    return {
      hostPlatform: 'darwin',
      installedApp: true,
      backend: 'core-audio-process-tap',
      artifactKinds: new Set(['file', 'tree']),
    }
  }
  return null
}

function artifactBound(receiptArtifact, artifacts) {
  return artifacts.some((artifact) => artifact.sha256 === receiptArtifact?.sha256
    && artifact.kind === receiptArtifact?.kind
    && artifact.bytes === receiptArtifact?.bytes
    && (artifact.kind !== 'tree' || artifact.files === receiptArtifact?.files))
}

/**
 * Validate the native receipt that proves an actual platform system-audio
 * stream. Linux deliberately binds the source-built PipeWire probe because the
 * installed short probe destroys its temporary WAV; macOS binds the permissioned
 * installed app because its Core Audio grant belongs to that app identity.
 */
export function nativeSystemAudioClaim(parsed, {
  source,
  sourceContentManifestSha256,
  surface,
  artifacts,
  evidenceName,
}) {
  if (parsed?.schema !== NATIVE_SYSTEM_AUDIO_SCHEMA) return null
  const errors = []
  const contract = platformContract(surface)
  if (!contract) errors.push('receipt surface has no native system-audio contract')
  if (parsed.status !== 'pass') errors.push('receipt status must be pass')
  if (parsed.surface !== surface) errors.push('receipt surface does not match exact-source surface')
  if (parsed.installedApp !== contract?.installedApp) {
    errors.push(contract?.installedApp
      ? 'macOS evidence must run against the permissioned installed app'
      : 'Linux evidence must declare the source-native PipeWire control honestly')
  }
  if (parsed.host?.platform !== contract?.hostPlatform || !String(parsed.host?.arch || '').trim()) {
    errors.push('receipt host does not match the native surface')
  }

  if (parsed.source?.gitDirty !== false) errors.push('receipt source must be clean')
  if (parsed.source?.gitCommit !== source.gitCommit || !COMMIT_RX.test(String(parsed.source?.gitCommit || ''))) {
    errors.push('receipt source commit does not match')
  }
  if (parsed.source?.version !== source.version) errors.push('receipt source version does not match')
  if (parsed.source?.contentManifestSha256 !== sourceContentManifestSha256
      || !digest(parsed.source?.contentManifestSha256)) {
    errors.push('receipt source content digest does not match')
  }

  const artifact = parsed.artifact
  if (!contract?.artifactKinds.has(artifact?.kind)
      || !digest(artifact?.sha256)
      || !integer(artifact?.bytes, { min: 1 })
      || artifact?.version !== source.version
      || artifact?.integrityVerified !== true
      || (artifact?.kind === 'tree' && !integer(artifact?.files, { min: 1 }))) {
    errors.push('native artifact identity is incomplete')
  } else if (!artifactBound(artifact, artifacts)) {
    errors.push('native artifact is not bound as an exact-source artifact')
  }

  const probe = parsed.probe
  if (probe?.backend !== contract?.backend
      || !integer(probe?.windowMs, { min: 500, max: 10_000 })
      || probe?.live !== true
      || probe?.signalDetected !== true
      || !integer(probe?.firstPacketOffsetMs, { min: 0, max: probe?.windowMs })
      || !integer(probe?.sampleFrames, { min: 1 })) {
    errors.push('native probe lacks a live non-silent first-packet fact')
  }

  const capture = parsed.capture
  const wav = capture?.systemWav
  if (!integer(capture?.durationMs, { min: 1, max: 10_500 })
      || !CAPTURE_ID_RX.test(String(capture?.captureId || ''))
      || !digest(wav?.sha256)
      || !integer(wav?.bytes, { min: 45 })
      || wav?.codec !== 'pcm_s16le'
      || wav?.sampleRate !== 48_000
      || wav?.channels !== 2
      || !integer(wav?.durationMs, { min: 1, max: 10_500 })) {
    errors.push('receipt has no bounded real PCM system.wav evidence')
  }
  if (capture?.firstPacketOffsetMs != null
      && !integer(capture.firstPacketOffsetMs, { min: 0, max: capture.durationMs })) {
    errors.push('capture first-packet timing is invalid')
  }
  if (surface === 'linux-control' && capture?.firstPacketOffsetMs == null) {
    errors.push('Linux PipeWire capture must retain its first-packet timing')
  }

  const signal = parsed.signal
  if (signal?.marker !== 'controlled-tone'
      || !integer(signal?.playbackStartedAfterCaptureMs, { min: 1, max: capture?.durationMs || 0 })
      || !Number.isFinite(signal?.peakDb) || signal.peakDb <= -60
      || !Number.isFinite(signal?.meanDb) || signal.meanDb <= -70
      || signal.detected !== true) {
    errors.push('receipt does not prove a controlled non-silent signal')
  }

  const logs = Array.isArray(parsed.logs) ? parsed.logs : []
  const logNames = logs.map((log) => log?.name)
  if (logs.length < 2 || new Set(logNames).size !== logNames.length
      || logs.some((log) => !/^[a-z0-9][a-z0-9-]*$/.test(String(log?.name || ''))
        || !digest(log?.sha256) || !integer(log?.bytes, { min: 0 }))) {
    errors.push('receipt logs are not uniquely digest-bound')
  }

  const checks = Array.isArray(parsed.checks) ? parsed.checks : []
  const checkIds = checks.map((check) => check?.id)
  if (!checks.length || new Set(checkIds).size !== checkIds.length
      || checks.some((check) => check?.pass !== true)
      || [...REQUIRED_CHECKS].some((id) => !checkIds.includes(id))) {
    errors.push('receipt checks are incomplete or non-passing')
  }
  fail(evidenceName, errors)
  return {
    surface,
    installedApp: parsed.installedApp,
    platform: parsed.host.platform,
    backend: probe.backend,
    gitCommit: parsed.source.gitCommit,
    version: parsed.source.version,
    sourceContentManifestSha256: parsed.source.contentManifestSha256,
    artifactSha256: artifact.sha256,
    artifactKind: artifact.kind,
    systemWavSha256: wav.sha256,
    probeFirstPacketOffsetMs: probe.firstPacketOffsetMs,
    captureFirstPacketOffsetMs: capture.firstPacketOffsetMs ?? null,
    checks: checkIds,
  }
}
