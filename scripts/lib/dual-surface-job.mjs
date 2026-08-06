// dual-surface-job.mjs — the PURE core of the dual-surface real-job harness.
//
// DOCTRINE (standing test policy): the same realistic editorial job is executed
// TWICE against fresh engine instances —
//   Mode A ("agent surface"): pure Debug-API verbs (POST /api/verb/*) — proves
//     the ENGINE + API layers.
//   Mode B ("UI surface"): the same job performed through real UI interactions
//     (Playwright over the data-cut-* selector contract) — proves the UI wiring
//     on top of the SAME engine.
// The two runs are then DIFFED (op log, final timeline, render verify.checks).
// A divergence is a bug in one of three layers, and this module says WHICH:
//   engine — both surfaces fail the same way, or identical landed args still
//            produce different state / different render-check outcomes.
//   api    — the agent-surface call errors while the UI path performs the same
//            edit successfully.
//   ui     — the UI control is missing/unclickable, or the UI dispatched a verb
//            whose args deviate from the shared job spec while Mode A matched.
// A UI-COVERAGE GAP (a job step with no clickable affordance at all) is NOT a
// divergence: it is recorded as an explicit `ui-gap` finding — the honest
// product answer, never silently verbed around.
//
// This module is PURE (no I/O, no network, no process state) so the differ,
// attribution rules, canonicalizer and receipt builder are unit-testable in CI
// (scripts/public-tests/dual-surface-job.test.mjs) without a live stack.
//
// Shapes consumed here were probed against a live cutd (2026-08-06, debug
// build): project.state clips carry {id, asset, src_in_ms, src_out_ms,
// xfade_in_ms?, xfade_kind?, grade?, speed_ramp?, fade?, title_text?, effects},
// gap rows are {kind:'gap', duration_ms}; project.ops rows are {op_id, verb,
// args, actor, effects, rationale, status, ts}; verify.checks returns
// {pass, checks:[{name, pass, details, evidence}], render_id, ...}.
//
// Primary callers: ui/public-tests/dual-surface-job-verify.mjs (live runner),
// scripts/release/dual-surface-job-gate.mjs (cold-start wrapper),
// scripts/public-tests/dual-surface-job.test.mjs (unit tests / red-proof).

// ── the shared job spec ──────────────────────────────────────────────────────
// ONE table both runners execute. Values are chosen to be expressible on BOTH
// surfaces (e.g. the speed ramp uses a UI preset curve, because arbitrary ramp
// points have no UI affordance — that asymmetry is itself a documented finding).
export const JOB_TARGETS = {
  projectName: 'dsj job',
  insertAtMs: 6042,
  split1Ms: 2000,
  split2Ms: 4400,
  // ripple range = [split1, split2] — the middle clip is extracted, gap closes.
  xfade1Ms: 400,
  xfade2Ms: 600,
  xfadeTransition: 'dissolve',
  // Grades mirror the UI Color tab payload exactly (it always sends the full
  // set; identity values are stripped by the canonicalizer before diffing).
  // VALUES ARE SLIDER-GRID-ALIGNED on purpose: the UI grade sliders quantize
  // to 0.05 steps (temperature to 100K) — an off-grid value like the marketing
  // demo's contrast 1.12 is AGENT-ONLY precision with no UI affordance (a
  // documented surface asymmetry this harness's job spec must avoid).
  gradeFirst: { contrast: 1.15, brightness: 0, saturation: 1.25, gamma: 1, temperature_k: 5000 },
  gradeInserted: { contrast: 1.3, brightness: 0, saturation: 1.3, gamma: 1, temperature_k: 7600 },
  // UI SpeedSection preset 'slow_fast_slow' over the clip's source duration d:
  // [{0,0.5},{d/2,2},{d,0.5}] — Mode A computes the same points from state.
  rampPreset: 'slow_fast_slow',
  title: { text: 'SHELLX CUT', range_ms: [300, 2300], preset: 'title_card' },
  fadeOutMs: 500,
  render: { preset: 'standard', format: 'h264', hardware: 'auto' },
}

/** UI preset curve (mirrors ui/src/panels/Inspector/SpeedSection.tsx
 *  RAMP_PRESETS — kept in sync so both surfaces target identical points). */
