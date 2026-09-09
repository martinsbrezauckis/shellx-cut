import type { ChatEvidenceAttachment } from '../../lib/evidenceAttachments'
import type { ProjectIdentity, VerbResults } from '../../lib/client'
import type { ChatTimelineTarget } from '../../lib/chatTimelineTarget'

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
  projectIdentity?: ProjectIdentity
  /** Target returned with the executed request/result receipt. */
  target?: ChatTimelineTarget
  /** Immutable request target, retained so replacement can re-resolve it. */
  requestTarget?: ChatTimelineTarget
  plan?: ChatResult['plan']
  review?: ChatResult['review']
  reviewState?: 'reverted' | 'replacement'
  reviewBusy?: boolean
  reviewError?: string | null
}

/** Right-rail tabs mount independently. Keep their active, project-keyed state
 * above the tab; `history.ts` separately persists validated completed context
 * on this device so a reload can reconstruct the conversation safely. */
export interface AgentChatSession {
  log: AgentChatTurn[]
  input: string
  attachments: string[]
  /** Target waiting in the existing composer; it is never inferred at Send. */
  target: ChatTimelineTarget | null
  busy: boolean
}

export const MAX_AGENT_CHAT_TURNS = 40

export function emptyAgentChatSession(): AgentChatSession {
  return { log: [], input: '', attachments: [], target: null, busy: false }
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
