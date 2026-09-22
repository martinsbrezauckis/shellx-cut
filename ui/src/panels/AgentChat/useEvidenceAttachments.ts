import { useCallback, useEffect, useState } from 'react'
import type { AgentChatPrefill, ChatEvidenceAttachment } from '../../lib/evidenceAttachments'
import { nextEvidenceAttachments } from './evidencePrefill'

export function useEvidenceAttachments(prefill: AgentChatPrefill | null | undefined) {
  const [selected, setSelected] = useState<ChatEvidenceAttachment[]>([])

  useEffect(() => {
    setSelected((current) => nextEvidenceAttachments(current, prefill))
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
