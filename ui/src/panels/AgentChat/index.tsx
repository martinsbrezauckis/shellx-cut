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

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { callVerb } from '../../lib/client'
import type { Project, VerbResults } from '../../lib/client'
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
import { chatAttachmentOptions, toggleChatAttachment } from './attachmentModel'
import { AGENT_PROMPT_CATEGORIES, AGENT_PROMPT_LIBRARY, AGENT_QUICK_PROMPTS } from './promptLibrary'
import { markReviewOps } from '../Review/reviewMarkers'
import './chat.css'

type ChatResult = VerbResults['agent.chat']
const errorText = (err: unknown): string => (err instanceof Error ? err.message : String(err))

interface Turn {
  role: 'user' | 'agent'
  text: string
  ok?: boolean
  agent?: string | null
  actions?: Array<{ op_id: string; verb: string }>
  cost?: number | null
  /** Error-transparency fields (ok:false path) — the machine category + the
   *  agent's OWN final words, rendered inline so a failure is never swallowed. */
  errorKind?: string | null
  agentMessage?: string | null
  attachments?: Array<{ id: string; label: string }>
  request?: string
  requestAttachments?: Array<{ id: string; label: string }>
  projectName?: string
  plan?: ChatResult['plan']
  review?: ChatResult['review']
  reviewState?: 'pending' | 'accepted' | 'reverted' | 'retry'
  reviewBusy?: boolean
  reviewError?: string | null
}

export interface AgentChatProps {
  /** The open project supplies only registered asset IDs to the attachment picker. */
  project: Project | null
  /** Prompt handed off while the chat tab was opening; nonce makes repeats apply. */
  prefill?: { prompt: string; nonce: number } | null
}

