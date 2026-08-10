import { readFileSync } from 'node:fs'
import { strict as assert } from 'node:assert'
import {
  healthRecoveryRows,
  journalReplaySummary,
  mergeProjectHealthPage,
  type AggregatedProjectHealth,
} from '../src/panels/Environment/healthRecoveryModel'
import {
  editingCacheRow,
  formatCacheBytes,
  formatCacheChange,
} from '../src/panels/Environment/cacheHealthModel'
import type { CaptureRecoveryItem, ProjectHealthResult } from '../src/lib/clientResults'
import {
  loadCaptureRecovery,
  type CaptureRecoveryInventory,
} from '../src/panels/Environment/captureRecoveryModel'

function page(overrides: Partial<ProjectHealthResult> = {}): ProjectHealthResult {
  return {
    schema: 'shellx-cut/project-health/1',
    project_revision: 'op_000001',
    journal: { status: 'verified', log_records: 1, cache: 'matched', snapshot: { status: 'not_present' }, notices: [] },
    editing_cache: {
      status: 'ready', bytes: 1536, files: 2, reclaimable_bytes: 512, reclaimable_files: 1, latest_modified_ms: 1_000,
      cleanup_preview: { status: 'ready', minimum_age_ms: 86_400_000, aged_unreferenced_bytes: 512, aged_unreferenced_files: 1, recent_unreferenced_bytes: 0, recent_unreferenced_files: 0 },
      categories: [
        { kind: 'proxies', status: 'ready', bytes: 1024, files: 1, reclaimable_bytes: 0, reclaimable_files: 0, scanned_entries: 1, skipped_entries: 0, truncated: false, entry_limit: 20_000, latest_modified_ms: 1_000 },
        { kind: 'thumbnails', status: 'ready', bytes: 512, files: 1, reclaimable_bytes: 512, reclaimable_files: 1, scanned_entries: 1, skipped_entries: 0, truncated: false, entry_limit: 20_000, latest_modified_ms: 1_000 },
      ],
    },
    media: {
      status: 'ready', asset_count: 4, checked_count: 2,
      page: {
        offline: 0, proxy_available: 2, proxy_missing: 0, proxy_not_recorded: 0, proxy_not_applicable: 0,
        filmstrip_available: 2, filmstrip_missing: 0, filmstrip_not_recorded: 0, filmstrip_not_applicable: 0,
      },
      assets: [
        { asset: 'a1', source: 'available', proxy: 'available', filmstrip: 'available' },
        { asset: 'a2', source: 'available', proxy: 'available', filmstrip: 'available' },
      ],
      limit: 2, cursor: null, next_cursor: 'a2', has_more: true,
    },
    ...overrides,
  }
}

function captureInventory(states: Array<'complete' | 'recovered' | 'quarantined' | 'interrupted' | 'owner_ambiguous' | 'torn_journal' | 'corrupt'> = []): CaptureRecoveryInventory {
  return {
    complete: true,
    captures: states.map((state, index) => ({
      capture_id: `cap-${String(index + 1).padStart(3, '0')}`,
      state,
      checkpoints: 0,
      has_open_segment: false,
      receipt: state === 'owner_ambiguous' || state === 'torn_journal' || state === 'corrupt'
        ? null
        : {
            state: state as 'complete' | 'recovered' | 'quarantined' | 'interrupted',
            recovered_segments: 0,
            lost_tail_ms: null,
            lost_tail_lower_bound_ms: 0,
            lost_tail_upper_bound_ms: null,
            audio_first_packet_offset_ms: null,
            source: null,
          },
    })),
  }
}

function rows(projectHealth: AggregatedProjectHealth | null, jobs: Parameters<typeof healthRecoveryRows>[0]['jobs'] = null) {
  return healthRecoveryRows({
    hasProject: true,
    projectHealth,
    jobs,
    captureDoctor: { ready: true, cards: [] },
    captureRecovery: captureInventory(),
    toolchain: { schema: 'doctor/1', scanned_at: '', os: '', arch: '', app_version: '', essential_ok: true, cards: [] },
  })
}

