import assert from 'node:assert/strict'
import type { Project } from '../src/lib/client'
import {
  boundedAgentChatTurns,
  patchAgentChatTurn,
  type AgentChatTurn,
} from '../src/panels/AgentChat/session'
import {
  agentChatSessionKey,
  updateAgentChatSessions,
} from '../src/panels/AgentChat/useAgentChatSessions'

const makeTurn = (id: string): AgentChatTurn => ({ id, role: 'agent', text: id })
const capped = boundedAgentChatTurns(Array.from({ length: 41 }, (_, index) => makeTurn(`turn-${index}`)))
const patched = patchAgentChatTurn(capped, 'turn-40', { reviewState: 'reverted' })

assert.equal(capped.length, 40, 'Agent Chat bounds its in-session log to forty turns')
assert.equal(capped[0]?.id, 'turn-1', 'Agent Chat evicts the oldest turn when capping the log')
assert.equal(
  patched.find((turn) => turn.id === 'turn-40')?.reviewState,
  'reverted',
  'Agent Chat patches an async review by stable turn identity after eviction',
)

const projectA = { project_identity: { origin_path_sha256: 'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' } } as Project
const projectB = { project_identity: { origin_path_sha256: 'sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' } } as Project
const keyA = agentChatSessionKey(projectA, 7)
const keyB = agentChatSessionKey(projectB, 7)
assert.ok(keyA)
assert.ok(keyB)
const afterA = updateAgentChatSessions(new Map(), keyA, (session) => ({ ...session, input: 'project A review' }))
const afterB = updateAgentChatSessions(afterA, keyB, (session) => ({ ...session, input: 'project B review' }))

assert.notEqual(keyA, keyB, 'Agent Chat does not key same-session projects by a display name')
assert.equal(afterB.get(keyA)?.input, 'project A review', 'Agent Chat keeps project A state isolated after an async project switch')
assert.equal(afterB.get(keyB)?.input, 'project B review', 'Agent Chat writes the switched project into its own session')

console.log('PASS Agent Chat in-session retention')
