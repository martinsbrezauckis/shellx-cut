#!/usr/bin/env node
import { lstatSync, realpathSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { artifactInfo } from '../lib/ignored-test-rig.mjs'

function fail(message) {
  throw new Error(`FAIL: ${message}`)
}

export function macosAppTreeIdentity(appPath, inspect = artifactInfo) {
  if (!/\.app$/.test(appPath)) {
    fail(`expected a .app bundle path: ${appPath}`)
  }
  let rootStat
  let realAppPath
  let realRootStat
  try {
    rootStat = lstatSync(appPath)
    realAppPath = realpathSync(appPath)
    realRootStat = lstatSync(realAppPath)
  } catch {
    fail(`expected a readable macOS app bundle: ${appPath}`)
  }
  if (!rootStat.isDirectory() || rootStat.isSymbolicLink() || !realRootStat.isDirectory() || realRootStat.isSymbolicLink()) {
    fail(`expected a real macOS app bundle directory, not a symlink: ${appPath}`)
  }
  const identity = inspect(realAppPath, { tree: true })
  if (!identity.exists || identity.kind !== 'tree') {
    fail(`expected a readable macOS app bundle: ${appPath}`)
  }
  return identity
}

export function assertMacosDmgAppIdentity({ builtApp, dmgApp, inspect = artifactInfo }) {
  const built = macosAppTreeIdentity(builtApp, inspect)
  const packaged = macosAppTreeIdentity(dmgApp, inspect)
  if (built.files !== packaged.files || built.bytes !== packaged.bytes || built.sha256 !== packaged.sha256) {
    fail(
      `DMG app identity mismatch: built sha256=${built.sha256} files=${built.files} bytes=${built.bytes}; `
      + `dmg sha256=${packaged.sha256} files=${packaged.files} bytes=${packaged.bytes}`,
    )
  }
  return built
}

function parseArgs(argv) {
  const options = { builtApp: '', dmgApp: '' }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--built-app') options.builtApp = argv[++index] ?? ''
    else if (arg === '--dmg-app') options.dmgApp = argv[++index] ?? ''
    else fail(`unknown argument: ${arg}`)
  }
  if (!options.builtApp || !options.dmgApp) {
    fail('usage: verify-macos-dmg-app.mjs --built-app <ShellX Cut.app> --dmg-app <ShellX Cut.app>')
  }
  return { builtApp: resolve(options.builtApp), dmgApp: resolve(options.dmgApp) }
}

export function main(argv = process.argv.slice(2)) {
  const { builtApp, dmgApp } = parseArgs(argv)
  const identity = assertMacosDmgAppIdentity({ builtApp, dmgApp })
  console.log(`MACOS_DMG_APP_IDENTITY_OK sha256=${identity.sha256} files=${identity.files} bytes=${identity.bytes}`)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main()
  } catch (error) {
    console.error(error.message)
    process.exitCode = 1
  }
}
