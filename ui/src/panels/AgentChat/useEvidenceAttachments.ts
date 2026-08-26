import { useCallback, useEffect, useState } from 'react'
import type { AgentChatPrefill, ChatEvidenceAttachment } from '../../lib/evidenceAttachments'

export function useEvidenceAttachments(prefill: AgentChatPrefill | null | undefined) {
  const [selected, setSelected] = useState<ChatEvidenceAttachment[]>([])

  useEffect(() => {
    setSelected(prefill?.evidence ?? [])
  }, [prefill?.nonce, prefill?.evidence])

  const clear = useCallback(() => setSelected([]), [])
  const remove = useCallback((evidenceId: string) => {
    setSelected((current) => current.filter((attachment) => attachment.evidence_id !== evidenceId))
  }, [])
  const restore = useCallback((attachments: ChatEvidenceAttachment[]) => {
    setSelected(attachments)
  }, [])

  return { selected, clear, remove, restore }
}
