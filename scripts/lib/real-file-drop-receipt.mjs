import { createHash } from 'node:crypto'
import { mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'

import { realFileDropContract } from './installed-evidence.mjs'

const COMMIT_RX = /^[a-f0-9]{40}$/
const SHA256_RX = /^[a-f0-9]{64}$/

function fail(message) {
  throw new Error(`real file-drop receipt: ${message}`)
}

function digestPath(path, label) {
  const resolved = resolve(String(path || ''))
  let stat
  let bytes
  try {
    stat = statSync(resolved)
    bytes = readFileSync(resolved)
  } catch (error) {
    fail(`${label} is not a readable file: ${error?.message || String(error)}`)
  }
  if (!stat.isFile()) fail(`${label} must be a file`)
  return {
    path: resolved,
    bytes: stat.size,
    sha256: createHash('sha256').update(bytes).digest('hex'),
  }
}

function state(value, label) {
  const current = value?.result ?? value?.state ?? value?.project ?? value
  if (!current || typeof current !== 'object' || Array.isArray(current)) fail(`${label} is not a project-state object`)
  return current
}

function clips(project) {
  return (project.tracks || []).flatMap((track) =>
    (track.clips || []).map((clip) => ({ ...clip, trackKind: track.kind })))
}

function mediaAsset(project, sha256, kind) {
  return Object.entries(project.assets || {}).find(([, asset]) =>
    String(asset?.hash || '').replace(/^sha256:/, '') === sha256 && asset?.probe?.kind === kind)
}

function clipDurationMs(clip) {
  const start = Number(clip.src_in_ms)
  const end = Number(clip.src_out_ms)
  const speed = Math.abs(Number(clip.speed) || 1)
  return Number.isFinite(start) && Number.isFinite(end) && speed > 0
    ? Math.round(Math.abs(end - start) / speed)
    : null
}

function validateProjectsFirst(value) {
  const ui = value?.result ?? value
  if (ui?.left?.active_tab !== 'projects' || ui.left?.collapsed !== false || ui?.project?.open !== false) {
    fail('projects-first state must show a visible Projects tab with no open project')
  }
  return ui
}

function projectClaim(entry, { kind, media, contract }) {
  if (entry?.kind !== kind) fail(`expected a ${kind} project entry`)
  const project = state(entry.state, `${kind} project`)
  if (!entry.name || project.name !== entry.name || !Object.keys(project.assets || {}).length) {
    fail(`${kind} project is not a named populated project state`)
  }
  const [assetId, asset] = mediaAsset(project, media.sha256, kind) || []
  if (!asset) fail(`${kind} project does not contain the exact dropped ${kind} asset`)
  const projectClips = clips(project).filter((clip) => clip.trackKind === 'video' && clip.asset === assetId)
  if (kind === 'video') {
    if (!projectClips.length) fail('video project has no video clip for the dropped asset')
    if (!(Number(asset.probe?.width) > 0 && Number(asset.probe?.height) > 0 && Number(asset.probe?.fps) > 0)) {
      fail('video project source probe has no positive geometry and frame rate')
    }
    if (Number(project.settings?.width) !== Number(asset.probe.width)
        || Number(project.settings?.height) !== Number(asset.probe.height)
        || Math.abs(Number(project.settings?.fps) - Number(asset.probe.fps)) >= 0.02) {
      fail('video project does not adopt its dropped source format')
    }
  } else if (!projectClips.some((clip) => clipDurationMs(clip) === 5_000)) {
    fail('image project has no five-second video clip for the dropped asset')
  }
  const native = entry.native || {}
  if (native.gesture !== contract.gesture || typeof native.protocol !== 'string' || !native.protocol.trim()) {
    fail(`${kind} native evidence does not declare the required real file-manager gesture`)
  }
  const evidence = digestPath(native.evidencePath, `${kind} native gesture evidence`)
  return {
    kind,
    name: entry.name,
    native: {
      gesture: contract.gesture,
      protocol: native.protocol.trim(),
      evidence,
    },
    state: project,
  }
}

function runtimeIntegrity(value, { contract, sourceHead, shell, cutd }) {
  if (value?.schema !== 'shellx-cut/real-file-drop-runtime-integrity@1' || value.status !== 'pass') {
    fail('runtime integrity input must be a passing real-file-drop runtime integrity receipt')
  }
  if (value.surface !== contract.surface || value.sourceHead !== sourceHead) {
    fail('runtime integrity input does not match the source surface')
  }
  const expected = { shellSha256: shell.sha256, cutdSha256: cutd.sha256 }
  for (const phase of ['pre', 'post']) {
    if (value[phase]?.shellSha256 !== expected.shellSha256
        || value[phase]?.cutdSha256 !== expected.cutdSha256) {
      fail(`runtime integrity ${phase}-use digests do not match the installed files`)
    }
  }
  if (!contract.runtimeVerificationModes.includes(value.verification?.mode)
      || value.verification?.shell !== true || value.verification?.cutd !== true) {
    fail('runtime integrity input has incomplete signing or package-integrity proof')
  }
  return value
}

/**
 * Produce the portable claim from a private, host-specific observation draft.
 * The draft and result intentionally stay outside the product source tree:
 * they can contain private machine paths and native-input evidence.
 */
export function createRealFileDropReceipt(draft, { generatedAt = new Date().toISOString() } = {}) {
  if (draft?.schema !== 'shellx-cut/real-file-drop-input@1') {
    fail('input schema must be shellx-cut/real-file-drop-input@1')
  }
  const contract = realFileDropContract(draft.schemaVersion)
  if (!contract || draft.surface !== contract.surface || draft.platform !== contract.platform) {
    fail('input schemaVersion, surface, and platform must name one supported installed surface')
  }
  const sourceHead = String(draft.source?.head || '')
  if (!COMMIT_RX.test(sourceHead)) fail('source.head must be a full Git SHA')
  if (draft.installedApp !== true) fail('input must affirm an installed app')
  const shell = digestPath(draft.runtime?.shellPath, 'installed shell')
  const cutd = digestPath(draft.runtime?.cutdPath, 'installed cutd')
  const integrity = runtimeIntegrity(draft.runtime?.integrity, { contract, sourceHead, shell, cutd })
  const video = digestPath(draft.media?.videoPath, 'dropped video')
  const image = digestPath(draft.media?.imagePath, 'dropped image')
  validateProjectsFirst(draft.projectsFirst)
  const projects = Array.isArray(draft.projects) ? draft.projects : []
  if (projects.length !== 2) fail('input requires exactly one video and one image project')
  const videoProject = projectClaim(projects.find((entry) => entry?.kind === 'video'), {
    kind: 'video', media: video, contract,
  })
  const imageProject = projectClaim(projects.find((entry) => entry?.kind === 'image'), {
    kind: 'image', media: image, contract,
  })
  return {
    schema: draft.schemaVersion,
    generatedAt,
    ok: true,
    installedApp: true,
    platform: contract.platform,
    gesture: contract.gesture,
    source: { head: sourceHead },
    runtime: {
      shell,
      cutd,
      integrity,
    },
    media: { video, image },
    checks: [
      { id: 'projects-first', pass: true },
      ...contract.checks.slice(1).map((id) => ({ id, pass: true })),
    ],
    projects: [videoProject, imageProject],
  }
}

export function parseRealFileDropArgs(argv) {
  const out = { inputPath: '', outPath: '', help: false }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--input') out.inputPath = argv[++index] || ''
    else if (arg === '--out') out.outPath = argv[++index] || ''
    else if (arg === '--help' || arg === '-h') out.help = true
    else fail(`unknown argument '${arg}'`)
  }
  return out
}

export function writeRealFileDropReceipt({ inputPath, outPath, generatedAt }) {
  if (!inputPath || !outPath) fail('--input and --out are required')
  const input = JSON.parse(readFileSync(resolve(inputPath), 'utf8'))
  const out = resolve(outPath)
  const receipt = createRealFileDropReceipt(input, { generatedAt })
  mkdirSync(dirname(out), { recursive: true })
  writeFileSync(out, `${JSON.stringify(receipt, null, 2)}\n`, { encoding: 'utf8', flag: 'wx' })
  return { receipt, outPath: out }
}
