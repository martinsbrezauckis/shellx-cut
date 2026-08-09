import type { Project } from '../lib/client'
import type { GenerateWorkspaceTab } from '../panels/GenerateTemplates/model'
import type { LeftTab } from '../layout/useLayout'

export const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))

/** A copied clip's descriptor — the Copy/Cut/Paste clipboard payload. A pure
 *  snapshot, not a live object reference, so paste can still use asset/range
 *  fallback data if the source clip is later deleted. */
export interface ClipSnapshot {
  clipId: string
  asset: string
  srcRange: [number, number]
  kind: 'video' | 'audio'
  trackId: string
}

/** Resolve a clip id to its Copy/Cut/Paste snapshot from the live project tree. */
export function snapshotClip(project: Project | null, clipId: string): ClipSnapshot | null {
  if (!project) return null
  for (const track of project.tracks) {
    if (track.kind !== 'video' && track.kind !== 'audio') continue
    for (const c of track.clips) {
      if ('id' in c && 'asset' in c && c.id === clipId) {
        return {
          clipId: c.id,
          asset: c.asset,
          srcRange: [c.src_in_ms, c.src_out_ms],
          kind: track.kind,
          trackId: track.id,
        }
      }
    }
  }
  return null
}

/** Select the paste destination track, preferring the active track when the kind matches. */
export function pasteTargetTrack(project: Project | null, snap: ClipSnapshot, activeTrackId: string | null): string {
  if (activeTrackId) {
    const t = project?.tracks.find((tr) => tr.id === activeTrackId)
    if (t && t.kind === snap.kind) return activeTrackId
  }
  return snap.trackId
}

export function normalizeGenerateTab(tab: unknown): GenerateWorkspaceTab {
  return tab === 'prompt' || tab === 'storyboard' || tab === 'media' ? tab : 'templates'
}

/** Choose the useful first left-sidebar surface for an explicitly opened or
 * created project. This runs once at the project boundary; later user choices
 * remain persisted and are never content-driven back to another tab. */
export function preferredProjectLeftTab(project: Project | null): Extract<LeftTab, 'assets' | 'transcript'> {
  const hasTranscript = Object.values(project?.assets ?? {}).some((asset) => !!asset.transcript)
  return hasTranscript ? 'transcript' : 'assets'
}

/** Decide whether a completed resync should return to Projects.
 * `undefined` means the response was superseded by a newer refresh, while
 * `null` is the server's authoritative "no project open" state. */
export function shouldReturnToProjectsAfterResync(project: Project | null | undefined): boolean {
  return project === null
}
