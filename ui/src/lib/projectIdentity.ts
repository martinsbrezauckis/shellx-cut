/** Path-free server identity for the project currently open in Cut. */
export interface ProjectIdentity {
  schema: 'shellx-cut/project-identity/1'
  origin_path_sha256: string
  project_name: string
}

export function isProjectIdentity(value: unknown): value is ProjectIdentity {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return false
  const identity = value as Record<string, unknown>
  return identity.schema === 'shellx-cut/project-identity/1'
    && /^sha256:[a-f0-9]{64}$/.test(String(identity.origin_path_sha256 ?? ''))
    && typeof identity.project_name === 'string'
    && identity.project_name.length > 0
}
