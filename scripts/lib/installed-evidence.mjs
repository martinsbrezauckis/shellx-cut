const SHA256_RX = /^[a-f0-9]{64}$/

const REAL_DROP_CONTRACTS = {
  'shellx-cut/windows-installed-real-file-drop@1': {
    surface: 'windows-installed',
    platform: 'win32',
    manager: 'explorer',
    gesture: 'real-explorer-ole-file-drag',
    runtimeVerificationModes: ['authenticode'],
    checks: ['projects-first', 'video-real-explorer-drop-create', 'image-real-explorer-drop-create'],
  },
  'shellx-cut/macos-installed-real-file-drop@1': {
    surface: 'macos-installed',
    platform: 'darwin',
    manager: 'finder',
    gesture: 'real-finder-file-window-drag',
    runtimeVerificationModes: ['codesign-notarization'],
    checks: ['projects-first', 'video-real-finder-drop-create', 'image-real-finder-drop-create'],
  },
  'shellx-cut/linux-installed-real-file-drop@1': {
    surface: 'linux-control',
    platform: 'linux',
    manager: 'nautilus',
    gesture: 'real-nautilus-x11-file-drag',
    runtimeVerificationModes: ['package-integrity'],
    checks: ['projects-first', 'video-real-nautilus-drop-create', 'image-real-nautilus-drop-create'],
  },
}

export function realFileDropContract(schema) {
  return REAL_DROP_CONTRACTS[schema] || null
}

function digest(value) {
  return SHA256_RX.test(String(value || ''))
}

function clips(project) {
  return (project?.tracks || []).flatMap((track) =>
    (track.clips || []).map((clip) => ({ ...clip, trackKind: track.kind })))
}

function clipDurationMs(clip) {
  const start = Number(clip.src_in_ms)
  const end = Number(clip.src_out_ms)
  const speed = Math.abs(Number(clip.speed) || 1)
  return Number.isFinite(start) && Number.isFinite(end) && speed > 0
    ? Math.round(Math.abs(end - start) / speed)
    : null
}

function projectState(entry) {
  return entry?.state ?? entry?.project
}

function normalizedAssetHash(value) {
  return String(value || '').replace(/^sha256:/, '')
}

function projectAsset(project, mediaSha256, kind) {
  return Object.values(project?.assets || {}).find((asset) =>
    normalizedAssetHash(asset?.hash) === mediaSha256 && asset?.probe?.kind === kind)
}

function clipForAsset(project, assetId) {
  return clips(project).filter((clip) => clip.trackKind === 'video' && clip.asset === assetId)
}

function validateVideoProject(entry, mediaSha256, errors) {
  const project = projectState(entry)
  const assets = Object.values(project?.assets || {})
  const asset = projectAsset(project, mediaSha256, 'video')
  if (!project || !entry?.name || project.name !== entry.name || assets.length < 1) {
    errors.push('video drop did not preserve its named populated project state')
    return
  }
  if (!entry.native || typeof entry.native !== 'object' || Object.keys(entry.native).length === 0) {
    errors.push('video drop has no native gesture telemetry')
  }
  if (!asset) {
    errors.push('video drop does not bind its video asset to the dropped media digest')
    return
  }
  const [assetId] = Object.entries(project.assets).find(([, item]) => item === asset) || []
  if (!clipForAsset(project, assetId).length) errors.push('video drop did not preserve a video timeline clip for its dropped asset')
  if (!(Number(asset?.probe?.width) > 0 && Number(asset?.probe?.height) > 0 && Number(asset?.probe?.fps) > 0)) {
    errors.push('video drop has no positive source geometry and frame-rate probe')
  }
  if (Number(project.settings?.width) !== Number(asset.probe.width)
      || Number(project.settings?.height) !== Number(asset.probe.height)) {
    errors.push('video project settings do not match the source geometry')
  }
  if (Math.abs(Number(project.settings?.fps) - Number(asset.probe.fps)) >= 0.02) {
    errors.push('video project frame rate does not match the source probe')
  }
}

