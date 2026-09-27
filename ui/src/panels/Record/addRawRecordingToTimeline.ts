import { callVerb, type Project } from '../../lib/client'
import { placeLinkedAV, trackEndMs } from '../../lib/placement'

type ImportReply = { asset_id?: string; job_id?: string }
type JobStatus = { state?: string; error?: { message?: string } }
type Result = { ok: true; assetId: string; placedClipId: string } | { ok: false; reason: string }
type Placement = { ok: boolean; error?: string | null }

function videoClipId(project: Project, assetId: string): string | null {
  for (const track of project.tracks ?? []) {
    if (track.kind !== 'video') continue
    const clip = track.clips?.find((item) => 'asset' in item && item.asset === assetId)
    if (clip && 'id' in clip && typeof clip.id === 'string') return clip.id
  }
  return null
}

/** Import the stopped raw MP4 and prove that this project gained its video clip. */
export async function addRawRecordingToTimeline(
  rawPath: string,
  deps: {
    call?: typeof callVerb
    place?: typeof placeLinkedAV
    wait?: (ms: number) => Promise<void>
    now?: () => number
  } = {},
): Promise<Result> {
  const call = deps.call ?? callVerb
  const place = deps.place ?? placeLinkedAV
  const wait = deps.wait ?? ((ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms)))
  const now = deps.now ?? Date.now
  const beforeReply = await call('project.state', {})
  const before = beforeReply.ok ? beforeReply.result as Project : null
  if (!before) return { ok: false, reason: beforeReply.error?.message ?? 'current project unavailable' }
  const hadMediaClips = before.tracks?.some((track) =>
    (track.kind === 'video' || track.kind === 'audio') && track.clips?.length > 0) ?? false
  const imported = await call('media.import', { path: rawPath, proxy: false })
  const assetId = (imported.result as ImportReply | undefined)?.asset_id
  const jobId = (imported.result as ImportReply | undefined)?.job_id
  if (!imported.ok || !assetId || !jobId) return { ok: false, reason: imported.error?.message ?? 'raw import returned no asset and job' }

  const deadline = now() + 180_000
  let project: Project | null = null
  let terminalSeen = false
  for (;;) {
    const state = await call('project.state', {})
    project = state.ok ? state.result as Project : null
    if (!project) return { ok: false, reason: state.error?.message ?? 'current project unavailable' }
    const asset = project.assets?.[assetId]
    const probed = asset?.probe as { kind?: string; duration_ms?: number } | undefined
    if (asset && probed?.kind === 'video' && Number.isFinite(probed.duration_ms)
      && (hadMediaClips || videoClipId(project, assetId))) break
    const reply = await call('jobs.status', { job_id: jobId })
    const job = reply.result as JobStatus | undefined
    if (!reply.ok || !job) return { ok: false, reason: reply.error?.message ?? 'raw import status unavailable' }
    if (job.state === 'failed') return { ok: false, reason: job.error?.message ?? 'raw import failed' }
    if (job.state === 'done') {
      if (terminalSeen) return { ok: false, reason: 'raw import finished without a probed timeline-ready video' }
      terminalSeen = true
      continue
    }
    if (now() >= deadline) return { ok: false, reason: 'raw video probe did not become ready within three minutes' }
    await wait(250)
  }

  const readProject = async () => {
    const reply = await call('project.state', {})
    return reply.ok ? reply.result as Project : null
  }
  if (!project?.assets?.[assetId]) return { ok: false, reason: 'raw import asset disappeared from this project' }
  if (!videoClipId(project, assetId)) {
    const videoTrack = project.tracks?.find((track) => track.kind === 'video')
    const placed: Placement = await place({
      asset: assetId, kind: 'video', at_ms: videoTrack ? trackEndMs(project, videoTrack.id) : 0,
      project, rationale: 'add the saved raw recording to the timeline',
    })
    if (!placed.ok) return { ok: false, reason: placed.error || 'raw recording placement failed' }
    project = await readProject()
  }
  const placedClipId = project && videoClipId(project, assetId)
  return placedClipId ? { ok: true, assetId, placedClipId }
    : { ok: false, reason: 'raw recording was imported but no video clip appeared on the timeline' }
}
