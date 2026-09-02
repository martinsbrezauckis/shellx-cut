import test from 'node:test'
import assert from 'node:assert/strict'

import { prepareWindowsInstalledStartupReadiness } from '../lib/windows-installed-startup-readiness.mjs'

function argumentReader(values) {
  return (name, fallback = '') => values[name] ?? fallback
}

test('Windows installed startup evidence carries a pre-launch t0 and only explicit budgets', () => {
  const prepared = prepareWindowsInstalledStartupReadiness({
    arg: argumentReader({
      '--startup-listener-budget-ms': '1000',
      '--startup-api-budget-ms': '1200',
      '--startup-ui-client-budget-ms': '1500',
      '--startup-dom-budget-ms': '2000',
      '--startup-slow-ffmpeg-marker': '~/fixtures/slow-ffmpeg.json',
    }),
    out: '/control/run',
    windowsPath: (path) => `WIN:${path}`,
    expandHome: (path) => path.replace('~', '/home/tester'),
    t0EpochMs: 123_456,
  })

  assert.equal(prepared.receiptPath, '/control/run/installed-startup-readiness.json')
  assert.deepEqual(prepared.probeArgs, [
    '--startup-out', 'WIN:/control/run/installed-startup-readiness.json',
    '--startup-t0-epoch-ms', '123456',
    '--startup-listener-budget-ms', '1000',
    '--startup-api-budget-ms', '1200',
    '--startup-ui-client-budget-ms', '1500',
    '--startup-dom-budget-ms', '2000',
    '--startup-slow-ffmpeg-marker', 'WIN:/home/tester/fixtures/slow-ffmpeg.json',
  ])
})

test('Windows installed startup evidence leaves budget judgment unconfigured by default', () => {
  const prepared = prepareWindowsInstalledStartupReadiness({
    arg: argumentReader({}),
    out: '/control/run',
    windowsPath: (path) => path,
    expandHome: (path) => path,
    t0EpochMs: 1,
  })
  assert.deepEqual(prepared.probeArgs, [
    '--startup-out', '/control/run/installed-startup-readiness.json',
    '--startup-t0-epoch-ms', '1',
  ])
})
