// Conversation mapping and legacy-migration builders for Agent Chat.

import type {
  ChatConversationOrigin,
  ChatConversationSummary,
  ChatTurnRecord,
} from '../../lib/chatConversationModel'
import type { ChatTimelineTarget } from '../../lib/chatTimelineTarget'
import type { Project } from '../../lib/client'
import type { AgentChatTurn } from './session'

function assetLabel(id: string, project: Project | null): string {
  const path = project?.assets?.[id]?.path
  if (!path) return id
  return path.split(/[\\/]/).pop() || id
}

export function turnsFromServerRecords(records: ChatTurnRecord[], project: Project | null): AgentChatTurn[] {
  return records.map((rec, index) => {
    const attachments = (rec.attachments ?? []).map((id) => ({ id, label: assetLabel(id, project) }))
    const isAgent = rec.role === 'agent'
    const plan = rec.plan as AgentChatTurn['plan'] | undefined
    const priorUserText = index > 0 && records[index - 1]?.role === 'user' ? records[index - 1].text : undefined
    const target = (rec.target as ChatTimelineTarget | undefined) ?? undefined
    return {
      id: rec.id,
      role: rec.role === 'user' ? 'user' : 'agent',
      text: rec.text ?? '',
      origin: rec.origin,
      status: rec.status,
      agent: rec.agent ?? null,
      ok: isAgent ? (rec.status === 'running' ? undefined : rec.status === 'ok') : undefined,
      errorKind: rec.error ?? null,
      actions: Array.isArray(rec.actions) ? rec.actions : [],
      attachments,
      projectName: project?.name,
      projectIdentity: project?.project_identity,
      request: plan?.request ?? priorUserText,
      target,
      requestTarget: (plan?.target as ChatTimelineTarget | undefined) ?? target,
      plan,
      review: rec.review as AgentChatTurn['review'],
      cost: typeof rec.cost_usd === 'number' ? rec.cost_usd : null,
      reviewState: rec.reverted ? 'reverted' : undefined,
    }
  })
}

export function hasAnyRunningConversation(list: ChatConversationSummary[]): boolean {
  return list.some((c) => c.running)
}

export function formatOriginLabel(origin: ChatConversationOrigin): string | null {
  return origin === 'agent' ? 'Started by an agent' : null
}

// Projects whose Chat conversations already live in the project (engine
// record). Once a project is listed here its browser copy is never sent again,
// so a cached copy of an engine conversation cannot come back as a duplicate.
// The browser copy itself is left in place: nothing is deleted on migration.
const ENGINE_KEPT_KEY = 'shellx-cut:agent-chat-engine-kept:v1'
const MAX_ENGINE_KEPT = 64

function readEngineKept(): string[] {
  try {
    const raw = localStorage.getItem(ENGINE_KEPT_KEY)
    const parsed: unknown = raw ? JSON.parse(raw) : []
    return Array.isArray(parsed) ? parsed.filter((v): v is string => typeof v === 'string') : []
  } catch {
    return []
  }
}

export function isEngineKeptProject(projectKey: string): boolean {
  return readEngineKept().includes(projectKey)
}

export function markEngineKeptProject(projectKey: string): void {
  try {
    const next = readEngineKept().filter((k) => k !== projectKey).concat(projectKey).slice(-MAX_ENGINE_KEPT)
    localStorage.setItem(ENGINE_KEPT_KEY, JSON.stringify(next))
  } catch {
    // Storage unavailable: the engine still refuses a second copy of the same
    // history by its legacy key.
  }
}

/** One key per browser history: the project plus the turns it holds. */
export function legacyHistoryKey(projectKey: string, turns: AgentChatTurn[]): string {
  const first = turns[0]?.id ?? ''
  const last = turns[turns.length - 1]?.id ?? ''
  return `${projectKey}|${turns.length}|${first}|${last}`
}

export function buildLegacyMigrationRequest(
  legacyKey: string,
  turns: AgentChatTurn[],
): {
  legacy_key: string
  conversations: Array<{ title?: string; origin: 'human'; turns: Record<string, unknown>[] }>
} | null {
  const usable = turns.filter((t) => t.role === 'user' || t.role === 'agent')
  if (usable.length === 0) return null
  const firstUser = usable.find((t) => t.role === 'user')?.text?.trim()
  return {
    legacy_key: legacyHistoryKey(legacyKey, usable),
    conversations: [
      {
        ...(firstUser ? { title: firstUser.slice(0, 72) } : {}),
        origin: 'human',
        turns: usable.map((t) => ({
          id: t.id,
          role: t.role,
          text: t.text,
          ...(t.origin ? { origin: t.origin } : {}),
          ...(t.status ? { status: t.status } : typeof t.ok === 'boolean' ? { ok: t.ok } : {}),
          ...(t.agent ? { agent: t.agent } : {}),
          ...(t.errorKind ? { error: t.errorKind } : {}),
          ...(t.actions ? { actions: t.actions } : {}),
          ...(t.attachments ? { attachments: t.attachments.map((a) => a.id) } : {}),
          ...(t.evidence ? { evidence_ids: t.evidence.map((e) => e.evidence_id) } : {}),
          ...(t.target ? { target: t.target } : {}),
          ...(t.plan ? { plan: t.plan } : {}),
          ...(t.review ? { review: t.review } : {}),
          ...(t.cost !== undefined ? { cost_usd: t.cost } : {}),
          ...(t.reviewState === 'reverted' ? { reverted: true } : {}),
        })),
      },
    ],
  }
}