const first = mergeProjectHealthPage(null, page())
assert.equal(first.complete, false, 'partial revision-bound page stays incomplete')
assert.equal(rows(first).find((row) => row.id === 'media')?.state, 'checking', 'partial media can never be green')

const continuation = page({
  media: {
    ...page().media,
    checked_count: 2,
    cursor: 'a2',
    has_more: false,
    page: { ...page().media.page, offline: 1, proxy_missing: 1, proxy_available: 1 },
    assets: [
      { asset: 'a3', source: 'offline', proxy: 'missing', filmstrip: 'available' },
      { asset: 'a4', source: 'available', proxy: 'available', filmstrip: 'available' },
    ],
  },
})
delete continuation.editing_cache
const final = mergeProjectHealthPage(first, continuation)
assert.equal(final.complete, true, 'final page completes the aggregate')
assert.equal(final.media.checked_count, 4, 'aggregate preserves all checked assets')
assert.equal(final.media.page.offline, 1, 'aggregate keeps page counts')
assert.deepEqual(final.editing_cache, first.editing_cache, 'first-page cache inventory survives continuation pages')
assert.equal(rows(final).find((row) => row.id === 'media')?.state, 'recoverable', 'offline source is an explicit recovery state')

assert.equal(formatCacheBytes(1536), '1.5 KB', 'cache bytes use a compact binary-unit summary')
assert.equal(formatCacheChange(1_000, 3_601_000), 'Latest cache change was 1 hour ago.', 'cache freshness is named as a file-change fact, not last use')
const cacheRow = editingCacheRow(first.editing_cache, true, false, 3_601_000)
assert.equal(cacheRow.state, 'healthy', 'a complete bounded cache inventory is healthy')
assert.equal(cacheRow.summary, '1.5 KB of rebuildable proxies and thumbnails.', 'cache summary names only the rebuildable classes')
assert.match(cacheRow.detail ?? '', /2 cached files\. 512 B across 1 file appears unreferenced and rebuildable\. 512 B across 1 file has not changed for at least 24 hours\. Active work must still be rechecked before any future removal\. Latest cache change was 1 hour ago\. Nothing is removed from this page\./, 'cache detail discloses the aged preview, freshness, and the read-only boundary')
assert.equal(editingCacheRow({ ...first.editing_cache!, status: 'partial' }, true, false, 3_601_000).state, 'attention', 'a skipped or truncated cache scan never becomes green')
assert.match(editingCacheRow({ ...first.editing_cache!, status: 'partial' }, true, false, 3_601_000).summary, /^At least /, 'a partial scan never presents its bytes as complete')
assert.match(editingCacheRow({ ...first.editing_cache!, status: 'partial', cleanup_preview: { ...first.editing_cache!.cleanup_preview, status: 'blocked' } }, true, false).detail ?? '', /Cleanup preview is blocked/, 'a partial inventory blocks cleanup preview')
assert.equal(editingCacheRow({ ...first.editing_cache!, bytes: 0, files: 0, reclaimable_bytes: 0, reclaimable_files: 0, cleanup_preview: { ...first.editing_cache!.cleanup_preview, aged_unreferenced_bytes: 0, aged_unreferenced_files: 0 } }, true, false).summary, 'No rebuildable proxies or thumbnails are stored.', 'an empty complete inventory stays explicit')

assert.equal(
  journalReplaySummary({
    ...final,
    journal: { ...final.journal, log_records: 120, snapshot: { status: 'verified', prefix_ops: 100 } },
  }),
  'The project cache matches durable history. A verified snapshot covers 100 records; 20 newer records will replay on reopen.',
  'journal health explains the bounded replay tail rather than exposing only an opaque snapshot status',
)
assert.equal(
  journalReplaySummary({
    ...final,
    journal: { ...final.journal, cache: 'rebuilt', log_records: 3, snapshot: { status: 'rejected' } },
  }),
  'The project cache was rebuilt from durable history. The prior snapshot was rejected, so all 3 durable records will replay on reopen.',
  'rejected snapshots and cache rebuilds remain explicit without claiming data loss',
)
assert.match(
  rows(final).find((row) => row.id === 'journal')?.detail ?? '',
  /No replay snapshot is stored, so all 1 durable record will replay on reopen/,
  'Health and Recovery surfaces replay cost in the existing journal row',
)

