#!/usr/bin/env node
// dual-surface-job.test.mjs — deterministic unit tests for the dual-surface
// real-job differ (scripts/lib/dual-surface-job.mjs). No stack, no network —
// synthetic run records reproduce the live runner's contract, including the
// RED-PROOF cases: a deliberately broken step in one mode must be CAUGHT and
// ATTRIBUTED to the right layer (ui / api / engine), and an intact pair must
// read convergent. Run: node scripts/public-tests/dual-surface-job.test.mjs

import { strict as assert } from 'node:assert'
import {
  DEFAULT_TOLERANCE_MS,
  JOB_STEPS,
  attributeStep,
  buildDualSurfaceReceipt,
  canonicalGrade,
  canonicalizeTimeline,
  diffArgs,
  diffRuns,
  diffSpecArgs,
  diffTimelines,
  normalizeOps,
  buildRoleMaps,
  rampPointsFor,
} from '../lib/dual-surface-job.mjs'

let passed = 0
function ok(name, fn) {
  fn()
  passed += 1
  console.log(`  ✓ ${name}`)
}

// ── fixtures: a minimal but complete convergent run pair ─────────────────────
// Shapes mirror the live probe (2026-08-06): state clips carry src windows +
// per-clip decorations; ops carry {verb,args,status}; verify carries checks.
function makeState({ splitAt = 2000, gradeExtras = {} } = {}) {
  return {
    assets: {
      a1: { path: '/run/x/clip-a.mp4', probe: { kind: 'video', duration_ms: 10000 } },
      a2: { path: '/run/x/clip-b.mp4', probe: { kind: 'video', duration_ms: 4000 } },
      a3: { path: '/run/x/title.png', probe: { kind: 'image', duration_ms: 2000 } },
    },
    tracks: [
      {
        id: 'v1', kind: 'video',
        clips: [
          { id: 'c1', asset: 'a1', src_in_ms: 0, src_out_ms: splitAt, effects: [], grade: { contrast: 1.12, brightness: 0, saturation: 1.25, gamma: 1, temperature_k: 5000, ...gradeExtras } },
          { id: 'c5', asset: 'a1', src_in_ms: 4400, src_out_ms: 6042, xfade_in_ms: 400, xfade_kind: 'dissolve', effects: [] },
          { id: 'c2', asset: 'a2', src_in_ms: 0, src_out_ms: 4000, xfade_in_ms: 600, xfade_kind: 'dissolve', speed_ramp: { points: rampPointsFor('slow_fast_slow', 4000), segments: 15 }, effects: [] },
          { id: 'c3', asset: 'a1', src_in_ms: 6042, src_out_ms: 10000, fade: { in_ms: 0, out_ms: 500, kind: 'both' }, effects: [] },
        ],
      },
      { id: 'title1', kind: 'video', clips: [{ kind: 'gap', duration_ms: 300 }, { id: 'c6', asset: 'a3', src_in_ms: 0, src_out_ms: 2000, title_text: 'SHELLX CUT', effects: [] }] },
      { id: 'a1t', kind: 'audio', clips: [] },
    ],
  }
}

function makeOps({ splitAt = 2000, xfade1 = 400 } = {}) {
  return [
    { verb: 'project.create', args: { name: 'dsj job' }, status: 'applied' },
    { verb: 'media.import', args: { path: '/run/x/clip-a.mp4' }, status: 'applied' },
    { verb: 'edit.insert', args: { asset: 'a1', at_ms: 0, ripple: false }, status: 'applied' },
    { verb: 'media.import', args: { path: '/run/x/clip-b.mp4' }, status: 'applied' },
    { verb: 'edit.insert', args: { asset: 'a2', at_ms: 6042, ripple: true, src_range_ms: [0, 4000], track: 'v1' }, status: 'applied' },
    { verb: 'edit.split', args: { at_ms: splitAt, track: 'v1' }, status: 'applied' },
    { verb: 'edit.split', args: { at_ms: 4400, track: 'v1' }, status: 'applied' },
    { verb: 'edit.ripple_delete', args: { range_ms: [splitAt, 4400], ripple: true, track: 'v1' }, status: 'applied' },
    { verb: 'edit.crossfade', args: { at_ms: splitAt, duration_ms: xfade1, track: 'v1', transition: 'dissolve' }, status: 'applied' },
    { verb: 'edit.crossfade', args: { at_ms: 3642, duration_ms: 600, track: 'v1', transition: 'dissolve' }, status: 'applied' },
    { verb: 'edit.grade', args: { clip: 'c1', contrast: 1.12, brightness: 0, saturation: 1.25, gamma: 1, temperature_k: 5000 }, status: 'applied' },
    { verb: 'edit.speed_ramp', args: { clip: 'c2', points: rampPointsFor('slow_fast_slow', 4000) }, status: 'applied' },
    { verb: 'media.import', args: { path: '/run/x/title.png' }, status: 'applied' },
    { verb: 'title.add', args: { preset: 'title_card', range_ms: [300, 2300], text: 'SHELLX CUT' }, status: 'applied' },
    { verb: 'edit.fade', args: { clip: 'c3', kind: 'both', out_ms: 500 }, status: 'applied' },
  ]
}

