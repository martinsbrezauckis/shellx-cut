/** Cross-platform filename extraction for project media paths. */
export function mediaBasename(path: string): string {
  return path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || path
}

/** Library ids are the first 16 hex characters of a project asset hash. */
export function libraryIdFromAssetHash(hash: string | undefined): string | null {
  if (!hash) return null
  const value = hash.startsWith('sha256:') ? hash.slice('sha256:'.length) : hash
  return /^[a-f0-9]{16,}$/i.test(value) ? value.slice(0, 16).toLowerCase() : null
}