export function rampPointsFor(preset, srcDurMs) {
  const d = Math.round(srcDurMs)
  switch (preset) {
    case 'slow_fast_slow':
      return [{ at_ms: 0, factor: 0.5 }, { at_ms: Math.round(d / 2), factor: 2 }, { at_ms: d, factor: 0.5 }]
    case 'fast_slow_fast':
      return [{ at_ms: 0, factor: 2 }, { at_ms: Math.round(d / 2), factor: 0.5 }, { at_ms: d, factor: 2 }]
    case 'ramp_up':
      return [{ at_ms: 0, factor: 0.5 }, { at_ms: d, factor: 2 }]
    case 'ramp_down':
      return [{ at_ms: 0, factor: 2 }, { at_ms: d, factor: 0.5 }]
    default:
      throw new Error(`unknown ramp preset: ${preset}`)
  }
}

// Step ids in EXECUTION ORDER. `verb` = the op-log verb a successful step lands
// (null = no op expected: bookkeeping / job / read steps).
export const JOB_STEPS = [
  { id: 'create-project', verb: 'project.create', desc: 'create a fresh project (name only — UI payload)' },
  { id: 'import-a', verb: 'media.import', desc: 'import clip A (auto-placed as the timeline)' },
  { id: 'import-b', verb: 'media.import', desc: 'import clip B (waits in Assets)' },
  { id: 'insert-b', verb: 'edit.insert', desc: 'insert clip B on v1 at the playhead (~6042ms)' },
  { id: 'split-1', verb: 'edit.split', desc: 'split v1 at ~2000ms' },
  { id: 'split-2', verb: 'edit.split', desc: 'split v1 at ~4400ms' },
  { id: 'ripple-delete', verb: 'edit.ripple_delete', desc: 'ripple-delete the middle clip (gap closes)' },
  { id: 'xfade-1', verb: 'edit.crossfade', desc: '400ms dissolve on seam 1' },
  { id: 'xfade-2', verb: 'edit.crossfade', desc: '600ms dissolve on seam 2' },
  { id: 'grade-first', verb: 'edit.grade', desc: 'grade the first clip (contrast/saturation/temp)' },
  { id: 'grade-inserted', verb: 'edit.grade', desc: 'grade the inserted clip (cooler variant)' },
  { id: 'speed-ramp', verb: 'edit.speed_ramp', desc: 'preset speed ramp on the inserted clip' },
  { id: 'title', verb: 'title.add', desc: 'typed title card "SHELLX CUT" @ 300–2300ms' },
  { id: 'fade-out', verb: 'edit.fade', desc: '500ms fade-out on the last clip' },
  { id: 'render', verb: null, desc: 'render.final standard/h264 and wait for the job' },
  { id: 'verify', verb: null, desc: 'verify.checks on the rendered output' },
]

// Op-log verbs that participate in the SEQUENCE diff. media.import is excluded
// on purpose: its args carry run-local absolute paths and title.add internally
// imports its rendered PNG — both are covered by their own step verdicts.
export const DIFF_OP_VERBS = new Set([
  'edit.insert', 'edit.split', 'edit.ripple_delete', 'edit.crossfade',
  'edit.grade', 'edit.speed_ramp', 'title.add', 'edit.fade',
])

// Args whose values derive from a playhead/click position — compared with the
// frame tolerance (a human clicking the ruler cannot hit an exact millisecond;
// the UI nudges land within half a frame). Everything else must match EXACTLY:
// typed parameters (durations, grade numbers, title text/range) carry no
// pointing imprecision, so any drift there is a real wiring bug.
const TOLERANT_ARG_KEYS = new Set(['at_ms', 'range_ms', 'src_range_ms'])
export const DEFAULT_TOLERANCE_MS = 40

const near = (a, b, tol) => Number.isFinite(a) && Number.isFinite(b) && Math.abs(a - b) <= tol

/** Compare landed args against a SPEC SUBSET: only the keys the spec names are
 *  checked (the engine normalizes/enriches recorded op args — e.g. it adds
 *  `kind` to edit.fade and default `settings` to project.create — and click-
 *  derived keys like at_ms are intentionally left out of specs). */
export function diffSpecArgs(spec, args, tol = DEFAULT_TOLERANCE_MS) {
  if (!spec) return null
  for (const k of Object.keys(spec)) {
    const d = diffArgs(spec[k], (args || {})[k], tol, k, TOLERANT_ARG_KEYS.has(k))
    if (d) return d
  }
  return null
}

