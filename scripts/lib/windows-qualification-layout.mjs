import { win32 } from 'node:path'

export const DEFAULT_WINDOWS_QUALIFICATION_ROOT = String.raw`C:\CutQ\shellx-cut`

export function isWslLocalhostQualificationRoot(value) {
  const normalized = win32.normalize(String(value || '').trim()).replace(/[\\/]+$/, '')
  const match = /^\\\\wsl\.localhost\\[^\\]+\\(.+)$/i.exec(normalized)
  if (!match) return false
  const segments = match[1].split('\\').filter(Boolean)
  return segments.length >= 2 && segments.every((segment) => segment !== '.' && segment !== '..')
}

export function normalizeWindowsQualificationRoot(value = DEFAULT_WINDOWS_QUALIFICATION_ROOT, { allowWslLocalhost = false } = {}) {
  const raw = String(value || '').trim()
  if (!raw || raw.includes('\0')) throw new Error('Windows qualification root is required')
  const normalized = win32.normalize(raw).replace(/[\\/]+$/, '')
  const driveRoot = /^[A-Za-z]:\\/.test(normalized)
  const wslLocalhostRoot = allowWslLocalhost && isWslLocalhostQualificationRoot(normalized)
  if (!win32.isAbsolute(normalized) || (!driveRoot && !wslLocalhostRoot)) {
    const expected = allowWslLocalhost
      ? 'an absolute drive path or exact \\\\wsl.localhost UNC path'
      : 'an absolute drive path'
    throw new Error(`Windows qualification root must be ${expected}: ${value}`)
  }
  const parsed = win32.parse(normalized)
  const segments = normalized.slice(parsed.root.length).split('\\').filter(Boolean)
  if (segments.length < 2) throw new Error(`Windows qualification root is too broad: ${value}`)
  return normalized
}

export function windowsQualificationLayout({ root = DEFAULT_WINDOWS_QUALIFICATION_ROOT, runId, allowWslLocalhost = false }) {
  const normalizedRoot = normalizeWindowsQualificationRoot(root, { allowWslLocalhost })
  const token = String(runId || '')
  if (!/^[A-Za-z0-9][A-Za-z0-9_-]{0,119}$/.test(token)) {
    throw new Error(`Invalid Windows qualification run id: ${runId}`)
  }
  const runsRoot = win32.join(normalizedRoot, 'runs')
  const evidenceRoot = win32.join(normalizedRoot, 'evidence')
  const artifactsRoot = win32.join(normalizedRoot, 'artifacts')
  const stage = win32.join(runsRoot, token)
  return {
    root: normalizedRoot,
    runId: token,
    stage,
    evidence: win32.join(evidenceRoot, token),
    artifacts: win32.join(artifactsRoot, token),
    localAppData: win32.join(stage, 'local-app-data'),
  }
}
