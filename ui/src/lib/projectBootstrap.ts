// First-run project helpers shared by the native drop-to-create flow.
//
// Keep this module pure: filename/path handling is cross-platform and its
// output becomes a project folder name on Windows and macOS. The actual
// project.create/media.import calls remain in DropZone so UI progress and
// partial failures stay visible.

export type SupportedMediaKind = 'video' | 'audio' | 'image'

const MEDIA_EXTENSIONS: Readonly<Record<string, SupportedMediaKind>> = Object.freeze({
  mp4: 'video',
  mov: 'video',
  mkv: 'video',
  webm: 'video',
  m4v: 'video',
  mp3: 'audio',
  m4a: 'audio',
  aac: 'audio',
  wav: 'audio',
  flac: 'audio',
  ogg: 'audio',
  opus: 'audio',
  png: 'image',
  jpg: 'image',
  jpeg: 'image',
  webp: 'image',
  gif: 'image',
})

const WINDOWS_RESERVED_NAME = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i

export function supportedMediaKind(path: string): SupportedMediaKind | null {
  const name = path.split(/[\\/]/).pop() ?? ''
  const ext = name.includes('.') ? name.split('.').pop()?.toLowerCase() : ''
  return ext ? MEDIA_EXTENSIONS[ext] ?? null : null
}

export function isSupportedMediaPath(path: string): boolean {
  return supportedMediaKind(path) !== null
}

/** Turn either a POSIX or Windows media path into a portable project name. */
export function projectNameFromMediaPath(path: string): string {
  const filename = path.split(/[\\/]/).pop() ?? ''
  const dot = filename.lastIndexOf('.')
  const stem = dot > 0 ? filename.slice(0, dot) : filename
  let name = stem
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/[. ]+$/g, '')
    .slice(0, 80)
    .trim()

  if (!name || name === '.' || name === '..') name = 'Untitled project'
  if (WINDOWS_RESERVED_NAME.test(name)) name = `${name} project`
  return name
}

/** Choose a deterministic, case-insensitive free name from the recent index. */
export function availableProjectName(base: string, existingNames: Iterable<string>): string {
  const used = new Set([...existingNames].map((name) => name.toLocaleLowerCase()))
  if (!used.has(base.toLocaleLowerCase())) return base
  for (let suffix = 2; suffix <= 999; suffix++) {
    const candidate = `${base} ${suffix}`
    if (!used.has(candidate.toLocaleLowerCase())) return candidate
  }
  return `${base} ${Date.now()}`
}