function makeSteps(overrides = {}) {
  return JOB_STEPS.map(({ id }) => ({
    id, surface: 'verb', ok: true,
    op: makeOps().find((o) => ({
      'create-project': 'project.create',
      'import-a': 'media.import',
      'import-b': 'media.import',
      'insert-b': 'edit.insert',
      'split-1': 'edit.split',
      'split-2': 'edit.split',
      'ripple-delete': 'edit.ripple_delete',
      'xfade-1': 'edit.crossfade',
      'xfade-2': 'edit.crossfade',
      'grade-first': 'edit.grade',
      'grade-inserted': 'edit.grade',
      'speed-ramp': 'edit.speed_ramp',
      'title': 'title.add',
      'fade-out': 'edit.fade',
    })[id] === o.verb) || null,
    ...(overrides[id] || {}),
  }))
}

function makeRun({ mode = 'agent', splitAt = 2000, stepOverrides = {}, verifyChecks } = {}) {
  return {
    mode,
    steps: makeSteps(stepOverrides),
    ops: makeOps({ splitAt }),
    state: makeState({ splitAt }),
    importedAssetIds: ['a1', 'a2'],
    verify: { pass: false, checks: verifyChecks ?? [{ name: 'cut_on_word', pass: true }, { name: 'lufs', pass: false }, { name: 'duration_matches_edl', pass: true }] },
    render: { ok: true, jobState: 'done' },
  }
}

console.log('dual-surface-job differ unit tests')

// ── canonicalization ─────────────────────────────────────────────────────────
ok('canonicalGrade strips identity values and keeps real ones', () => {
  assert.equal(canonicalGrade({ contrast: 1, brightness: 0, saturation: 1, gamma: 1 }), null)
  assert.deepEqual(canonicalGrade({ contrast: 1.12, brightness: 0, saturation: 1.25, gamma: 1, temperature_k: 5000 }),
    { contrast: 1.12, saturation: 1.25, temperature_k: 5000 })
})

ok('canonicalizeTimeline maps assets to import-order roles and drops empty tracks', () => {
  const canon = canonicalizeTimeline(makeState(), ['a1', 'a2'])
  assert.equal(canon.tracks.length, 2) // v1 + title1; empty audio bed dropped
  const v1 = canon.tracks.find((t) => t.id === 'v1')
  assert.equal(v1.clips[0].asset, 'asset#0')
  assert.equal(v1.clips[2].asset, 'asset#1')
  const title = canon.tracks.find((t) => t.id === 'title1')
  assert.equal(title.clips[0].gap, true)
  assert.equal(title.clips[1].title_text, 'SHELLX CUT')
  assert.match(title.clips[1].asset, /^aux:image:/) // internal title PNG → content role
})

ok('diffTimelines tolerates click-precision offsets and flags real drift', () => {
  const a = canonicalizeTimeline(makeState({ splitAt: 2000 }), ['a1', 'a2'])
  const b = canonicalizeTimeline(makeState({ splitAt: 2017 }), ['a1', 'a2']) // half-frame click offset
  assert.deepEqual(diffTimelines(a, b, DEFAULT_TOLERANCE_MS), [])
  const c = canonicalizeTimeline(makeState({ splitAt: 2120 }), ['a1', 'a2']) // beyond tolerance
  const drift = diffTimelines(a, c, DEFAULT_TOLERANCE_MS)
  assert.ok(drift.length >= 1 && drift[0].includes('src_out'), `expected src_out drift, got: ${drift}`)
})

ok('normalizeOps strips noise and remaps ids to roles', () => {
  const state = makeState()
  const maps = buildRoleMaps(state, ['a1', 'a2'])
  const norm = normalizeOps(makeOps(), maps)
  assert.ok(norm.every((o) => !('rationale' in o.args) && !('group_id' in o.args)))
  assert.ok(!norm.some((o) => o.verb === 'media.import' || o.verb === 'project.create'))
  const grade = norm.find((o) => o.verb === 'edit.grade')
  assert.equal(grade.args.clip, 'video[0]@v1')
  const insert = norm.find((o) => o.verb === 'edit.insert' && o.args.at_ms === 6042)
  assert.equal(insert.args.asset, 'asset#1')
})

