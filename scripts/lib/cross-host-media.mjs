import { existsSync, readFileSync } from 'node:fs'

export function joinHostPath(base, name) {
  if (!base) return name
  const sep = base.includes('\\') && !base.includes('/') ? '\\' : '/'
  return `${base.replace(/[\\/]+$/, '')}${sep}${name}`
}

export function basenameHostPath(path) {
  const normalized = String(path || '').replace(/[\\/]+$/, '')
  if (!normalized) return ''
  const splitAt = Math.max(normalized.lastIndexOf('/'), normalized.lastIndexOf('\\'))
  return normalized.slice(splitAt + 1)
}

export function dirnameHostPath(path) {
  const normalized = String(path || '').replace(/[\\/]+$/, '')
  if (!normalized) return '.'
  const splitAt = Math.max(normalized.lastIndexOf('/'), normalized.lastIndexOf('\\'))
  if (splitAt < 0) return '.'
  if (splitAt === 0) return normalized[0]
  if (splitAt === 2 && /^[A-Za-z]:/.test(normalized)) return normalized.slice(0, 3)
  return normalized.slice(0, splitAt)
}

function isWslDriver() {
  if (process.platform !== 'linux') return false
  if (process.env.WSL_INTEROP || existsSync('/run/WSL')) return true
  try {
    return /microsoft|wsl/i.test(readFileSync('/proc/version', 'utf8'))
  } catch {
    return false
  }
}

export function resolveDriverPath(path, opts = {}) {
  if (!path) return path
  const platform = opts.platform || process.platform
  const isWsl = opts.isWsl ?? (platform === 'linux' && isWslDriver())
  if (platform !== 'linux' || !isWsl) return path
  let driverPath = path
  if (driverPath.startsWith('\\\\?\\UNC\\')) driverPath = `\\\\${driverPath.slice('\\\\?\\UNC\\'.length)}`
  else if (driverPath.startsWith('\\\\?\\')) driverPath = driverPath.slice('\\\\?\\'.length)
  const wslUnc = /^\\\\wsl(?:\.localhost)?\\[^\\]+\\(.+)$/.exec(driverPath)
  if (wslUnc) return `/${wslUnc[1].replace(/[\\/]+/g, '/')}`
  const match = /^([A-Za-z]):[\\/](.*)$/.exec(driverPath)
  if (!match) return path
  return `/mnt/${match[1].toLowerCase()}/${match[2].replace(/[\\/]+/g, '/')}`
}

export function resolveMediaRole({
  localDir,
  engineDir = localDir,
  realName,
  fallback,
  role,
  exists = existsSync,
}) {
  const existsPath = joinHostPath(localDir, realName)
  if (exists(existsPath)) {
    return {
      role,
      realName,
      path: joinHostPath(engineDir || localDir, realName),
      existsPath,
      fallback,
      fallbackUsed: false,
    }
  }
  return {
    role,
    realName,
    path: fallback,
    existsPath,
    fallback,
    fallbackUsed: true,
  }
}
