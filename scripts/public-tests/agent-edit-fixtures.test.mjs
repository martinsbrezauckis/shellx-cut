import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createServer } from 'node:http'
import { resolve } from 'node:path'
import test from 'node:test'

const CODEX_FIXTURE = resolve('scripts/release/fixtures/codex')

function runFixture(path, input, env, args = ['exec', '-', '--json']) {
  return new Promise((resolveRun, reject) => {
    const child = spawn(process.execPath, [path, ...args], {
      env: { ...process.env, ...env },
      stdio: ['pipe', 'pipe', 'pipe'],
    })
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
    child.stdin.end(input)
  })
}

test('Codex fixture reports installed and authenticated to the real doctor probes', async () => {
  const version = await runFixture(CODEX_FIXTURE, '', {}, ['--version'])
  assert.equal(version.code, 0, version.stderr)
  assert.match(version.stdout, /codex fixture 1[.]0[.]0/)

  const auth = await runFixture(CODEX_FIXTURE, '', {}, ['login', 'status'])
  assert.equal(auth.code, 0, auth.stderr)
  assert.match(auth.stdout, /Logged in/i)
})

test('Codex fixture applies each requested marker through the attributed verb API', async () => {
  const mutations = []
  const server = createServer((request, response) => {
    const chunks = []
    request.on('data', (chunk) => chunks.push(chunk))
    request.on('end', () => {
      const body = JSON.parse(Buffer.concat(chunks).toString('utf8') || '{}')
      if (request.url === '/api/verb/project.state') {
        response.writeHead(200, { 'content-type': 'application/json' })
        response.end(JSON.stringify({
          ok: true,
          result: { tracks: [{ id: 'v1', kind: 'video', clips: [{ id: 'c1', asset: 'a1' }] }] },
        }))
        return
      }
      mutations.push({
        url: request.url,
        actor: request.headers['x-cut-actor'],
        body,
      })
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ ok: true, result: { op: { op_id: 'fixture-op' } } }))
    })
  })
  await new Promise((resolveListen) => server.listen(0, '127.0.0.1', resolveListen))
  try {
    const address = server.address()
    const result = await runFixture(
      CODEX_FIXTURE,
      'You are the editing agent inside ShellX Cut.\nUser request: add a marker at 4 seconds named FCV codex',
      {
        CUTD_PROXY_ADDR: `127.0.0.1:${address.port}`,
        CUTD_PROXY_ACTOR: 'agent:test:codex',
      },
    )
    assert.equal(result.code, 0, result.stderr)
    assert.match(result.stdout, /"type":"agent_message"/)
    assert.deepEqual(mutations, [{
      url: '/api/verb/edit.add_marker',
      actor: 'agent:test:codex',
      body: {
        at_ms: 4_000,
        label: 'fcv codex',
        rationale: 'fcv codex fixture: marker',
      },
    }])
  } finally {
    server.close()
  }
})