/** Deep-compare two arg values; keys in TOLERANT_ARG_KEYS compare numerically
 *  with `tol` ms slack (arrays element-wise). Returns a human-readable mismatch
 *  path or null when equal. */
export function diffArgs(a, b, tol = DEFAULT_TOLERANCE_MS, path = '', tolerant = false) {
  if (a === b) return null
  if (typeof a === 'number' && typeof b === 'number') {
    if (tolerant ? near(a, b, tol) : a === b) return null
    return `${path}: ${a} vs ${b}${tolerant ? ` (>±${tol}ms)` : ''}`
  }
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return `${path}: length ${a.length} vs ${b.length}`
    for (let i = 0; i < a.length; i++) {
      const d = diffArgs(a[i], b[i], tol, `${path}[${i}]`, tolerant)
      if (d) return d
    }
    return null
  }
  if (a && b && typeof a === 'object' && typeof b === 'object') {
    const keys = [...new Set([...Object.keys(a), ...Object.keys(b)])].sort()
    for (const k of keys) {
      const d = diffArgs(a[k], b[k], tol, path ? `${path}.${k}` : k, tolerant || TOLERANT_ARG_KEYS.has(k))
      if (d) return d
    }
    return null
  }
  return `${path}: ${JSON.stringify(a)} vs ${JSON.stringify(b)}`
}

// ── op-log normalization ─────────────────────────────────────────────────────
/** Map clip/asset ids to run-independent ROLES so two runs can be compared:
 *  assets → import-order index from the run's recorded import order (asset ids
 *  are sequence-assigned, but mapping via the recorded order keeps the diff
 *  honest even if an extra bookkeeping import shifts ids); clips → their final
 *  positional slot `trackKind[i]` from project.state (ids are history-dependent
 *  — an extra op in one run must not cascade into every later comparison). */
export function buildRoleMaps(state, importedAssetIds = []) {
  const assetRole = new Map()
  importedAssetIds.forEach((id, i) => assetRole.set(id, `asset#${i}`))
  const clipRole = new Map()
  for (const track of state?.tracks || []) {
    let slot = 0
    for (const clip of track.clips || []) {
      if (clip.kind === 'gap') continue
      clipRole.set(clip.id, `${track.kind}[${slot}]@${track.id}`)
      slot += 1
    }
  }
  return { assetRole, clipRole }
}

/** Filter the raw op log to the diffable edit verbs and strip run-local noise
 *  (op ids, timestamps, actor, rationale, undo group ids); remap clip/asset id
 *  args onto roles. Returns [{verb, args}] in landed order. */
export function normalizeOps(ops, roleMaps) {
  const out = []
  for (const op of ops || []) {
    if (!DIFF_OP_VERBS.has(op.verb)) continue
    if (op.status && op.status !== 'applied') continue
    const args = { ...(op.args || {}) }
    delete args.rationale
    delete args.group_id
    if (typeof args.clip === 'string') args.clip = roleMaps.clipRole.get(args.clip) || args.clip
    if (typeof args.asset === 'string') args.asset = roleMaps.assetRole.get(args.asset) || args.asset
    out.push({ verb: op.verb, args })
  }
  return out
}

// ── timeline canonicalization ────────────────────────────────────────────────
/** Strip identity values from a grade object (the UI always sends the full
 *  slider set; the agent may send only the interesting fields — identical
 *  grades must canonicalize identically). */
export function canonicalGrade(grade) {
  if (!grade || typeof grade !== 'object') return null
  const out = {}
  const round3 = (v) => Math.round(v * 1000) / 1000
  if (Number.isFinite(grade.contrast) && round3(grade.contrast) !== 1) out.contrast = round3(grade.contrast)
  if (Number.isFinite(grade.brightness) && round3(grade.brightness) !== 0) out.brightness = round3(grade.brightness)
  if (Number.isFinite(grade.saturation) && round3(grade.saturation) !== 1) out.saturation = round3(grade.saturation)
  if (Number.isFinite(grade.gamma) && round3(grade.gamma) !== 1) out.gamma = round3(grade.gamma)
  if (Number.isFinite(grade.temperature_k)) out.temperature_k = Math.round(grade.temperature_k)
  if (grade.lut) out.lut = String(grade.lut)
  return Object.keys(out).length ? out : null
}

