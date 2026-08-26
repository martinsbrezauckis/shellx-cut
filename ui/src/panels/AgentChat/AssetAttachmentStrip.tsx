import { Icon } from '../../icons'

interface AssetAttachment {
  id: string
  label: string
}

interface AssetAttachmentStripProps {
  attachments: AssetAttachment[]
  busy?: boolean
  turn?: boolean
  onRemove?: (id: string) => void
}

export default function AssetAttachmentStrip({ attachments, busy = false, turn = false, onRemove }: AssetAttachmentStripProps) {
  if (!attachments.length) return null
  if (turn) {
    return (
      <div className="chat__turn-attachments">
        {attachments.map((attachment) => (
          <span key={attachment.id} className="chat__turn-attachment" data-cut-chat-turn-attachment={attachment.id} title={attachment.id}>
            <Icon name="attach" size={14} />
            {attachment.label}
          </span>
        ))}
      </div>
    )
  }
  return (
    <div className="chat__attachments" data-cut-chat-attachments={attachments.length}>
      {attachments.map((attachment) => (
        <span key={attachment.id} className="chat__attachment-chip" title={attachment.id}>
          <span>{attachment.label}</span>
          <button type="button" data-cut-chat-attachment-remove={attachment.id} aria-label={`Remove ${attachment.label}`} disabled={busy} onClick={() => onRemove?.(attachment.id)}>
            <Icon name="close" size={14} />
          </button>
        </span>
      ))}
    </div>
  )
}
