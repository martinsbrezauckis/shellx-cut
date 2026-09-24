import type { ProjectIdentity } from '../../lib/client'
import { isProjectIdentity } from '../../lib/projectIdentity'
import type { ChatEvidenceAttachment } from '../../lib/evidenceAttachments'
import {
  CHAT_TIMELINE_TARGET_SCHEMA,
  type ChatTimelineTarget,
  type ChatTimelineTargetClip,
  type TargetKind,
} from '../../lib/chatTimelineTarget'
import {
  boundedAgentChatTurns,
  emptyAgentChatSession,
  type AgentChatSession,
  type AgentChatTurn,
} from './session'
const STORAGE_KEY = 'shellx-cut:agent-chat-history:v1'
const SCHEMA = 'shellx-cut/agent-chat-history/1'
const MAX_PROJECTS = 6
const MAX_TEXT = 16_000
const MAX_LOG_BYTES = 160_000
const MAX_ID = 256
export type AgentChatHistoryStatus = 'saved' | 'unavailable' | 'invalid' | 'memory'
interface StoredProjectHistory {
  identity: ProjectIdentity
  session: StoredSession
}
interface StoredHistory {
  schema: typeof SCHEMA
  projects: StoredProjectHistory[]
}
interface StoredSession {
  log: AgentChatTurn[]
  input: string
  attachments: string[]
  target: ChatTimelineTarget | null
  /** Written only as a recovery marker; the live busy state is never restored. */
  interrupted_request?: true
}
const record = (value: unknown): Record<string, unknown> | null =>
  value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
const string = (value: unknown, maximum = MAX_TEXT): string | null =>
  typeof value === 'string' && value.length <= maximum ? value : null
const optionalString = (value: unknown, maximum = MAX_TEXT): string | undefined =>
  value === undefined || value === null ? undefined : string(value, maximum) ?? undefined
