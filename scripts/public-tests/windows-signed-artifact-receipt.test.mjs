import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import { forwardEnvToWindows } from '../lib/wsl-interop-env.mjs'
import {
  packagedArtifactsFromSigningEvents,
  readWindowsSignedArtifactReceipt,
  sha256File,
  validateWindowsSignedArtifactReceipt,
} from '../lib/windows-signed-artifact-receipt.mjs'

const source = {
  gitCommit: '1'.repeat(40),
  gitTree: '2'.repeat(40),
  contentManifestSha256: '3'.repeat(64),
  version: '0.6.109',
}

function digest(value) {
  return createHash('sha256').update(value).digest('hex')
}

function signed(sha256) {
  return { sha256, signatureStatus: 'Valid' }
}

test('signed artifact receipt forwards its Windows artifact path through WSLENV without replacing caller entries', async () => {
  const writer = await readFile(
    new URL('../release/write-windows-signed-artifact-receipt.mjs', import.meta.url),
    'utf8',
  )
  const artifact = 'C:\\Users\\User\\shellx builds\\ShellX Cut_0.6.109_x64-setup.exe'
  const env = forwardEnvToWindows(
    { WSLENV: 'EXISTING/u:PATH/l', EXISTING: 'preserve-me' },
    { SHELLX_CUT_ARTIFACT: artifact },
  )

  assert.equal(env.SHELLX_CUT_ARTIFACT, artifact)
  assert.equal(env.WSLENV, 'EXISTING/u:PATH/l:SHELLX_CUT_ARTIFACT')
  assert.equal(env.EXISTING, 'preserve-me')
  assert.match(
    writer,
    /env:\s*forwardEnvToWindows\(process\.env,\s*\{\s*SHELLX_CUT_ARTIFACT:\s*windowsPath\(path\)\s*\}\s*\)/s,
    'the PowerShell-only artifact argument must be forwarded through WSLENV',
  )
})

test('Windows signed artifact receipt binds the frozen source, installer, and installed packaged binaries', async () => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-signed-artifact-'))
  const installerPath = join(root, 'ShellX Cut_0.6.109_x64-setup.exe')
  const receiptPath = join(root, 'ShellX Cut_0.6.109_x64-setup.exe.receipt.json')
  try {
    await writeFile(installerPath, 'signed installer')
    const installedArtifact = {
      shell: signed('a'.repeat(64)),
      cutd: signed('b'.repeat(64)),
    }
    const receipt = {
      schema: 'shellx-cut/windows-signed-artifact@1', status: 'pass', source,
      installer: { name: 'ShellX Cut_0.6.109_x64-setup.exe', ...signed(sha256File(installerPath)) },
      packaged: installedArtifact,
      standalone: installedArtifact,
    }
    await writeFile(receiptPath, `${JSON.stringify(receipt)}\n`)

    assert.doesNotThrow(() => readWindowsSignedArtifactReceipt(receiptPath, {
      installerPath, source, installedArtifact,
    }))
    assert.throws(() => validateWindowsSignedArtifactReceipt(receipt, {
      installerPath, source: { ...source, gitTree: '4'.repeat(40) }, installedArtifact,
    }), /source gitTree differs/)
    assert.throws(() => validateWindowsSignedArtifactReceipt(receipt, {
      installerPath, source, installedArtifact: { ...installedArtifact, shell: signed('c'.repeat(64)) },
    }), /installed shell hash differs/)
    await writeFile(installerPath, 'tampered installer')
    assert.throws(() => readWindowsSignedArtifactReceipt(receiptPath, {
      installerPath, source, installedArtifact,
    }), /installer hash differs/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test('receipt producer selects the final NSIS installer event and pre-installer packaged payload events', async () => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-signing-events-'))
  const installerPath = join(root, 'ShellX Cut_0.6.109_x64-setup.exe')
  try {
    await writeFile(installerPath, 'final installer')
    const events = [
      { artifactPath: join(root, 'cutd.exe'), ...signed(digest('bundled cutd')) },
      { artifactPath: join(root, 'shellx-cut.exe'), ...signed(digest('bundled shell')) },
      { artifactPath: installerPath, ...signed(sha256File(installerPath)) },
      { artifactPath: join(root, 'shellx-cut.exe'), ...signed(digest('standalone shell')) },
    ]
    const packaged = packagedArtifactsFromSigningEvents({ events, installerPath })
    assert.equal(packaged.installer.sha256, sha256File(installerPath))
    assert.equal(packaged.shell.sha256, digest('bundled shell'))
    assert.equal(packaged.cutd.sha256, digest('bundled cutd'))
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