// ── convergent pair ──────────────────────────────────────────────────────────
ok('intact A/B pair reads CONVERGENT (click offsets inside tolerance)', () => {
  const runA = makeRun({ mode: 'agent', splitAt: 2000 })
  const runB = makeRun({ mode: 'ui', splitAt: 2013 }) // UI nudge landed 13ms off
  const diff = diffRuns(runA, runB)
  assert.equal(diff.verdict, 'convergent', JSON.stringify(diff.findings, null, 2))
})

ok('ui-gap fallback is a FINDING, not a divergence', () => {
  const runA = makeRun()
  const runB = makeRun({ stepOverrides: { 'import-a': { surface: 'verb-fallback', uiGap: 'import is a native OS picker (Tauri-only); no DOM affordance' } } })
  const diff = diffRuns(runA, runB)
  assert.equal(diff.verdict, 'convergent')
  const gap = diff.findings.find((f) => f.kind === 'ui-gap')
  assert.ok(gap && gap.step === 'import-a' && gap.layer === 'ui')
})

// ── RED-PROOF: broken step in one mode is caught + attributed ────────────────
ok('RED: UI lands wrong xfade duration → divergent, layer=ui', () => {
  const spec = { at_ms: 2000, duration_ms: 400, track: 'v1', transition: 'dissolve' }
  const runA = makeRun()
  const runB = makeRun()
  // Mode B's popover dispatched 250ms instead of the spec's 400.
  const bStep = runB.steps.find((s) => s.id === 'xfade-1')
  bStep.op = { verb: 'edit.crossfade', args: { ...spec, duration_ms: 250 } }
  bStep.specArgs = spec
  runB.steps.find((s) => s.id === 'xfade-1').specArgs = spec
  runA.steps.find((s) => s.id === 'xfade-1').specArgs = spec
  runB.ops = makeOps({ xfade1: 250 })
  runB.state.tracks[0].clips[1].xfade_in_ms = 250
  const diff = diffRuns(runA, runB)
  assert.equal(diff.verdict, 'divergent')
  const f = diff.findings.find((x) => x.step === 'xfade-1')
  assert.ok(f, 'xfade-1 finding missing')
  assert.equal(f.layer, 'ui')
  assert.ok(diff.timelineDiff.some((d) => d.includes('xfade_in_ms')), `timeline diff should carry xfade drift: ${diff.timelineDiff}`)
})

ok('RED: agent call errors while UI succeeds → layer=api', () => {
  const f = attributeStep('grade-first',
    { id: 'grade-first', ok: false, error: 'HTTP 500: internal' },
    { id: 'grade-first', ok: true, op: { verb: 'edit.grade', args: {} } })
  assert.ok(f && f.kind === 'divergence' && f.layer === 'api')
})

ok('RED: both surfaces fail the same step → layer=engine', () => {
  const f = attributeStep('speed-ramp',
    { id: 'speed-ramp', ok: false, error: 'ramp refused' },
    { id: 'speed-ramp', ok: false, error: 'ramp refused' })
  assert.ok(f && f.layer === 'engine')
})

ok('RED: UI control missing (present-phase failure) → layer=ui', () => {
  const f = attributeStep('title',
    { id: 'title', ok: true, op: { verb: 'title.add', args: {} } },
    { id: 'title', ok: false, phase: 'present', error: '[data-cut-title-btn] not found' })
  assert.ok(f && f.layer === 'ui' && f.detail.includes('present'))
})

ok('RED: UI dispatch on-spec but engine refuses only the UI path → layer=engine', () => {
  const spec = { clip: 'video[0]@v1', out_ms: 500, kind: 'both' }
  const f = attributeStep('fade-out',
    { id: 'fade-out', ok: true, op: { verb: 'edit.fade', args: spec }, specArgs: spec },
    { id: 'fade-out', ok: false, phase: 'result', op: { verb: 'edit.fade', args: spec }, specArgs: spec, error: 'engine rejected' })
  assert.ok(f && f.layer === 'engine', JSON.stringify(f))
})

ok('RED: UI clicked but NO op landed at all → layer=ui (classic RESULT failure)', () => {
  const f = attributeStep('xfade-2',
    { id: 'xfade-2', ok: true, op: { verb: 'edit.crossfade', args: {} } },
    { id: 'xfade-2', ok: false, phase: 'result', op: null, error: 'edit.crossfade did not land in the op log' })
  assert.ok(f && f.layer === 'ui' && f.detail.includes('no op landed'), JSON.stringify(f))
})

