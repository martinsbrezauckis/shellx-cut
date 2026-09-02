import { existsSync, lstatSync, mkdirSync } from 'node:fs'
import { isAbsolute, join, relative, resolve } from 'node:path'

import { isWslLocalhostQualificationRoot } from './windows-qualification-layout.mjs'

function isStrictDescendant(root, target) {
  const remainder = relative(root, target)
  return Boolean(remainder) && !remainder.startsWith('..') && !isAbsolute(remainder)
}

function assertDirectory(path, label) {
  if (!existsSync(path)) mkdirSync(path)
  const stat = lstatSync(path)
  if (stat.isSymbolicLink() || !stat.isDirectory()) {
    throw new Error(`${label} must be a real directory, not a symlink or file: ${path}`)
  }
}

function createContainedDirectory(root, target) {
  assertDirectory(root, 'workspace scratch root')
  let current = root
  for (const segment of relative(root, target).split('/').filter(Boolean)) {
    current = join(current, segment)
    assertDirectory(current, 'workspace test root')
  }
}

// Managed WSL sessions can have a read-only Windows volume. This opt-in keeps
// all generated Windows-visible state in the checked-out .scratch tree and
// accepts only the exact UNC spelling emitted by `wslpath -w`.
export function resolveWindowsWorkspaceTestRoot({ sourceRoot, workspaceTestRoot, windowsPath }) {
  const source = resolve(String(sourceRoot || ''))
  const requested = String(workspaceTestRoot || '').trim()
  if (!isAbsolute(source) || !requested || !isAbsolute(requested)) {
    throw new Error('--workspace-test-root requires an absolute Linux path')
  }
  if (typeof windowsPath !== 'function') throw new Error('workspace test root requires a WSL-to-Windows path converter')

  const scratch = resolve(source, '.scratch')
  const target = resolve(requested)
  if (!isStrictDescendant(scratch, target)) {
    throw new Error('--workspace-test-root must be strictly below this checkout\'s .scratch directory')
  }
  createContainedDirectory(scratch, target)

  const windowsRoot = String(windowsPath(target) || '').trim()
  if (!isWslLocalhostQualificationRoot(windowsRoot)) {
    throw new Error('--workspace-test-root must convert to an exact \\wsl.localhost UNC path')
  }
  return { linuxRoot: target, windowsRoot }
}