assert.throws(
  () => mergeProjectHealthPage(first, page({ project_revision: 'op_000002' })),
  /changed revision/,
  'mixed revisions fail closed',
)

assert.equal(
  rows(final, { jobs: [], persistence_notices: [{ code: 'job_record_quarantined', record: 'job_2.json', message: 'corrupt record quarantined' }] })
    .find((row) => row.id === 'jobs')?.state,
  'recoverable',
  'quarantined job persistence notice is visible',
)
assert.equal(
  rows(final, { jobs: [{ job_id: 'job_1', kind: 'proxy', state: 'failed', progress: 1, created_ts: '', updated_ts: '', outcome: 'failed', outcome_reason: 'true_failure' }] })
    .find((row) => row.id === 'jobs')?.state,
  'unrecoverable',
  'true failure remains distinct from cancellation',
)
assert.equal(
  rows(final, { jobs: [{ job_id: 'job_2', kind: 'proxy', state: 'failed', progress: 1, created_ts: '', updated_ts: '', outcome: 'cancelled', outcome_reason: 'project_switch_cancelled' }] })
    .find((row) => row.id === 'jobs')?.state,
  'attention',
  'project-switch cancellation is never called a true failure',
)
assert.equal(rows(final).find((row) => row.id === 'capture')?.state, 'healthy', 'an empty completed capture inventory is healthy only after a successful read')
assert.equal(rows(final).find((row) => row.id === 'capture')?.detail, undefined, 'absence of an audio receipt never becomes a system-audio claim')

const audioEvidence = captureInventory(['complete'])
audioEvidence.captures[0]!.receipt!.audio_first_packet_offset_ms = 209
const audioEvidenceRow = healthRecoveryRows({
  hasProject: true,
  projectHealth: final,
  jobs: null,
  captureDoctor: {
    ready: true,
    cards: [{ name: 'system_audio', status: 'unknown', detail: 'Doctor did not open a live stream.' }],
  },
  captureRecovery: audioEvidence,
  toolchain: null,
}).find((row) => row.id === 'capture')
assert.equal(audioEvidenceRow?.state, 'healthy', 'durable audio packet evidence does not change capture recovery health')
assert.match(audioEvidenceRow?.detail ?? '', /System audio timing was recorded for 1 capture/, 'Health reports sealed packet-timing evidence without changing passive Doctor readiness')

function capture(capture_id: string, state: 'complete' | 'recovered' | 'quarantined' | 'interrupted' | 'owner_ambiguous' | 'torn_journal' | 'corrupt' = 'complete', source?: string | null): CaptureRecoveryItem {
  const receipt = state === 'owner_ambiguous' || state === 'torn_journal' || state === 'corrupt'
    ? null
    : state === 'complete'
      ? { state, recovered_segments: 1, lost_tail_ms: 0, lost_tail_lower_bound_ms: 0, lost_tail_upper_bound_ms: 0, audio_first_packet_offset_ms: null, source: source ?? 'source.mp4' }
      : state === 'recovered'
        ? { state, recovered_segments: 1, lost_tail_ms: null, lost_tail_lower_bound_ms: 0, lost_tail_upper_bound_ms: null, audio_first_packet_offset_ms: null, source: source ?? 'recovered.mp4' }
        : state === 'quarantined'
          ? { state, recovered_segments: 0, lost_tail_ms: null, lost_tail_lower_bound_ms: 0, lost_tail_upper_bound_ms: null, audio_first_packet_offset_ms: null, source: source ?? null }
          : { state, recovered_segments: 0, lost_tail_ms: null, lost_tail_lower_bound_ms: 0, lost_tail_upper_bound_ms: null, audio_first_packet_offset_ms: null, source: source ?? null }
  return {
    capture_id,
    state,
    checkpoints: 1,
    has_open_segment: false,
    receipt,
  }
}

