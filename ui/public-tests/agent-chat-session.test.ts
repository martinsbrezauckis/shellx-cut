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
import {
  chatAgentBadge,
  chatAgentsFrom,
  environmentCardStatus,
  type DoctorReport,
} from '../src/lib/doctor'

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

const unavailableRenderJudgeReport = {
  schema: 'shellx-cut/doctor/1',
  scanned_at: '',
  os: 'fixture',
  arch: 'fixture',
  app_version: '0.6.114',
  essential_ok: true,
  cards: [{
    id: 'judge.codex',
    kind: 'judge',
    status: 'ok',
    details: {
      found: true,
      judge_ready: false,
      availability_reason: 'render judge unavailable until restricted tool/file access is verified',
      chat: {
        installed: true,
        resolved: '/fixture/codex',
        wired: true,
        authenticated: 'yes',
        auth_detail: 'fixture session',
        capability_verified: true,
        ready: true,
        posture: 'fixture',
      },
    },
  }],
} satisfies DoctorReport
const codexJudge = unavailableRenderJudgeReport.cards[0]!
assert.equal(environmentCardStatus(codexJudge), 'degraded',
  'a detected but unadmitted Codex CLI never receives the render-review Ready presentation')
const codexChat = chatAgentsFrom(unavailableRenderJudgeReport).find((agent) => agent.name === 'codex')
assert.equal(chatAgentBadge('codex', codexChat?.state ?? null).label, 'Ready',
  'render-judge admission does not disable an independently ready Agent Chat provider')
assert.equal(
  environmentCardStatus({
    id: 'judge.claude', kind: 'judge', status: 'unknown',
    details: { judge_ready: false, availability_reason: 'render-judge admission could not be verified; re-scan before running review' },
  }),
  'unknown',
  'an unverified admission probe keeps the Doctor Check again state instead of being downgraded to a generic warning',
)
assert.equal(
  environmentCardStatus({ id: 'judge.grok', kind: 'judge', status: 'ok', details: {} }),
  'unknown',
  'an older Doctor payload without an explicit admission result never presents render review as Ready',
)

console.log('PASS Agent Chat in-session retention')