/** Project state → a comparable structure: per track (sorted by id) the gap/
 *  clip sequence with asset ids replaced by import-order roles and only the
 *  job-relevant fields kept. Times stay raw; the comparer applies tolerance. */
export function canonicalizeTimeline(state, importedAssetIds = []) {
  const { assetRole } = buildRoleMaps(state, importedAssetIds)
  // Assets not in the recorded import order (e.g. the title's internal PNG
  // import) get a stable content role from their probe kind + duration.
  const auxRole = (id) => {
    const a = state?.assets?.[id]
    const dur = a?.probe?.duration_ms
    return `aux:${a?.probe?.kind || 'asset'}:${Number.isFinite(dur) ? dur : '?'}`
  }
  const tracks = [...(state?.tracks || [])]
    .sort((x, y) => String(x.id).localeCompare(String(y.id)))
    .map((t) => ({
      id: t.id,
      kind: t.kind,
      clips: (t.clips || []).map((c) => {
        if (c.kind === 'gap') return { gap: true, duration_ms: c.duration_ms }
        const clip = {
          asset: assetRole.get(c.asset) || auxRole(c.asset),
          src_in_ms: c.src_in_ms,
          src_out_ms: c.src_out_ms,
        }
        if (c.xfade_in_ms) { clip.xfade_in_ms = c.xfade_in_ms; clip.xfade_kind = c.xfade_kind || null }
        const grade = canonicalGrade(c.grade)
        if (grade) clip.grade = grade
        if (c.speed_ramp?.points?.length) clip.speed_ramp_points = c.speed_ramp.points
        if (c.fade && (c.fade.in_ms || c.fade.out_ms)) clip.fade = { in_ms: c.fade.in_ms || 0, out_ms: c.fade.out_ms || 0 }
        if (c.title_text) clip.title_text = c.title_text
        if (Array.isArray(c.effects) && c.effects.length) clip.effects = c.effects.map((e) => e.type || e)
        return clip
      }),
    }))
    // Drop empty tracks: the default audio bed exists on both runs but carries
    // no clips in this job; keeping it would only add noise to the diff.
    .filter((t) => t.clips.length > 0)
  return { tracks }
}

/** Compare two canonical timelines. src_in/src_out/gap-duration compare with
 *  tolerance (split/insert positions are click-derived in Mode B); everything
 *  else exact. Returns a list of mismatch strings (empty = equal). */
export function diffTimelines(a, b, tol = DEFAULT_TOLERANCE_MS) {
  const out = []
  const ta = a?.tracks || []
  const tb = b?.tracks || []
  if (ta.length !== tb.length) {
    out.push(`track count: ${ta.length} vs ${tb.length} (${ta.map((t) => t.id)} vs ${tb.map((t) => t.id)})`)
    return out
  }
  for (let i = 0; i < ta.length; i++) {
    const x = ta[i]; const y = tb[i]
    const at = `${x.kind}:${x.id}`
    if (x.kind !== y.kind) { out.push(`${at}: kind ${x.kind} vs ${y.kind}`); continue }
    if (x.clips.length !== y.clips.length) {
      out.push(`${at}: clip count ${x.clips.length} vs ${y.clips.length}`)
      continue
    }
    for (let j = 0; j < x.clips.length; j++) {
      const c = x.clips[j]; const d = y.clips[j]
      const here = `${at}[${j}]`
      if (Boolean(c.gap) !== Boolean(d.gap)) { out.push(`${here}: gap vs clip`); continue }
      if (c.gap) {
        if (!near(c.duration_ms, d.duration_ms, tol)) out.push(`${here}: gap ${c.duration_ms} vs ${d.duration_ms} (>±${tol}ms)`)
        continue
      }
      if (c.asset !== d.asset) out.push(`${here}: asset ${c.asset} vs ${d.asset}`)
      if (!near(c.src_in_ms, d.src_in_ms, tol)) out.push(`${here}: src_in ${c.src_in_ms} vs ${d.src_in_ms} (>±${tol}ms)`)
      if (!near(c.src_out_ms, d.src_out_ms, tol)) out.push(`${here}: src_out ${c.src_out_ms} vs ${d.src_out_ms} (>±${tol}ms)`)
      for (const k of ['xfade_in_ms', 'xfade_kind', 'title_text']) {
        if ((c[k] ?? null) !== (d[k] ?? null)) out.push(`${here}: ${k} ${c[k] ?? 'none'} vs ${d[k] ?? 'none'}`)
      }
      const gradeDiff = diffArgs(c.grade ?? null, d.grade ?? null, tol, `${here}.grade`)
      if (gradeDiff) out.push(gradeDiff)
      const rampDiff = diffArgs(c.speed_ramp_points ?? null, d.speed_ramp_points ?? null, tol, `${here}.speed_ramp`)
      if (rampDiff) out.push(rampDiff)
      const fadeDiff = diffArgs(c.fade ?? null, d.fade ?? null, tol, `${here}.fade`)
      if (fadeDiff) out.push(fadeDiff)
      const fxDiff = diffArgs(c.effects ?? null, d.effects ?? null, tol, `${here}.effects`)
      if (fxDiff) out.push(fxDiff)
    }
  }
  return out
}