function recoveryPage(captures: ReturnType<typeof capture>[], next_cursor: string | null) {
  return { captures, next_cursor }
}

const recoveryRequests: Array<{ after?: string; limit?: number }> = []
const recoveryPages = [
  recoveryPage([capture('cap-001'), capture('cap-002')], 'cap-002'),
  recoveryPage([capture('cap-003')], null),
]
const recovery = await loadCaptureRecovery(async (args) => {
  recoveryRequests.push(args)
  return { ok: true, result: recoveryPages[recoveryRequests.length - 1] }
})
assert.deepEqual(recoveryRequests, [{ limit: 100 }, { after: 'cap-002', limit: 100 }], 'capture recovery uses sequential 100-row lexical pages')
assert.equal(recovery.captures.length, 3, 'only a completed capture traversal reaches the model')

await assert.rejects(
  () => loadCaptureRecovery(async () => ({
    ok: true,
    result: recoveryPage(
      Array.from({ length: 101 }, (_, index) => capture(`cap-${String(index + 1).padStart(3, '0')}`)),
      null,
    ),
  })),
  /more than the 100-row page limit/,
  'a recorder page cannot exceed the requested 100-row bound',
)
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: recoveryPage([capture('../escape')], null) })),
  /malformed capture id/,
  'malformed capture ids fail closed',
)
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: { captures: [] } as never })),
  /missing next_cursor/,
  'a malformed page is never treated as an empty inventory',
)
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: recoveryPage([capture('cap-001', 'complete', '../source.mp4')], null) })),
  /unsafe source basename/,
  'receipt paths never reach the capture model',
)
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: recoveryPage([capture('cap-001', 'complete', 'C:source.mp4')], null) })),
  /unsafe source basename/,
  'Windows-style drive prefixes are not display-safe basenames',
)
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: recoveryPage([capture('cap-001', 'complete', 'C:\\source.mp4')], null) })),
  /unsafe source basename/,
  'Windows-style path separators are not display-safe basenames',
)
const missingTerminalReceipt = { ...capture('cap-001'), receipt: null }
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: recoveryPage([missingTerminalReceipt], null) })),
  /matching sealed receipt/,
  'a terminal capture row cannot become healthy without a durable receipt',
)
const mismatchedTerminalReceipt = capture('cap-001', 'recovered')
if (!mismatchedTerminalReceipt.receipt) throw new Error('fixture receipt missing')
mismatchedTerminalReceipt.receipt.state = 'complete'
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: recoveryPage([mismatchedTerminalReceipt], null) })),
  /matching sealed receipt/,
  'terminal capture and receipt states must agree',
)
const malformedCompleteSource = capture('cap-001')
if (!malformedCompleteSource.receipt) throw new Error('fixture receipt missing')
malformedCompleteSource.receipt.source = 'other.mp4'
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: recoveryPage([malformedCompleteSource], null) })),
  /inconsistent complete receipt/,
  'a complete receipt can only name source.mp4',
)
const malformedCompleteCount = capture('cap-001')
if (!malformedCompleteCount.receipt) throw new Error('fixture receipt missing')
malformedCompleteCount.receipt.recovered_segments = 0
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: recoveryPage([malformedCompleteCount], null) })),
  /inconsistent complete receipt/,
  'a complete receipt must cover every checkpoint',
)
const malformedQuarantine = capture('cap-001', 'quarantined')
if (!malformedQuarantine.receipt) throw new Error('fixture receipt missing')
malformedQuarantine.receipt.recovered_segments = 1
malformedQuarantine.receipt.source = 'recovered.mp4'
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: recoveryPage([malformedQuarantine], null) })),
  /inconsistent quarantined receipt/,
  'a quarantined receipt must recover fewer than all checkpoints',
)
for (const state of ['owner_ambiguous', 'corrupt'] as const) {
  const malformedNonterminal = { ...capture('cap-001'), state }
  await assert.rejects(
    () => loadCaptureRecovery(async () => ({ ok: true, result: recoveryPage([malformedNonterminal], null) })),
    /owner-ambiguous or corrupt capture with a terminal receipt/,
    `${state} cannot carry a terminal receipt`,
  )
}
const tornWithPriorReceipt = { ...capture('cap-001'), state: 'torn_journal' as const }
assert.equal(
  (await loadCaptureRecovery(async () => ({ ok: true, result: recoveryPage([tornWithPriorReceipt], null) }))).captures[0]?.state,
  'torn_journal',
  'a torn journal can retain prior terminal receipt evidence',
)
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: recoveryPage([capture('cap-002'), capture('cap-001')], null) })),
  /non-increasing capture ids/,
  'a page must stay lexical',
)
let duplicatePage = 0
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: [
    recoveryPage([capture('cap-001')], 'cap-001'),
    recoveryPage([capture('cap-001')], null),
  ][duplicatePage++] })),
  /duplicate capture id/,
  'duplicate ids discard the partial inventory',
)
let repeatedCursorPage = 0
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: [
    recoveryPage([capture('cap-001')], 'cap-001'),
    recoveryPage([capture('cap-002')], 'cap-001'),
  ][repeatedCursorPage++] })),
  /repeated or nonprogress cursor/,
  'repeated cursors discard the partial inventory',
)
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: recoveryPage([capture('cap-001')], 'cap-002') })),
  /does not match the final capture id/,
  'a next cursor must be the final reported capture id',
)
const inconsistentLoss = capture('cap-001')
if (!inconsistentLoss.receipt) throw new Error('fixture receipt missing')
inconsistentLoss.receipt.lost_tail_ms = 8
inconsistentLoss.receipt.lost_tail_lower_bound_ms = 9
inconsistentLoss.receipt.lost_tail_upper_bound_ms = 10
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: recoveryPage([inconsistentLoss], null) })),
  /outside its declared bounds/,
  'receipt loss values must stay inside their reported bounds',
)
const tooManyCapturePages = Array.from({ length: 41 }, (_, pageIndex) => {
  const count = pageIndex === 40 ? 97 : 100
  const first = pageIndex * 100 + 1
  const captures = Array.from({ length: count }, (_, index) => capture(`cap-${String(first + index).padStart(4, '0')}`))
  return recoveryPage(captures, pageIndex === 40 ? null : captures[captures.length - 1].capture_id)
})
let tooManyCapturePage = 0
await assert.rejects(
  () => loadCaptureRecovery(async () => ({ ok: true, result: tooManyCapturePages[tooManyCapturePage++] })),
  /exceeded the 4096-capture traversal limit/,
  'the capture traversal rejects row 4097 before returning an inventory',
)