function validateImageProject(entry, mediaSha256, errors) {
  const project = projectState(entry)
  if (!project || !entry?.name || project.name !== entry.name
      || Object.keys(project.assets || {}).length < 1) {
    errors.push('image drop did not preserve its named populated project state')
    return
  }
  if (!entry.native || typeof entry.native !== 'object' || Object.keys(entry.native).length === 0) {
    errors.push('image drop has no native gesture telemetry')
  }
  const asset = projectAsset(project, mediaSha256, 'image')
  if (!asset) {
    errors.push('image drop does not bind its image asset to the dropped media digest')
    return
  }
  const [assetId] = Object.entries(project.assets).find(([, item]) => item === asset) || []
  if (!clipForAsset(project, assetId).some((clip) => clipDurationMs(clip) === 5_000)) {
    errors.push('image drop did not preserve a five-second video timeline clip for its dropped asset')
  }
}

function validateNativeGesture(entry, contract, kind, errors) {
  const native = entry?.native
  if (native?.gesture !== contract.gesture) {
    errors.push(`${kind} drop native telemetry does not name the required real gesture`)
  }
  if (!digest(native?.evidence?.sha256) || !Number.isInteger(native?.evidence?.bytes)
      || native.evidence.bytes < 1) {
    errors.push(`${kind} drop native telemetry does not bind gesture evidence`)
  }
}

function validateRuntimeIntegrity(parsed, contract, source, shellSha256, cutdSha256, errors) {
  const integrity = parsed.runtime?.integrity
  if (integrity?.schema !== 'shellx-cut/real-file-drop-runtime-integrity@1'
      || integrity.status !== 'pass') {
    errors.push('installed runtime integrity receipt is not passing')
    return
  }
  if (integrity.surface !== contract.surface || integrity.sourceHead !== source.gitCommit) {
    errors.push('installed runtime integrity receipt does not match the exact source surface')
  }
  const expected = { shellSha256, cutdSha256 }
  for (const phase of ['pre', 'post']) {
    if (integrity[phase]?.shellSha256 !== expected.shellSha256
        || integrity[phase]?.cutdSha256 !== expected.cutdSha256) {
      errors.push(`installed runtime integrity ${phase}-use digests do not match`)
    }
  }
  if (!contract.runtimeVerificationModes.includes(integrity.verification?.mode)
      || integrity.verification?.shell !== true || integrity.verification?.cutd !== true) {
    errors.push('installed shell/cutd signing or package-integrity evidence is incomplete')
  }
}

export function realFileDropClaim(parsed, { source, surface, artifacts, evidenceName }) {
  const contract = realFileDropContract(parsed?.schema)
  if (!contract) return null
  const errors = []
  if (parsed.ok !== true || parsed.installedApp !== true) errors.push('receipt must pass against an installed app')
  if (contract.surface !== surface) errors.push('receipt schema does not match the exact-source surface')
  if (parsed.platform !== contract.platform) errors.push('receipt platform does not match its schema')
  if (parsed.gesture !== contract.gesture) errors.push('receipt did not use the required real file-manager gesture')
  if (parsed.source?.head !== source.gitCommit) errors.push('receipt source commit does not match')

  const checks = Array.isArray(parsed.checks) ? parsed.checks : []
  const checkIds = checks.map((item) => item?.id)
  if (new Set(checkIds).size !== checkIds.length) errors.push('receipt contains duplicate check ids')
  if (checks.some((item) => item?.pass !== true)) errors.push('receipt contains a non-passing check')
  for (const id of contract.checks) {
    if (!checks.some((item) => item?.id === id && item.pass === true)) errors.push(`missing passing check '${id}'`)
  }

  const shellSha256 = parsed.runtime?.shell?.sha256
  if (!digest(shellSha256) || !artifacts.some((item) => item.sha256 === shellSha256)) {
    errors.push('installed shell digest is not bound as an exact-source artifact')
  }
  const cutdSha256 = parsed.runtime?.cutd?.sha256
  if (!digest(cutdSha256) || !artifacts.some((item) => item.sha256 === cutdSha256)) {
    errors.push('installed cutd digest is not bound as an exact-source artifact')
  }
  validateRuntimeIntegrity(parsed, contract, source, shellSha256, cutdSha256, errors)
  if (!digest(parsed.media?.video?.sha256) || !digest(parsed.media?.image?.sha256)) {
    errors.push('real drop media digests are incomplete')
  }

  const projects = Array.isArray(parsed.projects) ? parsed.projects : []
  const video = projects.filter((item) => item?.kind === 'video')
  const image = projects.filter((item) => item?.kind === 'image')
  if (video.length !== 1 || image.length !== 1) errors.push('receipt requires exactly one video and one image project')
  if (video.length === 1) {
    validateNativeGesture(video[0], contract, 'video', errors)
    validateVideoProject(video[0], parsed.media?.video?.sha256, errors)
  }
  if (image.length === 1) {
    validateNativeGesture(image[0], contract, 'image', errors)
    validateImageProject(image[0], parsed.media?.image?.sha256, errors)
  }

  if (errors.length) {
    throw new Error(`real file-drop evidence '${evidenceName}' is invalid: ${errors.join('; ')}`)
  }
  return {
    surface,
    platform: parsed.platform,
    gesture: parsed.gesture,
    gitCommit: parsed.source.head,
    shellSha256,
    cutdSha256: parsed.runtime.cutd.sha256,
    media: { videoSha256: parsed.media.video.sha256, imageSha256: parsed.media.image.sha256 },
    cases: ['video', 'image'],
    checks: contract.checks,
  }
}

