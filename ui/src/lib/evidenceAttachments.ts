import type { MediaEvidenceHit } from './mediaIntelligenceModel'
import type { Project } from './clientModel'

export const MAX_CHAT_EVIDENCE_ATTACHMENTS = 12

export interface ChatEvidenceAttachment {
  evidence_id: string
  index_id: string
  label: string
}

export interface AgentChatPrefill {
  prompt: string
  nonce: number
  evidence?: ChatEvidenceAttachment[]
}

const basename = (path: string | undefined, fallback: string): string =>
  path?.split(/[\\/]/).filter(Boolean).pop() || fallback

const shortTime = (ms: number): string => {
  const total = Math.max(0, Math.floor(ms / 1000))
  return `${Math.floor(total / 60)}:${(total % 60).toString().padStart(2, '0')}`
}

export function evidenceChatAttachments(
  hits: MediaEvidenceHit[],
  indexId: string | null,
  project: Project | null,
): ChatEvidenceAttachment[] {
  if (!indexId) return []
  return hits.slice(0, MAX_CHAT_EVIDENCE_ATTACHMENTS).map((hit) => ({
    evidence_id: hit.evidence_id,
    index_id: indexId,
    label: `${basename(project?.assets?.[hit.asset_id]?.path, hit.asset_id)} · ${shortTime(hit.anchor_ms)}`,
  }))
}

export function evidenceAttachmentIdentity(
  attachments: ChatEvidenceAttachment[],
): { evidence_ids?: string[]; evidence_index_id?: string } {
  if (!attachments.length) return {}
  const indexId = attachments[0].index_id
  if (!indexId || attachments.some((attachment) => attachment.index_id !== indexId)) return {}
  return {
    evidence_ids: attachments.map((attachment) => attachment.evidence_id),
    evidence_index_id: indexId,
  }
}