const failedScanRows = healthRecoveryRows({
  hasProject: true,
  projectHealth: first,
  projectHealthScanFailed: true,
  jobs: null,
  captureDoctor: null,
  captureRecovery: null,
  toolchain: null,
})
assert.equal(failedScanRows.find((row) => row.id === 'journal')?.state, 'attention', 'a failed later page invalidates an earlier journal green')
assert.equal(failedScanRows.find((row) => row.id === 'media')?.state, 'attention', 'a failed later page invalidates the partial media aggregate')

assert.equal(
  healthRecoveryRows({
    hasProject: true,
    projectHealth: final,
    jobs: null,
    captureDoctor: { ready: false, cards: [{ name: 'screen', status: 'unknown', detail: 'Readiness was not verified.' }] },
    captureRecovery: captureInventory(['recovered']),
    toolchain: null,
  }).find((row) => row.id === 'capture')?.state,
  'attention',
  'unverified readiness outranks a recoverable receipt',
)
assert.equal(
  healthRecoveryRows({
    hasProject: true,
    projectHealth: final,
    jobs: null,
    captureDoctor: { ready: true, cards: [] },
    captureRecovery: captureInventory(['recovered', 'quarantined']),
    toolchain: null,
  }).find((row) => row.id === 'capture')?.state,
  'attention',
  'failed-closed capture states outrank recovered rows',
)
assert.equal(
  healthRecoveryRows({
    hasProject: true,
    projectHealth: final,
    jobs: null,
    captureDoctor: { ready: true, cards: [] },
    captureRecovery: captureInventory(['interrupted', 'recovered']),
    toolchain: null,
  }).find((row) => row.id === 'capture')?.state,
  'recoverable',
  'interrupted and recovered captures remain recoverable',
)
assert.equal(
  healthRecoveryRows({
    hasProject: true,
    projectHealth: final,
    jobs: null,
    captureDoctor: {
      ready: true,
      cards: [{ name: 'system_audio', status: 'unknown', detail: 'A recording has not proved audio delivery yet.' }],
    },
    captureRecovery: captureInventory(),
    toolchain: null,
  }).find((row) => row.id === 'capture')?.state,
  'healthy',
  'the optional passive system-audio card does not override verified screen readiness',
)
assert.equal(
  healthRecoveryRows({
    hasProject: false,
    projectHealth: null,
    jobs: null,
    captureDoctor: null,
    captureRecovery: null,
    toolchain: null,
  }).find((row) => row.id === 'capture')?.state,
  'unsupported',
  'unsupported is reserved for no project',
)
assert.equal(
  healthRecoveryRows({
    hasProject: true,
    projectHealth: final,
    jobs: null,
    captureDoctor: { ready: true, cards: [] },
    captureRecovery: captureInventory(),
    toolchain: null,
    toolchainScanFailed: true,
  }).find((row) => row.id === 'toolchain')?.state,
  'attention',
  'a failed generation-bound system Doctor read cannot inherit a stale toolchain pass',
)

