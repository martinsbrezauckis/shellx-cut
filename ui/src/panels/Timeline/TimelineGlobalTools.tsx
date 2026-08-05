import { useMemo, useRef, useState } from 'react'
import { callVerb, type Project, type VerbResult } from '../../lib/client'
import { Icon } from '../../icons'

type ToolAction = 'trim_edges' | 'split_scenes' | 'mark_scenes'

const TOOL_ACTIONS: { id: ToolAction; label: string; title: string }[] = [
  {
    id: 'trim_edges',
    label: 'Trim dead air',
    title: 'Trim dead air from the top and tail of the timeline',
  },
  {
    id: 'split_scenes',
    label: 'Split scenes',
    title: 'Split the first imported video at detected scene cuts',
  },
  {
    id: 'mark_scenes',
    label: 'Mark scenes',
    title: 'Add timeline markers at detected scene cuts',
  },
]

function resultNumber(result: unknown, key: string): number {
  if (!result || typeof result !== 'object') return 0
  const value = (result as Record<string, unknown>)[key]
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

function toolResultMessage(id: ToolAction, r: VerbResult<unknown>): string {
  if (!r.ok) return `${id}: ${r.error?.message ?? r.error?.code ?? 'failed'}`
  if (id === 'split_scenes') {
    const splits = resultNumber(r.result, 'splits')
    const sceneCuts = resultNumber(r.result, 'scene_cuts')
    if (splits > 0) return `Split at scenes: ${splits} split${splits === 1 ? '' : 's'}`
    if (sceneCuts > 0) return 'Split at scenes: scene cuts were outside the current clip'
    return 'Split at scenes: no scene cuts found'
  }
  if (id === 'mark_scenes') {
    const markers = resultNumber(r.result, 'markers_added')
    const sceneCuts = resultNumber(r.result, 'scene_cuts')
    if (markers > 0) return `Mark scenes: ${markers} marker${markers === 1 ? '' : 's'} added`
    if (sceneCuts > 0) return 'Mark scenes: scene cuts were outside the current clip'
    return 'Mark scenes: no scene cuts found'
  }
  const trimmed = resultNumber(r.result, 'leading_trimmed_ms') + resultNumber(r.result, 'trailing_trimmed_ms')
  return trimmed > 0 ? `Trim dead air: trimmed ${trimmed}ms` : 'Trim dead air: nothing trimmed'
}

export default function TimelineGlobalTools({ project }: { project: Project | null }) {
  const firstAsset = useMemo(() => (project ? Object.keys(project.assets ?? {})[0] : undefined), [project])
  const [busy, setBusy] = useState<ToolAction | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const noteTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const flash = (message: string) => {
    setNote(message)
    if (noteTimer.current) clearTimeout(noteTimer.current)
    noteTimer.current = setTimeout(() => setNote(null), 3500)
  }

  const runTool = async (id: ToolAction) => {
    setBusy(id)
    try {
      let r: VerbResult<unknown>
      if (id === 'trim_edges') r = await callVerb('edit.trim_edges', {})
      else if (!firstAsset) {
        flash('No asset imported')
        return
      } else if (id === 'split_scenes') r = await callVerb('edit.split_at_scenes', { asset: firstAsset })
      else r = await callVerb('edit.mark_scenes', { asset: firstAsset })
      flash(toolResultMessage(id, r))
    } catch {
      flash(`${id}: server unreachable`)
    } finally {
      setBusy(null)
    }
  }

  return (
    <span className="tl-global-tools" data-cut-timeline-tools>
      {TOOL_ACTIONS.map((tool) => (
        <button
          key={tool.id}
          type="button"
          className="tl-tool"
          role="menuitem"
          data-cut-tool={tool.id}
          disabled={!project || busy !== null}
          title={tool.title}
          onClick={() => void runTool(tool.id)}
        >
          <Icon name={tool.id === 'trim_edges' ? 'rippleDelete' : 'marker'} size={14} />
          {busy === tool.id ? 'Working...' : tool.label}
        </button>
      ))}
      {note && <span className="tl-save-note" data-cut-timeline-tools-note>{note}</span>}
    </span>
  )
}
