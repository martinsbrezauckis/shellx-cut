import { useCallback, useEffect, useMemo, useState } from 'react'
import type { Project } from '../../lib/client'
import { emptyAgentChatSession, type AgentChatSession } from './session'
import {
  loadAgentChatHistory,
  saveAgentChatHistory,
  type AgentChatHistoryStatus,
} from './history'

const MAX_PROJECT_SESSIONS = 6

/**
 * Keep a small in-memory LRU above the tab, which unmounts as the right rail
 * changes. The identity-bound history helper separately persists safe session
 * state locally. During an older-server transition without an origin digest,
 * App's monotonic projectSession is safe for this app lifetime; it changes
 * before a confirmed open/close and therefore cannot mix same-named projects.
 * Receiving a real digest starts a fresh, identity-bound session.
 */
export function agentChatSessionKey(project: Project | null, projectSession: number): string | null {
  if (!project) return null
  const identity = project.project_identity
  return identity
    ? `${identity.origin_path_sha256}\u0000${identity.project_name}`
    : `session:${projectSession}`
}

export function updateAgentChatSessions(
  sessions: ReadonlyMap<string, AgentChatSession>,
  key: string,
  update: (current: AgentChatSession) => AgentChatSession,
  initialSession: AgentChatSession = emptyAgentChatSession(),
): Map<string, AgentChatSession> {
  const next = new Map(sessions)
  const previous = next.get(key) ?? initialSession
  // Reinsert the active project at the end so eviction is bounded LRU.
  next.delete(key)
  next.set(key, update(previous))
  while (next.size > MAX_PROJECT_SESSIONS) {
    const oldest = next.keys().next().value
    if (oldest === undefined) break
    next.delete(oldest)
  }
  return next
}

export function useAgentChatSession(project: Project | null, projectSession: number): {
  session: AgentChatSession
  updateSession: (update: (current: AgentChatSession) => AgentChatSession) => void
  historyStatus: AgentChatHistoryStatus
} {
  const key = agentChatSessionKey(project, projectSession)
  const identity = project?.project_identity
  const identityKey = identity
    ? `${identity.origin_path_sha256}\u0000${identity.project_name}`
    : null
  // A child effect can normalize attachments before this hook's hydration
  // effect runs. Capture the persisted session now so that first update has
  // the same base as hydration instead of creating an empty history entry.
  const restored = useMemo(
    () => identity ? loadAgentChatHistory(identity) : null,
    [identityKey],
  )
  const [sessions, setSessions] = useState<Map<string, AgentChatSession>>(() => new Map())
  const [hydratedKey, setHydratedKey] = useState<string | null>(null)
  const [historyStatus, setHistoryStatus] = useState<AgentChatHistoryStatus>('memory')
  const session = useMemo(
    () => key ? sessions.get(key) ?? emptyAgentChatSession() : emptyAgentChatSession(),
    [key, sessions],
  )
  useEffect(() => {
    if (!key || !identity) {
      setHydratedKey(null)
      setHistoryStatus('memory')
      return
    }
    if (!restored) return
    setSessions((current) => {
      if (current.has(key)) return current
      const next = new Map(current)
      next.set(key, restored.session)
      return next
    })
    setHistoryStatus(restored.status)
    setHydratedKey(key)
  }, [identity, identityKey, key, restored])
  useEffect(() => {
    if (!key || !identity || hydratedKey !== key) return
    setHistoryStatus(saveAgentChatHistory(identity, session))
  }, [hydratedKey, identity, identityKey, key, session])
  const updateSession = useCallback((update: (current: AgentChatSession) => AgentChatSession) => {
    if (!key) return
    setSessions((current) => updateAgentChatSessions(current, key, update, restored?.session))
  }, [key, restored])
  return { session, updateSession, historyStatus }
}
