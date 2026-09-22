/**
 * Clip ids are deterministic within a project, so c1 can name different media
 * after a project switch. Keep the transient engagement score in the App-owned
 * project session as well as the selected clip's local identity.
 */
export function engagementScopeKey(projectSession: number, clipId: string): string {
  return `${projectSession}\u0000${clipId}`
}
