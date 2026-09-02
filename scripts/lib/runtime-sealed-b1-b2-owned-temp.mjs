import { existsSync, lstatSync, mkdtempSync, realpathSync } from 'node:fs'
import { basename, dirname, join, resolve, win32 } from 'node:path'

import {
  assertContainedRealPath, assertRealPath, removeOwnedTree, sealedTree,
} from './runtime-sealed-b1-b2-files.mjs'

// Chromium's POSIX singleton socket has a bounded path length. Keep its
// Playwright profile in a short, newly-created system-temp child while HOME
// stays isolated below the qualification evidence root.
export function sealedSystemTemporaryParent({ platform = process.platform, systemRoot = process.env.SystemRoot || process.env.SYSTEMROOT || '' } = {}) {
  if (platform === 'win32') {
    if (!systemRoot || !win32.isAbsolute(systemRoot)) throw new Error('Windows sealed browser temporary root requires an absolute SystemRoot')
    return win32.join(win32.resolve(systemRoot), 'Temp')
  }
  return resolve(realpathSync.native('/tmp'))
}

export function createOwnedSystemTemporaryRoot(requested = sealedSystemTemporaryParent()) {
  const parent = assertRealPath(requested, 'system temporary root')
  const entry = lstatSync(parent)
  if (!entry.isDirectory()) throw new Error(`system temporary root is not a directory: ${parent}`)
  let created = ''
  try {
    created = mkdtempSync(join(parent, 'shellx-cut-runtime-sealed-'))
    const path = assertContainedRealPath(parent, created, 'flow temporary root')
    if (dirname(path) !== parent) throw new Error('flow temporary root must be a direct system temporary child')
    if (process.platform !== 'win32' && Buffer.byteLength(path) > 60) throw new Error(`flow temporary root is too long for Chromium's bounded POSIX socket path: ${path}`)
    return {
      parent,
      path,
      allocation: { mechanism: 'mkdtemp', prefix: 'shellx-cut-runtime-sealed-', leaf: basename(path) },
      initialTree: sealedTree(path, 'new flow temporary root', { allowEmpty: true }),
    }
  } catch (error) {
    if (created && existsSync(created)) {
      try { removeOwnedTree(parent, created, 'failed flow temporary root') } catch { /* preserve the original fail-closed error */ }
    }
    throw error
  }
}