const integer = (value: unknown): number | null =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null
function isIdentityFor(value: unknown, identity: ProjectIdentity): value is ProjectIdentity {
  return isProjectIdentity(value)
    && value.origin_path_sha256 === identity.origin_path_sha256
    && value.project_name === identity.project_name
}
function readRange(value: unknown, allowPoint: boolean): [number, number] | null {
  if (!Array.isArray(value) || value.length !== 2) return null
  const start = integer(value[0])
  const end = integer(value[1])
  if (start == null || end == null || end < start || (!allowPoint && end === start)) return null
  return [start, end]
}
function readTargetClip(value: unknown, range: [number, number]): ChatTimelineTargetClip | null {
  const candidate = record(value)
  if (!candidate) return null
  const trackId = string(candidate.track_id, MAX_ID)
  const clipId = string(candidate.clip_id, MAX_ID)
  const timeline = readRange(candidate.timeline_range_ms, true)
  const target = readRange(candidate.target_range_ms, true)
  if (!trackId || !clipId || !timeline || !target
    || target[0] < timeline[0] || target[1] > timeline[1]
    || target[0] < range[0] || target[1] > range[1]) return null
  return {
    track_id: trackId,
    clip_id: clipId,
    timeline_range_ms: timeline,
    target_range_ms: target,
  }
}
function readTarget(value: unknown, identity: ProjectIdentity): ChatTimelineTarget | null {
  const candidate = record(value)
  if (!candidate || candidate.schema !== CHAT_TIMELINE_TARGET_SCHEMA || !isIdentityFor(candidate.project_identity, identity)) return null
  const kind = candidate.kind
  if (kind !== 'range' && kind !== 'selection' && kind !== 'comment' && kind !== 'position') return null
  const range = readRange(candidate.range_ms, kind === 'comment' || kind === 'position')
  const revision = string(candidate.project_revision, MAX_ID)
  const label = string(candidate.label, 240)
  if (!range || !revision || !label) return null
  const commentId = optionalString(candidate.comment_id, MAX_ID)
  // The server's serde Option fields are present as null for range/selection
  // targets. Treat that wire form as absent, then retain the same strict point
  // requirement for comment/position targets below.
  const parsedPosition = candidate.position_ms == null ? undefined : integer(candidate.position_ms)
  if (parsedPosition === null) return null
  const position = parsedPosition
  if ((kind === 'comment' || kind === 'position') && position !== range[0]) return null
  if ((kind === 'range' || kind === 'selection') && position !== undefined) return null
  if (kind === 'comment' && !commentId) return null
  if (kind === 'position' && commentId) return null
  if (!Array.isArray(candidate.clips) || candidate.clips.length > 64) return null
  const clips = candidate.clips.map((clip) => readTargetClip(clip, range))
  if (clips.some((clip) => !clip)) return null
  const keys = new Set((clips as ChatTimelineTargetClip[]).map((clip) => `${clip.track_id}\u0000${clip.clip_id}`))
  if (keys.size !== clips.length) return null
  return {
    schema: CHAT_TIMELINE_TARGET_SCHEMA,
    kind: kind as TargetKind,
    project_identity: identity,
    project_revision: revision,
    ...(commentId ? { comment_id: commentId } : {}),
    range_ms: range,
    ...(position === undefined ? {} : { position_ms: position }),
    clips: clips as ChatTimelineTargetClip[],
    label,
  }
}
function readAttachments(value: unknown): Array<{ id: string; label: string }> | undefined {
  if (value === undefined) return undefined
  if (!Array.isArray(value) || value.length > 8) return undefined
  const attachments = value.map((item) => {
    const candidate = record(item)
    const id = candidate && string(candidate.id, MAX_ID)
    const label = candidate && string(candidate.label, 512)
    return id && label ? { id, label } : null
  })
  return attachments.some((item) => !item) ? undefined : attachments as Array<{ id: string; label: string }>
}
function readEvidence(value: unknown): ChatEvidenceAttachment[] | undefined {
  if (value === undefined) return undefined
  if (!Array.isArray(value) || value.length > 12) return undefined
  const evidence = value.map((item) => {
    const candidate = record(item)
    const evidenceId = candidate && string(candidate.evidence_id, MAX_ID)
    const indexId = candidate && string(candidate.index_id, MAX_ID)
    const label = candidate && string(candidate.label, 512)
    return evidenceId && indexId && label
      ? { evidence_id: evidenceId, index_id: indexId, label }
      : null
  })
  return evidence.some((item) => !item) ? undefined : evidence as ChatEvidenceAttachment[]
}
function readActions(value: unknown): AgentChatTurn['actions'] | undefined {
  if (value === undefined) return undefined
  if (!Array.isArray(value) || value.length > 128) return undefined
  const actions = value.map((item) => {
    const candidate = record(item)
    const opId = candidate && string(candidate.op_id, MAX_ID)
    const verb = candidate && string(candidate.verb, MAX_ID)
    return opId && verb ? { op_id: opId, verb } : null
  })
  return actions.some((item) => !item) ? undefined : actions as NonNullable<AgentChatTurn['actions']>
}
function readReview(value: unknown): AgentChatTurn['review'] | undefined {
  if (value === undefined || value === null) return undefined
  const candidate = record(value)
  const turnId = candidate && string(candidate.turn_id, MAX_ID)
  const baseline = candidate && string(candidate.baseline, MAX_ID)
  const checkpoint = candidate && optionalString(candidate.checkpoint, MAX_ID)
  const tip = candidate && optionalString(candidate.tip, MAX_ID)
  const diffError = candidate && optionalString(candidate.diff_error, MAX_TEXT)
  const revertSafe = candidate?.revert_safe
  if (!candidate || !turnId || !baseline || typeof revertSafe !== 'boolean') return undefined
  const concurrent = candidate.concurrent_actions
  if (!Array.isArray(concurrent) || concurrent.length > 128) return undefined
  const concurrentActions = concurrent.map((item) => {
    const action = record(item)
    const opId = action && string(action.op_id, MAX_ID)
    const verb = action && string(action.verb, MAX_ID)
    const actor = action && record(action.actor)
    return opId && verb && actor ? { op_id: opId, verb, actor } : null
  })
  if (concurrentActions.some((item) => !item)) return undefined
  return {
    turn_id: turnId,
    baseline,
    checkpoint: checkpoint ?? null,
    tip: tip ?? null,
    diff: null,
    diff_error: diffError ?? null,
    revert_safe: revertSafe,
    concurrent_actions: concurrentActions as NonNullable<AgentChatTurn['review']>['concurrent_actions'],
  }
}
function readTurn(value: unknown, identity: ProjectIdentity): AgentChatTurn | null {
  const candidate = record(value)
  const id = candidate && string(candidate.id, MAX_ID)
  const text = candidate && string(candidate.text)
  const role = candidate?.role
  if (!candidate || !id || !text || (role !== 'user' && role !== 'agent')) return null
  const target = candidate.target === undefined ? undefined : readTarget(candidate.target, identity)
  const requestTarget = candidate.requestTarget === undefined ? undefined : readTarget(candidate.requestTarget, identity)
  if ((candidate.target !== undefined && !target) || (candidate.requestTarget !== undefined && !requestTarget)) return null
  const projectIdentity = candidate.projectIdentity === undefined
    ? undefined
    : isIdentityFor(candidate.projectIdentity, identity) ? identity : null
  if (projectIdentity === null) return null
  const attachments = readAttachments(candidate.attachments)
  const requestAttachments = readAttachments(candidate.requestAttachments)
  const evidence = readEvidence(candidate.evidence)
  const requestEvidence = readEvidence(candidate.requestEvidence)
  const actions = readActions(candidate.actions)
  const review = readReview(candidate.review)
  if ((candidate.attachments !== undefined && !attachments)
    || (candidate.requestAttachments !== undefined && !requestAttachments)
    || (candidate.evidence !== undefined && !evidence)
    || (candidate.requestEvidence !== undefined && !requestEvidence)
    || (candidate.actions !== undefined && !actions)
    || (candidate.review !== undefined && candidate.review !== null && !review)) return null
  const request = optionalString(candidate.request)
  const projectName = optionalString(candidate.projectName, MAX_ID)
  if (projectName && projectName !== identity.project_name) return null
  const ok = candidate.ok === undefined ? undefined : typeof candidate.ok === 'boolean' ? candidate.ok : null
  const cost = candidate.cost === undefined || candidate.cost === null
    ? undefined
    : typeof candidate.cost === 'number' && Number.isFinite(candidate.cost) ? candidate.cost : null
  if (ok === null || cost === null) return null
  const reviewBusy = candidate.reviewBusy === true
  const reviewError = optionalString(candidate.reviewError)
  return {
    id,
    role,
    text,
    ...(ok === undefined ? {} : { ok }),
    ...(optionalString(candidate.agent, MAX_ID) ? { agent: optionalString(candidate.agent, MAX_ID) } : {}),
    ...(actions ? { actions } : {}),
    ...(cost === undefined ? {} : { cost }),
    ...(optionalString(candidate.errorKind, MAX_ID) ? { errorKind: optionalString(candidate.errorKind, MAX_ID) } : {}),
    ...(optionalString(candidate.agentMessage) ? { agentMessage: optionalString(candidate.agentMessage) } : {}),
    ...(attachments ? { attachments } : {}),
    ...(evidence ? { evidence } : {}),
    ...(request ? { request } : {}),
    ...(requestAttachments ? { requestAttachments } : {}),
    ...(requestEvidence ? { requestEvidence } : {}),
    ...(projectName ? { projectName } : {}),
    ...(projectIdentity ? { projectIdentity } : {}),
    ...(target ? { target } : {}),
    ...(requestTarget ? { requestTarget } : {}),
    ...(review ? { review } : {}),
    ...(candidate.reviewState === 'reverted' || candidate.reviewState === 'replacement'
      ? { reviewState: candidate.reviewState }
      : {}),
    ...(reviewBusy
      ? { reviewError: 'Step back did not finish before reload. Inspect the project history before retrying.' }
      : reviewError ? { reviewError } : {}),
  }
}
function readSession(value: unknown, identity: ProjectIdentity): AgentChatSession | null {
  const candidate = record(value)
  if (!candidate || !Array.isArray(candidate.log)) return null
  const log = candidate.log.map((turn) => readTurn(turn, identity))
  if (log.some((turn) => !turn)) return null
  const input = optionalString(candidate.input) ?? ''
  const attachments = Array.isArray(candidate.attachments)
    ? candidate.attachments.map((id) => string(id, MAX_ID))
    : null
  const target = candidate.target === null || candidate.target === undefined
    ? null
    : readTarget(candidate.target, identity)
  if (!attachments || attachments.some((id) => !id) || (candidate.target != null && !target)) return null
  const session: AgentChatSession = {
    log: boundedAgentChatTurns(log as AgentChatTurn[]),
    input,
    attachments: attachments as string[],
    target,
    busy: false,
  }
  if (candidate.interrupted_request === undefined) return session
  if (candidate.interrupted_request !== true) return null
  const lastUser = [...session.log].reverse().find((turn) => turn.role === 'user')
  const recoveryId = `recovered-${lastUser?.id ?? 'unknown'}`
  if (!session.log.some((turn) => turn.id === recoveryId)) {
    session.log = boundedAgentChatTurns([...session.log, {
      id: recoveryId,
      role: 'agent',
      text: 'The previous Agent Chat request was interrupted by a reload. It may have applied edits; inspect the current project before trying again.',
      ok: false,
      errorKind: 'interrupted',
      request: lastUser?.text,
      requestTarget: lastUser?.requestTarget ?? lastUser?.target,
      projectName: identity.project_name,
      projectIdentity: identity,
    }])
  }
  return session
}
function readStoredProject(value: unknown): StoredProjectHistory | null {
  const candidate = record(value)
  if (!candidate || !isProjectIdentity(candidate.identity)) return null
  const session = readSession(candidate.session, candidate.identity)
  return session ? { identity: candidate.identity, session } : null
}
function readHistory(): { history: StoredHistory | null; status: AgentChatHistoryStatus } {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return { history: { schema: SCHEMA, projects: [] }, status: 'memory' }
    const parsed = record(JSON.parse(raw))
    if (!parsed || parsed.schema !== SCHEMA || !Array.isArray(parsed.projects)) return { history: null, status: 'invalid' }
    if (parsed.projects.length > MAX_PROJECTS) return { history: null, status: 'invalid' }
    const projects = parsed.projects.map(readStoredProject)
    if (projects.some((project) => !project)) return { history: null, status: 'invalid' }
    return { history: { schema: SCHEMA, projects: projects as StoredProjectHistory[] }, status: 'saved' }
  } catch {
    return { history: null, status: 'unavailable' }
  }
}
export function loadAgentChatHistory(identity: ProjectIdentity): { session: AgentChatSession; status: AgentChatHistoryStatus } {
  const { history, status } = readHistory()
  if (!history) return { session: emptyAgentChatSession(), status }
  const entry = history.projects.find((project) => isIdentityFor(project.identity, identity))
  if (!entry || !isIdentityFor(entry.identity, identity)) return { session: emptyAgentChatSession(), status }
  const session = readSession(entry.session, identity)
  return session
    ? { session, status }
    : { session: emptyAgentChatSession(), status: 'invalid' }
}
function storedSession(session: AgentChatSession, identity: ProjectIdentity): StoredSession | null {
  if (session.input.length > MAX_TEXT || session.attachments.length > 8) return null
  const log = session.log.map((turn) => readTurn(turn, identity))
  if (log.some((turn) => !turn)) return null
  const target = session.target ? readTarget(session.target, identity) : null
  if (session.target && !target) return null
  const stored: StoredSession = {
    log: boundedAgentChatTurns(log as AgentChatTurn[]),
    input: session.input,
    attachments: session.attachments,
    target,
    ...(session.busy ? { interrupted_request: true } : {}),
  }
  const activeRequest = session.busy
    ? [...stored.log].reverse().find((turn) => turn.role === 'user')?.id
    : undefined
  while (stored.log.length > 0 && JSON.stringify(stored).length > MAX_LOG_BYTES) {
    const removable = stored.log.findIndex((turn) => turn.id !== activeRequest)
    if (removable < 0) return null
    stored.log.splice(removable, 1)
  }
  return stored
}
export function saveAgentChatHistory(identity: ProjectIdentity, session: AgentChatSession): AgentChatHistoryStatus {
  const { history, status } = readHistory()
  if (!history) return status
  const next = storedSession(session, identity)
  if (!next) return 'invalid'
  try {
    const projects = history.projects
      .filter((entry) => !isIdentityFor(entry.identity, identity))
      .concat({ identity, session: next })
      .slice(-MAX_PROJECTS)
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ schema: SCHEMA, projects }))
    return 'saved'
  } catch {
    return 'unavailable'
  }
}
