#!/usr/bin/env node
// dual-surface-job-gate.mjs — the stack-launching wrapper for the dual-surface
// real-job harness (ui/public-tests/dual-surface-job-verify.mjs).
//
// ROLE (mirrors full-coverage-gate.mjs, scaled to this job's footprint):
//   1. Cold-start TWO isolated cutd instances — one per surface — each with its
//      own free loopback port, SHELLX_CUT_HOME and SHELLX_CUT_PROJECTS_DIR
//      (the harness doctrine demands FRESH instances per mode; project.create
//      is sent name-only like the UI, so isolation must come from the process
//      env, not injected args). The Mode B instance serves the built ui/dist
//      on the same origin (production topology).
//   2. OR drive EXTERNAL stacks when SWEEP_CUTD_A / SWEEP_CUTD_B (or SWEEP_CUTD
//      for both) are set — then nothing is cold-started or torn down here, and
//      the rig launcher owns isolation (acknowledged via DSJ_EXTERNAL_ISOLATED,
//      which this wrapper only sets itself for stacks IT started).
//   3. Run the dual runner, surface its verdict, write a gate receipt, and tear
//      cold-started instances down BY EXACT process-group pid (never pkill).
//
// WSL: cold-start is REFUSED by default for consistency with the release-gate
// doctrine (rigs are the home for stack-launching gates). This job is far
// lighter than full-coverage — 2 synthetic silent 720p clips, one ~14s render,
// no torch — so DSJ_ALLOW_WSL=1 is a reasonable dev override here, and Mode B
// runs HEADLESS chromium (no X display / xvfb needed).
//
// USAGE
//   node scripts/release/dual-surface-job-gate.mjs               # cold-start
//   SWEEP_CUTD=http://127.0.0.1:6161 DSJ_EXTERNAL_ISOLATED=1 \
//     node scripts/release/dual-surface-job-gate.mjs             # external
// Flags / env passed through to the runner: DSJ_MODE, DSJ_BREAK, DSJ_RECEIPT,
//   DSJ_MEDIA_DIR, DSJ_TOLERANCE_MS, DSJ_REQUIRE_UI.
//   --build / DSJ_BUILD=1  rebuild ui/dist before serving it (Mode B needs a
//   current dist; a stale one tests yesterday's UI against today's engine).
//   CUTD_BIN=<path>        cutd binary (default app/target/release/cutd, falls
//                          back to app/target/debug/cutd with a warning).
// Exit codes: propagated from the runner (0 convergent / 1 divergent or step
// failure / 3 preflight error / 4 Mode B skipped) — plus 2/3 for gate errors.
//
// Callers: rig qualification runs, developers. Dependencies: node 18+, the
// cutd binary, a built ui/dist, Playwright under ui/, ffmpeg on PATH.

