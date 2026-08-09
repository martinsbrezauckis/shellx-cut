// DropZone.tsx — desktop drag-drop media import (issue: "tried to drop an image
// on the timeline and could not").
//
// Role: listens to the native shell's OS file-drop (lib/tauri.onFileDrop) and
// imports each dropped file via media.import, showing a full-window hint while a
// drag is over the window. With no project open, the first supported file creates
// a sensibly named project; the first video/audio becomes the timeline through
// the normal import chain, while a first still is placed for five seconds.
// No-op outside Tauri (a browser origin has no real filesystem paths to hand the
// engine). Mounted once by App.
//
// The op_applied WS event from each import auto-updates the timeline; onChanged
// is a belt-and-braces resync for the auto-place (mirrors the topbar Import).

import { useEffect, useRef, useState } from 'react'
import { callVerb, type JobRecord, type Project, type ProjectEntry } from './lib/client'
import { isTauri, onFileDrop } from './lib/tauri'
import { getGenerateProxies } from './lib/proxyPref'
import {
  availableProjectName,
  isSupportedMediaPath,
  projectNameFromMediaPath,
  supportedMediaKind,
} from './lib/projectBootstrap'
import { Icon } from './icons'
import './dropzone.css'

const FIRST_STILL_DURATION_MS = 5_000
const FIRST_IMPORT_TIMEOUT_MS = 45_000

type ImportResult = { asset_id?: string; job_id?: string }

async function waitForImport(jobId: string): Promise<JobRecord | null> {
  const deadline = Date.now() + FIRST_IMPORT_TIMEOUT_MS
  for (;;) {
    const status = await callVerb('jobs.status', { job_id: jobId })
    if (!status.ok || !status.result) return null
    if (status.result.state === 'done' || status.result.state === 'failed') return status.result
    if (Date.now() >= deadline) return null
    await new Promise((resolve) => window.setTimeout(resolve, 250))
  }
}

