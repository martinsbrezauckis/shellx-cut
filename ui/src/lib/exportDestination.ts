import { callVerb } from './client'
// The key moved to clientUrls: export URL construction has to know the chosen
// folder to pick a servable URL shape, and clientUrls is the dependency-free
// module both sides can import (importing it the other way round would make a
// cycle through client.ts). Re-exported so every existing importer of
// EXPORT_OUTPUT_DIR_STORAGE_KEY from this module keeps working.
import { EXPORT_OUTPUT_DIR_STORAGE_KEY } from './clientUrls'
import { outputFolderStatusFrom, type OutputFolderStatus } from './outputFolderStatus'
import { runUserVerb } from './userActionFeedback'

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

// ---------------------------------------------------------------------------
// The export destination is held by the ENGINE (project.get_output_dir, the
// output_folder_changed event). Everything below mirrors that one value: the
// localStorage copy above is only a cache of the engine's last answer, kept so
// URL construction can stay synchronous. Only the one-time carry-over of an
// older UI-only choice can push it back, and only when the engine has no choice.
// A folder an agent chose always replaces the browser copy.
// ---------------------------------------------------------------------------

let engineStatus: OutputFolderStatus | null = null
let statusRevision = 0
let refreshing: Promise<OutputFolderStatus | null> | null = null
// Keep this beside the legacy UI preference: another UI installation must not
// consume this one's carry-over. Claim BEFORE applying, including on refusal,
// so a restart or later clear never resurrects a stale cached folder.
const OUTPUT_DIR_MIGRATED_KEY = 'cut.outputDir.engine-owned.v1'
function claimOutputDirMigration(): boolean {
  try {
    if (localStorage.getItem(OUTPUT_DIR_MIGRATED_KEY)) return false
    localStorage.setItem(OUTPUT_DIR_MIGRATED_KEY, '1')
    return true
  } catch {
    // Without a durable marker we cannot promise a one-time authorization.
    return false
  }
}
// While a Save As authorizes ONE folder for one export, the engine changes
// are our own and must not show as the person's destination.
let authorizing = 0
const statusListeners = new Set<() => void>()

export function getOutputFolderStatus(): OutputFolderStatus | null {
  return engineStatus
}

export function subscribeOutputFolderStatus(listener: () => void): () => void {
  statusListeners.add(listener)
  return () => { statusListeners.delete(listener) }
}

/** Adopt the engine's answer: refresh the status and the cached folder. */
export function applyOutputFolderStatus(status: OutputFolderStatus): void {
  if (authorizing > 0) return
  claimOutputDirMigration()
  statusRevision += 1
  engineStatus = status
  if (getStoredOutputDir() !== status.dir) setStoredOutputDir(status.dir)
  for (const listener of statusListeners) listener()
}

/** Ask the engine where exports go now, and adopt the answer. */
export function refreshOutputFolder(): Promise<OutputFolderStatus | null> {
  refreshing ??= readOutputFolder().finally(() => { refreshing = null })
  return refreshing
}

async function readOutputFolder(): Promise<OutputFolderStatus | null> {
  const revision = statusRevision
  try {
    const reply = await callVerb('project.get_output_dir', {})
    let status = reply.ok ? outputFolderStatusFrom(reply.result) : null
    if (!status) return null
    // A change event (including an agent clearing the choice) beats an older
    // read or carry-over response that was still in flight.
    if (revision !== statusRevision) return engineStatus
    const legacy = getStoredOutputDir()
    if (claimOutputDirMigration() && !status.dir && legacy) {
      const carried = await runUserVerb('project.set_output_dir', { dir: legacy, only_if_empty: true }, 'Could not keep the previous export folder')
      if (revision !== statusRevision) return engineStatus
      if (carried?.ok) status = outputFolderStatusFrom(carried.result) ?? status
    }
    applyOutputFolderStatus(status)
    return status
  } catch {
    return null
  }
}

export async function clearStoredOutputDirIfAccepted(
  apply: () => Promise<boolean> = () => applyExportOutputDir(null),
): Promise<boolean> {
  if (!await apply()) return false
  setStoredOutputDir(null)
  return true
}

/**
 * Before an export that uses the default folder: read the engine's destination
 * and refuse, in plain words, a chosen folder that is gone or cannot be
 * written, instead of letting the encoder fail at the end.
 */
export async function ensureExportDestinationUsable(
  read: () => Promise<OutputFolderStatus | null> = refreshOutputFolder,
): Promise<string | null> {
  const status = await read()
  if (status && !status.usable) {
    throw new Error(`${status.problem ?? 'The export folder cannot be used.'}${status.suggested_action ? ` ${status.suggested_action}` : ''}`)
  }
  return status ? status.dir : getStoredOutputDir()
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
    await ensureExportDestinationUsable()
    return action()
  }
  const parent = outputDirectoryForPath(path)
  // Restore what the ENGINE held (a person's or an agent's choice), not the
  // browser copy.
  const before = (await refreshOutputFolder())?.dir ?? getStoredOutputDir()
  authorizing += 1
  try {
    if (!parent || !await applyExportOutputDir(parent)) {
      throw new Error('could not authorize the selected output folder')
    }
    try {
      return await action()
    } finally {
      // A restore the engine refuses (the folder it held has vanished meanwhile)
      // falls back to the project exports folder, which the status bar then shows.
      if (!await applyExportOutputDir(before)) await applyExportOutputDir(null)
    }
  } finally {
    authorizing -= 1
    void refreshOutputFolder()
  }
}

/** The folder's own name (the last segment): what fits in the status bar chip. */
export function folderName(dir: string): string {
  const parts = dir.replace(/[\\/]+$/, '').split(/[\\/]/).filter(Boolean)
  return parts.length ? parts[parts.length - 1] : dir
}

export function folderTail(dir: string): string {
  const parts = dir.replace(/[\\/]+$/, '').split(/[\\/]/).filter(Boolean)
  return parts.length <= 2 ? dir : '.../' + parts.slice(-2).join('/')
}
