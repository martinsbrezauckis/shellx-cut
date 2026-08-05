// lib/chatAgentPref.ts — persisted agent-selection for the AgentChat panel.
//
// The chat panel lets the user choose WHICH coding-agent CLI drives `agent.chat`
// (claude / codex / grok). The choice is explicit + user-controlled because "the
// calling mechanics are different for each agent. The default is Claude, the
// editor-sandboxed,
// path-of-least-resistance agent. The chosen name flows straight into
// `agent.chat {message, agent}` so cutd uses the right calling mechanics.
//
// localStorage-backed so the choice survives reloads; falls back to claude if
// storage is unavailable (private mode / quota) or holds an unknown value.

import { CHAT_AGENTS, type ChatAgentName } from './doctor'

const KEY = 'cut.chatAgent'
const DEFAULT: ChatAgentName = 'claude'

/** The persisted agent, or claude if none/invalid/storage-unavailable. */
export function getChatAgent(): ChatAgentName {
  try {
    const v = localStorage.getItem(KEY)
    if (v && (CHAT_AGENTS as readonly string[]).includes(v)) return v as ChatAgentName
  } catch {
    /* storage unavailable — use the default */
  }
  return DEFAULT
}

/** Persist the user's agent choice (best-effort — a failed write is non-fatal,
 *  the session still works, just not remembered across reloads). */
export function setChatAgent(name: ChatAgentName): void {
  try {
    localStorage.setItem(KEY, name)
  } catch {
    /* storage unavailable — keep going, just don't persist */
  }
}
