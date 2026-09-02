import { randomBytes } from 'node:crypto'
import { spawn, spawnSync } from 'node:child_process'
import {
  WINDOWS_AI_SERVICE_FIXTURE_DIARIZE_MODEL,
  WINDOWS_AI_SERVICE_FIXTURE_DUB_MODEL,
  WINDOWS_AI_SERVICE_FIXTURE_LIMITATION,
  WINDOWS_AI_SERVICE_FIXTURE_PROTOCOL,
  WINDOWS_AI_SERVICE_FIXTURE_PROVENANCE,
  wavDurationMs,
} from './windows-ai-service-fixture-protocol.mjs'
import {
  createAiServiceFixtureServers,
  deterministicPcm,
  deterministicTurns,
} from './windows-ai-service-fixture-server.mjs'

export {
  WINDOWS_AI_SERVICE_FIXTURE_DIARIZE_MODEL,
  WINDOWS_AI_SERVICE_FIXTURE_DUB_MODEL,
  WINDOWS_AI_SERVICE_FIXTURE_LIMITATION,
  WINDOWS_AI_SERVICE_FIXTURE_PROTOCOL,
  WINDOWS_AI_SERVICE_FIXTURE_PROVENANCE,
  wavDurationMs,
  createAiServiceFixtureServers,
  deterministicPcm,
  deterministicTurns,
}

const READY_MARKER = 'SHELLX_CUT_SERVICE_FIXTURES_READY '
const SUMMARY_MARKER = 'SHELLX_CUT_SERVICE_FIXTURES_SUMMARY '

function invariant(condition, message) {
  if (!condition) throw new Error(message)
}

function counters() {
  return {
    diarize: { health: 0, request: 0, rejected: 0, control: 0 },
    dub: { health: 0, request: 0, rejected: 0, control: 0 },
  }
}

function snapshotCounters(value) {
  return JSON.parse(JSON.stringify(value))
}

function parseFixtureManifest(value) {
  invariant(value?.protocol === WINDOWS_AI_SERVICE_FIXTURE_PROTOCOL, 'fixture host returned an unknown protocol')
  invariant(Number.isInteger(value?.pid) && value.pid > 0, 'fixture host returned an invalid PID')
  for (const name of ['diarize', 'dub']) {
    const endpoint = value?.endpoints?.[name]
    const url = new URL(String(endpoint || ''))
    invariant(
      url.protocol === 'http:'
        && url.hostname === '127.0.0.1'
        && url.pathname === '/'
        && !url.search
        && !url.hash
        && Number.isInteger(Number(url.port))
        && Number(url.port) > 0
        && Number(url.port) <= 65_535,
      'fixture host returned a non-loopback endpoint',
    )
  }
  invariant(value.endpoints?.diarize !== value.endpoints?.dub, 'fixture host reused one listener for both services')
  return {
    protocol: value.protocol,
    pid: value.pid,
    endpoints: value.endpoints,
    requestCounters: snapshotCounters(value.requestCounters || counters()),
  }
}

function hostEvents(child) {
  let resolveExit
  const exit = new Promise((resolve) => { resolveExit = resolve })
  let resolveReady
  let rejectReady
  const ready = new Promise((resolve, reject) => {
    resolveReady = resolve
    rejectReady = reject
  })
  const state = { summary: null, buffered: '', settled: false }
  child.stdout.on('data', (chunk) => {
    state.buffered = `${state.buffered}${chunk}`.slice(-65_536)
    const lines = state.buffered.split(/\r?\n/)
    state.buffered = lines.pop()
    for (const line of lines) {
      try {
        if (line.startsWith(READY_MARKER) && !state.settled) {
          state.settled = true
          resolveReady(parseFixtureManifest(JSON.parse(line.slice(READY_MARKER.length))))
        }
        if (line.startsWith(SUMMARY_MARKER)) {
          state.summary = parseFixtureManifest(JSON.parse(line.slice(SUMMARY_MARKER.length)))
        }
      } catch {
        if (!state.settled) {
          state.settled = true
          rejectReady(new Error('fixture host returned an invalid ready manifest'))
        }
      }
    }
  })
  child.once('error', () => {
    if (!state.settled) {
      state.settled = true
      rejectReady(new Error('fixture host failed before ready'))
    }
  })
  child.once('exit', () => {
    if (!state.settled) {
      state.settled = true
      rejectReady(new Error('fixture host exited before ready'))
    }
  })
  child.once('close', () => resolveExit())
  return { ready, exit, state }
}

function timeout(promise, milliseconds, message) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(message)), milliseconds)
    promise.then(
      (value) => { clearTimeout(timer); resolve(value) },
      (error) => { clearTimeout(timer); reject(error) },
    )
  })
}

async function runHostCommand({ nodeExecutable, hostScript, args, cwd }) {
  const child = spawn(nodeExecutable, [hostScript, ...args], {
    cwd,
    stdio: 'ignore',
    windowsHide: true,
  })
  await timeout(new Promise((resolve, reject) => {
    child.once('error', () => reject(new Error('fixture control command failed')))
    child.once('exit', (code) => {
      if (code === 0) resolve()
      else reject(new Error('fixture control command failed'))
    })
  }), 8_000, 'fixture control command timed out')
}