import { spawn, spawnSync } from 'node:child_process'
import { createServer } from 'node:net'
import { existsSync, mkdirSync, openSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = resolve(HERE, '..', '..')
const UI_DIR = join(REPO, 'ui')
const UI_DIST = join(UI_DIR, 'dist')
const args = process.argv.slice(2)

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const log = (...m) => console.log('[dsj-gate]', ...m)
const warn = (...m) => console.warn('[dsj-gate] ⚠', ...m)
const die = (msg, code = 3) => { console.error(`[dsj-gate] ✗ ${msg}`); process.exit(code) }

function isWslHost() {
  if (process.platform !== 'linux') return false
  try { if (/microsoft|wsl/i.test(readFileSync('/proc/version', 'utf8'))) return true } catch { /* fall through */ }
  return Boolean(process.env.WSL_INTEROP) || existsSync('/run/WSL')
}

function resolveCutd() {
  if (process.env.CUTD_BIN) return process.env.CUTD_BIN
  const rel = join(REPO, 'app', 'target', 'release', 'cutd')
  const dbg = join(REPO, 'app', 'target', 'debug', 'cutd')
  if (existsSync(rel)) return rel
  if (existsSync(dbg)) {
    warn(`release cutd missing — using the DEBUG binary (${dbg}); build the release one with: cd app && cargo build --release -p server`)
    return dbg
  }
  return rel
}

function freePort() {
  return new Promise((res, rej) => {
    const srv = createServer()
    srv.once('error', rej)
    srv.listen(0, '127.0.0.1', () => { const { port } = srv.address(); srv.close(() => res(port)) })
  })
}

async function waitForCutd(base, deadlineMs, child) {
  const deadline = Date.now() + deadlineMs
  while (Date.now() < deadline) {
    if (child && child.exitCode !== null) throw new Error(`cutd exited during startup (code ${child.exitCode})`)
    try {
      const r = await fetch(`${base}/api/verbs`, { signal: AbortSignal.timeout(2000) })
      if (r.ok) return true
    } catch { /* not up yet */ }
    await sleep(400)
  }
  return false
}

async function main() {
  const startedAt = new Date().toISOString()
  const stem = startedAt.replace(/[:.]/g, '-')
  const scratch = join(REPO, '.shellx-scratch', 'dual-surface', stem)
  const external = Boolean(process.env.SWEEP_CUTD || process.env.SWEEP_CUTD_A || process.env.SWEEP_CUTD_B)
  const wantA = (process.env.DSJ_MODE || 'both') !== 'b'
  const wantB = (process.env.DSJ_MODE || 'both') !== 'a'
  log(`mode=${external ? 'external-stack' : 'cold-start'} surfaces=${process.env.DSJ_MODE || 'both'} repo=${REPO}`)

  const env = { ...process.env }
  const children = [] // [{ pid, label }]
  let cleaned = false
  const cleanup = () => {
    if (cleaned) return
    cleaned = true
    for (const { pid, label } of children) {
      try { process.kill(-pid, 'SIGTERM') } catch { continue }
      const t0 = Date.now()
      let gone = false
      while (Date.now() - t0 < 4000) {
        try { process.kill(-pid, 0) } catch { gone = true; break }
        const s = spawnSync('sleep', ['0.2'])
        if (s.error) break
      }
      if (!gone) { try { process.kill(-pid, 'SIGKILL') } catch { /* gone */ } }
      log(`stopped ${label} (pgid ${pid})`)
    }
  }
  process.on('exit', cleanup)
  for (const sig of ['SIGINT', 'SIGTERM', 'SIGHUP']) process.on(sig, () => { cleanup(); process.exit(130) })

  if (external) {
    // The rig launcher owns the stacks AND their isolation acknowledgment —
    // never assert on its behalf what this wrapper cannot see.
    if (env.DSJ_EXTERNAL_ISOLATED !== '1') {
      die('external stacks require DSJ_EXTERNAL_ISOLATED=1 — confirm the launcher started cutd with an isolated SHELLX_CUT_HOME + SHELLX_CUT_PROJECTS_DIR')
    }
  } else {
    if (isWslHost() && env.DSJ_ALLOW_WSL !== '1') {
      die('REFUSING to cold-start on WSL by default (release-gate doctrine: stack-launching gates live on rigs). This job IS light — 2 synthetic clips, one short render — so DSJ_ALLOW_WSL=1 is a reasonable dev override.', 2)
    }
    const cutdBin = resolveCutd()
    if (!existsSync(cutdBin)) die(`cutd binary not found at ${cutdBin} — build it: cd app && cargo build --release -p server (or set CUTD_BIN)`)
    if (args.includes('--build') || env.DSJ_BUILD === '1') {
      log('building ui/dist (cd ui && npm run build)…')
      const r = spawnSync('npm', ['run', 'build'], { cwd: UI_DIR, stdio: 'inherit' })
      if (r.status !== 0) die('ui build failed')
    }
    if (wantB && !existsSync(join(UI_DIST, 'index.html'))) {
      die('ui/dist missing — Mode B serves the built UI from cutd; build it first (cd ui && npm run build, or pass --build)')
    }

    // One FRESH instance per surface: own port, own home, own projects dir.
    const bootInstance = async (label) => {
      const port = await freePort()
      const base = `http://127.0.0.1:${port}`
      const home = join(scratch, label, 'home')
      const projects = join(scratch, label, 'projects')
      mkdirSync(home, { recursive: true })
      mkdirSync(projects, { recursive: true })
      const logPath = join(scratch, label, 'cutd.log')
      const fd = openSync(logPath, 'a')
      const child = spawn(cutdBin, ['serve', '--addr', `127.0.0.1:${port}`, '--ui-dist', UI_DIST], {
        cwd: REPO,
        detached: true, // own process group → exact-pgid teardown
        stdio: ['ignore', fd, fd],
        env: { ...process.env, SHELLX_CUT_HOME: home, SHELLX_CUT_PROJECTS_DIR: projects },
      })
      children.push({ pid: child.pid, label: `${label} cutd :${port}` })
      log(`${label}: cutd pid=${child.pid} on ${base} (home=${home}) log=${logPath}`)
      const up = await waitForCutd(base, 60_000, child)
      if (!up) {
        try { console.error(readFileSync(logPath, 'utf8').split('\n').slice(-15).join('\n')) } catch { /* no log */ }
        die(`${label} cutd did not serve /api/verbs on ${base} within 60s`)
      }
      return base
    }
    if (wantA) env.SWEEP_CUTD_A = await bootInstance('mode-a')
    if (wantB) env.SWEEP_CUTD_B = await bootInstance('mode-b')
    env.DSJ_EXTERNAL_ISOLATED = '1' // this wrapper created the isolation above
  }

  env.DSJ_RECEIPT = env.DSJ_RECEIPT || join(scratch, 'receipt.json')
  log('running public-tests/dual-surface-job-verify.mjs …')
  const run = spawnSync('node', ['public-tests/dual-surface-job-verify.mjs'], { cwd: UI_DIR, stdio: 'inherit', env })
  const exitCode = run.status === null ? 2 : run.status
  cleanup()

  const gateReceipt = {
    schema: 'shellx-cut/dual-surface-job-gate@1',
    startedAt,
    endedAt: new Date().toISOString(),
    mode: external ? 'external-stack' : 'cold-start',
    surfaces: process.env.DSJ_MODE || 'both',
    exitCode,
    status: exitCode === 0 ? 'pass' : exitCode === 4 ? 'skip-mode-b' : 'fail',
    runnerReceipt: env.DSJ_RECEIPT,
    ...(process.env.DSJ_BREAK ? { deliberatelyBroken: process.env.DSJ_BREAK } : {}),
  }
  mkdirSync(scratch, { recursive: true })
  writeFileSync(join(scratch, 'gate-receipt.json'), JSON.stringify(gateReceipt, null, 2))
  log(`gate receipt → ${join(scratch, 'gate-receipt.json')}`)
  log(`${gateReceipt.status.toUpperCase()} (exit ${exitCode})`)
  process.exit(exitCode)
}

// Import-safe: only run as a CLI (mirrors the sibling gates).
if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((e) => { console.error('[dsj-gate] ✗ wrapper error:', e); process.exit(2) })
}
