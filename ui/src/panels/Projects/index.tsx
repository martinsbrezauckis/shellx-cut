// panels/Projects — the recent-projects LEFT-SIDEBAR TAB: open projects in
// the left sidebar as a tab"). Lists recent projects (project.list), reopens any one
// FULLY as it was (project.open → App's project-switch hard reset), with new-project +
// forget + search. Replaces the old right-side Projects drawer; "New project" lives
// here now (the topbar "New" button was removed — create is under Projects).
//
// Missing projects show greyed + un-clickable but are NOT
// auto-removed; Forget drops the index entry only (never deletes files).
//
// Callers: panels/LeftPanel (the 'projects' tab). Deps: lib/client, ./projects.css.

import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb, type ProjectEntry } from '../../lib/client'
import { confirmAction, showMessage } from '../../lib/tauri'
import { Icon } from '../../icons'
import './projects.css'

export interface ProjectsPanelProps {
  /** Hard-reset + reload after a reopen/create (App.onProjectSwitched). */
  onReopen: () => void
  /** Display name of the currently-open project (to badge it in the list). */
  currentName: string | null
  /** True when this tab is the active one (drives a refresh on (re)activation). */
  active: boolean
}

/** ms → "3m04s" / "45s" (whole-project duration). */
function fmtDur(ms?: number): string {
  if (!ms) return ''
  const s = Math.round(ms / 1000)
  return s >= 60 ? `${Math.floor(s / 60)}m${String(s % 60).padStart(2, '0')}s` : `${s}s`
}

/** A last-opened timestamp → a short relative label ("just now", "2h ago"). */
function fmtAgo(ms: number): string {
  if (!ms) return ''
  const sec = Math.max(0, (Date.now() - ms) / 1000)
  if (sec < 60) return 'just now'
  const min = sec / 60
  if (min < 60) return `${Math.floor(min)}m ago`
  const hr = min / 60
  if (hr < 24) return `${Math.floor(hr)}h ago`
  const day = hr / 24
  if (day < 30) return `${Math.floor(day)}d ago`
  return `${Math.floor(day / 30)}mo ago`
}

function explainProjectError(
  error: { code?: string; message?: string; suggested_action?: string } | null | undefined,
  fallback: string,
): string {
  const message = error?.message ?? error?.code ?? fallback
  return [message, error?.suggested_action].filter(Boolean).join(' · ')
}

