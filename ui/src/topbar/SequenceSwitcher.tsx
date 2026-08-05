import { useEffect, useMemo, useRef, useState } from 'react'
import { Icon } from '../icons'
import { callVerb, type Project, type SequenceSummary } from '../lib/client'
import { confirmAction } from '../lib/tauri'
import { useTopbarDismissibleMenu } from './useTopbarDismissibleMenu'

interface SequenceSwitcherProps {
  project: Project
  onProjectChanged?: () => void
  onSequenceChanged?: () => void
}

export default function SequenceSwitcher({ project, onProjectChanged, onSequenceChanged }: SequenceSwitcherProps) {
  const [open, setOpen] = useState(false)
  const [sequences, setSequences] = useState<SequenceSummary[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const [createName, setCreateName] = useState('')
  const [createFrom, setCreateFrom] = useState<'empty' | 'active'>('empty')
  const [renaming, setRenaming] = useState<string | null>(null)
  const [renameName, setRenameName] = useState('')
  const rootRef = useRef<HTMLDivElement>(null)
  useTopbarDismissibleMenu(rootRef, open, setOpen)

  const activeId = project.active_sequence ?? 'seq1'
  const activeName = useMemo(() => {
    const persisted = project.sequences?.find((sequence) => sequence.id === activeId)?.name
    const listed = sequences.find((sequence) => sequence.id === activeId)?.name
    return listed ?? persisted ?? 'Main'
  }, [activeId, project.sequences, sequences])

  const refresh = async () => {
    const response = await callVerb('project.sequence_list', {})
    if (response.ok && response.result) {
      setSequences(response.result.sequences)
      setError(null)
    } else {
      setError(response.error?.message ?? 'Could not load sequences')
    }
  }

  useEffect(() => {
    if (open) void refresh()
  }, [open, activeId])

  const run = async (action: () => ReturnType<typeof callVerb>, activeChanged = false) => {
    setBusy(true)
    setError(null)
    try {
      const response = await action()
      if (!response.ok) {
        setError(response.error?.message ?? 'Sequence action failed')
        return false
      }
      if (activeChanged) onSequenceChanged?.()
      else onProjectChanged?.()
      await refresh()
      return true
    } catch {
      setError('Server unreachable')
      return false
    } finally {
      setBusy(false)
    }
  }

  const switchTo = async (id: string) => {
    if (id === activeId || busy) return
    if (await run(() => callVerb('project.sequence_switch', { id, rationale: 'user: switch sequence' }), true)) {
      setOpen(false)
    }
  }

  const create = async () => {
    const name = createName.trim()
    if (!name || busy) return
    if (await run(() => callVerb('project.sequence_create', {
      name,
      from: createFrom,
      rationale: `user: create ${createFrom} sequence`,
    }), true)) {
      setCreateName('')
      setCreating(false)
      setOpen(false)
    }
  }

  const commitRename = async (id: string) => {
    const name = renameName.trim()
    if (!name || busy) return
    if (await run(() => callVerb('project.sequence_rename', {
      id,
      name,
      rationale: 'user: rename sequence',
    }))) {
      setRenaming(null)
    }
  }

  const remove = async (sequence: SequenceSummary) => {
    if (busy || sequence.active) return
    if (!await confirmAction(
      `Delete sequence "${sequence.name}"? Shared media will be kept.`,
      { title: 'Delete sequence?', okLabel: 'Delete', cancelLabel: 'Keep' },
    )) return
    await run(() => callVerb('project.sequence_delete', {
      id: sequence.id,
      rationale: 'user: delete sequence',
    }))
  }

  return (
    <div className="tb-sequences" ref={rootRef} data-cut-sequences>
      <button
        type="button"
        className={`tb-sequence-trigger${open ? ' tb-sequence-trigger--open' : ''}`}
        data-cut-sequence-trigger
        aria-haspopup="menu"
        aria-expanded={open}
        title={`Sequence: ${activeName}`}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon name="layers" size={16} tone="brand" />
        <span data-cut-sequence-active={activeId}>{activeName}</span>
        <Icon name="chevronDown" size={14} />
      </button>

      {open && (
        <div className="tb-sequence-menu" role="menu" data-cut-sequence-menu>
          <div className="tb-sequence-head">
            <span>Sequences</span>
            <button
              type="button"
              className="tb-sequence-icon"
              data-cut-sequence-new
              title="New sequence"
              aria-label="New sequence"
              onClick={() => { setCreating(true); setRenaming(null); setError(null) }}
            >
              <Icon name="plus" size={16} />
            </button>
          </div>

          <div className="tb-sequence-list" data-cut-sequence-list>
            {sequences.map((sequence) => (
              <div
                className={`tb-sequence-row${sequence.active ? ' tb-sequence-row--active' : ''}`}
                data-cut-sequence-row={sequence.id}
                key={sequence.id}
              >
                {renaming === sequence.id ? (
                  <form
                    className="tb-sequence-rename"
                    onSubmit={(event) => { event.preventDefault(); void commitRename(sequence.id) }}
                  >
                    <input
                      autoFocus
                      value={renameName}
                      maxLength={80}
                      data-cut-sequence-rename-input={sequence.id}
                      onChange={(event) => setRenameName(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === 'Escape') setRenaming(null)
                      }}
                    />
                    <button type="submit" data-cut-sequence-rename-save={sequence.id} disabled={!renameName.trim() || busy} aria-label="Save sequence name">
                      <Icon name="check" size={14} />
                    </button>
                  </form>
                ) : (
                  <>
                    <button
                      type="button"
                      className="tb-sequence-main"
                      role="menuitemradio"
                      aria-checked={sequence.active}
                      disabled={busy}
                      data-cut-sequence-switch={sequence.id}
                      onClick={() => void switchTo(sequence.id)}
                    >
                      <span className="tb-sequence-name">{sequence.name}</span>
                      <span className="tb-sequence-meta">
                        {sequence.clip_count} clip{sequence.clip_count === 1 ? '' : 's'}
                      </span>
                      {sequence.active && <Icon name="check" size={14} tone="brand" />}
                    </button>
                    <button
                      type="button"
                      className="tb-sequence-icon"
                      data-cut-sequence-rename={sequence.id}
                      title={`Rename ${sequence.name}`}
                      aria-label={`Rename ${sequence.name}`}
                      disabled={busy}
                      onClick={() => { setRenaming(sequence.id); setRenameName(sequence.name); setCreating(false) }}
                    >
                      <Icon name="edit" size={14} />
                    </button>
                    <button
                      type="button"
                      className="tb-sequence-icon tb-sequence-icon--danger"
                      data-cut-sequence-delete={sequence.id}
                      title={sequence.active ? 'Switch sequences before deleting this one' : `Delete ${sequence.name}`}
                      aria-label={`Delete ${sequence.name}`}
                      disabled={busy || sequence.active}
                      onClick={() => void remove(sequence)}
                    >
                      <Icon name="trash" size={14} />
                    </button>
                  </>
                )}
              </div>
            ))}
          </div>

          {creating && (
            <form
              className="tb-sequence-create"
              data-cut-sequence-create
              onSubmit={(event) => { event.preventDefault(); void create() }}
            >
              <input
                autoFocus
                value={createName}
                maxLength={80}
                placeholder="Sequence name"
                data-cut-sequence-name
                onChange={(event) => setCreateName(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Escape') setCreating(false)
                }}
              />
              <div className="tb-sequence-source" aria-label="Starting point">
                <button
                  type="button"
                  className={createFrom === 'empty' ? 'is-selected' : ''}
                  data-cut-sequence-from="empty"
                  onClick={() => setCreateFrom('empty')}
                >Empty</button>
                <button
                  type="button"
                  className={createFrom === 'active' ? 'is-selected' : ''}
                  data-cut-sequence-from="active"
                  onClick={() => setCreateFrom('active')}
                >Duplicate</button>
              </div>
              <div className="tb-sequence-create-actions">
                <button type="button" data-cut-sequence-create-cancel onClick={() => setCreating(false)}>Cancel</button>
                <button type="submit" className="is-primary" data-cut-sequence-create-submit disabled={!createName.trim() || busy}>Create</button>
              </div>
            </form>
          )}

          {sequences.length === 0 && !error && <div className="tb-sequence-status">Loading…</div>}
          {error && <div className="tb-sequence-error" role="alert" data-cut-sequence-error>{error}</div>}
        </div>
      )}
    </div>
  )
}
