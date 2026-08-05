import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdtemp, readFile, rm, stat } from 'node:fs/promises'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import test from 'node:test'

const RUNNER = resolve('app/perception/py/dub_runner.py')

function runPython(input) {
  return new Promise((resolveRun, reject) => {
    const child = spawn('python3', [RUNNER], { stdio: ['pipe', 'pipe', 'pipe'] })
    const stdout = []
    const stderr = []
    child.stdout.on('data', (chunk) => stdout.push(chunk))
    child.stderr.on('data', (chunk) => stderr.push(chunk))
    child.on('error', reject)
    child.on('exit', (code) => resolveRun({
      code,
      stdout: Buffer.concat(stdout).toString('utf8'),
      stderr: Buffer.concat(stderr).toString('utf8'),
    }))
    child.stdin.end(JSON.stringify(input))
  })
}

test('dub runner retries one successful empty OmniVoice stream', async () => {
  let calls = 0
  const server = createServer((request, response) => {
    if (request.url !== '/synthesize') {
      response.writeHead(404).end()
      return
    }
    calls += 1
    response.writeHead(200, { 'content-type': 'audio/pcm' })
    response.end(calls === 1 ? Buffer.alloc(0) : Buffer.alloc(4_800, 1))
  })
  await new Promise((resolveListen) => server.listen(0, '127.0.0.1', resolveListen))
  const dir = await mkdtemp(join(tmpdir(), 'shellx-cut-dub-retry-'))
  try {
    const address = server.address()
    const outWav = join(dir, 'dub.wav')
    const result = await runPython({
      endpoint: `http://127.0.0.1:${address.port}`,
      voice: 'fixture',
      sample_rate: 24_000,
      out_wav: outWav,
      segments: [{ i: 0, start_ms: 0, slot_ms: 1_000, text: 'Sveiki' }],
    })
    assert.equal(result.code, 0, result.stderr)
    assert.equal(calls, 2)
    const receipt = JSON.parse(result.stdout)
    assert.equal(receipt.segments[0].synth_attempts, 2)
    assert.ok((await stat(outWav)).size > 44)
    assert.equal((await readFile(outWav)).subarray(0, 4).toString('ascii'), 'RIFF')
  } finally {
    server.close()
    await rm(dir, { recursive: true, force: true })
  }
})
