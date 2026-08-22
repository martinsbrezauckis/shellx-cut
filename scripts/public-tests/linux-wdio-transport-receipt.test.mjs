import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

const helper = fileURLToPath(new URL('../lib/linux-wdio-transport-receipt.mjs', import.meta.url))

test('Linux WDIO transport keeps a create-only log and a hash-bound terminal receipt', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-wdio-transport-'))
  t.after(() => rm(root, { recursive: true, force: true }))
  const log = join(root, 'wdio-native.log')

  const create = spawnSync(process.execPath, [helper, '--create-log', log], { encoding: 'utf8' })
  assert.equal(create.status, 0, create.stderr)
  assert.equal((await stat(log)).mode & 0o777, 0o600)
  const duplicate = spawnSync(process.execPath, [helper, '--create-log', log], { encoding: 'utf8' })
  assert.notEqual(duplicate.status, 0, 'existing logs are never overwritten')

  const body = 'checkpoint 033 complete\naudio start\n'
  await writeFile(log, body)
  const summary = spawnSync(process.execPath, [helper, '--summarize', '--log', log, '--exit', '143'], { encoding: 'utf8' })
  assert.equal(summary.status, 0, summary.stderr)
  assert.match(summary.stdout, /WDIO native terminal: exit=143 log=/)
  const receipt = JSON.parse(await readFile(join(root, 'wdio-transport-summary.json'), 'utf8'))
  assert.equal(receipt.exitCode, 143)
  assert.equal(receipt.log.path, log)
  assert.equal(receipt.log.bytes, Buffer.byteLength(body))
  assert.equal(receipt.log.sha256, createHash('sha256').update(body).digest('hex'))
})