export default function DropZone({
  project,
  onChanged,
  onProjectCreated,
}: {
  project: Project | null
  onChanged?: () => void
  onProjectCreated?: () => void
}) {
  const [over, setOver] = useState(false)
  const [msg, setMsg] = useState<string | null>(null)
  // Read latest project/onChanged via refs so we subscribe ONCE (re-subscribing
  // OS listeners on every project change risks duplicate/leaked handlers).
  const projectRef = useRef(project)
  projectRef.current = project
  const onChangedRef = useRef(onChanged)
  onChangedRef.current = onChanged
  const onProjectCreatedRef = useRef(onProjectCreated)
  onProjectCreatedRef.current = onProjectCreated
  const busyRef = useRef(false)
  const flashTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const flash = (t: string) => {
    setMsg(t)
    if (flashTimer.current) clearTimeout(flashTimer.current)
    flashTimer.current = setTimeout(() => setMsg(null), 3200)
  }

  useEffect(() => {
    if (!isTauri()) return
    const off = onFileDrop({
      onOver: () => setOver(true),
      onLeave: () => setOver(false),
      onPaths: async (paths) => {
        setOver(false)
        if (busyRef.current) {
          flash('Finish the current import before dropping more media')
          return
        }
        const unsupported = paths.filter((path) => !isSupportedMediaPath(path))
        const mediaPaths = paths.filter(isSupportedMediaPath)
        if (mediaPaths.length === 0) {
          flash(`Skipped ${unsupported.length} unsupported file${unsupported.length === 1 ? '' : 's'}`)
          return
        }
        busyRef.current = true
        let createdName: string | null = null
        let createdProject: Project | null = null
        let firstReady = true
        let firstPlaced = false
        let ok = 0
        let firstError: string | null = null
        try {
          if (!projectRef.current) {
            setMsg('Creating a project from your media…')
            const listed = await callVerb('project.list', { sort: 'recent' })
            const existing = listed.ok
              ? (listed.result?.projects ?? []).map((entry: ProjectEntry) => entry.name)
              : []
            const base = projectNameFromMediaPath(mediaPaths[0])
            let name = availableProjectName(base, existing)
            let created = await callVerb('project.create', { name })
            // A stale/reconciled index can miss a folder that already exists.
            // Retry deterministic suffixes only for that exact conflict.
            for (let attempt = 2; !created.ok && created.error?.code === 'conflict' && attempt <= 20; attempt++) {
              name = availableProjectName(`${base} ${attempt}`, existing)
              created = await callVerb('project.create', { name })
            }
            if (!created.ok || !created.result?.project) {
              flash(`Could not create project: ${created.error?.message ?? 'unknown error'}`)
              return
            }
            createdName = name
            createdProject = created.result.project
          }

          for (const [index, path] of mediaPaths.entries()) {
            setMsg(`${createdName ? `Creating “${createdName}”` : 'Importing media'} · ${index + 1}/${mediaPaths.length}`)
            const imported = await callVerb('media.import', {
              path,
              proxy: getGenerateProxies(),
              rationale: createdName ? 'drop-to-create project import' : 'drag-drop import',
            })
            if (!imported.ok) {
              firstError ??= imported.error?.message ?? imported.error?.code ?? 'import failed'
              if (createdName && index === 0) {
                firstReady = false
                break
              }
              continue
            }
            ok++

            // Only the first import needs a barrier. Its completed probe makes
            // the engine's first-clip auto-placement deterministic before later
            // imports start; it also lets us identify and place a still image.
            if (createdName && index === 0) {
              const result = imported.result as ImportResult | undefined
              const terminal = result?.job_id ? await waitForImport(result.job_id) : null
              firstReady = terminal?.state === 'done'
              if (!firstReady) {
                firstError ??= terminal?.error?.message ?? 'first media is still processing'
                break
              }
              if (createdProject && result?.asset_id && supportedMediaKind(path) === 'image') {
                const videoTrack = createdProject.tracks.find((track) => track.kind === 'video')?.id
                if (videoTrack) {
                  const inserted = await callVerb('edit.insert', {
                    asset: result.asset_id,
                    track: videoTrack,
                    at_ms: 0,
                    duration_ms: FIRST_STILL_DURATION_MS,
                    rationale: 'drop-to-create: first still becomes a five-second timeline',
                  })
                  firstPlaced = inserted.ok
                  if (!inserted.ok) firstError ??= inserted.error?.message ?? 'could not place the first image'
                }
              } else if (createdProject) {
                // Timed first media should be auto-placed by the completed import
                // chain. Read it back instead of claiming placement from success
                // alone (a malformed/zero-duration source can finish unplaced).
                const state = await callVerb('project.state', {})
                firstPlaced = !!state.result?.tracks.some((track) =>
                  track.clips.some((clip) => 'asset' in clip && clip.asset === result?.asset_id),
                )
                if (!firstPlaced) firstError ??= 'first media imported but was not placed on the timeline'
              }
            }
          }

          const skipped = unsupported.length > 0 ? ` · skipped ${unsupported.length} unsupported` : ''
          const partial = ok < mediaPaths.length || !firstReady || firstError
          if (createdName) {
            const placed = firstPlaced ? ' · first item added to timeline' : ''
            flash(
              `${partial ? 'Partially created' : 'Created'} “${createdName}” · imported ${ok}/${mediaPaths.length}${placed}${skipped}`
              + (firstError ? ` · ${firstError}` : ''),
            )
            onProjectCreatedRef.current?.()
          } else {
            flash(ok === mediaPaths.length
              ? `Imported ${ok} file${ok === 1 ? '' : 's'}${skipped}`
              : `Imported ${ok}/${mediaPaths.length}${skipped}${firstError ? ` · ${firstError}` : ''}`)
            onChangedRef.current?.()
          }
        } finally {
          busyRef.current = false
        }
      },
    })
    return () => {
      off()
      if (flashTimer.current) clearTimeout(flashTimer.current)
    }
  }, [])

  if (!over && !msg) return null
  return (
    <div
      className={`dz ${over ? 'dz--over' : ''}`}
      data-cut-dropzone={over ? 'over' : 'msg'}
      role="status"
      aria-live="polite"
    >
      <div className="dz__card">
        {msg ?? (
          <>
            <Icon name="import" size={18} tone="asset" />
            {project ? 'Drop media to import' : 'Drop media to start a project'}
          </>
        )}
      </div>
    </div>
  )
}
