import assert from 'node:assert/strict'
import { mkdtemp, mkdir, rm, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import test from 'node:test'

import {
  assertWindowsInstalledLocalDrivePath,
  assertWindowsInstalledLocalRoots,
  copyWindowsOwnedFile,
  prepareWindowsInstalledRunWorkspace,
  provisionWindowsInstalledRunDirectories,
  removeWindowsInstalledRunDirectory,
  resolveWindowsInstalledWorkspaceTestRoot,
} from '../lib/windows-installed-run-workspace.mjs'

test('Windows runner workspace provisions C:-local runtime state while preserving WSL evidence output', async () => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-windows-run-workspace-'))
  const scene = join(root, 'scene.mp4')
  const mappedWindowsPaths = new Map()
  const linuxPath = (windowsPath) => {
    const mapped = join(root, 'mapped', windowsPath.replaceAll(':', '_').replaceAll('\\', '_'))
    mappedWindowsPaths.set(resolve(mapped), windowsPath)
    return mapped
  }
  const copies = []
  try {
    await writeFile(scene, 'scene')
    const workspace = prepareWindowsInstalledRunWorkspace({
      root,
      workspaceTestRoot: '',
      windowsTestRootProvided: false,
      requestedWindowsTestRoot: 'C:\\CutQ\\shellx-cut',
      inheritedWindowsTestRoot: '',
      runId: 'windows-installed-candidate-test',
      outArgument: '',
      testControlBindingPath: '',
      finalResumeRequest: null,
      assertFinalResumeOutput: () => {},
      roleArgs: { scene },
      windowsPath: (path) => {
        const requested = resolve(path)
        for (const [mapped, windows] of mappedWindowsPaths) {
          if (requested === mapped) return windows
          if (requested.startsWith(`${mapped}/`)) return `${windows}\\${requested.slice(mapped.length + 1).replaceAll('/', '\\')}`
        }
        return String.raw`\\wsl.localhost\Ubuntu-24.04${requested.replaceAll('/', '\\')}`
      },
      linuxPath,
      captureSync: (_command, args) => args.includes('$env:LOCALAPPDATA') ? 'C:\\Users\\Test\\AppData\\Local' : 'C:\\Windows\\System32',
      hash: () => 'a'.repeat(64),
      provisionWindowsRun: ({ stageWin, artifactsWin, directories }) => ({
        stage: stageWin,
        artifacts: artifactsWin,
        directories,
      }),
      copyWindowsFile: ({ source, destinationWin, ownedRootWin }) => {
        copies.push({ source, destinationWin, ownedRootWin })
        return { path: destinationWin, sha256: 'a'.repeat(64) }
      },
      stamp: () => '2026-08-21T12-00-00-000Z',
    })
    assert.equal(workspace.layout.root, 'C:\\CutQ\\shellx-cut')
    assert.equal(workspace.runsRootWin, 'C:\\CutQ\\shellx-cut\\runs')
    assert.equal(workspace.webviewDataWin, 'C:\\CutQ\\shellx-cut\\runs\\windows-installed-candidate-test\\app-home\\ShellX Cut WebView Tests\\ShellXCutFinalAction-2026-08-21T12-00-00-000Z')
    assert.equal(workspace.staged.scene, 'C:\\CutQ\\shellx-cut\\runs\\windows-installed-candidate-test\\media\\scene.mp4')
    assert.equal(workspace.nativeCompileWin, 'C:\\CutQ\\shellx-cut\\runs\\windows-installed-candidate-test\\app-home\\native-fixture-compile')
    assert.deepEqual(workspace.mediaIdentity, { scene: { sha256: 'a'.repeat(64) } })
    assert.deepEqual(copies, [{ source: scene, destinationWin: workspace.staged.scene, ownedRootWin: workspace.stageWin }])
    workspace.copyOwnedFile(scene, join(workspace.stage, 'installer', 'candidate.exe'))
    workspace.copyOwnedArtifactFile(scene, join(workspace.artifacts, 'candidate.exe'))
    assert.deepEqual(copies.slice(1), [
      { source: scene, destinationWin: 'C:\\CutQ\\shellx-cut\\runs\\windows-installed-candidate-test\\installer\\candidate.exe', ownedRootWin: workspace.stageWin },
      { source: scene, destinationWin: 'C:\\CutQ\\shellx-cut\\artifacts\\windows-installed-candidate-test\\candidate.exe', ownedRootWin: workspace.artifactsWin },
    ])
    assert.equal(workspace.out, join(root, '.scratch', 'windows-installed-evidence', 'windows-installed-candidate-test'))
    for (const path of [workspace.runtimeHomeWin, workspace.projectsWin, workspace.exportWin, workspace.verifierTempWin]) {
      assert.match(path, /^[A-Za-z]:\\/)
      assert.doesNotMatch(path, /^\\\\wsl[.]/i)
    }
    assert.equal(workspace.coverageRunReceipt, workspace.fullCoverageReceipt)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test('Windows installed workspace rejects WSL UNC runtime roots and provisions only exact C: children', () => {
  const stageWin = String.raw`C:\CutQ\shellx-cut\runs\run-1`
  const artifactsWin = String.raw`C:\CutQ\shellx-cut\artifacts\run-1`
  const directories = {
    runtime: String.raw`C:\CutQ\shellx-cut\runs\run-1\app-home`,
    projects: String.raw`C:\CutQ\shellx-cut\runs\run-1\projects`,
    verifier: String.raw`C:\CutQ\shellx-cut\runs\run-1\verifier-temp`,
  }
  let provisionScript = ''
  const provisioned = provisionWindowsInstalledRunDirectories({
    stageWin,
    artifactsWin,
    directories,
    captureSync: (command, args) => {
      assert.equal(command, 'powershell.exe')
      provisionScript = String(args.at(-1))
      return JSON.stringify({ stage: stageWin, artifacts: artifactsWin, directories: Object.values(directories) })
    },
  })
  assert.deepEqual(provisioned, { stage: stageWin, artifacts: artifactsWin, directories })
  assert.match(provisionScript, /Assert-DirectOwnedChild/)
  assert.match(provisionScript, /Assert-NoReparseAncestors/)
  assert.match(provisionScript, /Assert-RealDirectory/)
  assert.match(provisionScript, /FileAttributes]::ReparsePoint/)
  assert.match(provisionScript, /\[System[.]IO[.]Directory\]::CreateDirectory/)
  assert.doesNotMatch(provisionScript, /wsl[.]localhost/i)
  for (const value of [String.raw`\\wsl.localhost\Ubuntu-24.04\home\runner\workspace`, String.raw`\\?\UNC\wsl.localhost\Ubuntu-24.04\home\runner\workspace`]) {
    assert.throws(() => assertWindowsInstalledLocalDrivePath(value, 'runtime'), /local drive path, never a WSL UNC path/)
  }
  assert.throws(
    () => assertWindowsInstalledLocalRoots({ projects: String.raw`\\wsl.localhost\Ubuntu-24.04\home\runner\projects` }),
    /Windows installed projects root must resolve to a Windows local drive path/,
  )
  assert.deepEqual(
    resolveWindowsInstalledWorkspaceTestRoot({
      workspaceTestRoot: '/mnt/c/CutQ/shellx-cut',
      windowsPath: () => String.raw`C:\CutQ\shellx-cut`,
    }),
    { linuxRoot: '/mnt/c/CutQ/shellx-cut', windowsRoot: String.raw`C:\CutQ\shellx-cut` },
  )
  assert.throws(
    () => resolveWindowsInstalledWorkspaceTestRoot({
      workspaceTestRoot: '/home/runner/.scratch/workstation-matrix',
      windowsPath: () => String.raw`\\wsl.localhost\Ubuntu-24.04\home\runner\.scratch\workstation-matrix`,
    }),
    /never a WSL UNC path/,
  )
})

test('Windows owned-file staging copies only regular WSL fixture sources into an exact C: owned root', async () => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-windows-owned-copy-'))
  const source = join(root, 'scene.mp4')
  const sourceLink = join(root, 'scene-link.mp4')
  const sourceDirectory = join(root, 'not-a-file')
  const sourceWin = String.raw`\\wsl.localhost\Ubuntu-24.04\home\runner\shellx-cut\testdata\scene.mp4`
  const ownedRootWin = String.raw`C:\CutQ\shellx-cut\runs\run-1`
  const destinationWin = String.raw`C:\CutQ\shellx-cut\runs\run-1\media\scene.mp4`
  const digest = 'b'.repeat(64)
  let copyScript = ''
  try {
    await writeFile(source, 'scene')
    await symlink(source, sourceLink)
    await mkdir(sourceDirectory)
    const copied = copyWindowsOwnedFile({
      source,
      destinationWin,
      ownedRootWin,
      windowsPath: (path) => {
        assert.equal(path, source)
        return sourceWin
      },
      captureSync: (command, args) => {
        assert.equal(command, 'powershell.exe')
        copyScript = String(args.at(-1))
        return JSON.stringify({ path: destinationWin, sha256: digest })
      },
      hash: (path) => {
        assert.equal(path, source)
        return digest
      },
    })
    assert.deepEqual(copied, { path: destinationWin, sha256: digest })
    assert.match(copyScript, /Copy-Item -LiteralPath/)
    assert.doesNotMatch(copyScript, /Copy-Item[^;]+-Force/)
    assert.match(copyScript, /owned destination already exists/)
    assert.match(copyScript, /Assert-NoReparseAncestors \$ownedRoot/)
    assert.match(copyScript, /Assert-NoReparseAncestors \$parent/)
    assert.match(copyScript, /FileAttributes]::ReparsePoint/)
    assert.match(copyScript, /Get-FileHash -Algorithm SHA256 -LiteralPath/)
    assert.match(copyScript, /\\\\wsl[.]localhost\\Ubuntu-24[.]04/)
    assert.match(copyScript, /C:\\CutQ\\shellx-cut\\runs\\run-1\\media\\scene[.]mp4/)
    assert.throws(
      () => copyWindowsOwnedFile({
        source,
        destinationWin,
        ownedRootWin,
        windowsPath: () => sourceWin,
        captureSync: () => JSON.stringify({ path: destinationWin, sha256: 'c'.repeat(64) }),
        hash: () => digest,
      }),
      /readback differs/,
    )
    for (const invalidSource of [sourceLink, sourceDirectory]) {
      assert.throws(
        () => copyWindowsOwnedFile({
          source: invalidSource,
          destinationWin,
          ownedRootWin,
          windowsPath: () => sourceWin,
          captureSync: () => { throw new Error('PowerShell must not run for a nonregular source') },
          hash: () => digest,
        }),
        /regular non-symlink file/,
      )
    }
    assert.throws(
      () => copyWindowsOwnedFile({
        source,
        destinationWin: String.raw`C:\Windows\scene.mp4`,
        ownedRootWin,
        windowsPath: () => sourceWin,
        captureSync: () => { throw new Error('PowerShell must not run for an escaped destination') },
        hash: () => digest,
      }),
      /must stay beneath the exact owned root/,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test('Windows run cleanup accepts only its exact direct C: child and rejects reparse points', () => {
  const runsRootWin = String.raw`C:\CutQ\shellx-cut\runs`
  const stageWin = String.raw`C:\CutQ\shellx-cut\runs\run-1`
  let cleanupScript = ''
  removeWindowsInstalledRunDirectory({
    runsRootWin,
    stageWin,
    captureSync: (command, args) => {
      assert.equal(command, 'powershell.exe')
      cleanupScript = String(args.at(-1))
      return ''
    },
  })
  assert.match(cleanupScript, /Assert-DirectOwnedChild \$parent \$target/)
  assert.match(cleanupScript, /Assert-NoReparseAncestors \$parent/)
  assert.match(cleanupScript, /Assert-RealDirectory \$parent/)
  assert.match(cleanupScript, /GetDirectoryName/)
  assert.match(cleanupScript, /FileAttributes]::ReparsePoint/)
  assert.match(cleanupScript, /\[System[.]IO[.]Directory\]::Delete/)
  assert.match(cleanupScript, /Remove-OwnedTreeTolerant/)
  assert.match(cleanupScript, /function Test-MissingPathException/)
  assert.match(cleanupScript, /\$cursor=\$cursor[.]InnerException/)
  assert.match(cleanupScript, /if\(Test-MissingPathException \$_[.]Exception\)/)
  assert.match(cleanupScript, /owned directory contains a reparse entry/)
  assert.match(cleanupScript, /if\(\$null -eq \$remaining\)\{break\}/)
  assert.doesNotMatch(cleanupScript, /Remove-Item/)
  assert.throws(
    () => removeWindowsInstalledRunDirectory({
      runsRootWin,
      stageWin: String.raw`\\wsl.localhost\Ubuntu-24.04\home\runner\run-1`,
      captureSync: () => '',
    }),
    /local drive path, never a WSL UNC path/,
  )
  assert.throws(
    () => removeWindowsInstalledRunDirectory({
      runsRootWin: String.raw`C:\CutQ\shellx-cut\other-runs`,
      stageWin,
      captureSync: () => { throw new Error('PowerShell must not run for an escaped cleanup root') },
    }),
    /exact direct child of its owned parent/,
  )
})
