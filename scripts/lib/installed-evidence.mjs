const SHA256_RX = /^[a-f0-9]{64}$/

const REAL_DROP_CONTRACTS = {
  'shellx-cut/windows-installed-real-file-drop@1': {
    surface: 'windows-installed',
    platform: 'win32',
    gesture: 'real-explorer-ole-file-drag',
    checks: ['projects-first', 'video-real-explorer-drop-create', 'image-real-explorer-drop-create'],
  },
  'shellx-cut/macos-installed-real-file-drop@1': {
    surface: 'macos-installed',
    platform: 'darwin',
    gesture: 'real-finder-file-window-drag',
    checks: ['projects-first', 'video-real-finder-drop-create', 'image-real-finder-drop-create'],
  },
  'shellx-cut/linux-installed-real-file-drop@1': {
    surface: 'linux-control',
    platform: 'linux',
    gesture: 'real-nautilus-x11-file-drag',
    checks: ['projects-first', 'video-real-nautilus-drop-create', 'image-real-nautilus-drop-create'],
  },
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

function validateVideoProject(entry, errors) {
  const project = projectState(entry)
  const assets = Object.values(project?.assets || {})
  const asset = assets.find((item) => Number(item?.probe?.width) > 0
    && Number(item?.probe?.height) > 0 && Number(item?.probe?.fps) > 0)
  if (!project || !entry?.name || project.name !== entry.name || assets.length < 1) {
    errors.push('video drop did not preserve its named populated project state')
    return
  }
  if (!entry.native || typeof entry.native !== 'object' || Object.keys(entry.native).length === 0) {
    errors.push('video drop has no native gesture telemetry')
  }
  if (!clips(project).some((clip) => clip.trackKind === 'video')) {
    errors.push('video drop did not preserve a video timeline clip')
  }
  if (!asset) {
    errors.push('video drop has no positive source geometry and frame-rate probe')
    return
  }
  if (Number(project.settings?.width) !== Number(asset.probe.width)
      || Number(project.settings?.height) !== Number(asset.probe.height)) {
    errors.push('video project settings do not match the source geometry')
  }
  if (Math.abs(Number(project.settings?.fps) - Number(asset.probe.fps)) >= 0.02) {
    errors.push('video project frame rate does not match the source probe')
  }
}

function validateImageProject(entry, errors) {
  const project = projectState(entry)
  if (!project || !entry?.name || project.name !== entry.name
      || Object.keys(project.assets || {}).length < 1) {
    errors.push('image drop did not preserve its named populated project state')
    return
  }
  if (!entry.native || typeof entry.native !== 'object' || Object.keys(entry.native).length === 0) {
    errors.push('image drop has no native gesture telemetry')
  }
  if (!clips(project).some((clip) =>
    clip.trackKind === 'video' && clipDurationMs(clip) === 5_000)) {
    errors.push('image drop did not preserve a five-second video timeline clip')
  }
}

export function realFileDropClaim(parsed, { source, surface, artifacts, evidenceName }) {
  const contract = REAL_DROP_CONTRACTS[parsed?.schema]
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
  if (!digest(parsed.runtime?.cutd?.sha256)) errors.push('installed cutd digest is invalid')
  if (!digest(parsed.media?.video?.sha256) || !digest(parsed.media?.image?.sha256)) {
    errors.push('real drop media digests are incomplete')
  }

  const projects = Array.isArray(parsed.projects) ? parsed.projects : []
  const video = projects.filter((item) => item?.kind === 'video')
  const image = projects.filter((item) => item?.kind === 'image')
  if (video.length !== 1 || image.length !== 1) errors.push('receipt requires exactly one video and one image project')
  if (video.length === 1) validateVideoProject(video[0], errors)
  if (image.length === 1) validateImageProject(image[0], errors)

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
