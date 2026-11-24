import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb, type Project } from '../../lib/client'
import type { ChatConversationSummary } from '../../lib/chatConversationModel'
import { events } from '../../lib/events'
import {
  buildLegacyMigrationRequest,
  hasAnyRunningConversation,
  isEngineKeptProject,
  markEngineKeptProject,
  turnsFromServerRecords,
} from './conversations'
import { loadAgentChatHistory } from './history'
import type { AgentChatSession } from './session'

export interface UnreadableConversationDetail {
  id: string
  title: string
  reason: string
}

export interface EngineConversationsController {
  engineBacked: boolean
  conversations: ChatConversationSummary[]
  listNote: string | null
  activeConversationId: string | null
  activeConversation: ChatConversationSummary | null
  unreadableDetail: UnreadableConversationDetail | null
  selectConversation: (id: string) => void
  startNewConversation: () => void
  beginOwnSend: () => string | undefined
  finishOwnSend: (conversationId?: string) => void
  markTurnReverted: (turnId: string) => Promise<void>
}

export function useEngineConversations({
  project,
  projectKey,
  projectSession,
  updateSession,
}: {
  project: Project | null
  projectKey: string | null
  projectSession: number
  updateSession: (updater: (cur: AgentChatSession) => AgentChatSession) => void
}): EngineConversationsController {
  const [engineBacked, setEngineBacked] = useState(false)
  const [conversations, setConversations] = useState<ChatConversationSummary[]>([])
  const [listNote, setListNote] = useState<string | null>(null)
  const [activeConversationId, setActiveConversationId] = useState<string | null>(null)
  const [unreadableDetail, setUnreadableDetail] = useState<UnreadableConversationDetail | null>(null)

  const activeIdRef = useRef<string | null>(null)
  const explicitNewRef = useRef(false)
  const awaitingOwnNewChatRef = useRef(false)
  const migratedKeysRef = useRef(new Set<string>())
  const mountedRef = useRef(true)
  const projectRef = useRef(project)
  const projectKeyRef = useRef(projectKey)

  activeIdRef.current = activeConversationId
  projectRef.current = project
  projectKeyRef.current = projectKey

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  const loadConversationDetail = useCallback(async (conversationId: string) => {
    const res = await callVerb('agent.chat_get', { conversation_id: conversationId })
    if (!mountedRef.current || !res.ok || !res.result?.conversation) return
    const conv = res.result.conversation
    if (!conv.readable) {
      setUnreadableDetail({
        id: conv.id,
        title: conv.title,
        reason: conv.unreadable_reason || 'This conversation format cannot be opened in this version.',
      })
      updateSession((cur) => ({ ...cur, log: [], busy: false, pendingUserText: null }))
      return
    }
    setUnreadableDetail(null)
    const turns = turnsFromServerRecords(conv.turns ?? [], projectRef.current)
    updateSession((cur) => ({
      ...cur,
      log: turns,
      busy: Boolean(conv.running),
      pendingUserText: null,
      lastError: null,
    }))
  }, [updateSession])

  const refreshEngine = useCallback(async (preferredId?: string | null) => {
    if (!projectRef.current) {
      if (mountedRef.current) {
        setEngineBacked(false)
        setConversations([])
        setListNote(null)
        setActiveConversationId(null)
        setUnreadableDetail(null)
      }
      return
    }

    const firstList = await callVerb('agent.chat_list', {})
    if (!mountedRef.current) return
    if (!firstList.ok || !Array.isArray(firstList.result?.conversations)) {
      setEngineBacked(false)
      return
    }
    setEngineBacked(true)

    let nextList = firstList.result.conversations
    let nextNote = typeof firstList.result.note === 'string' && firstList.result.note.trim()
      ? firstList.result.note
      : null
    const key = projectKeyRef.current
    const identity = projectRef.current?.project_identity
    // First read for this project: copy the conversation this browser kept
    // (0.6.121 and older) into the project once. The browser copy stays; a
    // failed copy is retried on the next read.
    if (key && identity && !migratedKeysRef.current.has(key) && !isEngineKeptProject(key)) {
      migratedKeysRef.current.add(key)
      const legacy = loadAgentChatHistory(identity)
      const payload = buildLegacyMigrationRequest(key, legacy.session.log)
      if (!payload) {
        // Mark only a history that was read: an unreadable one is tried again.
        if (legacy.status === 'saved' || legacy.status === 'memory') markEngineKeptProject(key)
      } else {
        const mig = await callVerb('agent.chat_migrate', payload)
        if (!mountedRef.current) return
        if (mig.ok) {
          markEngineKeptProject(key)
          const reloaded = await callVerb('agent.chat_list', {})
          if (!mountedRef.current) return
          if (reloaded.ok && Array.isArray(reloaded.result?.conversations)) {
            nextList = reloaded.result.conversations
            nextNote = typeof reloaded.result.note === 'string' && reloaded.result.note.trim()
              ? reloaded.result.note
              : null
          }
        } else {
          migratedKeysRef.current.delete(key)
        }
      }
    }

    setConversations(nextList)
    setListNote(nextNote)
    const requested = preferredId !== undefined ? preferredId : activeIdRef.current
    const matching = requested ? nextList.find((c) => c.id === requested) : undefined

    if (matching) {
      if (activeIdRef.current !== matching.id) setActiveConversationId(matching.id)
      await loadConversationDetail(matching.id)
      return
    }
    if (!explicitNewRef.current && nextList.length > 0) {
      const initial = nextList.find((c) => c.running) ?? nextList.find((c) => c.readable) ?? nextList[0]
      setActiveConversationId(initial.id)
      await loadConversationDetail(initial.id)
      return
    }
    if (nextList.length === 0 && !awaitingOwnNewChatRef.current) {
      // The project has no conversation: do not show a browser-cached log
      // that a send would not continue.
      if (activeIdRef.current !== null) setActiveConversationId(null)
      updateSession((cur) => cur.log.length === 0 || cur.busy ? cur : { ...cur, log: [] })
    }
  }, [loadConversationDetail, updateSession])

  useEffect(() => {
    explicitNewRef.current = false
    awaitingOwnNewChatRef.current = false
    setActiveConversationId(null)
    setUnreadableDetail(null)
    void refreshEngine(null)
  }, [projectKey, projectSession, refreshEngine])

  useEffect(() => {
    return events.subscribe((ev) => {
      if (ev.type === 'chat_updated') {
        if (awaitingOwnNewChatRef.current && ev.state === 'running') {
          awaitingOwnNewChatRef.current = false
          explicitNewRef.current = false
          setActiveConversationId(ev.conversation_id)
          void refreshEngine(ev.conversation_id)
          return
        }
        void refreshEngine(activeIdRef.current)
        return
      }
      if (ev.type === 'project_changed') {
        void refreshEngine(activeIdRef.current)
      }
    })
  }, [refreshEngine])

  const anyRunning = hasAnyRunningConversation(conversations)
  useEffect(() => {
    if (!engineBacked || !anyRunning) return
    const timer = window.setInterval(() => {
      void refreshEngine(activeIdRef.current)
    }, 750)
    return () => window.clearInterval(timer)
  }, [engineBacked, anyRunning, refreshEngine])

  const selectConversation = useCallback((id: string) => {
    explicitNewRef.current = false
    setActiveConversationId(id)
    const summary = conversations.find((c) => c.id === id)
    if (summary && !summary.readable) {
      setUnreadableDetail({
        id: summary.id,
        title: summary.title,
        reason: summary.unreadable_reason || 'This conversation format cannot be opened in this version.',
      })
      updateSession((cur) => ({ ...cur, log: [], busy: false, pendingUserText: null }))
      return
    }
    void loadConversationDetail(id)
  }, [conversations, loadConversationDetail, updateSession])

  const startNewConversation = useCallback(() => {
    explicitNewRef.current = true
    setActiveConversationId(null)
    setUnreadableDetail(null)
    updateSession((cur) => ({ ...cur, log: [], busy: false, pendingUserText: null, lastError: null }))
  }, [updateSession])

  const beginOwnSend = useCallback(() => {
    const current = activeIdRef.current
    awaitingOwnNewChatRef.current = !current
    return current ?? undefined
  }, [])

  const finishOwnSend = useCallback((conversationId?: string) => {
    awaitingOwnNewChatRef.current = false
    if (!conversationId) return
    explicitNewRef.current = false
    setActiveConversationId(conversationId)
    void refreshEngine(conversationId)
  }, [refreshEngine])

  const markTurnReverted = useCallback(async (turnId: string) => {
    const convId = activeIdRef.current
    if (!engineBacked || !convId) return
    await callVerb('agent.chat_patch', {
      conversation_id: convId,
      op: 'mark_reverted',
      turn_id: turnId,
    })
  }, [engineBacked])

  return {
    engineBacked,
    conversations,
    listNote,
    activeConversationId,
    activeConversation: conversations.find((c) => c.id === activeConversationId) ?? null,
    unreadableDetail,
    selectConversation,
    startNewConversation,
    beginOwnSend,
    finishOwnSend,
    markTurnReverted,
  }
}