// ── layer attribution ────────────────────────────────────────────────────────
/** Attribution for ONE step given both runs' step records and (optionally) the
 *  spec args the step should have dispatched. Deterministic rules — see the
 *  file header for the layer model. Returns null when the step is convergent
 *  (a ui-gap fallback is reported as a finding, not a divergence). */
export function attributeStep(stepId, a, b, tol = DEFAULT_TOLERANCE_MS) {
  if (!a && !b) return null
  if (!a || !b) {
    return {
      step: stepId, kind: 'divergence', layer: !a ? 'harness' : 'harness',
      detail: `step recorded in only one mode (A=${Boolean(a)}, B=${Boolean(b)}) — runner defect, fix the harness before trusting the verdict`,
    }
  }
  if (b.uiGap) {
    // No clickable affordance — Mode B fell back to the verb and SAID SO.
    return { step: stepId, kind: 'ui-gap', layer: 'ui', detail: b.uiGap }
  }
  // Operator-filtered steps (DSJ_SKIP_STEPS — diagnostic runs around a known
  // blocker) are excluded from the verdict when BOTH modes skipped them; a
  // one-sided skip is a runner defect.
  if (a.phase === 'filtered' && b.phase === 'filtered') return null
  if (a.phase === 'filtered' || b.phase === 'filtered') {
    return { step: stepId, kind: 'divergence', layer: 'harness', detail: `step filtered in only one mode (A=${a.phase}, B=${b.phase}) — the skip list must apply to both` }
  }
  // A step skipped because an EARLIER step already failed is downstream
  // fallout, not a fresh divergence — the earlier finding carries the blame.
  if (a.phase === 'skipped' || b.phase === 'skipped') {
    return { step: stepId, kind: 'downstream', layer: 'downstream', detail: `${a.phase === 'skipped' ? 'A' : 'B'} skipped after an earlier step failed` }
  }
  if (a.ok && b.ok) {
    // Both landed: compare what they landed (normalized per-step ops).
    const argDiff = diffArgs(a.op?.args ?? null, b.op?.args ?? null, tol)
    if (a.op?.verb !== b.op?.verb) {
      return { step: stepId, kind: 'divergence', layer: 'ui', detail: `verb ${a.op?.verb || 'none'} vs ${b.op?.verb || 'none'}` }
    }
    if (argDiff) {
      // Which side deviated from the shared spec? If A matches the spec and B
      // does not → the UI mis-wired the control's args; the reverse blames the
      // agent runner (harness); neither matching is an engine/spec drift.
      const aSpec = a.specArgs ? diffSpecArgs(a.specArgs, a.op?.args ?? null, tol) : null
      const bSpec = b.specArgs ? diffSpecArgs(b.specArgs, b.op?.args ?? null, tol) : null
      const layer = aSpec && !bSpec ? 'harness' : 'ui'
      return { step: stepId, kind: 'divergence', layer, detail: `landed args differ: ${argDiff}${aSpec ? `; A off-spec: ${aSpec}` : ''}${bSpec ? `; B off-spec: ${bSpec}` : ''}` }
    }
    return null
  }
  if (a.ok && !b.ok) {
    // UI side failed. Where in the four-dimension ladder did it stop?
    if (b.phase === 'present' || b.phase === 'render' || b.phase === 'click') {
      return { step: stepId, kind: 'divergence', layer: 'ui', detail: `UI ${b.phase} failure: ${b.error || 'control unusable'}` }
    }
    // The UI reached the click but the RESULT is missing or rejected. If NO op
    // landed at all this is the classic "clicked but nothing happened" — a UI
    // wiring bug by the PRESENT/RENDER/CLICK/RESULT discipline. Only when an
    // op DID land (engine recorded the UI's dispatch) can the blame move: on-
    // spec args + engine refusal = the engine treats the UI path differently;
    // off-spec args = the UI sent the wrong thing.
    if (!b.op) {
      return { step: stepId, kind: 'divergence', layer: 'ui', detail: `UI result failure — clicked but no op landed: ${b.error || 'error'}` }
    }
    const bSpec = b.specArgs ? diffSpecArgs(b.specArgs, b.op.args ?? null, tol) : null
    return {
      step: stepId, kind: 'divergence', layer: bSpec ? 'ui' : 'engine',
      detail: `UI dispatch recorded but step failed while the agent call succeeded: ${b.error || 'error'}${bSpec ? `; B off-spec: ${bSpec}` : ' (args on spec — engine treats the UI path differently)'}`,
    }
  }
  if (!a.ok && b.ok) {
    return { step: stepId, kind: 'divergence', layer: 'api', detail: `agent-surface call failed while the UI performed the same edit: ${a.error || 'error'}` }
  }
  return { step: stepId, kind: 'divergence', layer: 'engine', detail: `both surfaces failed: A=${a.error || 'error'}; B=${b.error || 'error'}` }
}

