import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, relative } from 'node:path'
import test from 'node:test'

import { resolveWindowsWorkspaceTestRoot } from '../lib/windows-workspace-test-root.mjs'

function wslPath(sourceRoot, linuxPath) {
  const tail = relative(sourceRoot, linuxPath).split('/').join('\\')
  return `\\\\wsl.localhost\\Ubuntu-24.04\\checkout\\${tail}`
}

test('workspace qualification root creates only a real descendant of checkout .scratch', () => {
  const root = mkdtempSync(join(tmpdir(), 'shellx-cut-workspace-root-'))
  try {
    mkdirSync(join(root, '.scratch'))
    const target = join(root, '.scratch', 'windows-native', 'candidate')
    const result = resolveWindowsWorkspaceTestRoot({
      sourceRoot: root,
      workspaceTestRoot: target,
      windowsPath: (path) => wslPath(root, path),
    })
    assert.deepEqual(result, {
      linuxRoot: target,
      windowsRoot: String.raw`\\wsl.localhost\Ubuntu-24.04\checkout\.scratch\windows-native\candidate`,
    })
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('workspace qualification root rejects broad, sibling, and non-WSL mappings', () => {
  const root = mkdtempSync(join(tmpdir(), 'shellx-cut-workspace-root-'))
  try {
    mkdirSync(join(root, '.scratch'))
    for (const target of [root, join(root, '.scratch'), join(root, 'sibling')]) {
      assert.throws(() => resolveWindowsWorkspaceTestRoot({
        sourceRoot: root,
        workspaceTestRoot: target,
        windowsPath: (path) => wslPath(root, path),
      }), /strictly below/)
    }
    assert.throws(() => resolveWindowsWorkspaceTestRoot({
      sourceRoot: root,
      workspaceTestRoot: join(root, '.scratch', 'windows-native'),
      windowsPath: () => String.raw`C:\CutQ\shellx-cut`,
    }), /wsl[.]localhost/)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
