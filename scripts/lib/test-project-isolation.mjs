import { homedir } from 'node:os'
import { join, posix, win32 } from 'node:path'

export function resolveTestProjectsIsolation({
  external,
  configuredDir,
  repoDir,
  receiptStem,
  homeDir = homedir(),
} = {}) {
  const configured = String(configuredDir || '').trim()
  const normalized = configured.replaceAll('\\', '/').replace(/\/+$/, '').toLowerCase()
  const normalizedHome = String(homeDir || '').replaceAll('\\', '/').replace(/\/+$/, '').toLowerCase()
  const defaults = new Set([
    `${normalizedHome}/shellx cut projects`,
    `${normalizedHome}/documents/shellx cut projects`,
  ])
  const usesDefaultLibraryName = /\/(?:documents\/)?shellx cut projects$/.test(normalized)
  if (configured && (defaults.has(normalized) || usesDefaultLibraryName)) {
    return {
      ok: false,
      error: 'SHELLX_CUT_PROJECTS_DIR must not point at the default user project library during tests',
    }
  }
  if (external && !configured) {
    return {
      ok: false,
      error: 'external UI tests require SHELLX_CUT_PROJECTS_DIR from the native isolated runner',
    }
  }
  return {
    ok: true,
    dir: configured || join(repoDir, '.shellx-scratch', 'full-coverage', 'projects', receiptStem),
    ownedByRun: !configured,
  }
}

export function requireIsolatedTestProjectsDir(configuredDir, options = {}) {
  const isolation = resolveTestProjectsIsolation({
    external: true,
    configuredDir,
    repoDir: '',
    receiptStem: '',
    ...options,
  })
  if (!isolation.ok) throw new Error(isolation.error)
  return isolation.dir
}

export function withIsolatedProjectCreate(verbName, args, projectsDir) {
  if (verbName !== 'project.create') return args
  const pathApi = /^[a-z]:[\\/]/i.test(projectsDir) || /^\\\\/.test(projectsDir)
    ? win32
    : posix
  if (args?.dir) {
    const relative = pathApi.relative(pathApi.resolve(projectsDir), pathApi.resolve(args.dir))
    if (relative === '..' || relative.startsWith(`..${pathApi.sep}`) || pathApi.isAbsolute(relative)) {
      throw new Error('test project.create dir must stay inside SHELLX_CUT_PROJECTS_DIR')
    }
    return args
  }
  const name = String(args?.name || '').trim()
  if (!name) throw new Error('test project.create requires a non-empty name')
  const safeName = name.replace(/[\\/]/g, '_')
  return { ...args, dir: pathApi.join(projectsDir, `${safeName}.cutproj`) }
}