export default function AgentChat({ project, prefill }: AgentChatProps) {
  const [log, setLog] = useState<Turn[]>([])
  const [input, setInput] = useState('')
  const [attachments, setAttachments] = useState<string[]>([])
  const [busy, setBusy] = useState(false)
  const [promptLibraryOpen, setPromptLibraryOpen] = useState(false)
  const logRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const promptLibraryRef = useRef<HTMLDivElement>(null)
  const hasProject = project !== null
  const attachmentOptions = useMemo(() => chatAttachmentOptions(project), [project])

  useEffect(() => {
    const registered = new Set(attachmentOptions.map((option) => option.id))
    setAttachments((selected) => selected.filter((id) => registered.has(id)))
  }, [attachmentOptions])

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

  // External surfaces can open Agent Chat with a prepared request. This mirrors
  // suggestion chips: prefill only, never auto-send or spend an agent turn.
  useEffect(() => {
    const onPrompt = (e: Event) => {
      if (!(e instanceof CustomEvent)) return
      const detail = e.detail
      const prompt = typeof detail === 'string' ? detail : detail?.prompt
      if (!prompt?.trim()) return
      setInput(prompt)
      window.setTimeout(() => inputRef.current?.focus(), 0)
    }
    document.addEventListener('cut:agent-chat-prompt', onPrompt)
    return () => document.removeEventListener('cut:agent-chat-prompt', onPrompt)
  }, [])

  // App-level handoff for prompts emitted while the lazy chat bundle is still
  // loading. Event listeners cannot catch events fired before mount; this prop
  // path makes Environment/Agent handoffs durable.
  useEffect(() => {
    if (!prefill?.prompt.trim()) return
    setInput(prefill.prompt)
    window.setTimeout(() => inputRef.current?.focus(), 0)
  }, [prefill?.nonce, prefill?.prompt])

  const send = useCallback(async () => {
    const message = input.trim()
    if (!message || busy) return
    const turnAttachments = attachments.map((id) => ({
      id,
      label: attachmentOptions.find((option) => option.id === id)?.label ?? id,
    }))
    const turnProjectName = project?.name
    setInput('')
    setAttachments([])
    setLog((l) => [...l, { role: 'user', text: message, attachments: turnAttachments }])
    setBusy(true)
    try {
      // Pass the selected provider. The backend rejects a provider without an
      // enabled Agent Chat route with a structured response.
      const r = await callVerb('agent.chat', {
        message,
        agent,
        attachments: turnAttachments.length > 0 ? turnAttachments.map((attachment) => attachment.id) : undefined,
      })
      const res: ChatResult | null | undefined = r.ok ? r.result : null
      if (res && res.ok) {
        // SUCCESS path — unchanged: reply + applied ops + agent/cost meta.
        setLog((l) => [
          ...l,
          {
            role: 'agent',
            text: res.reply,
            ok: true,
            agent: res.agent,
            actions: res.actions,
            cost: res.cost_usd,
            request: message,
            requestAttachments: turnAttachments,
            projectName: turnProjectName,
            plan: res.plan,
            review: res.review,
            reviewState: res.actions.length > 0 && res.review ? 'pending' : undefined,
          },
        ])
      } else if (res) {
        // ok:FALSE — the agent's honest failure. Surface the structured reason
        // (+ the machine category + the agent's OWN words) inline; never swallow.
        const reason = res.reason ?? res.reply ?? 'the agent could not complete the request'
        setLog((l) => [
          ...l,
          {
            role: 'agent',
            text: reason,
            ok: false,
            agent: res.agent,
            errorKind: res.error ?? null,
            agentMessage: res.agent_message ?? null,
            actions: res.actions,
            request: message,
            requestAttachments: turnAttachments,
            projectName: turnProjectName,
            plan: res.plan,
            review: res.review,
            reviewState: res.actions.length > 0 && res.review ? 'pending' : undefined,
          },
        ])
      } else {
        // Transport / dispatch error (not the agent's honest ok:false).
        const msg = !r.ok && r.error ? r.error.message : 'the chat request failed'
        setLog((l) => [...l, { role: 'agent', text: msg, ok: false }])
      }
    } catch (e) {
      setLog((l) => [...l, { role: 'agent', text: errorText(e), ok: false }])
    } finally {
      setBusy(false)
    }
  }, [input, busy, agent, attachments, attachmentOptions, project?.name])

  const patchTurn = useCallback((index: number, patch: Partial<Turn>) => {
    setLog((current) => current.map((turn, candidate) => candidate === index ? { ...turn, ...patch } : turn))
  }, [])

  const acceptTurn = useCallback((index: number, turn: Turn) => {
    if (!turn.projectName || turn.projectName !== project?.name || !turn.actions?.length) return
    markReviewOps(turn.projectName, turn.actions.map((action) => action.op_id), 'accepted')
    patchTurn(index, { reviewState: 'accepted', reviewError: null })
  }, [patchTurn, project?.name])

  const revertTurn = useCallback(async (index: number, turn: Turn): Promise<boolean> => {
    const review = turn.review
    if (!review || !review.revert_safe || !review.tip || !turn.projectName || turn.projectName !== project?.name) return false
    patchTurn(index, { reviewBusy: true, reviewError: null })
    try {
      const result = await callVerb('project.revert', {
        to: review.baseline,
        if_tip: review.tip,
        rationale: `revert Agent Chat turn ${review.turn_id}`,
      })
      if (!result.ok) {
        patchTurn(index, {
          reviewBusy: false,
          reviewError: result.error?.message ?? 'could not revert this turn',
        })
        return false
      }
      if (turn.actions?.length) {
        markReviewOps(turn.projectName, turn.actions.map((action) => action.op_id), 'rejected')
      }
      patchTurn(index, { reviewBusy: false, reviewState: 'reverted', reviewError: null })
      return true
    } catch (error) {
      patchTurn(index, { reviewBusy: false, reviewError: errorText(error) })
      return false
    }
  }, [patchTurn, project?.name])

  const retryTurn = useCallback(async (index: number, turn: Turn) => {
    if (!turn.request) return
    if (turn.reviewState !== 'reverted') {
      const reverted = await revertTurn(index, turn)
      if (!reverted) return
    }
    const registered = new Set(attachmentOptions.map((option) => option.id))
    setInput(turn.request)
    setAttachments((turn.requestAttachments ?? []).map((attachment) => attachment.id).filter((id) => registered.has(id)))
    patchTurn(index, { reviewState: 'retry', reviewError: null })
    window.setTimeout(() => inputRef.current?.focus(), 0)
  }, [attachmentOptions, patchTurn, revertTurn])

  const inspectDiff = useCallback((turn: Turn) => {
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
  // user reviews/edits before sending (never auto-spends a CLI turn). Discoverability
  // over automation — the agent runs the actual verb when the user sends.
  const choosePrompt = useCallback((prompt: string) => {
    if (!hasProject || busy) return
    setInput(prompt)
    setPromptLibraryOpen(false)
    inputRef.current?.focus()
  }, [hasProject, busy])

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
        {log.map((t, i) => (
          <div
            key={i}
            className={`chat__turn chat__turn--${t.role} ${t.ok === false ? 'chat__turn--failed' : ''}`}
            data-cut-chat-turn={t.role}
            data-cut-chat-error={t.role === 'agent' && t.ok === false ? (t.errorKind ?? 'error') : undefined}
          >
            <div className="chat__bubble">{t.text}</div>
            {t.role === 'user' && t.attachments && t.attachments.length > 0 && (
              <div className="chat__turn-attachments">
                {t.attachments.map((attachment) => (
                  <span
                    key={attachment.id}
                    className="chat__turn-attachment"
                    data-cut-chat-turn-attachment={attachment.id}
                    title={attachment.id}
                  >
                    <Icon name="attach" size={14} />
                    {attachment.label}
                  </span>
                ))}
              </div>
            )}
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
                data-cut-chat-review={t.reviewState ?? 'pending'}
                data-cut-chat-revert-safe={t.review.revert_safe ? 'true' : 'false'}
              >
                <div className="chat__review-head">
                  <span className="chat__review-title">Turn review</span>
                  <span className={`chat__review-state chat__review-state--${t.reviewState ?? 'pending'}`}>
                    {t.reviewState === 'accepted' ? 'Accepted' : t.reviewState === 'reverted' ? 'Reverted' : t.reviewState === 'retry' ? 'Ready to retry' : 'Needs review'}
                  </span>
                </div>
                <div className="chat__review-plan" data-cut-chat-plan>
                  <span>Plan</span>
                  <p>{t.plan?.request ?? t.request}</p>
                </div>
                {!t.review.revert_safe && t.review.concurrent_actions.length > 0 && (
                  <div className="chat__review-warning" data-cut-chat-review-concurrent>
                    {t.review.concurrent_actions.length} concurrent change{t.review.concurrent_actions.length === 1 ? '' : 's'} detected. Inspect in Review; whole-turn revert is disabled.
                  </div>
                )}
                {t.review.diff_error && <div className="chat__review-warning">{t.review.diff_error}</div>}
                {t.reviewError && <div className="chat__review-error" data-cut-chat-review-error>{t.reviewError}</div>}
                <div className="chat__review-actions">
                  <button type="button" data-cut-chat-preview onClick={previewTurn} title="Show the current composed frame">
                    <Icon name="eye" size={14} /> Preview
                  </button>
                  <button type="button" data-cut-chat-diff onClick={() => inspectDiff(t)} disabled={!t.review.tip} title="Inspect this turn in Review Diff">
                    <Icon name="diff" size={14} /> Diff
                  </button>
                  <button type="button" data-cut-chat-accept onClick={() => acceptTurn(i, t)} disabled={t.reviewBusy || t.reviewState === 'accepted'} title="Accept these op-log changes">
                    <Icon name="check" size={14} /> Accept
                  </button>
                  <button type="button" data-cut-chat-revert onClick={() => void revertTurn(i, t)} disabled={t.reviewBusy || !t.review.revert_safe || t.reviewState === 'reverted' || t.reviewState === 'retry'} title="Revert the complete turn to its history baseline">
                    <Icon name="undo" size={14} /> Revert
                  </button>
                  <button type="button" data-cut-chat-retry onClick={() => void retryTurn(i, t)} disabled={t.reviewBusy || !t.review.revert_safe || !t.request} title="Revert this turn and put its request back in the composer">
                    <Icon name="redo" size={14} /> Try again
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
          <div className="chat__promptlib" role="menu" aria-label="Prompt library" data-cut-chat-prompt-menu>
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
      {attachments.length > 0 && (
        <div className="chat__attachments" data-cut-chat-attachments={attachments.length}>
          {attachments.map((id) => {
            const label = attachmentOptions.find((option) => option.id === id)?.label ?? id
            return (
              <span key={id} className="chat__attachment-chip" title={id}>
                <span>{label}</span>
                <button
                  type="button"
                  data-cut-chat-attachment-remove={id}
                  aria-label={`Remove ${label}`}
                  disabled={busy}
                  onClick={() => setAttachments((selected) => selected.filter((candidate) => candidate !== id))}
                >
                  <Icon name="close" size={14} />
                </button>
              </span>
            )
          })}
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
