import { createConnection } from 'node:net'
import { readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

import { AGENT_DOCS, verifyAgentDocsApi } from './agent-docs.mjs'
import { sourceContentManifest } from './source-content-manifest.mjs'

export const INSTALLED_RUNTIME_SCHEMA = 'shellx-cut/installed-runtime-walkthrough@1'
export const WEBDRIVER_TEST_BUILD_MARKER = 'shellx-cut/webdriver-test-enabled@1'
export const SURFACE_PLATFORMS = {
  'windows-installed': 'win32',
  'macos-installed': 'darwin',
  'linux-control': 'linux',
}
export const REQUIRED_OPEN_SURFACES = ['settings-agent-control', 'library', 'settings-about']

const SHA256_RX = /^[a-f0-9]{64}$/
const COMMIT_RX = /^[a-f0-9]{40}$/

function assert(condition, message) {
  if (!condition) throw new Error(message)
}

function loopbackBase(value) {
  const url = new URL(value)
  assert(url.protocol === 'http:', 'installed runtime evidence requires HTTP')
  assert(new Set(['127.0.0.1', 'localhost', '[::1]']).has(url.hostname),
    `installed runtime evidence requires loopback, got ${url.origin}`)
  return url.origin
}

async function getJson(url, timeoutMs) {
  const response = await fetch(url, {
    headers: { connection: 'close' },
    signal: AbortSignal.timeout(timeoutMs),
  })
  if (!response.ok) throw new Error(`${url} returned ${response.status}`)
  return response.json()
}

async function callVerb(engineBase, name, args, timeoutMs) {
  const response = await fetch(`${engineBase}/api/verb/${encodeURIComponent(name)}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', connection: 'close' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(timeoutMs),
  })
  const body = await response.json()
  if (!response.ok || body?.ok !== true) {
    throw new Error(`${name} failed: ${body?.error?.code || response.status} ${body?.error?.message || ''}`.trim())
  }
  return body
}

function verbNames(registry) {
  const entries = Array.isArray(registry) ? registry : registry?.verbs
  assert(Array.isArray(entries), 'installed /api/verbs response is not a verb registry')
  return entries.map((entry) => typeof entry === 'string' ? entry : entry?.name).filter(Boolean).sort()
}

function schemaVerbNames(repoRoot) {
  const schema = JSON.parse(readFileSync(join(repoRoot, 'schema/verbs.json'), 'utf8'))
  return schema.verbs.map((entry) => entry.name).sort()
}

function validateMcpResult(value) {
  return value?.schema === 'shellx-cut/mcp-self-test/1'
    && value.mode === 'proxy'
    && value.read_only === true
    && value.ping === true
    && value.same_engine === true
    && Number.isInteger(value.tools)
    && value.tools > 0
    && value.tools === value.expected_tools
    && Number.isInteger(value.tools_list_bytes)
    && value.tools_list_bytes > 0
    && value.tools_list_bytes <= value.tools_list_max_bytes
    && value.command?.[0] === value.executable
    && value.command?.[1] === 'mcp'
    && typeof value.protocol_version === 'string'
    && value.protocol_version.length > 0
    && typeof value.proxy_addr === 'string'
    && value.proxy_addr.length > 0
}

export function probeClosedPort(port = 4445, timeoutMs = 1500) {
  return new Promise((resolveProbe) => {
    let settled = false
    const finish = (result) => {
      if (settled) return
      settled = true
      socket.destroy()
      resolveProbe(result)
    }
    const socket = createConnection({ host: '127.0.0.1', port })
    socket.once('connect', () => finish({ port, closed: false, outcome: 'connected' }))
    socket.once('error', (error) => finish({
      port,
      closed: error?.code === 'ECONNREFUSED',
      outcome: error?.code || 'error',
    }))
    socket.setTimeout(timeoutMs, () => finish({ port, closed: false, outcome: 'timeout' }))
  })
}

async function openSurface(engineBase, panel, timeoutMs, onSurfaceOpened) {
  const before = await callVerb(engineBase, 'ui.state', {}, timeoutMs)
  if (before.result?.open_surface_ids?.includes(panel)) {
    const detour = REQUIRED_OPEN_SURFACES.find((candidate) =>
      candidate !== panel && !before.result.open_surface_ids.includes(candidate)) || 'projects'
    await callVerb(engineBase, 'ui.open', { panel: detour }, timeoutMs)
  }
  const opened = await callVerb(engineBase, 'ui.open', { panel }, timeoutMs)
  const state = opened.result?.state
  assert(opened.result?.applied === true, `ui.open did not apply ${panel}`)
  assert(opened.result?.surface === panel, `ui.open returned the wrong surface for ${panel}`)
  assert(state?.schema === 'shellx-cut/ui-state/2', `ui.open ${panel} returned invalid UI state`)
  assert(state.open_surface_ids?.includes(panel), `ui.open ${panel} did not become observable`)
  const visual = onSurfaceOpened ? await onSurfaceOpened(panel, opened.result) : null
  if (onSurfaceOpened) assert(visual?.ok === true, `visual evidence failed for ${panel}`)
  return {
    panel,
    applied: true,
    selector: opened.result.selector,
    stateRevision: state.state_revision,
    visual,
  }
}

export async function collectInstalledRuntimeEvidence({
  engineBase,
  installedAppPath = '',
  nativeProvider = '',
  repoRoot,
  surface,
  source,
  timeoutMs = 20_000,
  onSurfaceOpened,
}) {
  const root = resolve(repoRoot)
  const engine = loopbackBase(engineBase)
  const expectedPlatform = SURFACE_PLATFORMS[surface]
  assert(expectedPlatform, `unknown installed surface: ${surface}`)
  assert(source?.platform === expectedPlatform,
    `runtime platform ${source?.platform || 'missing'} does not match ${surface}`)
  assert(COMMIT_RX.test(String(source?.gitCommit || '')), 'source git commit is invalid')
  assert(/^\d+\.\d+\.\d+/.test(String(source?.version || '')), 'source version is invalid')
  assert(SHA256_RX.test(String(source?.contentManifestSha256 || '')), 'source content digest is invalid')

  const currentVersion = JSON.parse(readFileSync(
    join(root, 'app/desktop/src-tauri/tauri.conf.json'), 'utf8',
  )).version
  assert(currentVersion === source.version, 'runtime source version does not match synchronized source')
  assert(sourceContentManifest(root).sha256 === source.contentManifestSha256,
    'runtime source content digest does not match synchronized source')

  const registry = await getJson(`${engine}/api/verbs`, timeoutMs)
  const installedVerbs = verbNames(registry)
  const expectedVerbs = schemaVerbNames(root)
  assert(JSON.stringify(installedVerbs) === JSON.stringify(expectedVerbs),
    `installed verb registry differs from synchronized schema (${installedVerbs.length}/${expectedVerbs.length})`)

  const initialState = await callVerb(engine, 'ui.state', {}, timeoutMs)
  assert(initialState.result?.schema === 'shellx-cut/ui-state/2', 'installed ui.state schema is invalid')
  assert(initialState.result?.connected === true && initialState.result?.ui_clients > 0,
    'installed ui.state has no connected UI client')

  const opens = []
  for (const panel of REQUIRED_OPEN_SURFACES) {
    opens.push(await openSurface(engine, panel, timeoutMs, onSurfaceOpened))
  }

  const agentDocs = await verifyAgentDocsApi({
    engineBase: engine,
    sourceRoot: root,
    expectedVersion: source.version,
    timeoutMs,
  })
  assert(agentDocs.ok && agentDocs.checked === AGENT_DOCS.length && agentDocs.served === AGENT_DOCS.length,
    `installed agent docs failed: ${agentDocs.failures.join('; ')}`)

  const mcpEnvelope = await callVerb(engine, 'system.mcp_test', {}, Math.max(timeoutMs, 30_000))
  assert(validateMcpResult(mcpEnvelope.result), 'installed MCP self-test result is incomplete')
  const webdriverTestPort = await probeClosedPort()
  let webdriverTestFeature
  if (nativeProvider === 'external') {
    assert(surface === 'linux-control', 'external installed WebDriver proof is Linux-only')
    assert(installedAppPath, 'external installed WebDriver proof requires the exact app binary')
    const binary = readFileSync(resolve(installedAppPath))
    const markerAbsent = !binary.includes(Buffer.from(WEBDRIVER_TEST_BUILD_MARKER))
    assert(markerAbsent, 'shipping app contains the WebDriver test build marker')
    webdriverTestFeature = {
      absent: true,
      proof: 'binary-marker-absent',
      marker: WEBDRIVER_TEST_BUILD_MARKER,
      bytesChecked: binary.length,
    }
  } else {
    assert(webdriverTestPort.closed === true,
      `shipping app exposes the embedded WebDriver test port (${webdriverTestPort.outcome})`)
    webdriverTestFeature = {
      absent: true,
      proof: 'closed-port',
      marker: WEBDRIVER_TEST_BUILD_MARKER,
    }
  }

  return {
    schema: INSTALLED_RUNTIME_SCHEMA,
    generatedAt: new Date().toISOString(),
    status: 'pass',
    installedApp: true,
    surface,
    source: {
      gitCommit: source.gitCommit,
      version: source.version,
      contentManifestSha256: source.contentManifestSha256,
    },
    host: { platform: source.platform, arch: source.arch || process.arch },
    agentDocs: {
      schema: agentDocs.schema,
      version: agentDocs.version,
      checked: agentDocs.checked,
      served: agentDocs.served,
      failures: [],
    },
    debugApi: {
      registryVerbs: installedVerbs.length,
      expectedVerbs: expectedVerbs.length,
      uiStateSchema: initialState.result.schema,
      uiClients: initialState.result.ui_clients,
      opens,
    },
    mcp: {
      schema: mcpEnvelope.result.schema,
      mode: mcpEnvelope.result.mode,
      readOnly: mcpEnvelope.result.read_only,
      ping: mcpEnvelope.result.ping,
      sameEngine: mcpEnvelope.result.same_engine,
      tools: mcpEnvelope.result.tools,
      expectedTools: mcpEnvelope.result.expected_tools,
      protocolVersion: mcpEnvelope.result.protocol_version,
    },
    webdriverTestFeature,
    webdriverTestPort,
  }
}