// ── the run diff ─────────────────────────────────────────────────────────────
/** Diff two complete run records (see the runner for the record contract).
 *  Returns { verdict, findings, opsDiff, timelineDiff, verifyDiff } where
 *  verdict is 'convergent' | 'divergent'; ui-gap findings do NOT make a run
 *  divergent (they are product findings, not harness failures). */
export function diffRuns(runA, runB, { toleranceMs = DEFAULT_TOLERANCE_MS } = {}) {
  const findings = []

  // 1. Per-step verdicts + landed-arg comparison.
  const aSteps = new Map((runA.steps || []).map((s) => [s.id, s]))
  const bSteps = new Map((runB.steps || []).map((s) => [s.id, s]))
  for (const { id } of JOB_STEPS) {
    const f = attributeStep(id, aSteps.get(id), bSteps.get(id), toleranceMs)
    if (f) findings.push(f)
  }

  // 2. Normalized op sequences (order matters — an extra/missing/reordered op
  //    is real divergence even if the final states happen to converge).
  const mapsA = buildRoleMaps(runA.state, runA.importedAssetIds)
  const mapsB = buildRoleMaps(runB.state, runB.importedAssetIds)
  const opsA = normalizeOps(runA.ops, mapsA)
  const opsB = normalizeOps(runB.ops, mapsB)
  const opsDiff = []
  const n = Math.max(opsA.length, opsB.length)
  for (let i = 0; i < n; i++) {
    const x = opsA[i]; const y = opsB[i]
    if (!x || !y) { opsDiff.push(`op[${i}]: ${x ? x.verb : 'missing'} vs ${y ? y.verb : 'missing'}`); continue }
    if (x.verb !== y.verb) { opsDiff.push(`op[${i}]: ${x.verb} vs ${y.verb}`); continue }
    const d = diffArgs(x.args, y.args, toleranceMs)
    if (d) opsDiff.push(`op[${i}] ${x.verb}: ${d}`)
  }
  if (opsDiff.length) {
    findings.push({ step: 'op-log', kind: 'divergence', layer: 'engine', detail: `${opsDiff.length} op-sequence mismatch(es); first: ${opsDiff[0]}` })
  }

  // 3. Final timelines.
  const canonA = canonicalizeTimeline(runA.state, runA.importedAssetIds)
  const canonB = canonicalizeTimeline(runB.state, runB.importedAssetIds)
  const timelineDiff = diffTimelines(canonA, canonB, toleranceMs)
  if (timelineDiff.length) {
    // If a step already diverged, the state diff is downstream fallout of that
    // step; only when every step converged is a state mismatch an ENGINE bug
    // (same landed ops, different materialized state).
    const stepDiverged = findings.some((f) => f.kind === 'divergence' && f.step !== 'op-log')
    findings.push({
      step: 'timeline', kind: 'divergence',
      layer: stepDiverged ? 'downstream' : 'engine',
      detail: `${timelineDiff.length} timeline mismatch(es); first: ${timelineDiff[0]}`,
    })
  }

  // 4. Render verify.checks — compare OUTCOME EQUALITY by check name (absolute
  //    pass/fail is environment-dependent and reported separately; the dual-
  //    surface question is only "do both surfaces produce the same result?").
  const verifyDiff = []
  const checksA = new Map((runA.verify?.checks || []).map((c) => [c.name, Boolean(c.pass)]))
  const checksB = new Map((runB.verify?.checks || []).map((c) => [c.name, Boolean(c.pass)]))
  for (const name of new Set([...checksA.keys(), ...checksB.keys()])) {
    if (!checksA.has(name) || !checksB.has(name)) { verifyDiff.push(`${name}: present A=${checksA.has(name)} B=${checksB.has(name)}`); continue }
    if (checksA.get(name) !== checksB.get(name)) verifyDiff.push(`${name}: A=${checksA.get(name)} B=${checksB.get(name)}`)
  }
  if ((runA.verify == null) !== (runB.verify == null)) verifyDiff.push(`verify receipt present: A=${runA.verify != null} B=${runB.verify != null}`)
  if (verifyDiff.length) {
    findings.push({ step: 'verify', kind: 'divergence', layer: 'engine', detail: verifyDiff.join('; ') })
  }

  const verdict = findings.some((f) => f.kind === 'divergence') ? 'divergent' : 'convergent'
  return { verdict, findings, opsDiff, timelineDiff, verifyDiff, canonA, canonB }
}

