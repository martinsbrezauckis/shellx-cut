import { callVerb } from './client'
// The key moved to clientUrls: export URL construction has to know the chosen
// folder to pick a servable URL shape, and clientUrls is the dependency-free
// module both sides can import (importing it the other way round would make a
// cycle through client.ts). Re-exported so every existing importer of
// EXPORT_OUTPUT_DIR_STORAGE_KEY from this module keeps working.
import { EXPORT_OUTPUT_DIR_STORAGE_KEY } from './clientUrls'

export { EXPORT_OUTPUT_DIR_STORAGE_KEY }

export function getStoredOutputDir(): string | null {
  try {
    return localStorage.getItem(EXPORT_OUTPUT_DIR_STORAGE_KEY)
  } catch {
    return null
  }
}

export function setStoredOutputDir(dir: string | null): void {
  try {
    if (dir) localStorage.setItem(EXPORT_OUTPUT_DIR_STORAGE_KEY, dir)
    else localStorage.removeItem(EXPORT_OUTPUT_DIR_STORAGE_KEY)
  } catch {
    // Storage can be unavailable in restricted webviews; the engine call still applies.
  }
  if (typeof window !== 'undefined') {
    window.dispatchEvent(new CustomEvent('cut:export-output-dir', { detail: dir }))
  }
}

export async function applyExportOutputDir(dir: string | null): Promise<boolean> {
  try {
    return (await callVerb('project.set_output_dir', dir ? { dir } : {})).ok
  } catch {
    return false
  }
}

export async function ensureStoredOutputDirApplied(): Promise<string | null> {
  const dir = getStoredOutputDir()
  if (dir) await applyExportOutputDir(dir)
  return dir
}

/**
 * Return the parent directory of a host path without assuming the UI host and
 * engine host use the same separator. Tauri can return ordinary Windows paths,
 * verbatim `\\?\C:\...` paths, UNC paths, or POSIX paths.
 */
export function outputDirectoryForPath(path: string): string | null {
  const value = path.trim().replace(/[\\/]+$/, '')
  const slash = Math.max(value.lastIndexOf('/'), value.lastIndexOf('\\'))
  if (slash < 0) return null
  // Keep POSIX, ordinary Windows-drive, and verbatim Windows-drive roots
  // intact (`/file` -> `/`, `C:\file` -> `C:\`,
  // `\\?\C:\file` -> `\\?\C:\`).
  if (slash === 0) return value[0]
  if (slash === 2 && /^[A-Za-z]:/.test(value)) return value.slice(0, 3)
  if (slash === 6 && /^[\\/]{2}\?[\\/][A-Za-z]:/.test(value)) {
    return value.slice(0, 7)
  }
  return value.slice(0, slash) || '/'
}

/**
 * A native Save As choice is an explicit user authorization for that one
 * output directory. The engine's output fence still refuses arbitrary REST
 * paths, so temporarily register the picker-selected parent, run the verb, and
 * restore the user's persistent default destination afterwards.
 */
export async function withAuthorizedOutputPath<T>(
  path: string | null | undefined,
  action: () => Promise<T>,
): Promise<T> {
  if (!path) {
    await ensureStoredOutputDirApplied()
    return action()
  }
  const parent = outputDirectoryForPath(path)
  const stored = getStoredOutputDir()
  if (!parent || !await applyExportOutputDir(parent)) {
    throw new Error('could not authorize the selected output folder')
  }
  try {
    return await action()
  } finally {
    await applyExportOutputDir(stored)
  }
}

export function folderTail(dir: string): string {
  const parts = dir.replace(/[\\/]+$/, '').split(/[\\/]/).filter(Boolean)
  return parts.length <= 2 ? dir : '.../' + parts.slice(-2).join('/')
}
