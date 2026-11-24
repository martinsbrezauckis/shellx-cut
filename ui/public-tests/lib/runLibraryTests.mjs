import { spawn, spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { realpathSync } from 'node:fs'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { DEFAULT_PUBLIC_TESTS_ROOT, discoverLibraryTests } from './libraryTestDiscovery.mjs'

/** Default per-file limit for the library suite. The slowest public test file
 * takes well under a minute; ten minutes leaves room for slow CI machines and
 * still turns a hung file into a named failure instead of a stuck job. */
export const DEFAULT_TEST_FILE_TIMEOUT_MS = 10 * 60 * 1000

/** Per-file limit from CUT_TEST_LIB_TIMEOUT_SEC (whole seconds), else the default. */
export function testFileTimeoutMs(env = process.env) {
  const raw = env.CUT_TEST_LIB_TIMEOUT_SEC
  if (raw === undefined || raw === '') return DEFAULT_TEST_FILE_TIMEOUT_MS
  const seconds = Number(raw)
  if (!Number.isInteger(seconds) || seconds < 1) throw new Error(`CUT_TEST_LIB_TIMEOUT_SEC must be a whole number of seconds, got ${JSON.stringify(raw)}`)
  return seconds * 1000
}

/** Stop a child and everything it started (a test file may leave a browser or
 * a dev server running). POSIX: the child leads its own process group, so the
 * whole group is killed. Windows: taskkill ends the process tree. */
function killTree(child) {
  if (!child.pid) return
  if (process.platform === 'win32') {
    spawnSync('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore', windowsHide: true })
    return
  }
  try { process.kill(-child.pid, 'SIGKILL') } catch { /* group already gone */ }
  try { child.kill('SIGKILL') } catch { /* already gone */ }
}

/** Run one test file to completion or until `timeoutMs`, whichever is first.
 * Resolves { status, signal, timedOut, error, ms }; never rejects. Output is
 * inherited so the caller's log shows the file's own report. `abort` (an
 * AbortSignal) stops the file and its children, e.g. when the runner itself is
 * interrupted: the file runs in its own process group, so a terminal Ctrl+C
 * does not reach it directly. */
export function runTestFile({ execPath = process.execPath, args, cwd, env = process.env, timeoutMs, abort }) {
  const started = Date.now()
  return new Promise((resolve) => {
    let settled = false
    let timedOut = false
    const child = spawn(execPath, args, {
      cwd,
      env,
      stdio: 'inherit',
      // Own process group on POSIX so a timeout can stop the file's children too.
      detached: process.platform !== 'win32',
      windowsHide: true,
    })
    const timer = setTimeout(() => {
      timedOut = true
      killTree(child)
    }, timeoutMs)
    const finish = (result) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      abort?.removeEventListener('abort', onAbort)
      // A finished file may still have left children in its group.
      if (timedOut || result.error) killTree(child)
      resolve({ ...result, timedOut, ms: Date.now() - started })
    }
    const onAbort = () => killTree(child)
    abort?.addEventListener('abort', onAbort, { once: true })
    child.once('error', (error) => finish({ status: null, signal: null, error }))
    child.once('exit', (status, signal) => finish({ status, signal, error: null }))
  })
}

/** The failure line for one file, or null when it passed. */
export function testFileFailure(result, timeoutMs) {
  if (result.timedOut) return `timed out after ${Math.round(timeoutMs / 1000)} s (stopped with its child processes)`
  if (result.error) return result.error.message
  if (result.signal) return `terminated by ${result.signal}`
  if (result.status !== 0) return `exited with status ${result.status ?? 1}`
  return null
}

/** Run every discovered library test file in order (the `npm run test:lib` entry point). */
async function main() {
  const here = dirname(fileURLToPath(import.meta.url))
  const uiRoot = resolve(here, '../..')
  const tsxCli = resolve(uiRoot, 'node_modules/tsx/dist/cli.mjs')
  const noBail = process.argv.slice(2).some((arg) => arg === '--no-bail' || arg === '--collect-all')
  const unsupported = process.argv.slice(2).filter((arg) => arg !== '--no-bail' && arg !== '--collect-all')
  if (unsupported.length > 0) throw new Error(`unsupported test:lib argument(s): ${unsupported.join(', ')}`)
  // Every file gets a time limit: a file that hangs (for example a dev server or
  // browser left open after an early failure) ends as a named failure.
  const timeoutMs = testFileTimeoutMs()
  const { included, excluded } = await discoverLibraryTests()
  const failures = []
  const durations = []
  const interrupted = new AbortController()
  for (const signal of ['SIGINT', 'SIGTERM']) {
    process.once(signal, () => {
      interrupted.abort()
      console.error(`[test:lib] interrupted by ${signal}`)
      process.exitCode = 130
    })
  }

  const cspPreloadPath = resolve(here, 'cspPlaywrightPreload.mjs')
  console.log(`[test:lib] discovered ${included.length} eligible tests; ${excluded.length} explicit exclusions; limit ${Math.round(timeoutMs / 1000)} s per file`)
  for (const relativePath of included) {
    if (interrupted.signal.aborted) break
    const testPath = resolve(DEFAULT_PUBLIC_TESTS_ROOT, relativePath)
    const args = ['--import', pathToFileURL(cspPreloadPath).href, ...(relativePath.endsWith('.ts') ? [tsxCli, testPath] : [testPath])]
    console.log(`[test:lib] RUN ${relativePath}`)
    const result = await runTestFile({ args, cwd: uiRoot, env: process.env, timeoutMs, abort: interrupted.signal })
    durations.push({ path: relativePath, ms: result.ms })
    const message = testFileFailure(result, timeoutMs)
    if (message) {
      failures.push({ path: relativePath, message })
      console.error(`[test:lib] FAIL ${relativePath}: ${message}`)
      if (!noBail) break
    }
  }

  const slowest = durations.sort((a, b) => b.ms - a.ms).slice(0, 5)
  console.log(`[test:lib] slowest: ${slowest.map((d) => `${d.path} ${(d.ms / 1000).toFixed(1)} s`).join('; ')}`)
  if (interrupted.signal.aborted) {
    console.error('[test:lib] run interrupted; results are incomplete')
  } else if (failures.length > 0) {
    console.error(`[test:lib] ${failures.length} of ${included.length} tests failed`)
    for (const failure of failures) console.error(`- ${failure.path}: ${failure.message}`)
    process.exitCode = 1
  } else {
    console.log(`[test:lib] PASS ${included.length} tests${noBail ? ' (no-bail)' : ''}`)
  }
}

// Run only as the entry point; tests import the helpers above without running
// the suite. Real paths on both sides, so a symlinked checkout still runs it.
const entry = process.argv[1] ? realpathSync(resolve(process.argv[1])) : ''
if (entry === realpathSync(fileURLToPath(import.meta.url))) await main()
