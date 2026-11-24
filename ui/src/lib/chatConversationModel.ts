export type ChatConversationOrigin = 'agent' | 'human' | 'unknown'
export type ChatTurnStatus = 'ok' | 'error' | 'interrupted' | 'running'

export interface ChatActionRef {
  op_id: string
  verb: string
}

export interface ChatTurnRecord {
  id: string
  role: 'user' | 'agent'
  text: string
  created_at?: string
  origin?: 'agent' | 'human'
  status?: ChatTurnStatus
  agent?: string
  model?: string
  error?: string
  actions?: ChatActionRef[]
  attachments?: string[]
  evidence_ids?: string[]
  evidence_index_id?: string | null
  target?: unknown
  plan?: unknown
  review?: unknown
  cost_usd?: number | null
  reverted?: boolean
}

export interface ChatConversationSummary {
  id: string
  title: string
  origin: ChatConversationOrigin
  readable: boolean
  unreadable_reason: string | null
  running: boolean
  turn_count: number
  created_at: string | null
  updated_at: string | null
  last_preview: string | null
}

export interface ChatConversationView {
  id: string
  title: string
  origin: ChatConversationOrigin
  readable: boolean
  unreadable_reason: string | null
  running: boolean
  created_at: string | null
  updated_at: string | null
  turns: ChatTurnRecord[]
}

export interface AgentChatListResult {
  conversations: ChatConversationSummary[]
  truncated?: boolean
  not_shown?: number
  note?: string
}

export interface AgentChatGetResult {
  conversation: ChatConversationView
}

export interface AgentChatMigrateResult {
  migrated: number
  conversation_ids: string[]
}

export interface AgentChatPatchResult {
  updated: boolean
}