export default function ProjectsPanel({ onReopen, currentName, active }: ProjectsPanelProps) {
  const [loading, setLoading] = useState(true)
  const [projects, setProjects] = useState<ProjectEntry[]>([])
  const [err, setErr] = useState<string | null>(null)
  const [q, setQ] = useState('')
  const [busy, setBusy] = useState(false)
  const [newName, setNewName] = useState('')
  const newRef = useRef<HTMLInputElement | null>(null)

  const load = useCallback(async () => {
    setLoading(true)
    setErr(null)
    const r = await callVerb('project.list', { sort: 'recent' })
    if (r.ok && r.result) setProjects(r.result.projects)
    else setErr(explainProjectError(r.error, 'Could not load projects'))
    setLoading(false)
  }, [])
  // Refresh whenever the tab becomes active (a new project may have been created/opened).
  useEffect(() => {
    if (active) void load()
  }, [active, load])

  const reopen = useCallback(
    async (p: ProjectEntry) => {
      if (p.missing || busy) return
      setBusy(true)
      setErr(null)
      const r = await callVerb('project.open', { path: p.path })
      if (r.ok) onReopen()
      else setErr(explainProjectError(r.error, 'Could not open the project'))
      setBusy(false)
    },
    [busy, onReopen],
  )

  const create = useCallback(async () => {
    const name = newName.trim()
    if (!name || busy) return
    setBusy(true)
    setErr(null)
    // Omit technical format settings: the first video adopts its own geometry
    // and frame rate. Experts can deliberately change the timeline later.
    const r = await callVerb('project.create', { name })
    if (r.ok) {
      setNewName('')
      onReopen()
    } else {
      setErr(explainProjectError(r.error, 'Could not create the project'))
    }
    setBusy(false)
  }, [newName, busy, onReopen])

  // Forget drops the index entry only - never deletes files on disk.
  const forget = useCallback(
    async (p: ProjectEntry, e: React.MouseEvent) => {
      e.stopPropagation()
      await callVerb('project.forget', { id: p.id })
      void load()
    },
    [load],
  )

  // Bulk registry hygiene: forget EVERY entry whose .cutproj is gone from disk
  // (the server re-checks each path at call time - a project on a remounted
  // drive survives). Never deletes files; only the greyed-out ghosts vanish.
  const clearMissing = useCallback(async () => {
    if (busy) return
    setBusy(true)
    await callVerb('project.forget', { missing: true })
    setBusy(false)
    void load()
  }, [busy, load])

  // Delete PERMANENTLY removes the ShellX Cut project folder. Confirm
  // first - it's destructive. Source media linked from outside the project is never
  // touched by the verb; the server also refuses to delete the currently-open one.
  const del = useCallback(
    async (p: ProjectEntry, e: React.MouseEvent) => {
      e.stopPropagation()
      if (!await confirmAction(
        `Delete "${p.name}" permanently?\n\nThis removes the ShellX Cut project from this machine. Original media files are not deleted. This cannot be undone.`,
        { title: 'Delete project permanently?', okLabel: 'Delete project', cancelLabel: 'Keep project' },
      )) return
      const r = await callVerb('project.delete', { id: p.id })
      if (!r.ok) await showMessage(
        explainProjectError(r.error, 'Could not delete the project'),
        { title: 'Project was not deleted', kind: 'error' },
      )
      void load()
    },
    [load],
  )

  const needle = q.trim().toLowerCase()
  const shown = needle ? projects.filter((p) => p.name.toLowerCase().includes(needle)) : projects
  const missingCount = projects.filter((p) => p.missing).length

  return (
    <section className="panel projects-panel" data-cut-panel="projects" data-cut-projects>
      <div className="panel__header">
        <span>Projects</span>
        <span className="pj-header-spacer" />
        {missingCount > 0 && (
          <button
            className="pj-clear-missing"
            data-cut-projects-clear-missing={missingCount}
            disabled={busy}
            title="Remove the greyed-out entries whose project folder no longer exists. Nothing on disk is deleted; a project on a disconnected drive comes back once the drive does."
            onClick={() => void clearMissing()}
          >
            Clear missing ({missingCount})
          </button>
        )}
        <span className="pj-count" data-cut-projects-count={projects.length}>
          {projects.length}
        </span>
      </div>
      <div className="panel__body pj-body" data-cut-projects-list>
        {/* New project */}
        <div className="pj-new" data-cut-projects-new>
          <input
            ref={newRef}
            className="pj-input"
            data-cut-projects-newname
            placeholder="New project name…"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void create()
            }}
          />
          <button
            className="pj-create-btn"
            data-cut-projects-create
            disabled={!newName.trim() || busy}
            onClick={() => void create()}
          >
            Create project
          </button>
        </div>

        <div className="pj-auto-format" data-cut-projects-auto-format>
          <Icon name="info" size={14} tone="brand" />
          <span>
            <strong>Video format is automatic.</strong> Your first video sets the
            timeline size and frame rate. Delivery size and quality are chosen when rendering.
          </span>
        </div>

        {projects.length > 4 && (
          <input
            className="pj-search"
            data-cut-projects-search
            placeholder="Search projects…"
            value={q}
            onChange={(e) => setQ(e.target.value)}
          />
        )}

        {loading && <div className="pj-empty" data-cut-projects-loading>Loading projects…</div>}
        {err && <div className="pj-err" data-cut-projects-error role="alert">{err}</div>}
        {!loading && !err && shown.length === 0 && (
          <div className="pj-empty" data-cut-projects-empty>
            {projects.length === 0
              ? 'No projects yet — name one above, or drop a video or image anywhere to start.'
              : 'No projects match your search.'}
          </div>
        )}

	        <div className="pj-list">
	          {shown.map((p) => {
	            const isCurrent = currentName != null && p.name === currentName
	            return (
	              <div
	                className={`pj-card${p.missing ? ' pj-card--missing' : ''}${isCurrent ? ' pj-card--current' : ''}`}
	                data-cut-project-card={p.id}
	                data-cut-project-missing={p.missing ? 'true' : 'false'}
	                data-cut-project-disabled={p.missing || busy ? 'true' : 'false'}
	                key={p.id}
	              >
	                <button
	                  type="button"
	                  className="pj-open"
	                  data-cut-project-open={p.id}
	                  disabled={p.missing || busy}
	                  title={p.missing ? `${p.name} is missing. Remove it from this list or locate the project file.` : `Reopen ${p.name}`}
	                  onClick={() => void reopen(p)}
	                >
	                  <div className="pj-card-name" data-cut-project-name>
	                    {p.name}
	                    {isCurrent && <span className="pj-badge pj-badge--current">open</span>}
	                    {p.missing && <span className="pj-badge pj-badge--missing">missing</span>}
	                  </div>
	                  <div className="pj-card-meta" data-cut-project-meta>
	                    {fmtAgo(p.last_opened_ms)}
	                    {p.duration_ms ? ` · ${fmtDur(p.duration_ms)}` : ''}
	                    {p.clip_count != null ? ` · ${p.clip_count} clip${p.clip_count === 1 ? '' : 's'}` : ''}
	                  </div>
	                </button>
	                <button
	                  type="button"
	                  className="pj-forget"
	                  data-cut-project-forget={p.id}
                  title="Remove from this list; files stay on disk"
                  onClick={(e) => void forget(p, e)}
                >
                  <Icon name="close" size={14} label="Remove from list" />
                </button>
                <button
                  type="button"
                  className="pj-delete"
                  data-cut-project-delete={p.id}
                  title="Delete project from this machine; original media files are not deleted"
                  onClick={(e) => void del(p, e)}
                >
                  <Icon name="trash" size={14} label="Delete project" />
                </button>
              </div>
            )
          })}
        </div>
      </div>
    </section>
  )
}
