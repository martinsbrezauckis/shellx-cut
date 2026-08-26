import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import type { MediaEvidenceHit, MediaIntelligenceStatusResult, Project } from '../src/lib/client'
import {
  assetLabel,
  coverageSummary,
  evidenceKinds,
  formatEvidenceTime,
  nearestActiveOccurrence,
  projectIdentity,
  selectedEvidencePrompt,
} from '../src/panels/Search/model'
import { evidenceAttachmentIdentity, evidenceChatAttachments } from '../src/lib/evidenceAttachments'

const project = {
  name: 'Evidence project',
  active_sequence: 'sequence-main',
  assets: {
    'asset-b': { path: 'D:\\Media\\interview.mov', hash: 'sha256:b' },
    'asset-a': { path: '/media/title.mp4', hash: 'sha256:a' },
  },
} as unknown as Project

const hit = {
  schema: 'shellx-cut/evidence-hit/1',
  evidence_id: 'ev_exact',
  asset_id: 'asset-b',
  source_start_ms: 61_000,
  source_end_ms: 64_500,
  anchor_ms: 62_000,
  kind: 'transcript',
  excerpt: 'The cited sentence from the interview.',
  match: 'exact',
  provenance: { kind: 'transcript', sha256: 'sha256:evidence' },
  available: true,
  occurrence_count: 3,
  occurrences: [
    { sequence_id: 'sequence-other', clip_id: 'other', track_id: 'V1', timeline_start_ms: 100, timeline_end_ms: 200 },
    { sequence_id: 'sequence-main', clip_id: 'far', track_id: 'V1', timeline_start_ms: 8_000, timeline_end_ms: 9_000 },
    { sequence_id: 'sequence-main', clip_id: 'near', track_id: 'V1', timeline_start_ms: 2_000, timeline_end_ms: 3_000 },
  ],
} satisfies MediaEvidenceHit

assert.deepEqual(evidenceKinds('all'), ['transcript', 'visual', 'scene', 'beat', 'marker', 'metadata'])
assert.deepEqual(evidenceKinds('rhythm'), ['scene', 'beat'])
assert.equal(formatEvidenceTime(61_000), '1:01')
assert.equal(formatEvidenceTime(3_661_000), '1:01:01')
assert.equal(assetLabel(project, 'asset-b'), 'interview.mov')
assert.match(projectIdentity(project), /asset-a:sha256:a\|asset-b:sha256:b/, 'project identity must be deterministic, not object-order dependent')
assert.equal(nearestActiveOccurrence(hit, project, 2_500)?.clip_id, 'near', 'timeline navigation stays on the active sequence and picks the nearest use')

const status = {
  schema: 'shellx-cut/media-intelligence-status/1',
  index_id: 'idx_current',
  complete: false,
  stale: false,
  entry_count: 23,
  coverage: {},
  assets: [],
} as unknown as MediaIntelligenceStatusResult
assert.equal(coverageSummary(status), '23 cited moments ready from available analysis.')
assert.equal(coverageSummary({ ...status, stale: true }), '23 cited moments ready; changed analysis is excluded until refresh.')

const prompt = selectedEvidencePrompt([hit], project)
assert.match(prompt, /inspect the current project before proposing or applying any edit/)
assert.match(prompt, /interview\.mov @ 1:01–1:04 \[transcript; ev_exact\]/)
assert.doesNotMatch(prompt, /D:\\Media/, 'agent handoff uses registered identity and basename, never a host path')
const evidence = evidenceChatAttachments([hit], 'idx_current', project)
assert.deepEqual(evidenceAttachmentIdentity(evidence), {
  evidence_ids: ['ev_exact'],
  evidence_index_id: 'idx_current',
})
assert.match(evidence[0].label, /interview\.mov · 1:02/)
assert.doesNotMatch(JSON.stringify(evidence), /D:\\Media/)

const hook = readFileSync(new URL('../src/panels/Search/useMediaIntelligence.ts', import.meta.url), 'utf8')
const controls = readFileSync(new URL('../src/panels/Search/SearchControls.tsx', import.meta.url), 'utf8')
const coverage = readFileSync(new URL('../src/panels/Search/Coverage.tsx', import.meta.url), 'utf8')
const results = readFileSync(new URL('../src/panels/Search/SearchResults.tsx', import.meta.url), 'utf8')
const searchSurface = `${controls}\n${coverage}\n${results}`
const schema = JSON.parse(readFileSync(new URL('../../schema/verbs.json', import.meta.url), 'utf8')) as {
  verbs: Array<{ name: string; behavior?: { ui_exposure?: string } }>
}
const packageJson = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')) as { scripts?: Record<string, string> }

for (const verb of ['media.intelligence_status', 'media.intelligence_rebuild', 'media.intelligence_search']) {
  assert.match(hook, new RegExp(`callVerb\\('${verb.replace('.', '\\.')}\\'`), `Find moment must call ${verb}`)
  assert.equal(schema.verbs.find((candidate) => candidate.name === verb)?.behavior?.ui_exposure, 'human')
}
for (const verb of ['inspect.media', 'inspect.range']) {
  assert.equal(schema.verbs.find((candidate) => candidate.name === verb)?.behavior?.ui_exposure, 'agent_only')
}
assert.doesNotMatch(hook, /callVerb\('media\.(?:index|search)'/, 'the unified UI must not fall back to the legacy visual-only route')
assert.match(controls, /Describe a shot or type words that were spoken/)
assert.match(controls, /data-cut-intelligence-kind/)
assert.match(controls, /data-cut-intelligence-scope/)
assert.match(coverage, /Prepare search only derives citations from analysis already stored in this project/)
assert.match(results, /evidenceChatAttachments\(selectedHits, indexId, project\)/)
for (const selector of [
  'data-cut-intelligence-query', 'data-cut-intelligence-kind', 'data-cut-intelligence-scope',
  'data-cut-intelligence-search', 'data-cut-intelligence-prepare', 'data-cut-intelligence-cancel',
  'data-cut-intelligence-refresh', 'data-cut-intelligence-coverage-toggle', 'data-cut-intelligence-hit',
  'data-cut-intelligence-select', 'data-cut-intelligence-preview', 'data-cut-intelligence-timeline',
  'data-cut-intelligence-ask-agent', 'data-cut-intelligence-more',
]) {
  assert.match(searchSurface, new RegExp(selector), `Find moment exposes ${selector}`)
}
assert.doesNotMatch(results, /toFixed|score\s/, 'the human surface must not present cross-system scores as universal confidence')
assert.equal(packageJson.scripts?.['test:media-intelligence'], 'tsx public-tests/media-intelligence-search.test.ts')

console.log('PASS cited media intelligence search UI contract')
