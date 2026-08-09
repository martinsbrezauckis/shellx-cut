import { useEffect, useRef, useState } from 'react'
import { Icon } from '../../icons'
import { MAX_CHAT_ATTACHMENTS, type ChatAttachmentOption } from './attachmentModel'

interface AttachmentPickerProps {
  options: ChatAttachmentOption[]
  selected: string[]
  disabled: boolean
  onToggle: (id: string) => void
}

export default function AttachmentPicker({ options, selected, disabled, onToggle }: AttachmentPickerProps) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const onDown = (event: MouseEvent) => {
      if (!(event.target instanceof Node) || !rootRef.current?.contains(event.target)) setOpen(false)
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.preventDefault()
      event.stopPropagation()
      setOpen(false)
      rootRef.current?.querySelector<HTMLButtonElement>('[data-cut-chat-attach]')?.focus()
    }
    document.addEventListener('mousedown', onDown)
    document.addEventListener('keydown', onKey, true)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('keydown', onKey, true)
    }
  }, [open])

  useEffect(() => {
    if (disabled || options.length === 0) setOpen(false)
  }, [disabled, options.length])

  const atLimit = selected.length >= MAX_CHAT_ATTACHMENTS

  return (
    <div className="chat__attachment-picker" ref={rootRef}>
      <button
        type="button"
        className={`chat__attach ${open ? 'chat__attach--open' : ''}`}
        data-cut-chat-attach
        aria-label="Attach project asset"
        aria-haspopup="listbox"
        aria-expanded={open}
        disabled={disabled || options.length === 0}
        title={options.length === 0 ? 'Import an asset before attaching it' : 'Attach project asset'}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon name="attach" size={16} />
      </button>
      {open && (
        <div className="chat__attachment-menu" data-cut-chat-attachment-menu>
          <div className="chat__attachment-menu-head">
            <span>Project assets</span>
            <span>{selected.length}/{MAX_CHAT_ATTACHMENTS}</span>
          </div>
          <div className="chat__attachment-options" role="listbox" aria-label="Project assets" aria-multiselectable="true">
            {options.map((option) => {
              const isSelected = selected.includes(option.id)
              return (
                <button
                  key={option.id}
                  type="button"
                  role="option"
                  aria-selected={isSelected}
                  className={`chat__attachment-option ${isSelected ? 'chat__attachment-option--selected' : ''}`}
                  data-cut-chat-attachment={option.id}
                  disabled={atLimit && !isSelected}
                  title={option.label}
                  onClick={() => onToggle(option.id)}
                >
                  <Icon name={isSelected ? 'check' : 'file'} size={14} />
                  <span>{option.label}</span>
                </button>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}
