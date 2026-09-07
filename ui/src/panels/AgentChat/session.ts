import type { ChatEvidenceAttachment } from '../../lib/evidenceAttachments'
import type { VerbResults } from '../../lib/client'

type ChatResult = VerbResults['agent.chat']

export interface AgentChatTurn {
  /** Stable across bounded-log eviction; async review patches never use an index. */
  id: string
  role: 'user' | 'agent'
  text: string
  ok?: boolean
  agent?: string | null
  actions?: Array<{ op_id: string; verb: string }>
  cost?: number | null
  /** Error-transparency fields (ok:false path) — the machine category + the
   * agent's own final words, rendered inline so a failure is never swallowed. */
  errorKind?: string | null
  agentMessage?: string | null
  attachments?: Array<{ id: string; label: string }>
  evidence?: ChatEvidenceAttachment[]
  request?: string
  requestAttachments?: Array<{ id: string; label: string }>
  requestEvidence?: ChatEvidenceAttachment[]
  projectName?: string
  plan?: ChatResult['plan']
  review?: ChatResult['review']
  reviewState?: 'pending' | 'accepted' | 'reverted' | 'retry'
  reviewBusy?: boolean
  reviewError?: string | null
}

/** Right-rail tabs mount independently. Keep the bounded, project-keyed state
 * above the tab so review/revert receipts survive a tab switch without making a
 * project-persistent chat journal. */
export interface AgentChatSession {
  log: AgentChatTurn[]
  input: string
  attachments: string[]
  busy: boolean
}

export const MAX_AGENT_CHAT_TURNS = 40

export function emptyAgentChatSession(): AgentChatSession {
  return { log: [], input: '', attachments: [], busy: false }
}

export function boundedAgentChatTurns(turns: AgentChatTurn[]): AgentChatTurn[] {
  return turns.length <= MAX_AGENT_CHAT_TURNS ? turns : turns.slice(-MAX_AGENT_CHAT_TURNS)
}

/** Patch by the turn's receipt identity, never its rendered array index. A
 * capped log may evict older turns while an async revert is in flight. */
export function patchAgentChatTurn(
  turns: AgentChatTurn[],
  turnId: string,
  patch: Partial<AgentChatTurn>,
): AgentChatTurn[] {
  return turns.map((turn) => turn.id === turnId ? { ...turn, ...patch } : turn)
}
