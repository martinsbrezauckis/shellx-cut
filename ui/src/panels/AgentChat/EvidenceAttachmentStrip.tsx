import type { ChatEvidenceAttachment } from '../../lib/evidenceAttachments'
import { Icon } from '../../icons'

interface EvidenceAttachmentStripProps {
  attachments: ChatEvidenceAttachment[]
  busy?: boolean
  turn?: boolean
  onRemove?: (evidenceId: string) => void
}

export default function EvidenceAttachmentStrip({ attachments, busy = false, turn = false, onRemove }: EvidenceAttachmentStripProps) {
  if (!attachments.length) return null
  if (turn) {
    return (
      <div className="chat__turn-attachments" data-cut-chat-turn-evidence-count={attachments.length}>
        {attachments.map((attachment) => (
          <span key={attachment.evidence_id} className="chat__turn-attachment" data-cut-chat-turn-evidence={attachment.evidence_id} title={attachment.evidence_id}>
            <Icon name="marker" size={14} />
            {attachment.label}
          </span>
        ))}
      </div>
    )
  }
  return (
    <div className="chat__attachments" data-cut-chat-evidence-attachments={attachments.length}>
      {attachments.map((attachment) => (
        <span key={attachment.evidence_id} className="chat__attachment-chip" data-cut-chat-evidence={attachment.evidence_id} title={`${attachment.evidence_id} · ${attachment.index_id}`}>
          <Icon name="marker" size={14} />
          <span>{attachment.label}</span>
          <button type="button" data-cut-chat-evidence-remove={attachment.evidence_id} aria-label={`Remove cited moment ${attachment.label}`} disabled={busy} onClick={() => onRemove?.(attachment.evidence_id)}>
            <Icon name="close" size={14} />
          </button>
        </span>
      ))}
    </div>
  )
}
