import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { createServer } from 'node:net'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { test } from 'node:test'
import { preflightStagedUpdateFeed, sha256File } from '../lib/staged-update-feed.mjs'
import { buildStagedUpdateReceipt, validateStagedUpdateUiResults, writeStagedUpdateReceipt } from '../lib/staged-update-receipt.mjs'
import { requestTrustedBytes, startLoopbackUpdateServer, validateLoopbackTls } from '../lib/staged-update-server.mjs'

const ROOT = resolve(new URL('../..', import.meta.url).pathname)
const HASH_A = 'a'.repeat(64)
const HASH_B = 'b'.repeat(64)
const HASH_C = 'c'.repeat(64)

function freePort() {
  return new Promise((resolvePort, reject) => {
    const server = createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      server.close((error) => error ? reject(error) : resolvePort(address.port))
    })
  })
}

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`)
}

function fixture(port) {
  const root = mkdtempSync(join(tmpdir(), 'shellx-cut-staged-update-'))
  const artifact = join(root, 'ShellX-Cut_0.6.108_x64-setup.exe')
  const signature = `${artifact}.sig`
  const manifest = join(root, 'latest.json')
  const receipt = join(root, 'updater-manifest-verify.json')
  const url = `https://127.0.0.1:${port}/v0.6.108/${encodeURIComponent(artifact.split('/').at(-1))}`
  writeFileSync(artifact, 'candidate artifact bytes')
  writeFileSync(signature, 'candidate-signature')
  writeJson(manifest, {
    version: '0.6.108', notes: 'staged candidate', pub_date: '2026-08-08T00:00:00.000Z',
    platforms: { 'windows-x86_64': { url, signature: 'candidate-signature' } },
  })
  writeJson(receipt, {
    schema: 'shellx-cut/updater-manifest-verify@1', status: 'pass',
    source: { gitCommit: '1'.repeat(40), version: '0.6.108', cargoLockSha256: HASH_A, contentManifestSha256: HASH_B },
    release: { version: '0.6.108', tag: 'v0.6.108' },
    manifest: { sha256: sha256File(manifest), bytes: readFileSync(manifest).length, platforms: ['windows-x86_64'] },
    artifacts: [{
      platform: 'windows-x86_64', name: artifact.split('/').at(-1), bytes: readFileSync(artifact).length, sha256: sha256File(artifact),
      signatureName: signature.split('/').at(-1), signatureBytes: readFileSync(signature).length, signatureSha256: sha256File(signature),
      signatureVerified: true, url,
    }],
    checks: ['artifact-minisign-verified-against-embedded-pubkey', 'all-required-platforms-present', 'release-url-version-bound'],
  })
  return { root, artifact, signature, manifest, receipt, url }
}

function preflight(files, port, verifySignature = () => {}) {
  return preflightStagedUpdateFeed({
    manifestPath: files.manifest, receiptPath: files.receipt, artifactPath: files.artifact, signaturePath: files.signature,
    platform: 'windows-x86_64', installedVersion: '0.6.107', port, verifySignature,
  })
}

function uiResults({ runId = 'run-1', feedUrl = 'https://127.0.0.1:9443/v0.6.108/latest.json' } = {}) {
  return {
    schema: 'shellx-cut/staged-update-ui@1', status: 'pass', runId, feedUrl,
    installedVersion: '0.6.107', candidateVersion: '0.6.108', usedJsBridgeFixture: false,
    downloaded: false, installed: false, appRunningAfterDecline: true,
    rows: [
      { id: 'about-check-updates', outcome: 'available', interaction: 'native', version: '0.6.108' },
      { id: 'update-btn', outcome: 'available', interaction: 'native', version: '0.6.108' },
      { id: 'about-install-update', outcome: 'declined', interaction: 'native', version: '0.6.108' },
      { id: 'restart-safe', outcome: 'running', interaction: 'native', version: '0.6.107' },
    ],
    evidence: [{ kind: 'native-screenshot', sha256: HASH_C }],
  }
}

test('staged update preflight requires a higher, signed, receipt-bound loopback candidate', async () => {
  const port = await freePort()
  const files = fixture(port)
  try {
    const verified = []
    const checked = preflight(files, port, (...args) => verified.push(args))
    assert.equal(checked.feedUrl, `https://127.0.0.1:${port}/v0.6.108/latest.json`)
    assert.equal(checked.artifact.sha256, sha256File(files.artifact))
    assert.deepEqual(verified, [[files.artifact, files.signature]])
    assert.throws(() => preflightStagedUpdateFeed({
      manifestPath: files.manifest, receiptPath: files.receipt, artifactPath: files.artifact, signaturePath: files.signature,
      platform: 'windows-x86_64', installedVersion: '0.7.0', port, verifySignature: () => {},
    }), /must be higher/)
    assert.throws(() => preflight(files, port, () => { throw new Error('not trusted') }), /signature verification failed: not trusted/)
    const receipt = JSON.parse(readFileSync(files.receipt, 'utf8'))
    receipt.artifacts[0].signatureSha256 = HASH_C
    writeJson(files.receipt, receipt)
    assert.throws(() => preflight(files, port), /artifact, signature, or platform binding/)
    const manifest = JSON.parse(readFileSync(files.manifest, 'utf8'))
    manifest.platforms['darwin-aarch64'] = { url: 'https://example.invalid/v0.6.108/asset', signature: 'candidate-signature' }
    writeJson(files.manifest, manifest)
    assert.throws(() => preflight(files, port), /configured 127[.]0[.]0[.]1 HTTPS feed/)
  } finally {
    rmSync(files.root, { recursive: true, force: true })
  }
})