ok('RED: same landed ops but different materialized state → layer=engine', () => {
  const runA = makeRun()
  const runB = makeRun()
  // Ops + steps identical; the engine materialized a different fade for B.
  runB.state.tracks[0].clips[3].fade = { in_ms: 0, out_ms: 900, kind: 'both' }
  const diff = diffRuns(runA, runB)
  assert.equal(diff.verdict, 'divergent')
  const f = diff.findings.find((x) => x.step === 'timeline')
  assert.ok(f && f.layer === 'engine', JSON.stringify(diff.findings))
})

ok('RED: verify.checks outcome flips between surfaces → divergent verify finding', () => {
  const runA = makeRun()
  const runB = makeRun({ verifyChecks: [{ name: 'cut_on_word', pass: true }, { name: 'lufs', pass: false }, { name: 'duration_matches_edl', pass: false }] })
  const diff = diffRuns(runA, runB)
  assert.equal(diff.verdict, 'divergent')
  const f = diff.findings.find((x) => x.step === 'verify')
  assert.ok(f && f.detail.includes('duration_matches_edl'), JSON.stringify(diff.findings))
})

// ── receipt ──────────────────────────────────────────────────────────────────
ok('receipt: convergent pair → ok=true; broken pair → ok=false with findings', () => {
  const startedAt = new Date().toISOString()
  const good = buildDualSurfaceReceipt({ runA: makeRun(), runB: makeRun(), diff: diffRuns(makeRun(), makeRun()), stack: {}, startedAt })
  assert.equal(good.ok, true)
  assert.equal(good.schema, 'shellx-cut/dual-surface-job@1')
  assert.equal(good.diff.verdict, 'convergent')

  const runB = makeRun({ verifyChecks: [{ name: 'lufs', pass: true }] })
  const bad = buildDualSurfaceReceipt({ runA: makeRun(), runB, diff: diffRuns(makeRun(), runB), stack: {}, startedAt })
  assert.equal(bad.ok, false)
  assert.ok(bad.diff.findings.length >= 1)
})

ok('receipt: ui-gap steps surface in modeB.uiGaps and keep ok=true', () => {
  const runB = makeRun({ stepOverrides: { 'import-a': { surface: 'verb-fallback', ok: false, uiGap: 'native picker only' } } })
  const receipt = buildDualSurfaceReceipt({ runA: makeRun(), runB, diff: diffRuns(makeRun(), runB), stack: {}, startedAt: new Date().toISOString() })
  assert.deepEqual(receipt.modeB.uiGaps, [{ step: 'import-a', gap: 'native picker only' }])
  assert.equal(receipt.ok, true) // a documented gap is not a harness failure
})

ok('skipped steps after an earlier failure attribute as downstream, not fresh divergence', () => {
  const f = attributeStep('xfade-1',
    { id: 'xfade-1', ok: true, op: { verb: 'edit.crossfade', args: {} } },
    { id: 'xfade-1', ok: false, phase: 'skipped', error: 'skipped: run aborted after "ripple-delete" failed' })
  assert.ok(f && f.kind === 'downstream' && f.layer === 'downstream', JSON.stringify(f))
})

ok('diffSpecArgs checks only the spec subset (engine-enriched args pass)', () => {
  // Engine adds `kind` to edit.fade and default `settings` to project.create.
  assert.equal(diffSpecArgs({ name: 'dsj job' }, { name: 'dsj job', settings: { fps: 30 } }), null)
  assert.equal(diffSpecArgs({ out_ms: 500 }, { clip: 'c3', kind: 'both', out_ms: 500 }), null)
  assert.ok(diffSpecArgs({ out_ms: 500 }, { out_ms: 900 }))
  assert.equal(diffSpecArgs({ at_ms: 2000 }, { at_ms: 2020, track: 'v1' }), null) // tolerant key
})

// ── arg comparator edges ─────────────────────────────────────────────────────
ok('diffArgs: tolerant keys tolerate, typed keys stay exact', () => {
  assert.equal(diffArgs({ at_ms: 2000 }, { at_ms: 2030 }), null)
  assert.ok(diffArgs({ at_ms: 2000 }, { at_ms: 2100 }))
  assert.ok(diffArgs({ duration_ms: 400 }, { duration_ms: 430 })) // typed param: exact
  assert.equal(diffArgs({ range_ms: [2000, 4400] }, { range_ms: [2010, 4390] }), null)
  assert.ok(diffArgs({ temperature_k: 5000 }, { temperature_k: 5030 })) // not a time
})

console.log(`PASS — ${passed} checks`)
