// panels/AgentChat — the agent chat box (the headline natural-language editor).
// Role: a right-rail tab where the user types an edit request; the cutd
// `agent.chat` launches the selected local CLI wired to cutd's MCP server, so
// its Cut tool calls are editing verbs on the live project. Claude uses Cut's
// contained route; Codex keeps the user's normal Codex settings and permissions.
// Grok runs from a disposable config/home with only Cut's MCP route while its
// existing login file remains in place. Antigravity uses its normal settings,
// native sandbox, and permissions with a workspace-local Cut MCP entry.
// Request/response per turn (no token streaming): a "working…"
// state, then one agent bubble with the reply, the ops it applied (each a normal,
// undoable op — pairs with Ctrl+Z / project.undo), the agent name + an
// API-equivalent cost estimate (NOT billed — the turn runs on the user's own
// logged-in subscription; the figure only proxies how much work it did).
//
// Multi-agent selection: a dropdown lets the
// user pick WHICH agent drives the turn, because the calling mechanics differ per
// agent. It is populated from `system.doctor` → `judge.<agent>.details.chat`
// (refreshed on open — auth-state changes don't fire the doctor change-detector,
// so we re-scan when the menu opens; it's cheap). Each agent shows a containment
  // badge (ready / needs-login / install / disabled). DEFAULT = Claude; the
// choice persists (lib/chatAgentPref) and rides into
// `agent.chat {message, agent}`.
//
// Error transparency (HARD REQUIREMENT): `agent.chat` NEVER fails silently. On
// `ok:false` the verb returns a structured {reason, error, agent_message}; we
// render the reason inline (red) PLUS the agent's OWN final message verbatim —
// never swallowed, so the user always knows WHY a turn did not execute.
//
// Every element carries data-cut-* for the debug API.
// Callers: App.tsx (rightTab === 'chat'). Deps: lib/client (callVerb + types),
// lib/doctor (chat-agent state), lib/chatAgentPref (persisted choice).

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type SetStateAction } from 'react'
import { callVerb } from '../../lib/client'
import type { Project, VerbResults } from '../../lib/client'
import type { AgentChatPrefill } from '../../lib/evidenceAttachments'
import { evidenceAttachmentIdentity } from '../../lib/evidenceAttachments'
import { rebaseChatTimelineTarget, type ChatTimelineTarget } from '../../lib/chatTimelineTarget'
import type { AgentChatHistoryStatus } from './history'
import {
  fetchDoctor,
  chatAgentsFrom,
  chatAgentBadge,
  chatAgentPosture,
  type ChatAgentName,
  type ChatAgentOption,
} from '../../lib/doctor'
import { getChatAgent, setChatAgent } from '../../lib/chatAgentPref'
import { Icon } from '../../icons'
import AttachmentPicker from './AttachmentPicker'
import AssetAttachmentStrip from './AssetAttachmentStrip'
import EvidenceAttachmentStrip from './EvidenceAttachmentStrip'
import { chatAttachmentOptions, toggleChatAttachment } from './attachmentModel'
import { AGENT_PROMPT_CATEGORIES, AGENT_PROMPT_LIBRARY, AGENT_QUICK_PROMPTS } from './promptLibrary'
import { useEvidenceAttachments } from './useEvidenceAttachments'
import {
  boundedAgentChatTurns,
  patchAgentChatTurn,
  type AgentChatSession,
  type AgentChatTurn,
} from './session'
import './chat.css'

type ChatResult = VerbResults['agent.chat']
const errorText = (err: unknown): string => (err instanceof Error ? err.message : String(err))
const newTurnId = () => `chat-${crypto.randomUUID()}`

export interface AgentChatProps {
  /** The open project supplies only registered asset IDs to the attachment picker. */
  project: Project | null
  /** Prompt handed off while the chat tab was opening; nonce makes repeats apply. */
  prefill?: AgentChatPrefill | null
  /** Bounded, project-keyed right-rail state. It remains in memory only. */
  session: AgentChatSession
  /** The parent binds this updater to the project session that mounted this tab. */
  onSessionChange: (update: (current: AgentChatSession) => AgentChatSession) => void
  /** Bounded Chat history is local to this device and project identity. */
  historyStatus: AgentChatHistoryStatus
}

