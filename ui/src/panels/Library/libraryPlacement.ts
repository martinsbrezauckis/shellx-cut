import { callVerb, type LibItem, type Project } from '../../lib/client'
import { placeLinkedAV } from '../../lib/placement'

export interface LibraryPlacementRequest {
  item: LibItem
  project: Project | null
  playheadMs: number
}

export interface LibraryPlacementResult {
  ok: boolean
  projectChanged: boolean
  message: string
}

async function currentProject(fallback: Project | null): Promise<Project | null> {
  const state = await callVerb('project.state', {})
  return state.ok ? (state.result as Project) : fallback
}

/**
 * Import one Library item and explicitly place it at the live playhead.
 *
 * The first media import is auto-placed by the engine, so an empty timeline
 * must not receive a second edit.insert. Later imports wait for the normal
 * media probe, then use the same linked-A/V planner as Project Assets.
 */
export async function insertLibraryItemAtPlayhead({
  item,
  project,
  playheadMs,
}: LibraryPlacementRequest): Promise<LibraryPlacementResult> {
  const beforeProject = await currentProject(project)
  const timelineEmpty = (beforeProject?.tracks ?? []).every((track) => track.clips.length === 0)
  const imported = await callVerb('library.add_to_project', { id: item.id })
  const assetId = imported.ok
    ? (imported.result as { asset_id?: string } | null)?.asset_id
    : undefined

  if (!assetId) {
    return {
      ok: false,
      projectChanged: false,
      message: `${imported.error?.code ?? 'failed'}: ${imported.error?.message ?? 'could not add to project'}`,
    }
  }

  if (timelineEmpty) {
    return {
      ok: true,
      projectChanged: true,
      message: `"${item.name}" is becoming the first timeline clip`,
    }
  }

  let latestProject = beforeProject
  for (let attempt = 0; attempt < 60; attempt++) {
    latestProject = await currentProject(latestProject)
    if (latestProject?.assets?.[assetId]?.probe) break
    await new Promise((resolve) => setTimeout(resolve, 1000))
  }

  if (!latestProject?.assets?.[assetId]?.probe) {
    return {
      ok: false,
      projectChanged: true,
      message: `"${item.name}" was added to Project Assets, but media analysis did not finish. Try Insert from Assets.`,
    }
  }

  const placement = await placeLinkedAV({
    asset: assetId,
    kind: item.type,
    at_ms: Math.max(0, Math.round(playheadMs)),
    duration_ms: item.type === 'image' ? 5000 : undefined,
    project: latestProject,
    rationale: `insert "${item.name}" from Library at the playhead`,
  })

  return placement.ok
    ? {
        ok: true,
        projectChanged: true,
        message: `Inserted "${item.name}" at the playhead`,
      }
    : {
        ok: false,
        projectChanged: true,
        message: `Insert failed: ${placement.error ?? 'unknown error'}`,
      }
}
