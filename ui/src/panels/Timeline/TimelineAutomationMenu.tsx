import { useEffect, useRef, useState } from 'react'
import type { Project } from '../../lib/client'
import { Icon } from '../../icons'
import TimelineGlobalTools from './TimelineGlobalTools'

interface TimelineAutomationMenuProps {
  project: Project | null
  selectedMediaCount: number
  hasBeatMarkers: boolean
  canMulticam: boolean
  syncNote: string | null
  onSyncByAudio: () => void | Promise<void>
  onMulticamSwitch: () => void | Promise<void>
  onCutToBeat: () => void | Promise<void>
}

export default function TimelineAutomationMenu({
  project,
  selectedMediaCount,
  hasBeatMarkers,
  canMulticam,
  syncNote,
  onSyncByAudio,
  onMulticamSwitch,
  onCutToBeat,
}: TimelineAutomationMenuProps) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const onDown = (event: MouseEvent) => {
      if (!(event.target instanceof Node) || !rootRef.current?.contains(event.target)) setOpen(false)
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      setOpen(false)
      triggerRef.current?.focus()
    }
    window.addEventListener('mousedown', onDown)
    window.addEventListener('keydown', onKey)
    const focusFrame = window.requestAnimationFrame(() => {
      menuRef.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
    })
    return () => {
      window.cancelAnimationFrame(focusFrame)
      window.removeEventListener('mousedown', onDown)
      window.removeEventListener('keydown', onKey)
    }
  }, [open])

  const runAndClose = (action: () => void | Promise<void>) => {
    setOpen(false)
    void action()
  }

  const moveMenuFocus = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return
    const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')]
    if (!items.length) return
    event.preventDefault()
    const current = items.indexOf(document.activeElement as HTMLButtonElement)
    const next = event.key === 'Home'
      ? 0
      : event.key === 'End'
        ? items.length - 1
        : event.key === 'ArrowUp'
          ? (current <= 0 ? items.length - 1 : current - 1)
          : (current + 1) % items.length
    items[next]?.focus()
  }

  return (
    <div className="tl-automation" ref={rootRef} data-cut-timeline-automation>
      <button
        ref={triggerRef}
        type="button"
        className={`tl-tool tl-automation__trigger${open ? ' tl-tool--on' : ''}`}
        data-cut-timeline-automation-trigger
        aria-haspopup="menu"
        aria-expanded={open}
        title="Timeline automation - sync, multicam, beat, silence, and scene tools"
        onClick={() => setOpen((value) => !value)}
      >
        <Icon name="autopilot" size={14} />
        Automate
        <Icon name="chevronDown" size={14} className="tl-automation__caret" />
      </button>
      <div
        ref={menuRef}
        className="tl-automation__menu"
        data-cut-timeline-automation-menu
        role="menu"
        hidden={!open}
        onKeyDown={moveMenuFocus}
      >
        <button
          type="button"
          className="tl-tool"
          role="menuitem"
          data-cut-action="sync-by-audio"
          disabled={selectedMediaCount < 2}
          title="Align two or more recordings of the same event by matching their audio"
          onClick={() => runAndClose(onSyncByAudio)}
        >
          <Icon name="waveform" size={14} />
          Sync by audio
        </button>
        <button
          type="button"
          className="tl-tool"
          role="menuitem"
          data-cut-action="multicam-switch"
          disabled={!canMulticam}
          title={canMulticam
            ? 'Cut a program track to the loudest active speaker across synced camera angles'
            : 'Needs at least 2 video tracks holding clips - add an angle on its own track'}
          onClick={() => runAndClose(onMulticamSwitch)}
        >
          <Icon name="split" size={14} />
          Auto multicam
        </button>
        <button
          type="button"
          className="tl-tool"
          role="menuitem"
          data-cut-action="cut-to-beat"
          disabled={!hasBeatMarkers}
          title={hasBeatMarkers
            ? 'Split the video track on each music beat for a beat-synced montage'
            : 'Add a music bed with beat markers first'}
          onClick={() => runAndClose(onCutToBeat)}
        >
          <Icon name="marker" size={14} />
          Cut to beat
        </button>
        <div className="tl-automation__sep" role="separator" />
        <TimelineGlobalTools project={project} />
        {syncNote && <span className="tl-automation__note" data-cut-sync-note>{syncNote}</span>}
      </div>
    </div>
  )
}
