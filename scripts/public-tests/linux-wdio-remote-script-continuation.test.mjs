import assert from 'node:assert/strict'
import test from 'node:test'

import { renderLinuxWdioRemoteScript } from '../lib/linux-wdio-remote-script.mjs'

test('Linux WDIO environment uses one shell continuation per rendered line', () => {
  const script = renderLinuxWdioRemoteScript({
    testControl: false,
    remoteNode: '/usr/bin/node',
    remoteWorkerReceipt: '/tmp/remote-worker.json',
    dropCase: 'mouse',
    trace: false,
    fcvTraceValue: '0',
  })
  const lines = script.split('\n')
  const first = lines.findIndex((line) => line.startsWith('XDG_RUNTIME_DIR='))
  const last = lines.findIndex((line) => line.startsWith('npm --prefix ui exec'))
  assert.ok(first >= 0 && last > first)
  for (const line of lines.slice(first, last)) {
    assert.ok(line.endsWith(' \\'), 'expected one shell continuation: ' + line)
    assert.ok(!line.endsWith(' \\\\'), 'refused doubled shell continuation: ' + line)
  }
})
