import type { AgentChatPrefill, ChatEvidenceAttachment } from '../../lib/evidenceAttachments'

/**
 * A null prefill is the App's acknowledgement that Chat consumed a one-shot
 * handoff. It must not clear evidence that the mounted composer already owns.
 */
export function nextEvidenceAttachments(
  current: ChatEvidenceAttachment[],
  prefill: AgentChatPrefill | null | undefined,
): ChatEvidenceAttachment[] {
  return prefill ? (prefill.evidence ?? []) : current
}
