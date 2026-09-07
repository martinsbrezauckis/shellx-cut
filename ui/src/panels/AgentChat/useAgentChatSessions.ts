import { useCallback, useMemo, useState } from 'react'
import type { Project } from '../../lib/client'
import { emptyAgentChatSession, type AgentChatSession } from './session'

const MAX_PROJECT_SESSIONS = 6

/**
 * Chat is deliberately in-session only, but the Chat tab itself unmounts when
 * the right rail changes tab. Keep a small LRU above that tab and key it by the
 * server's path-free origin digest. During an older-server transition without
 * that digest, App's monotonic projectSession is safe for this app lifetime;
 * it changes before a confirmed open/close and therefore cannot mix same-named
 * projects. Receiving a real digest starts a fresh, identity-bound session.
 */
export function agentChatSessionKey(project: Project | null, projectSession: number): string | null {
  if (!project) return null
  return project.project_identity?.origin_path_sha256 ?? `session:${projectSession}`
}

export function updateAgentChatSessions(
  sessions: ReadonlyMap<string, AgentChatSession>,
  key: string,
  update: (current: AgentChatSession) => AgentChatSession,
): Map<string, AgentChatSession> {
  const next = new Map(sessions)
  const previous = next.get(key) ?? emptyAgentChatSession()
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
} {
  const key = agentChatSessionKey(project, projectSession)
  const [sessions, setSessions] = useState<Map<string, AgentChatSession>>(() => new Map())
  const session = useMemo(
    () => key ? sessions.get(key) ?? emptyAgentChatSession() : emptyAgentChatSession(),
    [key, sessions],
  )
  const updateSession = useCallback((update: (current: AgentChatSession) => AgentChatSession) => {
    if (!key) return
    setSessions((current) => updateAgentChatSessions(current, key, update))
  }, [key])
  return { session, updateSession }
}