// ── receipt ──────────────────────────────────────────────────────────────────
/** Build the JSON receipt for a dual (or single-mode) run. Follows the repo's
 *  receipt discipline: schema-tagged, self-describing, shareable (no secrets —
 *  paths inside are run-local scratch paths). */
export function buildDualSurfaceReceipt({ runA = null, runB = null, diff = null, stack = {}, startedAt, tolerance = DEFAULT_TOLERANCE_MS, broken = '', filtered = [] }) {
  const stepSummary = (run) => (run?.steps || []).map((s) => ({
    id: s.id,
    surface: s.surface,
    ok: Boolean(s.ok),
    ...(s.uiGap ? { uiGap: s.uiGap } : {}),
    ...(s.phase ? { phase: s.phase } : {}),
    ...(s.op ? { op: s.op } : {}), // the landed (normalized) op — diff evidence
    ...(s.shot ? { shot: s.shot } : {}), // failure screenshot (Mode B)
    ...(s.error ? { error: String(s.error).slice(0, 300) } : {}),
    ...(s.detail ? { detail: String(s.detail).slice(0, 300) } : {}),
  }))
  const uiGaps = (runB?.steps || []).filter((s) => s.uiGap).map((s) => ({ step: s.id, gap: s.uiGap }))
  const ok = Boolean(
    (runA ? runA.steps.every((s) => s.ok) : true)
    && (runB ? runB.steps.every((s) => s.ok || s.uiGap) : true)
    && (!diff || diff.verdict === 'convergent'),
  )
  return {
    schema: 'shellx-cut/dual-surface-job@1',
    startedAt,
    endedAt: new Date().toISOString(),
    toleranceMs: tolerance,
    ...(broken ? { deliberatelyBroken: broken } : {}),
    // A filtered run is a DIAGNOSTIC run — never a release verdict.
    ...(filtered.length ? { filteredSteps: filtered, diagnosticOnly: true } : {}),
    stack,
    job: JOB_STEPS.map(({ id, desc }) => ({ id, desc })),
    modeA: runA ? { steps: stepSummary(runA), verify: runA.verify ?? null, render: runA.render ?? null } : null,
    modeB: runB ? { steps: stepSummary(runB), verify: runB.verify ?? null, render: runB.render ?? null, uiGaps } : null,
    diff: diff ? { verdict: diff.verdict, findings: diff.findings, opsDiff: diff.opsDiff, timelineDiff: diff.timelineDiff, verifyDiff: diff.verifyDiff } : null,
    ok,
  }
}
