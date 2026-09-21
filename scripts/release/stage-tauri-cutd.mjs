// Stage the host-built Cut engine under Tauri's target-qualified externalBin
// name. The release smoke workflow deliberately builds an unsigned bundle, but
// it must still contain the same engine layout an installed app launches.

import { copyFileSync, existsSync, lstatSync, mkdirSync, chmodSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const WINDOWS_SIGNING_TARGET = 'x86_64-pc-windows-msvc'

const HOST_TARGETS = {
  'linux:x64': { target: 'x86_64-unknown-linux-gnu', executable: 'cutd' },
  'darwin:arm64': { target: 'aarch64-apple-darwin', executable: 'cutd' },
  'darwin:x64': { target: 'x86_64-apple-darwin', executable: 'cutd' },
  'win32:x64': { target: WINDOWS_SIGNING_TARGET, executable: 'cutd.exe' },
}

export function tauriTargetForHost(platform = process.platform, arch = process.arch) {
  const target = HOST_TARGETS[`${platform}:${arch}`]
  if (!target) {
    throw new Error(`unsupported hosted Tauri smoke target: ${platform}/${arch}`)
  }
  return target
}

function cutdSourceForStage({ root, platform, arch, target }) {
  const hosted = tauriTargetForHost(platform, arch)
  if (target === undefined) {
    return { ...hosted, source: resolve(root, 'app', 'target', 'release', hosted.executable) }
  }
  if (target !== WINDOWS_SIGNING_TARGET || hosted.target !== WINDOWS_SIGNING_TARGET) {
    throw new Error(`unsupported explicit Cut Tauri staging target: ${target}`)
  }
  return {
    ...hosted,
    source: resolve(root, 'app', 'target', WINDOWS_SIGNING_TARGET, 'release', hosted.executable),
  }
}

/** Parse the one explicit target route; zero arguments retains hosted-smoke staging. */
export function parseStageTauriCutdArgs(argv = process.argv.slice(2)) {
  if (argv.length === 0) return {}
  if (argv.length === 2 && argv[0] === '--target' && argv[1]) return { target: argv[1] }
  throw new Error('usage: stage-tauri-cutd.mjs [--target x86_64-pc-windows-msvc]')
}

export function stageTauriCutd({
  root = repoRoot,
  platform = process.platform,
  arch = process.arch,
  target,
} = {}) {
  const stagedTarget = cutdSourceForStage({ root, platform, arch, target })
  const { executable } = stagedTarget
  const source = stagedTarget.source
  if (!existsSync(source) || !lstatSync(source).isFile()) {
    throw new Error(`expected ${target === undefined ? 'host-built' : 'target-qualified'} Cut engine at ${source}`)
  }

  const binaries = resolve(root, 'app', 'desktop', 'src-tauri', 'binaries')
  const destination = resolve(binaries, `${executable === 'cutd.exe' ? 'cutd' : executable}-${stagedTarget.target}${executable === 'cutd.exe' ? '.exe' : ''}`)
  mkdirSync(binaries, { recursive: true })
  copyFileSync(source, destination)
  if (platform !== 'win32') chmodSync(destination, 0o755)

  if (!lstatSync(destination).isFile()) {
    throw new Error(`failed to stage Tauri external binary at ${destination}`)
  }
  return { source, destination, target: stagedTarget.target }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const staged = stageTauriCutd(parseStageTauriCutdArgs())
  console.log(`staged ${staged.source} -> ${staged.destination}`)
}