test('loopback HTTPS server exposes only verified bytes and closes cleanly', async () => {
  const port = await freePort()
  const files = fixture(port)
  const cert = join(files.root, 'loopback-cert.pem')
  const key = join(files.root, 'loopback-key.pem')
  try {
    execFileSync('openssl', [
      'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', key, '-out', cert, '-days', '2', '-subj', '/CN=ShellX-Cut-loopback',
      '-addext', 'subjectAltName=IP:127.0.0.1', '-addext', 'basicConstraints=critical,CA:TRUE', '-addext', 'keyUsage=critical,keyCertSign,digitalSignature',
    ], { stdio: 'ignore' })
    const tls = validateLoopbackTls({ certPath: cert, keyPath: key, caPath: cert })
    const server = await startLoopbackUpdateServer({ preflight: preflight(files, port), tls })
    assert.equal(server.metrics.manifestRequests, 0, 'TLS self-probe is not app evidence')
    writeFileSync(files.manifest, '{"version":"mutated"}')
    writeFileSync(files.artifact, 'mutated artifact bytes')
    const manifest = await requestTrustedBytes(server.url, tls.ca)
    const asset = await requestTrustedBytes(files.url, tls.ca)
    assert.equal(manifest.status, 200)
    assert.equal(asset.status, 200)
    assert.match(manifest.body.toString(), /"version": "0[.]6[.]108"/)
    assert.equal(asset.body.toString(), 'candidate artifact bytes')
    assert.deepEqual(server.metrics, { manifestRequests: 1, artifactRequests: 1, rejectedRequests: 0 })
    await server.close()
    await assert.rejects(requestTrustedBytes(server.url, tls.ca))
    writeFileSync(cert, 'not a certificate')
    assert.throws(() => validateLoopbackTls({ certPath: cert, keyPath: key, caPath: cert }), /TLS material is invalid/)
  } finally {
    rmSync(files.root, { recursive: true, force: true })
  }
})

test('staged update receipt binds only hashes, versions, UI results, and cleanup', () => {
  const ui = validateStagedUpdateUiResults(uiResults(), {
    runId: 'run-1', feedUrl: 'https://127.0.0.1:9443/v0.6.108/latest.json', installedVersion: '0.6.107', candidateVersion: '0.6.108',
  })
  const candidate = {
    feedUrl: 'https://127.0.0.1:9443/v0.6.108/latest.json', version: '0.6.108', platform: 'windows-x86_64',
    source: { gitCommit: '2'.repeat(40), cargoLockSha256: HASH_A, contentManifestSha256: HASH_B },
    manifest: { name: 'latest.json', bytes: 100, sha256: HASH_A },
    artifact: { name: 'candidate.exe', bytes: 200, sha256: HASH_B },
    signature: { name: 'candidate.exe.sig', bytes: 90, sha256: HASH_C },
  }
  const receipt = buildStagedUpdateReceipt({
    generatedAt: '2026-08-08T00:00:00.000Z', runId: 'run-1',
    rigSource: { gitCommit: '3'.repeat(40), version: '0.6.107', cargoLockSha256: HASH_A, contentManifestSha256: HASH_B },
    installed: { version: '0.6.107', before: { name: 'ShellX-Cut.exe', bytes: 123, sha256: HASH_C }, after: { name: 'ShellX-Cut.exe', bytes: 123, sha256: HASH_C } },
    candidate, tls: { certificateSha256: HASH_A, caSha256: HASH_B }, ui, uiResultSha256: HASH_C,
    server: { manifestRequests: 2, artifactRequests: 0, rejectedRequests: 1 }, cleanup: { app: 'terminated-owned-process', server: 'closed' },
  })
  assert.equal(receipt.status, 'pass')
  assert.equal(receipt.installed.unchangedAfterDecline, true)
  assert.equal(receipt.feed.artifactRequests, 0)
  assert.equal(receipt.ui.resultSha256, HASH_C)
  assert.doesNotMatch(JSON.stringify(receipt), /\/tmp|private-candidate|loopback-key[.]pem/)
  const leakingUi = uiResults()
  leakingUi.evidence[0].kind = '/tmp/native-shot.png'
  assert.throws(
    () => validateStagedUpdateUiResults(leakingUi, {
      runId: 'run-1', feedUrl: 'https://127.0.0.1:9443/v0.6.108/latest.json', installedVersion: '0.6.107', candidateVersion: '0.6.108',
    }),
    /path-safe identifier/,
  )
  const out = mkdtempSync(join(tmpdir(), 'shellx-cut-staged-update-receipt-'))
  try {
    const path = writeStagedUpdateReceipt(out, receipt)
    assert.equal(existsSync(path), true)
    assert.throws(() => buildStagedUpdateReceipt({
      generatedAt: '2026-08-08T00:00:00.000Z', runId: 'run-1', rigSource: receipt.rigSource,
      installed: { version: '0.6.107', before: { name: 'app.exe', bytes: 1, sha256: HASH_A }, after: { name: 'app.exe', bytes: 2, sha256: HASH_A } },
      candidate, tls: { certificateSha256: HASH_A, caSha256: HASH_B }, ui, uiResultSha256: HASH_C,
      server: { manifestRequests: 1, artifactRequests: 0, rejectedRequests: 0 }, cleanup: { app: 'closed', server: 'closed' },
    }), /bytes changed/)
  } finally {
    rmSync(out, { recursive: true, force: true })
  }
})