const surface = readFileSync(new URL('../src/panels/Environment/HealthRecovery.tsx', import.meta.url), 'utf8')
for (const selector of ['data-cut-health-refresh', 'data-cut-health-row', 'data-cut-health-capture', 'data-cut-health-open-assets', 'data-cut-health-open-recording', 'data-cut-health-open-toolchain']) {
  assert.ok(surface.includes(selector), `Health & Recovery has stable ${selector}`)
}
for (const evidence of ['data-cut-health-refresh-id', 'data-cut-health-settled', 'data-cut-health-capture-complete', 'data-cut-health-capture-count']) {
  assert.ok(surface.includes(evidence), `Health & Recovery exposes settled model evidence through ${evidence}`)
}
assert.ok(!surface.includes('media.relink'), 'Health & Recovery cannot relink media without an owning confirmed workflow')
assert.ok(surface.includes('const doctorRefresh = useRef(onRefreshDoctor)'), 'ordinary App refresh callbacks are held in a ref')
assert.ok(surface.includes('}, [projectSession, refresh])'), 'health loading does not rerun for ordinary project object updates')
assert.ok(surface.includes('setProjectHealthScanFailed(true)'), 'failed or revision-conflicted health scans stay explicit')
assert.ok(surface.includes("loadCaptureRecovery((args) => callVerb('screen_record.recovery_status', args))"), 'capture recovery reads the recorder inventory rather than Doctor recovery fields')
assert.ok(surface.includes('attempt(doctorRefresh.current())'), 'the system Doctor result is awaited inside the same request generation')
assert.ok(surface.includes('setToolchainScanFailed(true)'), 'a failed system Doctor request cannot render the shared stale report as healthy')

const recordSurface = readFileSync(new URL('../src/panels/Record/index.tsx', import.meta.url), 'utf8')
assert.ok(recordSurface.includes("system_audio: 'System audio'"), 'Record gives the passive system-audio card a human label')

console.log('health-recovery.test.ts passed')