export default function AgentChat({ project, prefill, session, onSessionChange, historyStatus }: AgentChatProps) {
  const { log, input, attachments, target, busy } = session
  const setLog = useCallback((update: SetStateAction<AgentChatTurn[]>) => {
    onSessionChange((current) => {
      const next = typeof update === 'function' ? update(current.log) : update
      return { ...current, log: boundedAgentChatTurns(next) }
    })
  }, [onSessionChange])
  const setInput = useCallback((update: SetStateAction<string>) => {
    onSessionChange((current) => ({
      ...current,
      input: typeof update === 'function' ? update(current.input) : update,
    }))
  }, [onSessionChange])
  const setAttachments = useCallback((update: SetStateAction<string[]>) => {
    onSessionChange((current) => ({
      ...current,
      attachments: typeof update === 'function' ? update(current.attachments) : update,
    }))
  }, [onSessionChange])
  const setTarget = useCallback((update: SetStateAction<ChatTimelineTarget | null>) => {
    onSessionChange((current) => ({
      ...current,
      target: typeof update === 'function' ? update(current.target) : update,
    }))
  }, [onSessionChange])
  const setBusy = useCallback((update: SetStateAction<boolean>) => {
    onSessionChange((current) => ({
      ...current,
      busy: typeof update === 'function' ? update(current.busy) : update,
    }))
  }, [onSessionChange])
  const evidenceAttachments = useEvidenceAttachments(prefill)
  const [promptLibraryOpen, setPromptLibraryOpen] = useState(false)
  const [promptLibraryMaxHeight, setPromptLibraryMaxHeight] = useState<number | null>(null)
  const [targetError, setTargetError] = useState<string | null>(null)
  const logRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const promptLibraryRef = useRef<HTMLDivElement>(null)
  const hasProject = project !== null
  const attachmentOptions = useMemo(() => chatAttachmentOptions(project), [project])

  useEffect(() => {
    const registered = new Set(attachmentOptions.map((option) => option.id))
    setAttachments((selected) => selected.filter((id) => registered.has(id)))
  }, [attachmentOptions, setAttachments])

  // --- Agent selection ------------------------------------------------------
  // The chosen backend (persisted; default claude). `options` is the per-agent
  // chat state from the doctor (null state until first load). `agentsLoaded`
  // gates the trigger badge so it doesn't flash "install" before the scan lands.
  const [agent, setAgent] = useState<ChatAgentName>(() => getChatAgent())
  const [options, setOptions] = useState<ChatAgentOption[]>(() => chatAgentsFrom(null))
  const [agentsLoaded, setAgentsLoaded] = useState(false)
  const [menuOpen, setMenuOpen] = useState(false)
  const [agentsBusy, setAgentsBusy] = useState(false)
  const agentRef = useRef<HTMLDivElement>(null)

  /** Load the per-agent chat state from the doctor. `refresh` forces a re-scan
   *  (used on dropdown open — auth state changes don't fire the doctor's
   *  change-detector, so a fresh scan is the only way to catch a new login). */
  const loadAgents = useCallback(async (refresh: boolean) => {
    setAgentsBusy(true)
    try {
      const report = await fetchDoctor(refresh)
      setOptions(chatAgentsFrom(report))
      setAgentsLoaded(true)
    } catch {
      // Doctor unreachable (transport) — keep whatever we had; the reactive
      // error path on send still surfaces any real failure honestly.
    } finally {
      setAgentsBusy(false)
    }
  }, [])

  // Seed the selector from a live doctor scan on mount so the trigger badge does
  // not keep showing a stale cached "ready" after login/install state changes.
  useEffect(() => {
    void loadAgents(true)
  }, [loadAgents])

  // Re-scan with refresh:true each time the dropdown opens (catch a fresh login).
  useEffect(() => {
    if (menuOpen) void loadAgents(true)
  }, [menuOpen, loadAgents])

  // Close the agent menu on outside click / Esc (mirror of the topbar menus).
  useEffect(() => {
    if (!menuOpen) return
    const onDown = (e: MouseEvent) => {
      if (!(e.target instanceof Node) || !agentRef.current?.contains(e.target)) setMenuOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setMenuOpen(false)
    }
    document.addEventListener('mousedown', onDown)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('keydown', onKey)
    }
  }, [menuOpen])

  useEffect(() => {
    if (!promptLibraryOpen) return
    const onDown = (e: MouseEvent) => {
      if (!(e.target instanceof Node) || !promptLibraryRef.current?.contains(e.target)) setPromptLibraryOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.preventDefault()
      e.stopPropagation()
      setPromptLibraryOpen(false)
    }
    document.addEventListener('mousedown', onDown)
    document.addEventListener('keydown', onKey, true)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('keydown', onKey, true)
    }
  }, [promptLibraryOpen])

  useLayoutEffect(() => {
    if (!promptLibraryOpen) return
    const chips = promptLibraryRef.current
    const panel = chips?.closest('.chat')
    if (!chips || !panel) return
    const measure = () => {
      // The inspector clips the menu. Limit it to the space above the chips
      // inside this chat panel, then let the prompt list scroll.
      const available = Math.floor(chips.getBoundingClientRect().top
        - Math.max(0, panel.getBoundingClientRect().top) - 14)
      setPromptLibraryMaxHeight(Math.max(0, Math.min(420, available)))
    }
    measure()
    const resize = new ResizeObserver(measure)
    resize.observe(chips)
    resize.observe(panel)
    window.addEventListener('resize', measure)
    return () => {
      resize.disconnect()
      window.removeEventListener('resize', measure)
    }
  }, [promptLibraryOpen])

  const chooseAgent = useCallback((name: ChatAgentName, state: ChatAgentOption['state']) => {
    if (state && !state.wired) return
    setAgent(name)
    setChatAgent(name)
    setMenuOpen(false)
  }, [])

  // Keep the newest turn in view.
  useEffect(() => {
    logRef.current?.scrollTo({ top: logRef.current.scrollHeight })
  }, [log, busy])

  // External surfaces can attach a target and prefill the existing composer.
  useEffect(() => {
    const onPrompt = (e: Event) => {
      if (!(e instanceof CustomEvent)) return
      const detail = e.detail as string | { prompt?: string; target?: ChatTimelineTarget } | undefined
      const prompt = typeof detail === 'string' ? detail : detail?.prompt
      const nextTarget = typeof detail === 'string' ? undefined : detail?.target
      if (!prompt?.trim() && !nextTarget) return
      setInput(prompt ?? '')
      setTarget(nextTarget ?? null)
      setTargetError(null)
      window.setTimeout(() => inputRef.current?.focus(), 0)
    }
    document.addEventListener('cut:agent-chat-prompt', onPrompt)
    return () => document.removeEventListener('cut:agent-chat-prompt', onPrompt)
  }, [setInput, setTarget])

  // App-level handoff survives lazy mounting, including a Comment request that
  // intentionally submits through the regular Agent Chat turn after it mounts.
  useEffect(() => {
    if (!prefill || (!prefill.prompt.trim() && !prefill.target)) return
    setInput(prefill.prompt)
    setTarget(prefill.target ?? null)
    setTargetError(null)
    // Timeline prefill opens the normal composer without submitting. Its
    // app-level handoff must be consumed after this mounted session copied the
    // draft and target; otherwise reopening Chat can overwrite text the user
    // typed after leaving the tab. Comment requests retain their later
    // auto-submit claim so their existing one-shot send and evidence flow stay
    // intact.
    if (!prefill.submit) {
      const detail: { nonce: number; claimed?: boolean } = { nonce: prefill.nonce }
      document.dispatchEvent(new CustomEvent('cut:claim-agent-chat-prefill', { detail }))
    }
    window.setTimeout(() => inputRef.current?.focus(), 0)
  }, [prefill?.nonce, prefill?.prompt, prefill?.submit, prefill?.target, setInput, setTarget])

  const send = useCallback(async () => {
    const message = input.trim()
    if (!message || busy) return
    const turnTarget = target ? rebaseChatTimelineTarget(project, target) : null
    if (target && !turnTarget) {
      setTargetError('This target changed before sending. Choose the current timeline range or comment again.')
      return
    }
    const turnAttachments = attachments.map((id) => ({
      id,
      label: attachmentOptions.find((option) => option.id === id)?.label ?? id,
    }))
    const turnProjectName = project?.name
    const turnProjectIdentity = project?.project_identity
    const turnEvidence = evidenceAttachments.selected
    const evidenceIdentity = evidenceAttachmentIdentity(turnEvidence)
    setInput('')
    setAttachments([])
    setTarget(null)
    setTargetError(null)
    evidenceAttachments.clear()
    setLog((l) => [...l, {
      id: newTurnId(),
      role: 'user',
      text: message,
      attachments: turnAttachments,
      evidence: turnEvidence,
      target: turnTarget ?? undefined,
      requestTarget: turnTarget ?? undefined,
      projectName: turnProjectName,
      projectIdentity: turnProjectIdentity,
    }])
    setBusy(true)
    try {
      // Pass the selected provider. The backend rejects a provider without an
      // enabled Agent Chat route with a structured response.
      const r = await callVerb('agent.chat', {
        message,
        agent,
        attachments: turnAttachments.length > 0 ? turnAttachments.map((attachment) => attachment.id) : undefined,
        target: turnTarget ?? undefined,
        ...evidenceIdentity,
      })
      const res: ChatResult | null | undefined = r.ok ? r.result : null
      if (res && res.ok) {
        // SUCCESS path — unchanged: reply + applied ops + agent/cost meta.
        setLog((l) => [
          ...l,
          {
            id: newTurnId(),
            role: 'agent',
            text: res.reply,
            ok: true,
            agent: res.agent,
            actions: res.actions,
            cost: res.cost_usd,
            request: message,
            requestAttachments: turnAttachments,
            requestEvidence: turnEvidence,
            projectName: turnProjectName,
            projectIdentity: turnProjectIdentity,
            target: res.plan?.target ?? res.target ?? turnTarget ?? undefined,
            requestTarget: turnTarget ?? undefined,
            plan: res.plan,
            review: res.review,
          },
        ])
      } else if (res) {
        // ok:FALSE — the agent's honest failure. Surface the structured reason
        // (+ the machine category + the agent's OWN words) inline; never swallow.
        const reason = res.reason ?? res.reply ?? 'the agent could not complete the request'
        setLog((l) => [
          ...l,
          {
            id: newTurnId(),
            role: 'agent',
            text: reason,
            ok: false,
            agent: res.agent,
            errorKind: res.error ?? null,
            agentMessage: res.agent_message ?? null,
            actions: res.actions,
            request: message,
            requestAttachments: turnAttachments,
            requestEvidence: turnEvidence,
            projectName: turnProjectName,
            projectIdentity: turnProjectIdentity,
            target: res.plan?.target ?? res.target ?? turnTarget ?? undefined,
            requestTarget: turnTarget ?? undefined,
            plan: res.plan,
            review: res.review,
          },
        ])
      } else {
        // Transport / dispatch error (not the agent's honest ok:false).
        const msg = !r.ok && r.error ? r.error.message : 'the chat request failed'
        setLog((l) => [...l, { id: newTurnId(), role: 'agent', text: msg, ok: false }])
      }
    } catch (e) {
      setLog((l) => [...l, { id: newTurnId(), role: 'agent', text: errorText(e), ok: false }])
    } finally {
      setBusy(false)
    }
  }, [input, busy, agent, attachments, attachmentOptions, evidenceAttachments, project, setAttachments, setBusy, setInput, setLog, setTarget, target])

  // A Comment already contains an explicit request, so its Make changes action
  // may send through the normal Chat turn after the target reaches this mounted
  // composer. Timeline uses the same target path without submit and waits for
  // the user's normal typed request.
  useEffect(() => {
    if (!prefill?.submit || !prefill.prompt.trim() || busy) return
    if (input !== prefill.prompt || target !== (prefill.target ?? null)) return
    const detail: { nonce: number; claimed?: boolean } = { nonce: prefill.nonce }
    document.dispatchEvent(new CustomEvent('cut:claim-agent-chat-prefill', { detail }))
    if (!detail.claimed) return
    void send()
  }, [busy, input, prefill, send, target])

  const patchTurn = useCallback((turnId: string, patch: Partial<AgentChatTurn>) => {
    setLog((current) => patchAgentChatTurn(current, turnId, patch))
  }, [setLog])

  const revertTurn = useCallback(async (turnId: string, turn: AgentChatTurn): Promise<boolean> => {
    const review = turn.review
    const currentIdentity = project?.project_identity
    const sameProject = turn.projectIdentity && currentIdentity
      && turn.projectIdentity.origin_path_sha256 === currentIdentity.origin_path_sha256
      && turn.projectIdentity.project_name === currentIdentity.project_name
    if (!review || !review.revert_safe || !review.tip || !sameProject) return false
    patchTurn(turnId, { reviewBusy: true, reviewError: null })
    try {
      const result = await callVerb('project.revert', {
        to: review.baseline,
        if_tip: review.tip,
        rationale: `revert Agent Chat turn ${review.turn_id}`,
      })
      if (!result.ok) {
        patchTurn(turnId, {
          reviewBusy: false,
          reviewError: result.error?.message ?? 'could not revert this turn',
        })
        return false
      }
      patchTurn(turnId, { reviewBusy: false, reviewState: 'reverted', reviewError: null })
      return true
    } catch (error) {
      patchTurn(turnId, { reviewBusy: false, reviewError: errorText(error) })
      return false
    }
  }, [patchTurn, project?.project_identity])

  const replaceTurn = useCallback((turnId: string, turn: AgentChatTurn) => {
    if (!turn.request) return
    const currentIdentity = project?.project_identity
    const sameProject = turn.projectIdentity && currentIdentity
      && turn.projectIdentity.origin_path_sha256 === currentIdentity.origin_path_sha256
      && turn.projectIdentity.project_name === currentIdentity.project_name
    if (!sameProject) {
      const message = 'This request belongs to a different project. Open its project or start a new request here.'
      patchTurn(turnId, {
        reviewError: message,
      })
      setTargetError(message)
      return
    }
    const rebased = turn.requestTarget
      ? rebaseChatTimelineTarget(project, turn.requestTarget)
      : null
    if (turn.requestTarget && !rebased) {
      const message = 'This target no longer exists in the current project. Step back this turn if it is safe, then choose a new target.'
      patchTurn(turnId, {
        reviewError: message,
      })
      setTargetError(message)
      return
    }
    const registered = new Set(attachmentOptions.map((option) => option.id))
    setInput(turn.request)
    setTarget(rebased)
    setTargetError(null)
    setAttachments((turn.requestAttachments ?? []).map((attachment) => attachment.id).filter((id) => registered.has(id)))
    evidenceAttachments.restore(turn.requestEvidence ?? [])
    patchTurn(turnId, { reviewState: 'replacement', reviewError: null })
    window.setTimeout(() => inputRef.current?.focus(), 0)
  }, [attachmentOptions, evidenceAttachments, patchTurn, project, setAttachments, setInput, setTarget])

  const inspectDiff = useCallback((turn: AgentChatTurn) => {
    const review = turn.review
    if (!review?.baseline || !review.tip) return
    document.dispatchEvent(new CustomEvent('cut:open-review-tab', {
      detail: { tab: 'diff', from: review.baseline, to: review.tip },
    }))
  }, [])

  const previewTurn = useCallback(() => {
    document.dispatchEvent(new CustomEvent('cut:show-composed'))
    document.dispatchEvent(new CustomEvent('cut:focus-preview'))
  }, [])

  const onKeyDown = (e: React.KeyboardEvent) => {
    // Enter sends; Shift+Enter inserts a newline (the chat convention).
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      void send()
    }
  }

  // Chip click: PRE-FILL the compose box with the chip's request + focus it, so the
  // user can refine it before sending (never auto-spends a CLI turn). Discoverability
  // over automation — the agent runs the actual verb when the user sends.
  const choosePrompt = useCallback((prompt: string) => {
    if (!hasProject || busy) return
    setInput(prompt)
    setPromptLibraryOpen(false)
    inputRef.current?.focus()
  }, [hasProject, busy, setInput])

  // The current agent's row (for the trigger's badge). `agentsLoaded` gates the
  // badge so it stays neutral until the first scan resolves.
  const current = options.find((o) => o.name === agent) ?? { name: agent, state: null }
  const currentBadge = chatAgentBadge(current.name, current.state)

  return (
    <div className="chat" data-cut-chat data-cut-chat-agent={agent}>
      {/* Agent selector — pick which coding-agent CLI drives the turn. Always
          visible above the log; the menu re-scans the doctor on open. */}
      <div className="chat__agentbar" data-cut-chat-agentbar ref={agentRef}>
        <span className="chat__agentbar-lead">Agent</span>
        <button
          type="button"
          className={`chat__agentsel ${menuOpen ? 'chat__agentsel--open' : ''}`}
          data-cut-chat-agent-select={agent}
          aria-haspopup="listbox"
          aria-expanded={menuOpen}
          disabled={busy}
          title="Choose which coding-agent CLI runs your edit"
          onClick={() => setMenuOpen((o) => !o)}
        >
          <Icon name="agent" size={14} />
          <span className="chat__agentsel-name">{agent}</span>
          {agentsLoaded && (
            <span
              className={`chat__badge chat__badge--${currentBadge.kind}`}
              title={currentBadge.hint || undefined}
            >
              {currentBadge.label}
            </span>
          )}
          <Icon name="chevronUp" size={14} />
        </button>
        {menuOpen && (
          <ul className="chat__agentmenu" role="listbox" aria-label="Chat agent" data-cut-chat-agent-menu>
            {options.map((o) => {
              const badge = chatAgentBadge(o.name, o.state)
              const posture = chatAgentPosture(o.state)
              return (
                <li
                  key={o.name}
                  role="option"
                  aria-selected={o.name === agent}
                  className={`chat__agentopt ${o.name === agent ? 'chat__agentopt--active' : ''}`}
                  data-cut-action="chat-agent-option"
                  data-cut-chat-agent-option={o.name}
                  aria-disabled={o.state?.wired === false || undefined}
                  onClick={() => chooseAgent(o.name, o.state)}
                >
                  <div className="chat__agentopt-top">
                    <span className="chat__agentopt-name">{o.name}</span>
                    <span className={`chat__badge chat__badge--${badge.kind}`}>{badge.label}</span>
                    {posture && (
                      <span
                        className={`chat__posture ${posture.warn ? 'chat__posture--warn' : ''}`}
                        title="Headless containment status enforced by Cut"
                      >
                        {posture.text}
                      </span>
                    )}
                  </div>
                  {badge.hint && <div className="chat__agentopt-hint">{badge.hint}</div>}
                </li>
              )
            })}
            {agentsBusy && <li className="chat__agentmenu-note">checking agents…</li>}
          </ul>
        )}
      </div>
      <div className="chat__log" ref={logRef} data-cut-chat-log>
        {log.length === 0 && (
          <div className="chat__empty" data-cut-chat-empty>
            <Icon name="agent" size={18} />
            <p>Ask the agent to edit your timeline in plain language.</p>
            <p className="chat__hint">
              e.g. <em>“add a marker at 2 seconds”</em>, <em>“split the clip at the playhead and delete the first half”</em>,
              <em> “mute the music track”</em>. The agent runs on your own logged-in CLI; every change is a normal,
              undoable edit.
            </p>
          </div>
        )}
        {log.map((t) => (
          <div
            key={t.id}
            className={`chat__turn chat__turn--${t.role} ${t.ok === false ? 'chat__turn--failed' : ''}`}
            data-cut-chat-turn={t.role}
            data-cut-chat-error={t.role === 'agent' && t.ok === false ? (t.errorKind ?? 'error') : undefined}
          >
            <div className="chat__bubble">{t.text}</div>
            {t.target && <div className="chat__target" data-cut-chat-target>{t.target.label}</div>}
            {t.role === 'user' && <AssetAttachmentStrip attachments={t.attachments ?? []} turn />}
            {t.role === 'user' && <EvidenceAttachmentStrip attachments={t.evidence ?? []} turn />}
            {/* Error transparency: the agent's OWN final words on a failed turn
                (a refusal, "couldn't find a clip at 2s", an answer) — rendered
                verbatim below the reason, never swallowed. */}
            {t.role === 'agent' && t.ok === false && t.agentMessage && t.agentMessage !== t.text && (
              <div className="chat__agent-said" data-cut-chat-agent-said>
                <span className="chat__agent-said-lead">The agent said</span>
                <span className="chat__agent-said-text">{t.agentMessage}</span>
              </div>
            )}
            {t.role === 'agent' && t.actions && t.actions.length > 0 && (
              <div className="chat__actions" data-cut-chat-actions>
                {t.actions.map((a) => (
                  <span key={a.op_id} className="chat__action" title={a.op_id}>
                    {a.verb}
                  </span>
                ))}
              </div>
            )}
            {t.role === 'agent' && t.actions && t.actions.length > 0 && t.review && (
              <div
                className="chat__review"
                data-cut-chat-review={t.reviewState ?? 'applied'}
                data-cut-chat-revert-safe={t.review.revert_safe ? 'true' : 'false'}
              >
                <div className="chat__review-head">
                  <span className="chat__review-title">Applied change</span>
                  <span className={`chat__review-state chat__review-state--${t.reviewState ?? 'applied'}`}>
                    {t.reviewState === 'reverted' ? 'Stepped back' : t.reviewState === 'replacement' ? 'Ready to replace' : 'Applied'}
                  </span>
                </div>
                <div className="chat__review-plan" data-cut-chat-plan>
                  <span>Request</span>
                  <p>{t.plan?.request ?? t.request}</p>
                </div>
                {!t.review.revert_safe && t.review.concurrent_actions.length > 0 && (
                  <div className="chat__review-warning" data-cut-chat-review-concurrent>
                    {t.review.concurrent_actions.length} concurrent change{t.review.concurrent_actions.length === 1 ? '' : 's'} detected. Inspect Diff; Step back is unavailable.
                  </div>
                )}
                {t.review.diff_error && <div className="chat__review-warning">{t.review.diff_error}</div>}
                {t.reviewError && <div className="chat__review-error" data-cut-chat-review-error>{t.reviewError}</div>}
                <div className="chat__review-actions">
                  <button type="button" data-cut-chat-preview onClick={previewTurn} title="Show the current composed frame">
                    <Icon name="eye" size={14} /> Preview
                  </button>
                  <button type="button" data-cut-chat-diff onClick={() => inspectDiff(t)} disabled={!t.review.tip} title="Inspect this turn in Diff">
                    <Icon name="diff" size={14} /> Diff
                  </button>
                  <button type="button" data-cut-chat-step-back onClick={() => void revertTurn(t.id, t)} disabled={t.reviewBusy || !t.review.revert_safe || t.reviewState === 'reverted'} title="Undo this complete Agent Chat turn to its history baseline">
                    <Icon name="undo" size={14} /> Step back
                  </button>
                  <button type="button" data-cut-chat-replace onClick={() => replaceTurn(t.id, t)} disabled={t.reviewBusy || !t.request} title="Reuse this request with the same target re-resolved against the current project">
                    <Icon name="redo" size={14} /> Ask replacement
                  </button>
                </div>
              </div>
            )}
            {t.role === 'agent' && (t.agent || (t.ok === false && t.errorKind) || (t.cost != null && t.cost > 0)) && (
              <div className="chat__meta">
                {t.agent && <span>{t.agent}</span>}
                {t.ok === false && t.errorKind && (
                  <span className="chat__errkind" title="Why the turn did not execute (machine category)">
                    {t.errorKind}
                  </span>
                )}
                {t.cost != null && t.cost > 0 && (
                  <span
                    className="chat__cost"
                    title="Estimated API-equivalent cost the CLI reports for this turn. You run on your own logged-in subscription, so this is NOT billed — it only reflects how much work the turn did."
                  >
                    ≈${t.cost.toFixed(2)} API-equiv
                  </span>
                )}
              </div>
            )}
          </div>
        ))}
        {busy && (
          <div className="chat__turn chat__turn--agent" data-cut-chat-busy>
            <div className="chat__bubble chat__bubble--busy">
              <span className="chat__spinner" /> working on it…
            </div>
          </div>
        )}
      </div>
      <div className="chat__chips" data-cut-chat-chips ref={promptLibraryRef}>
        <span className="chat__chips-lead">Ask for…</span>
        {AGENT_QUICK_PROMPTS.map((preset) => (
          <button
            key={preset.id}
            type="button"
            className="chat__chip"
            data-cut-chat-chip={preset.label}
            disabled={!hasProject || busy}
            title={preset.prompt}
            onClick={() => choosePrompt(preset.prompt)}
          >
            {preset.label}
          </button>
        ))}
        <button
          type="button"
          className={`chat__promptlib-trigger${promptLibraryOpen ? ' chat__promptlib-trigger--open' : ''}`}
          data-cut-chat-prompt-library
          aria-haspopup="menu"
          aria-expanded={promptLibraryOpen}
          disabled={!hasProject || busy}
          title="Open prompt library"
          onClick={() => setPromptLibraryOpen((open) => !open)}
        >
          <Icon name="library" size={14} />
          Prompt library
          <Icon name="chevronUp" size={14} />
        </button>
        {promptLibraryOpen && (
          <div className="chat__promptlib" role="menu" aria-label="Prompt library" data-cut-chat-prompt-menu
            style={promptLibraryMaxHeight == null ? undefined : { maxHeight: promptLibraryMaxHeight }}>
            {AGENT_PROMPT_CATEGORIES.map((category) => (
              <section className="chat__promptlib-group" key={category} data-cut-chat-prompt-group={category}>
                <h3>{category}</h3>
                {AGENT_PROMPT_LIBRARY.filter((preset) => preset.category === category).map((preset) => (
                  <button
                    key={preset.id}
                    type="button"
                    role="menuitem"
                    data-cut-chat-prompt={preset.id}
                    data-cut-chat-prompt-verbs={preset.verbs.join(',')}
                    onClick={() => choosePrompt(preset.prompt)}
                  >
                    <span>{preset.label}</span>
                    <small>{preset.prompt}</small>
                  </button>
                ))}
              </section>
            ))}
          </div>
        )}
      </div>
      <AssetAttachmentStrip
        attachments={attachments.map((id) => ({ id, label: attachmentOptions.find((option) => option.id === id)?.label ?? id }))}
        busy={busy}
        onRemove={(id) => setAttachments((selected) => selected.filter((candidate) => candidate !== id))}
      />
      <EvidenceAttachmentStrip attachments={evidenceAttachments.selected} busy={busy} onRemove={evidenceAttachments.remove} />
      {target && (
        <div className="chat__target chat__target--composer" data-cut-chat-target>
          <span>{target.label}</span>
          <button
            type="button"
            data-cut-action="chat-clear-timeline-target"
            disabled={busy}
            onClick={() => { setTarget(null); setTargetError(null) }}
            aria-label="Clear Agent Chat timeline target"
            title="Clear timeline target"
          >×</button>
        </div>
      )}
      {targetError && <div className="chat__target-error" data-cut-chat-target-error>{targetError}</div>}
      {historyStatus !== 'memory' && (
        <div className={`chat__history chat__history--${historyStatus}`} data-cut-chat-history-status={historyStatus}>
          {historyStatus === 'saved'
            ? 'Chat history is saved on this device for this project.'
            : historyStatus === 'invalid'
              ? 'Chat history could not be saved safely on this device; this session will be lost after reload.'
              : 'Local storage is unavailable; this Chat session will be lost after reload.'}
        </div>
      )}
      <div className="chat__compose">
        <AttachmentPicker
          options={attachmentOptions}
          selected={attachments}
          disabled={!hasProject || busy}
          onToggle={(id) => setAttachments((selected) => toggleChatAttachment(selected, id))}
        />
        <textarea
          className="chat__input"
          data-cut-chat-input
          ref={inputRef}
          placeholder={hasProject ? 'Ask for an edit…' : 'Open a project first'}
          value={input}
          disabled={!hasProject || busy}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={onKeyDown}
          rows={2}
        />
        <button
          className="chat__send"
          data-cut-chat-send
          disabled={!hasProject || busy || !input.trim()}
          onClick={() => void send()}
          title="Send (Enter)"
        >
          <Icon name="return" size={16} label="send" />
        </button>
      </div>
    </div>
  )
}
