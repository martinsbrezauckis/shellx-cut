// Stage the host-built Cut engine under Tauri's target-qualified externalBin
// name. The release smoke workflow deliberately builds an unsigned bundle, but
// it must still contain the same engine layout an installed app launches.

import { copyFileSync, existsSync, lstatSync, mkdirSync, chmodSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')

const HOST_TARGETS = {
  'linux:x64': { target: 'x86_64-unknown-linux-gnu', executable: 'cutd' },
  'darwin:arm64': { target: 'aarch64-apple-darwin', executable: 'cutd' },
  'darwin:x64': { target: 'x86_64-apple-darwin', executable: 'cutd' },
  'win32:x64': { target: 'x86_64-pc-windows-msvc', executable: 'cutd.exe' },
}

export function tauriTargetForHost(platform = process.platform, arch = process.arch) {
  const target = HOST_TARGETS[`${platform}:${arch}`]
  if (!target) {
    throw new Error(`unsupported hosted Tauri smoke target: ${platform}/${arch}`)
  }
  return target
}

export function stageTauriCutd({
  root = repoRoot,
  platform = process.platform,
  arch = process.arch,
} = {}) {
  const { target, executable } = tauriTargetForHost(platform, arch)
  const source = resolve(root, 'app', 'target', 'release', executable)
  if (!existsSync(source) || !lstatSync(source).isFile()) {
    throw new Error(`expected host-built Cut engine at ${source}`)
  }

  const binaries = resolve(root, 'app', 'desktop', 'src-tauri', 'binaries')
  const destination = resolve(binaries, `${executable === 'cutd.exe' ? 'cutd' : executable}-${target}${executable === 'cutd.exe' ? '.exe' : ''}`)
  mkdirSync(binaries, { recursive: true })
  copyFileSync(source, destination)
  if (platform !== 'win32') chmodSync(destination, 0o755)

  if (!lstatSync(destination).isFile()) {
    throw new Error(`failed to stage Tauri external binary at ${destination}`)
  }
  return { source, destination, target }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const staged = stageTauriCutd()
  console.log(`staged ${staged.source} -> ${staged.destination}`)
}
