import assert from 'node:assert/strict'
import { mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { mkdtempSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { tmpdir } from 'node:os'
import { dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'

import {
  assertControlledWindowsPeArtifact,
  candidateCoordinatesFromPath,
  createWindowsSigningCandidate,
  validateWindowsSigningCandidate,
  windowsSigningCandidatePath,
} from '../lib/windows-signing-candidate.mjs'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const SOURCE = {
  gitCommit: 'a'.repeat(40),
  gitTree: 'b'.repeat(40),
  contentManifestSha256: 'c'.repeat(64),
  version: '0.6.110',
}

function read(path) {
  return readFileSync(resolve(ROOT, path), 'utf8')
}

function writePe(path) {
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(path, Buffer.from('MZcandidate-fixture', 'ascii'))
}

test('Cut Windows signing candidate binds source hash and exactly three controlled PE artifacts', () => {
  const candidate = createWindowsSigningCandidate({
    source: SOURCE,
    target: 'x86_64-pc-windows-msvc',
    mode: 'release',
  })
  assert.deepEqual(candidate.artifacts, [
    { id: 'cutd', path: 'app/target/x86_64-pc-windows-msvc/release/cutd.exe' },
    { id: 'shell', path: 'app/desktop/src-tauri/target/x86_64-pc-windows-msvc/release/shellx-cut.exe' },
    { id: 'installer', path: 'app/desktop/src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/ShellX Cut_0.6.110_x64-setup.exe' },
  ])
  assert.deepEqual(validateWindowsSigningCandidate(candidate, {
    source: SOURCE,
    target: 'x86_64-pc-windows-msvc',
    mode: 'release',
  }), candidate)

  const wrongSource = structuredClone(candidate)
  wrongSource.source.contentManifestSha256 = 'd'.repeat(64)
  assert.throws(
    () => validateWindowsSigningCandidate(wrongSource, {
      source: SOURCE,
      target: 'x86_64-pc-windows-msvc',
      mode: 'release',
    }),
    /does not exactly match the Cut source hash/,
  )

  const arbitraryRoot = structuredClone(candidate)
  arbitraryRoot.artifactRoot = '/tmp/not-cut-owned'
  assert.throws(
    () => validateWindowsSigningCandidate(arbitraryRoot, {
      source: SOURCE,
      target: 'x86_64-pc-windows-msvc',
      mode: 'release',
    }),
    /does not exactly match the Cut source hash/,
  )
})

test('candidate path and PE boundary reject arbitrary roots, names, and symlinks', () => {
  const candidate = createWindowsSigningCandidate({ source: SOURCE, mode: 'release' })
  const temporaryRoot = mkdtempSync(join(tmpdir(), 'shellx-cut-signing-candidate-'))
  try {
    const candidatePath = windowsSigningCandidatePath(ROOT, { mode: 'release' })
    assert.deepEqual(candidateCoordinatesFromPath(ROOT, candidatePath), {
      target: 'x86_64-pc-windows-msvc', mode: 'release',
    })
    assert.throws(
      () => candidateCoordinatesFromPath(ROOT, join(tmpdir(), 'candidate.json')),
      /must be beneath|not the Cut-owned Windows target manifest/,
    )

    const cutd = join(temporaryRoot, candidate.artifacts[0].path)
    const installer = join(temporaryRoot, candidate.artifacts[2].path)
    writePe(cutd)
    writePe(installer)
    assert.equal(assertControlledWindowsPeArtifact(temporaryRoot, candidate, cutd).id, 'cutd')
    assert.equal(assertControlledWindowsPeArtifact(temporaryRoot, candidate, installer).id, 'installer')

    const injected = join(temporaryRoot, candidate.artifactRoot, 'injected.exe')
    writePe(injected)
    assert.throws(
      () => assertControlledWindowsPeArtifact(temporaryRoot, candidate, injected),
      /outside the exact Cut candidate boundary/,
    )

    const shell = join(temporaryRoot, candidate.artifacts[1].path)
    mkdirSync(dirname(shell), { recursive: true })
    symlinkSync(cutd, shell)
    assert.throws(
      () => assertControlledWindowsPeArtifact(temporaryRoot, candidate, shell),
      /regular non-symlink/,
    )
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true })
  }
})

test('signed Cut build wires its fixed candidate helper before the first signing event', () => {
  const build = read('scripts/build-windows.sh')
  const hook = read('app/desktop/scripts/windows-artifact-sign.sh')
  const helper = read('scripts/windows-artifact-sign-command.mjs')
  const signer = read('scripts/windows-artifact-sign.ps1')
  const candidateWriter = read('scripts/release/write-windows-signing-candidate.mjs')

  assert.match(build, /write-windows-signing-candidate[.]mjs[\s\S]+--target "\$TARGET"[\s\S]+--mode "\$MODE"/)
  assert.match(build, /SHELLX_WINDOWS_SIGNING_HELPER="\$signing_helper"/)
  assert.match(build, /SHELLX_CUT_WINDOWS_SIGNING_CANDIDATE="\$signing_candidate"/)
  assert.ok(
    build.indexOf('write-windows-signing-candidate.mjs') < build.indexOf('windows-artifact-sign.sh "$cutd_exe"'),
    'the candidate must exist before the first sidecar sign callback',
  )
  assert.ok(
    hook.indexOf('"$helper" "$artifact"') < hook.lastIndexOf('record_signed_artifact'),
    'the event is recorded only after the helper signs and verifies the artifact',
  )
  assert.match(candidateWriter, /assertCleanTrackedSource\(ROOT\)/)
  assert.match(helper, /readCurrentWindowsSigningCandidate/)
  assert.match(helper, /assertControlledWindowsPeArtifact/)
  assert.match(helper, /execFileSync\('wslpath', \['-w', resolve\(value\)\]/)
  assert.match(helper, /'-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass'/)
  assert.doesNotMatch(helper, /azure login|az login|pass show|TAURI_SIGNING_PRIVATE_KEY/)
  assert.match(signer, /\/fd SHA256 \/tr "http:\/\/timestamp[.]acs[.]microsoft[.]com" \/td SHA256 \/dlib \$DlibPath \/dmdf \$MetadataPath \$Artifact/)
  assert.ok(
    signer.indexOf('& $SignToolPath sign') < signer.indexOf('& $SignToolPath verify'),
    'Authenticode verification must follow the Azure Artifact Signing command',
  )
})
