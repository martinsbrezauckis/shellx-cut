import assert from 'node:assert/strict'
import {
  loadAgentChatHistory,
  saveAgentChatHistory,
} from '../src/panels/AgentChat/history'
import type { AgentChatSession } from '../src/panels/AgentChat/session'
import type { ProjectIdentity } from '../src/lib/client'
import type { ChatTimelineTarget } from '../src/lib/chatTimelineTarget'

class MemoryStorage {
  readonly values = new Map<string, string>()
  getItem(key: string): string | null { return this.values.get(key) ?? null }
  setItem(key: string, value: string): void { this.values.set(key, value) }
  removeItem(key: string): void { this.values.delete(key) }
}

const originalStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
const storage = new MemoryStorage()
Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: storage })

const identity: ProjectIdentity = {
  schema: 'shellx-cut/project-identity/1',
  origin_path_sha256: `sha256:${'a'.repeat(64)}`,
  project_name: 'history-project',
}
const otherIdentity: ProjectIdentity = {
  ...identity,
  origin_path_sha256: `sha256:${'b'.repeat(64)}`,
  project_name: 'other-project',
}
const target: ChatTimelineTarget = {
  schema: 'shellx-cut/chat-timeline-target/1',
  kind: 'range',
  project_identity: identity,
  project_revision: 'op_000007',
  range_ms: [1_250, 2_750],
  clips: [{
    track_id: 'v1',
    clip_id: 'c1',
    timeline_range_ms: [0, 5_000],
    target_range_ms: [1_250, 2_750],
  }],
  label: 'Selected range 0:01.250–0:02.750 · 1 clip',
}
const session: AgentChatSession = {
  input: '',
  attachments: [],
  target: null,
  busy: false,
  log: [
    {
      id: 'chat-user-1',
      role: 'user',
      text: 'Tighten this exact section',
      target,
      requestTarget: target,
      projectName: identity.project_name,
      projectIdentity: identity,
    },
    {
      id: 'chat-agent-1',
      role: 'agent',
      text: 'Tightened the selected range.',
      ok: true,
      actions: [{ op_id: 'op_000007', verb: 'edit.trim' }],
      request: 'Tighten this exact section',
      target,
      requestTarget: target,
      projectName: identity.project_name,
      projectIdentity: identity,
      review: {
        turn_id: 'turn-1',
        baseline: 'op_000006',
        checkpoint: null,
        tip: 'op_000007',
        diff: null,
        diff_error: null,
        revert_safe: true,
        concurrent_actions: [],
      },
    },
  ],
}

assert.equal(saveAgentChatHistory(identity, session), 'saved', 'a bounded identity-bound completed turn is persisted locally')
const restored = loadAgentChatHistory(identity)
assert.equal(restored.status, 'saved')
assert.equal(restored.session.log.length, 2, 'reopen restores completed request and reply turns')
assert.equal(restored.session.log[1]?.actions?.[0]?.verb, 'edit.trim', 'reopen retains applied action receipts')
assert.deepEqual(restored.session.log[1]?.target, target, 'reopen retains the immutable target snapshot and revision')
assert.equal(loadAgentChatHistory(otherIdentity).session.log.length, 0, 'a different immutable project identity cannot load this Chat history')

const serverTarget = { ...target, comment_id: null, position_ms: null } as unknown as ChatTimelineTarget
const serverTurn = { ...session.log[1]!, target: serverTarget }
assert.equal(saveAgentChatHistory(identity, { ...session, log: [session.log[0]!, serverTurn] }), 'saved', 'a completed Agent turn accepts the server wire target with null optional fields')
assert.deepEqual(loadAgentChatHistory(identity).session.log[1]?.target, target, 'the server target restores as the same optional-field-free range')

const interrupted: AgentChatSession = {
  ...session,
  busy: true,
  log: [session.log[0]!],
}
assert.equal(saveAgentChatHistory(identity, interrupted), 'saved')
const recovered = loadAgentChatHistory(identity).session
assert.equal(recovered.busy, false, 'a live busy flag never revives after reload')
assert.equal(recovered.log.at(-1)?.errorKind, 'interrupted', 'an interrupted provider turn becomes an explicit recovery entry')

const largeInterrupted: AgentChatSession = {
  ...interrupted,
  log: [
    ...Array.from({ length: 20 }, (_, index) => ({
      id: `old-agent-${index}`,
      role: 'agent' as const,
      text: 'x'.repeat(16_000),
    })),
    interrupted.log[0]!,
  ],
}
assert.equal(saveAgentChatHistory(identity, largeInterrupted), 'saved', 'history may discard old bounded turns while retaining an interrupted request')
const recoveredLarge = loadAgentChatHistory(identity).session
assert.equal(recoveredLarge.log.at(-1)?.request, interrupted.log[0]?.text, 'size trimming retains the active request for the honest reload recovery')

storage.setItem('shellx-cut:agent-chat-history:v1', JSON.stringify({
  schema: 'shellx-cut/agent-chat-history/1',
  projects: [{
    identity,
    session: {
      log: [{ ...session.log[0], target: { ...target, project_identity: otherIdentity } }],
      input: '', attachments: [], target: null,
    },
  }],
}))
assert.equal(loadAgentChatHistory(identity).status, 'invalid', 'a persisted target copied from another project identity is rejected')

Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: { getItem: () => null, setItem: () => { throw new Error('denied') }, removeItem: () => {} },
})
assert.equal(saveAgentChatHistory(identity, session), 'unavailable', 'a local write refusal is observable to the Chat UI')

if (originalStorage) Object.defineProperty(globalThis, 'localStorage', originalStorage)
else delete (globalThis as { localStorage?: Storage }).localStorage

console.log('PASS Agent Chat local history is bounded, identity-bound, validated, and reload-honest')
