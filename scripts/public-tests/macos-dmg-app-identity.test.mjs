import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import {
  assertMacosDmgAppIdentity,
  macosAppTreeIdentity,
} from '../release/verify-macos-dmg-app.mjs'

function fixtureApp(parent, name, executableBytes = 'shell-bytes') {
  const app = join(parent, name)
  mkdirSync(join(app, 'Contents', 'MacOS'), { recursive: true })
  mkdirSync(join(app, 'Contents', 'Resources'), { recursive: true })
  writeFileSync(join(app, 'Contents', 'MacOS', 'shellx-cut'), executableBytes)
  writeFileSync(join(app, 'Contents', 'MacOS', 'cutd'), 'cutd-bytes')
  writeFileSync(join(app, 'Contents', 'Resources', 'payload.txt'), 'payload-bytes')
  return app
}

function withApps(callback) {
  const root = mkdtempSync(join(tmpdir(), 'shellx-cut-macos-app-identity-'))
  try {
    return callback(root)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
}

test('DMG app identity accepts the exact packaged app tree', () => withApps((root) => {
  const builtApp = fixtureApp(root, 'built/ShellX Cut.app')
  const dmgApp = fixtureApp(root, 'mounted/ShellX Cut.app')

  const identity = assertMacosDmgAppIdentity({ builtApp, dmgApp })

  assert.match(identity.sha256, /^[a-f0-9]{64}$/)
  assert.equal(identity.files, 3)
}))

test('DMG app identity rejects a changed packaged shell or sidecar byte', () => withApps((root) => {
  const builtApp = fixtureApp(root, 'built/ShellX Cut.app')
  const dmgApp = fixtureApp(root, 'mounted/ShellX Cut.app', 'different-shell-bytes')

  assert.throws(
    () => assertMacosDmgAppIdentity({ builtApp, dmgApp }),
    /DMG app identity mismatch/,
  )
}))

test('app tree identity requires a .app directory rather than a loose file', () => withApps((root) => {
  const loose = join(root, 'shellx-cut')
  writeFileSync(loose, 'not an app')

  assert.throws(() => macosAppTreeIdentity(loose), /expected a [.]app bundle path/)
}))

test('DMG app identity rejects either app root when it is a symlink', () => withApps((root) => {
  const builtApp = fixtureApp(root, 'built/ShellX Cut.app')
  const dmgApp = fixtureApp(root, 'mounted/ShellX Cut.app')
  const builtLink = join(root, 'built-link/ShellX Cut.app')
  const dmgLink = join(root, 'mounted-link/ShellX Cut.app')
  mkdirSync(join(root, 'built-link'), { recursive: true })
  mkdirSync(join(root, 'mounted-link'), { recursive: true })
  symlinkSync(builtApp, builtLink, 'dir')
  symlinkSync(dmgApp, dmgLink, 'dir')

  assert.throws(() => assertMacosDmgAppIdentity({ builtApp: builtLink, dmgApp }), /not a symlink/)
  assert.throws(() => assertMacosDmgAppIdentity({ builtApp, dmgApp: dmgLink }), /not a symlink/)
}))

test('identity command reports the bound app-tree digest for relative script execution', () => withApps((root) => {
  const builtApp = fixtureApp(root, 'built/ShellX Cut.app')
  const dmgApp = fixtureApp(root, 'mounted/ShellX Cut.app')
  const result = spawnSync('node', [
    'scripts/release/verify-macos-dmg-app.mjs', '--built-app', builtApp, '--dmg-app', dmgApp,
  ], { encoding: 'utf8' })

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /MACOS_DMG_APP_IDENTITY_OK sha256=[a-f0-9]{64} files=3 bytes=\d+/)
}))
