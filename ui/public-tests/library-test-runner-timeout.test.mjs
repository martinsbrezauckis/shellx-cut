import assert from 'node:assert/strict'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import { DEFAULT_TEST_FILE_TIMEOUT_MS, runTestFile, testFileFailure, testFileTimeoutMs } from './lib/runLibraryTests.mjs'

const alive = (pid) => {
  try { process.kill(pid, 0); return true } catch { return false }
}

test('a hung test file ends at its time limit, with its child processes', { timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'cut-test-lib-timeout-'))
  try {
    // The fixture leaves a child process running and never exits by itself,
    // like a test that failed early and left a dev server or browser open.
    await writeFile(join(dir, 'hang.mjs'), `
      import { spawn } from 'node:child_process'
      import { writeFileSync } from 'node:fs'
      const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore' })
      writeFileSync(${JSON.stringify(join(dir, 'child.pid'))}, String(child.pid))
      setInterval(() => {}, 1000)
    `)
    const result = await runTestFile({ args: [join(dir, 'hang.mjs')], cwd: dir, timeoutMs: 1500 })
    assert.equal(result.timedOut, true)
    assert.ok(result.ms < 15_000, `ended ${result.ms} ms after start`)
    assert.match(testFileFailure(result, 1500), /^timed out after 2 s/)
    const grandchild = Number(await readFile(join(dir, 'child.pid'), 'utf8'))
    const deadline = Date.now() + 5000
    while (alive(grandchild) && Date.now() < deadline) await new Promise((r) => setTimeout(r, 50))
    assert.equal(alive(grandchild), false, 'the hung file\'s own child process was stopped too')
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('a finished test file reports its own exit status', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'cut-test-lib-exit-'))
  try {
    await writeFile(join(dir, 'fail.mjs'), 'process.exitCode = 3\n')
    await writeFile(join(dir, 'pass.mjs'), 'console.log("ok")\n')
    const failed = await runTestFile({ args: [join(dir, 'fail.mjs')], cwd: dir, timeoutMs: 30_000 })
    assert.equal(failed.timedOut, false)
    assert.equal(testFileFailure(failed, 30_000), 'exited with status 3')
    const passed = await runTestFile({ args: [join(dir, 'pass.mjs')], cwd: dir, timeoutMs: 30_000 })
    assert.equal(testFileFailure(passed, 30_000), null)
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('the per-file limit comes from CUT_TEST_LIB_TIMEOUT_SEC or the default', () => {
  assert.equal(testFileTimeoutMs({}), DEFAULT_TEST_FILE_TIMEOUT_MS)
  assert.equal(testFileTimeoutMs({ CUT_TEST_LIB_TIMEOUT_SEC: '90' }), 90_000)
  assert.throws(() => testFileTimeoutMs({ CUT_TEST_LIB_TIMEOUT_SEC: 'soon' }), /whole number of seconds/)
  assert.throws(() => testFileTimeoutMs({ CUT_TEST_LIB_TIMEOUT_SEC: '0' }), /whole number of seconds/)
})