const WALKTHROUGH_ROWS = [
  'installed-agent-docs',
  'settings',
  'library',
  'about',
  'debug-api',
  'mcp-self-test',
]

export function installedWalkthroughClaim(parsed, {
  source,
  sourceContentManifestSha256,
  surface,
  artifacts,
  evidenceName,
}) {
  if (parsed?.schema !== 'shellx-cut/installed-surface-walkthrough@1') return null
  const errors = []
  if (parsed.status !== 'pass' || parsed.installedApp !== true) {
    errors.push('receipt must pass against an installed app')
  }
  if (parsed.surface !== surface) errors.push('receipt surface does not match')
  if (parsed.source?.gitCommit !== source.gitCommit) errors.push('receipt source commit does not match')
  if (parsed.source?.version !== source.version) errors.push('receipt version does not match')
  if (parsed.source?.contentManifestSha256 !== sourceContentManifestSha256) {
    errors.push('receipt synchronized-content digest does not match')
  }
  if (!digest(parsed.artifact?.sha256)
      || !artifacts.some((item) => item.sha256 === parsed.artifact.sha256)) {
    errors.push('installed artifact digest is not bound as an exact-source artifact')
  }
  if (parsed.artifact?.version !== source.version) errors.push('installed artifact version does not match')
  if (parsed.artifact?.integrityVerified !== true) errors.push('installed artifact integrity is unverified')
  if (parsed.artifact?.webdriverTestFeatureAbsent !== true) {
    errors.push('shipping artifact does not prove the WebDriver test feature is absent')
  }
  if (surface === 'macos-installed' && parsed.artifact?.notarized !== true) {
    errors.push('macOS shipping artifact is not notarized')
  }
  if (surface === 'windows-installed' && parsed.artifact?.signed !== true) {
    errors.push('Windows shipping artifact is not signed')
  }

  const rows = Array.isArray(parsed.rows) ? parsed.rows : []
  const ids = rows.map((row) => row?.id)
  if (new Set(ids).size !== ids.length) errors.push('walkthrough contains duplicate row ids')
  if (rows.some((row) => row?.status !== 'pass')) errors.push('walkthrough contains a non-passing row')
  for (const id of WALKTHROUGH_ROWS) {
    if (!rows.some((row) => row?.id === id && row.status === 'pass')) {
      errors.push(`missing passing row '${id}'`)
    }
  }
  if (errors.length) {
    throw new Error(`installed walkthrough evidence '${evidenceName}' is invalid: ${errors.join('; ')}`)
  }
  return {
    surface,
    gitCommit: parsed.source.gitCommit,
    version: parsed.source.version,
    sourceContentManifestSha256: parsed.source.contentManifestSha256,
    artifactSha256: parsed.artifact.sha256,
    integrityVerified: true,
    webdriverTestFeatureAbsent: true,
    notarized: parsed.artifact.notarized === true,
    signed: parsed.artifact.signed === true,
    rows: WALKTHROUGH_ROWS,
  }
}