function stopExactWindowsFixturePid({ pid, runToken, cwd }) {
  const command = [
    `$pidValue=${Number(pid)}; $token='${runToken}';`,
    '$process=Get-CimInstance Win32_Process -Filter "ProcessId = $pidValue" -ErrorAction SilentlyContinue;',
    'if ($null -eq $process) { exit 0 }',
    "if ($process.Name -notmatch '^node(?:\\.exe)?$' -or $process.CommandLine -notlike '*ai-service-fixture-host.mjs*' -or $process.CommandLine -notlike \"*--run-token $token*\") { throw 'refusing to terminate a process not owned by this fixture run' }",
    'Stop-Process -Id $pidValue -Force -ErrorAction Stop; Wait-Process -Id $pidValue -Timeout 5 -ErrorAction SilentlyContinue; exit 0',
  ].join(' ')
  const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', command], {
    cwd,
    encoding: 'utf8',
  })
  if (result.status !== 0) throw new Error(`failed to terminate owned fixture PID ${pid}`)
}

export async function startWindowsAiServiceFixtures({ nodeExecutable, hostScript, scriptHash, cwd }) {
  invariant(
    nodeExecutable && hostScript && /^[a-f0-9]{64}$/.test(String(scriptHash || '')),
    'fixture host launch requires a native Node executable, script, and SHA-256',
  )
  const runToken = randomBytes(16).toString('hex')
  const child = spawn(nodeExecutable, [hostScript, '--run-token', runToken], {
    cwd,
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  })
  const events = hostEvents(child)
  try {
    const manifest = await timeout(
      events.ready,
      15_000,
      'fixture host did not emit a ready manifest within 15 seconds',
    )
    return {
      ...manifest,
      child,
      events,
      nodeExecutable,
      hostScript,
      runToken,
      script: {
        file: 'scripts/windows/ai-service-fixture-host.mjs',
        sha256: scriptHash,
      },
      cleanup: { state: 'running', method: null },
    }
  } catch (error) {
    child.kill('SIGTERM')
    throw error
  }
}

export async function stopWindowsAiServiceFixtures(session, { cwd }) {
  if (!session || session.cleanup?.state === 'stopped') return session
  let method = 'graceful-loopback'
  try {
    await runHostCommand({
      nodeExecutable: session.nodeExecutable,
      hostScript: session.hostScript,
      args: [
        '--shutdown',
        '--endpoint',
        session.endpoints.diarize,
        '--run-token',
        session.runToken,
      ],
      cwd,
    })
    await timeout(session.events.exit, 8_000, 'fixture host did not exit after graceful shutdown')
  } catch {
    method = 'exact-pid-fallback'
    stopExactWindowsFixturePid({ pid: session.pid, runToken: session.runToken, cwd })
    await timeout(session.events.exit, 8_000, 'fixture host did not exit after exact PID termination')
  }
  const summary = session.events.state.summary
  session.requestCounters = summary?.requestCounters || session.requestCounters
  session.cleanup = { state: 'stopped', method, countersComplete: Boolean(summary) }
  return session
}

export function serviceFixtureReceipt(session) {
  if (!session) return null
  return {
    mode: 'candidate-only',
    protocol: WINDOWS_AI_SERVICE_FIXTURE_PROTOCOL,
    script: {
      file: session.script?.file || 'scripts/windows/ai-service-fixture-host.mjs',
      sha256: session.script?.sha256 || '',
    },
    endpoints: {
      diarize: session.endpoints?.diarize || '',
      dub: session.endpoints?.dub || '',
    },
    pid: session.pid || null,
    requestCounters: snapshotCounters(session.requestCounters || counters()),
    cleanup: session.cleanup || { state: 'unknown', method: null, countersComplete: false },
    limitation: WINDOWS_AI_SERVICE_FIXTURE_LIMITATION,
  }
}

export function assertCandidateServiceFixturesRequest({
  enabled,
  signedFinal,
  diarizeEndpoint = '',
  dubEndpoint = '',
  env = process.env,
} = {}) {
  if (!enabled) return
  invariant(!signedFinal, '--service-fixtures is candidate-only and forbidden with --signed-final')
  invariant(
    !String(diarizeEndpoint).trim() && !String(dubEndpoint).trim(),
    '--service-fixtures refuses explicit live CUT_DIARIZE_ENDPOINT or CUT_DUB_ENDPOINT',
  )
  const secretNames = Object.keys(env).filter((name) => {
    const pattern = /^(?:CUT_(?:DIARIZE|DUB)|(?:DIARIZE|DUB|OMNIVOICE|SORTFORMER))_[A-Z0-9_]*(?:SECRET|TOKEN|KEY|AUTH(?:ORIZATION)?|PASSWORD)[A-Z0-9_]*$/i
    return pattern.test(name) && env[name]
  })
  invariant(
    secretNames.length === 0,
    `--service-fixtures refuses service-secret environment variables: ${secretNames.join(', ')}`,
  )
}

export async function runAiServiceFixtureHost({
  runToken,
  emit = (line) => process.stdout.write(`${line}\n`),
} = {}) {
  const fixture = await createAiServiceFixtureServers({ runToken })
  emit(`${READY_MARKER}${JSON.stringify(fixture.manifest())}`)
  const stop = async () => {
    await fixture.close()
    emit(`${SUMMARY_MARKER}${JSON.stringify(fixture.manifest())}`)
  }
  await Promise.race([
    fixture.shutdownRequested,
    new Promise((resolve) => {
      process.once('SIGINT', resolve)
      process.once('SIGTERM', resolve)
    }),
  ])
  await stop()
}
