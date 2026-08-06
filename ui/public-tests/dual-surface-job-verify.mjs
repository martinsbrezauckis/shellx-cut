// dual-surface-job-verify.mjs — the LIVE dual-surface real-job harness.
//
// ONE realistic editorial job (create project → import 2 clips → insert →
// splits → ripple delete → crossfades → grades → speed ramp → typed title →
// fade → render → verify) executed TWICE against fresh cutd instances:
//
//   Mode A (agent surface): pure Debug-API verbs — exercises ENGINE + API.
//   Mode B (UI surface): the same job through real Playwright-chromium UI
//     interactions over the data-cut-* selector contract (ruler clicks +
//     keyboard splits, seam popovers, Color-tab sliders, Inspector sections,
//     a typed title, the topbar Render button) — exercises UI WIRING on the
//     same engine. Where a step has NO clickable affordance the step falls
//     back to the verb and records an explicit UI-COVERAGE GAP finding — the
//     gap is the product answer, never silently verbed around.
//
// The two runs are DIFFED by scripts/lib/dual-surface-job.mjs (op sequences,
// canonical final timelines, render verify.checks outcomes) and the receipt
// says WHICH layer a divergence lives in (engine / api / ui).
//
// STACKS — this runner NEVER spawns cutd. It drives externally-provided
// stacks, one per mode (the fresh-instance guarantee is the launcher's job —
// use scripts/release/dual-surface-job-gate.mjs for one-command cold-start):
//   SWEEP_CUTD_A=<base>   Mode A engine (API only; no UI needed)
//   SWEEP_CUTD_B=<base>   Mode B engine (must serve the built UI or pair with
//   SWEEP_APP_B=<url>     a separate UI origin; defaults to SWEEP_CUTD_B)
//   SWEEP_CUTD=<base>     fallback for BOTH modes (sequential, one instance —
//                         weaker isolation: each mode still gets its own
//                         project, but engine-process state is shared)
//   DSJ_EXTERNAL_ISOLATED=1  acknowledgment that the launcher started the
//                         stack(s) with an ISOLATED SHELLX_CUT_HOME +
//                         SHELLX_CUT_PROJECTS_DIR (project.create is sent
//                         name-only, mirroring the UI payload, so isolation
//                         cannot be arg-injected here). REQUIRED.
// Options:
//   DSJ_MODE=both|a|b     which surfaces to run (default both; single-mode
//                         runs skip the diff and verdict on step results only)
//   DSJ_RECEIPT=<path>    receipt destination (default <repo>/.shellx-scratch/
//                         dual-surface/receipt-<ts>.json)
//   DSJ_MEDIA_DIR=<dir>   where the 2 synthetic test clips are generated
//                         (default: a run-owned temp dir; requires ffmpeg).
//                         Clips are VIDEO-ONLY by design: the UI's "Add at
//                         playhead" places linked A/V pairs for clips with
//                         audio while the agent's edit.insert targets v1 only
//                         — silent sources keep the job expressible
//                         identically on both surfaces.
//   DSJ_BREAK=<step>:<a|b>  deliberately sabotage one step in one mode (the
//                         live red-proof; supported: xfade-1, grade-first)
//   DSJ_SKIP_STEPS=<id,…> DIAGNOSTIC filter: skip these steps in BOTH modes
//                         (e.g. around a known blocking bug so the rest of the
//                         pipe still gets exercised). The receipt is stamped
//                         diagnosticOnly — NEVER a release verdict.
//   DSJ_TOLERANCE_MS=<n>  boundary tolerance (default 40 — Mode B split/seek
//                         positions are click+frame-nudge derived, ≤½ frame
//                         per seek; typed parameters always compare exact)
//   DSJ_REQUIRE_UI=1      make an unlaunchable browser a FAIL instead of an
//                         honest SKIP (exit 4)
// Exit codes: 0 = convergent (or single-mode all-ok), 1 = divergence or step
// failure, 3 = preflight/bootstrap error, 4 = Mode B skipped (no browser).
//
// Local dev note: Mode B uses HEADLESS chromium (no X display needed). The
// Linux/CI rigs run it identically; xvfb is only needed for headed debugging.
// KNOWN KILLER (Linux WebKitGTK, software rendering): never
// drive the right-rail Color tab under a SOFTWARE-RENDERED WebKit surface —
// headless Chromium here is unaffected (the full-coverage suite drives the
// same tab the same way).
//
// Callers: scripts/release/dual-surface-job-gate.mjs, rig qualification runs.
// Dependencies: playwright (ui/node_modules), ffmpeg on PATH (media synth),
// node 18+ (global fetch). Sibling-lib reuse: fullCoverageProject.mjs +
// fullCoverageJobs.mjs are dependency-injected and side-effect-free.

import { chromium } from 'playwright'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  DEFAULT_TOLERANCE_MS,
  JOB_STEPS,
  JOB_TARGETS,
  buildDualSurfaceReceipt,
  buildRoleMaps,
  diffRuns,
  rampPointsFor,
} from '../../scripts/lib/dual-surface-job.mjs'
import { createProjectWaiters } from './lib/fullCoverageProject.mjs'
import { createJobWaiters } from './lib/fullCoverageJobs.mjs'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = resolve(HERE, '..', '..')

// ── env / config ─────────────────────────────────────────────────────────────
const CUTD_A = process.env.SWEEP_CUTD_A || process.env.SWEEP_CUTD || ''
const CUTD_B = process.env.SWEEP_CUTD_B || process.env.SWEEP_CUTD || ''
const APP_B = process.env.SWEEP_APP_B || CUTD_B
const MODE = (process.env.DSJ_MODE || 'both').toLowerCase()
const TOL = Number(process.env.DSJ_TOLERANCE_MS || DEFAULT_TOLERANCE_MS)
const BREAK = (process.env.DSJ_BREAK || '').trim() // "<step>:<a|b>"
const REQUIRE_UI = process.env.DSJ_REQUIRE_UI === '1'
const RECEIPT_PATH = process.env.DSJ_RECEIPT
  || join(REPO, '.shellx-scratch', 'dual-surface', `receipt-${new Date().toISOString().replace(/[:.]/g, '-')}.json`)

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const log = (...m) => console.log('[dsj]', ...m)
const fail = (msg, code = 3) => { console.error(`[dsj] ✗ ${msg}`); process.exit(code) }

/** Parse DSJ_BREAK and validate it names a supported sabotage. */
function parseBreak(raw) {
  if (!raw) return null
  const m = /^([a-z0-9-]+):([ab])$/.exec(raw)
  if (!m) fail(`DSJ_BREAK must be "<step>:<a|b>", got "${raw}"`)
  const supported = new Set(['xfade-1', 'grade-first'])
  if (!supported.has(m[1])) fail(`DSJ_BREAK step "${m[1]}" unsupported (supported: ${[...supported].join(', ')})`)
  return { step: m[1], mode: m[2] }
}
const BROKEN = parseBreak(BREAK)
const sab = (step, mode) => Boolean(BROKEN && BROKEN.step === step && BROKEN.mode === mode)
const SKIP_STEPS = new Set((process.env.DSJ_SKIP_STEPS || '').split(',').map((s) => s.trim()).filter(Boolean))

// ── verb transport (per stack) ───────────────────────────────────────────────
/** POST /api/verb/<name>. Mode A sends NO actor header (the pure REST-agent
 *  default, mirroring the demo choreography); Mode B fallback verbs mimic the
 *  UI actor exactly like the established full-coverage harness does. */
function makeVerb(base, { actor = '' } = {}) {
  return async function verb(name, args = {}, opts = {}) {
    const timeoutMs = opts.timeoutMs ?? 60_000
    try {
      const r = await fetch(`${base}/api/verb/${name}`, {
        method: 'POST',
        headers: { 'content-type': 'application/json', ...(actor ? { 'x-cut-actor': actor } : {}) },
        body: JSON.stringify(args),
        signal: AbortSignal.timeout(timeoutMs),
      })
      return await r.json()
    } catch (e) {
      return { ok: false, error: { message: String(e) } }
    }
  }
}

async function stackUp(base, label) {
  try {
    const r = await fetch(`${base}/api/verbs`, { signal: AbortSignal.timeout(5000) })
    if (!r.ok) throw new Error(`HTTP ${r.status}`)
  } catch (e) {
    fail(`${label} stack ${base} did not answer GET /api/verbs: ${e.message}`)
  }
}

// ── test media (video-only by design — see header) ───────────────────────────
function ensureMedia() {
  const dir = process.env.DSJ_MEDIA_DIR || mkdtempSync(join(tmpdir(), 'dsj-media-'))
  mkdirSync(dir, { recursive: true })
  const clipA = join(dir, 'dsj-clip-a.mp4')
  const clipB = join(dir, 'dsj-clip-b.mp4')
  const gen = (out, filter) => {
    if (existsSync(out)) return
    const r = spawnSync('ffmpeg', ['-nostats', '-hide_banner', '-loglevel', 'error', '-y',
      '-f', 'lavfi', '-i', filter,
      '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '24', '-pix_fmt', 'yuv420p', '-an', out], { encoding: 'utf8' })
    if (r.error || r.status !== 0) fail(`ffmpeg could not generate ${out}: ${r.error?.message || r.stderr}`)
  }
  gen(clipA, 'testsrc2=size=1280x720:rate=30:duration=10')
  gen(clipB, 'smptebars=size=1280x720:rate=30:duration=4')
  return { dir, clipA, clipB }
}

// ── shared per-run helpers ───────────────────────────────────────────────────
/** Step recorder: runs `fn`, captures the op-log delta, extracts the LAST op
 *  matching the step's verb, and normalizes failures into the step contract
 *  the differ understands ({ok, phase, error, op, specArgs}). A failed step
 *  ABORTS the run: later steps are recorded as phase='skipped' so the differ
 *  blames the first failure instead of every cascaded one. */
function makeRecorder({ verb, waiters, onFail = null }) {
  const steps = []
  let abortedBy = ''
  async function step(id, surface, expectVerb, fn, { specArgs = null, uiGap = '' } = {}) {
    if (SKIP_STEPS.has(id)) {
      // Operator-filtered (DSJ_SKIP_STEPS) — recorded as excluded, applied to
      // BOTH modes by construction; the receipt is stamped diagnosticOnly.
      const rec = { id, surface, ok: true, phase: 'filtered', detail: 'skipped by DSJ_SKIP_STEPS (diagnostic filter — NOT a release verdict)' }
      steps.push(rec)
      log(`  ~ ${id} (${surface}) — FILTERED by DSJ_SKIP_STEPS`)
      return rec
    }
    if (abortedBy) {
      const rec = { id, surface, ok: false, phase: 'skipped', error: `skipped: run aborted after "${abortedBy}" failed`, specArgs }
      steps.push(rec)
      log(`  – ${id} (${surface}) — skipped after ${abortedBy}`)
      return rec
    }
    const before = await waiters.opsLen()
    const rec = { id, surface, ok: false, rawOp: null, specArgs }
    if (uiGap) rec.uiGap = uiGap
    try {
      const out = await fn(before)
      rec.ok = true
      if (out && typeof out === 'object') rec.detail = out.detail || ''
      if (expectVerb) {
        // POLL for the landed op (UI dispatches are async and some verbs do
        // real work before the op lands — title.add renders its PNG first; a
        // single read right after the click false-fails on slow/debug builds).
        const deadline = Date.now() + 30_000
        let landed = []
        for (;;) {
          const all = await waiters.ops()
          landed = all.slice(before).filter((o) => o.verb === expectVerb)
          if (landed.length || Date.now() > deadline) break
          await sleep(400)
        }
        if (!landed.length) {
          rec.ok = false
          rec.phase = 'result'
          rec.error = `${expectVerb} did not land in the op log within 30s`
        } else {
          rec.rawOp = landed[landed.length - 1]
        }
      }
    } catch (e) {
      rec.ok = false
      rec.phase = rec.phase || e.phase || 'result'
      rec.error = String(e.message || e)
    }
    if (!rec.ok) {
      abortedBy = id
      if (onFail) await Promise.resolve(onFail(rec)).catch(() => {})
    }
    steps.push(rec)
    const gapNote = rec.uiGap ? ' [UI-GAP → verb fallback]' : ''
    log(`  ${rec.ok ? '✓' : '✗'} ${id} (${surface})${gapNote}${rec.error ? ` — ${rec.error}` : ''}`)
    return rec
  }
  return { steps, step }
}

/** Post-process a finished run: strip run-local noise from each step's landed
 *  op and remap clip/asset args onto positional roles so the differ compares
 *  run-independent shapes. */
function finalizeRun(run) {
  const maps = buildRoleMaps(run.state, run.importedAssetIds)
  for (const s of run.steps) {
    if (!s.rawOp) continue
    const args = { ...(s.rawOp.args || {}) }
    delete args.rationale
    delete args.group_id
    if (typeof args.clip === 'string') args.clip = maps.clipRole.get(args.clip) || args.clip
    if (typeof args.asset === 'string') args.asset = maps.assetRole.get(args.asset) || args.asset
    // media.import records the absolute source path — reduce to the basename
    // so A/B stacks with different media roots still compare equal.
    if (s.rawOp.verb === 'media.import' && typeof args.path === 'string') {
      args.path = args.path.split(/[\\/]/).pop()
    }
    s.op = { verb: s.rawOp.verb, args }
    delete s.rawOp
  }
  return run
}

/** Video clips of v1 in timeline order (state helper). */
const v1Clips = (s) => (s?.tracks || []).find((t) => t.id === 'v1')?.clips?.filter((c) => c.kind !== 'gap') || []

/** Wait until the render job (newest render-kind job) completes; returns its
 *  final state. Uses jobs.list + jobs.status — API READS are allowed in both
 *  modes (assertion channel), only MUTATIONS must go through the surface. */
async function awaitRender(verb, { timeoutMs = 420_000 } = {}) {
  const t0 = Date.now()
  let jobId = ''
  while (Date.now() - t0 < timeoutMs) {
    if (!jobId) {
      const list = await verb('jobs.list', {})
      const renders = (list.result?.jobs || []).filter((j) => /render/.test(`${j.kind || ''}${j.verb || ''}${j.label || ''}`))
      if (renders.length) jobId = renders[renders.length - 1].id || renders[renders.length - 1].job_id || ''
    }
    if (jobId) {
      const st = (await verb('jobs.status', { job_id: jobId })).result
      if (st?.state === 'done') return { ok: true, jobId, jobState: 'done' }
      if (st?.state === 'failed') return { ok: false, jobId, jobState: 'failed', error: JSON.stringify(st.error || st).slice(0, 300) }
    }
    await sleep(1500)
  }
  return { ok: false, jobId, jobState: 'timeout', error: `render did not finish within ${timeoutMs}ms` }
}

// ═════════════════════════════════════════════════════════════════════════════
// MODE A — the agent surface (pure verbs)
// ═════════════════════════════════════════════════════════════════════════════
async function runModeA(base, media) {
  log(`MODE A (agent surface) against ${base}`)
  const verb = makeVerb(base) // no actor header: the pure REST-agent default
  const waiters = createProjectWaiters({ verb, sleep })
  const jobs = createJobWaiters({ verb, sleep })
  const { steps, step } = makeRecorder({ verb, waiters })
  const T = JOB_TARGETS
  const importedAssetIds = []

  await step('create-project', 'verb', 'project.create', async () => {
    // Name-only payload — exactly what the UI's Projects panel sends.
    const r = await verb('project.create', { name: T.projectName })
    if (!r.ok) throw new Error(JSON.stringify(r.error).slice(0, 200))
  }, { specArgs: { name: T.projectName } })

  for (const [id, clip] of [['import-a', media.clipA], ['import-b', media.clipB]]) {
    await step(id, 'verb', 'media.import', async () => {
      const r = await verb('media.import', { path: clip })
      if (!r.ok || !r.result?.asset_id) throw new Error(JSON.stringify(r.error || r).slice(0, 200))
      importedAssetIds.push(r.result.asset_id)
      await jobs.awaitImportJobs(r)
    })
  }

  await step('insert-b', 'verb', 'edit.insert', async () => {
    const r = await verb('edit.insert', { asset: importedAssetIds[1], track: 'v1', at_ms: T.insertAtMs })
    if (!r.ok) throw new Error(JSON.stringify(r.error).slice(0, 200))
  })

  for (const [id, at] of [['split-1', T.split1Ms], ['split-2', T.split2Ms]]) {
    await step(id, 'verb', 'edit.split', async () => {
      const r = await verb('edit.split', { track: 'v1', at_ms: at })
      if (!r.ok) throw new Error(JSON.stringify(r.error).slice(0, 200))
    })
  }

  await step('ripple-delete', 'verb', 'edit.ripple_delete', async () => {
    // The UI's Delete key sends the SELECTED clip's exact boundaries — mirror
    // that payload shape (track + range + ripple:true).
    const r = await verb('edit.ripple_delete', { track: 'v1', range_ms: [T.split1Ms, T.split2Ms], ripple: true })
    if (!r.ok) throw new Error(JSON.stringify(r.error).slice(0, 200))
  })

  // Seam positions and clip references are read from LIVE state inside each
  // step (seam N = end of clip N) so the agent targets the engine's actual
  // boundaries, like a user targets what they see — and so an aborted run
  // never dereferences state a failed step didn't produce.
  const clipsNow = async () => {
    const clips = v1Clips(await waiters.state())
    if (clips.length < 3) throw new Error(`expected ≥3 v1 clips after ripple, got ${clips.length}`)
    return clips
  }
  const durOf = (c) => c.src_out_ms - c.src_in_ms // constant-speed at this point

  for (const [id, seamIndex, dur] of [['xfade-1', 0, sab('xfade-1', 'a') ? 250 : T.xfade1Ms], ['xfade-2', 1, T.xfade2Ms]]) {
    await step(id, 'verb', 'edit.crossfade', async () => {
      const clips = await clipsNow()
      const at = clips.slice(0, seamIndex + 1).reduce((sum, c) => sum + durOf(c), 0)
      const r = await verb('edit.crossfade', { track: 'v1', at_ms: at, duration_ms: dur, transition: T.xfadeTransition })
      if (!r.ok) throw new Error(JSON.stringify(r.error).slice(0, 200))
    }, { specArgs: { track: 'v1', duration_ms: id === 'xfade-1' ? T.xfade1Ms : T.xfade2Ms, transition: T.xfadeTransition } })
  }

  const gradeArgs = (clipId, g, brokenContrast) => ({
    clip: clipId,
    contrast: brokenContrast ?? g.contrast,
    brightness: g.brightness,
    saturation: g.saturation,
    gamma: g.gamma,
    temperature_k: g.temperature_k,
  })
  await step('grade-first', 'verb', 'edit.grade', async () => {
    const clips = await clipsNow()
    const args = gradeArgs(clips[0].id, T.gradeFirst, sab('grade-first', 'a') ? 1.3 : undefined)
    const r = await verb('edit.grade', args)
    if (!r.ok) throw new Error(JSON.stringify(r.error).slice(0, 200))
  }, { specArgs: { ...gradeArgs('video[0]@v1', T.gradeFirst) } })
  await step('grade-inserted', 'verb', 'edit.grade', async () => {
    const clips = await clipsNow()
    const r = await verb('edit.grade', gradeArgs(clips[2].id, T.gradeInserted))
    if (!r.ok) throw new Error(JSON.stringify(r.error).slice(0, 200))
  }, { specArgs: { ...gradeArgs('video[2]@v1', T.gradeInserted) } })

  await step('speed-ramp', 'verb', 'edit.speed_ramp', async () => {
    // Same preset curve the UI applies, computed over the clip's SOURCE window.
    const clips = await clipsNow()
    const src = clips[2].src_out_ms - clips[2].src_in_ms
    const r = await verb('edit.speed_ramp', { clip: clips[2].id, points: rampPointsFor(T.rampPreset, src) })
    if (!r.ok) throw new Error(JSON.stringify(r.error).slice(0, 200))
  })

  await step('title', 'verb', 'title.add', async () => {
    const r = await verb('title.add', { text: T.title.text, range_ms: T.title.range_ms, preset: T.title.preset })
    if (!r.ok) throw new Error(JSON.stringify(r.error).slice(0, 200))
  }, { specArgs: { text: T.title.text, range_ms: T.title.range_ms, preset: T.title.preset } })

  await step('fade-out', 'verb', 'edit.fade', async () => {
    const last = v1Clips(await waiters.state()).at(-1)
    const r = await verb('edit.fade', { clip: last.id, out_ms: T.fadeOutMs })
    if (!r.ok) throw new Error(JSON.stringify(r.error).slice(0, 200))
  })

  let render = null
  await step('render', 'verb', null, async () => {
    // Args mirror the UI Render button's defaults exactly (topbar/index.tsx).
    const r = await verb('render.final', { ...T.render })
    if (!r.ok || !r.result?.job_id) throw new Error(JSON.stringify(r.error || r).slice(0, 200))
    const done = await awaitRender(verb)
    render = done
    if (!done.ok) throw new Error(`render job ${done.jobId || '?'}: ${done.jobState} ${done.error || ''}`)
  })

  let verify = null
  await step('verify', 'verb', null, async () => {
    const r = await verb('verify.checks', {})
    if (!r.ok || !Array.isArray(r.result?.checks)) throw new Error(JSON.stringify(r.error || r).slice(0, 200))
    verify = { pass: Boolean(r.result.pass), checks: r.result.checks.map((c) => ({ name: c.name, pass: Boolean(c.pass) })) }
  })

  const run = {
    mode: 'agent',
    steps,
    ops: await waiters.ops(),
    state: await waiters.state(),
    importedAssetIds,
    verify,
    render,
  }
  return finalizeRun(run)
}

// ═════════════════════════════════════════════════════════════════════════════
// MODE B — the UI surface (Playwright chromium over data-cut-* selectors)
// ═════════════════════════════════════════════════════════════════════════════

/** Throw with a phase tag so the differ can attribute UI failures precisely. */
function phaseError(phase, msg) {
  const e = new Error(msg)
  e.phase = phase
  return e
}

/** Locate + visibility-gate a control, then click it. present/click phases. */
async function uiClick(page, selector, { timeout = 8000, label = selector } = {}) {
  const loc = page.locator(selector).first()
  if (!(await loc.count())) throw phaseError('present', `${label} not in DOM`)
  try { await loc.waitFor({ state: 'visible', timeout }) } catch { throw phaseError('render', `${label} not visible`) }
  try { await loc.click({ timeout }) } catch (e) { throw phaseError('click', `${label} click failed: ${e.message}`) }
}

async function uiFill(page, selector, value, { label = selector } = {}) {
  const loc = page.locator(selector).first()
  if (!(await loc.count())) throw phaseError('present', `${label} not in DOM`)
  try { await loc.fill(String(value)) } catch (e) { throw phaseError('click', `${label} fill failed: ${e.message}`) }
}

async function runModeB(base, appUrl, media) {
  log(`MODE B (UI surface) against engine ${base}, app ${appUrl}`)
  const verb = makeVerb(base, { actor: 'human:ui:ui' }) // fallback/read channel
  const waiters = createProjectWaiters({ verb, sleep })
  const jobs = createJobWaiters({ verb, sleep })
  // Failure screenshots — visual evidence for every failed UI step, saved next
  // to the receipt (pageRef is filled once the browser is up).
  const pageRef = { current: null }
  const shotsDir = join(dirname(RECEIPT_PATH), 'shots')
  const { steps, step } = makeRecorder({
    verb,
    waiters,
    onFail: async (rec) => {
      if (!pageRef.current) return
      mkdirSync(shotsDir, { recursive: true })
      const shot = join(shotsDir, `fail-${rec.id}.png`)
      await pageRef.current.screenshot({ path: shot, fullPage: false })
      rec.shot = shot
      log(`    screenshot → ${shot}`)
    },
  })
  const T = JOB_TARGETS
  const importedAssetIds = []

  let browser
  try {
    browser = await chromium.launch({ headless: true })
  } catch (e) {
    if (REQUIRE_UI) fail(`DSJ_REQUIRE_UI=1 but chromium failed to launch: ${e.message}`, 1)
    log(`SKIP Mode B — chromium failed to launch on this host: ${e.message}`)
    log('     (run Mode B on a Linux rig / CI ubuntu runner; headless chromium needs no X display, only its system libraries)')
    return null
  }
  const page = await (await browser.newContext({ viewport: { width: 1600, height: 950 } })).newPage()
  pageRef.current = page
  const consoleErrors = []
  page.on('pageerror', (e) => consoleErrors.push(String(e).slice(0, 200)))

  /** DOM + topbar settle after load/reload (networkidle never fires — the app
   *  polls jobs sub-500ms; same rationale as the full-coverage bootstrap). */
  async function settleApp() {
    await page.waitForSelector('[data-cut-panel="topbar"]', { timeout: 20_000 })
    await sleep(500)
  }
  async function reloadApp() {
    await page.reload({ waitUntil: 'domcontentloaded' })
    await settleApp()
  }

  /** ui.state read helper (server-confirmed UI state: playhead + selection). */
  async function uiState() {
    return (await verb('ui.state', {}, { timeoutMs: 5000 })).result || {}
  }

  /** Seek the playhead to `target` ms like a user: Shift+Z fit-to-window, one
   *  calibration click on the ruler (learns px↔ms), a corrected click, then
   *  ArrowLeft/ArrowRight frame nudges (Shift = 10 frames) until within half a
   *  frame. Returns the ACHIEVED playhead (recorded, never pretended exact). */
  async function seekPlayhead(target) {
    await page.keyboard.press('Escape')
    await page.keyboard.press('Shift+Z') // fit the whole timeline
    await sleep(250)
    const ruler = page.locator('[data-cut-ruler]').first()
    if (!(await ruler.count())) throw phaseError('present', 'timeline ruler ([data-cut-ruler]) not in DOM')
    const box = await ruler.boundingBox()
    if (!box) throw phaseError('render', 'timeline ruler has no bounding box')
    const readPh = async () => {
      const s = await uiState()
      return Number.isFinite(s.playhead_ms) ? s.playhead_ms : NaN
    }
    // Calibration click at 25% width, then linear-fit a corrected click.
    const x1 = box.x + box.width * 0.25
    await page.mouse.click(x1, box.y + box.height / 2)
    await sleep(350)
    const p1 = await readPh()
    if (!Number.isFinite(p1)) throw phaseError('result', 'ui.state has no playhead_ms after a ruler click')
    const x2 = box.x + box.width * 0.75
    await page.mouse.click(x2, box.y + box.height / 2)
    await sleep(350)
    const p2 = await readPh()
    const msPerPx = (p2 - p1) / (x2 - x1)
    if (!(msPerPx > 0)) throw phaseError('result', `ruler calibration failed (msPerPx=${msPerPx})`)
    const xt = Math.min(box.x + box.width - 2, Math.max(box.x + 2, x1 + (target - p1) / msPerPx))
    await page.mouse.click(xt, box.y + box.height / 2)
    await sleep(350)
    // Frame-nudge to the target (fps from project settings; default 30).
    const fps = (await waiters.state())?.settings?.fps || 30
    const frameMs = 1000 / fps
    let ph = await readPh()
    for (let i = 0; i < 80 && Math.abs(ph - target) > frameMs / 2; i++) {
      const frames = Math.round((target - ph) / frameMs)
      const big = Math.abs(frames) >= 10
      const key = frames > 0 ? 'ArrowRight' : 'ArrowLeft'
      await page.keyboard.press(big ? `Shift+${key}` : key)
      await sleep(160)
      ph = await readPh()
    }
    if (Math.abs(ph - target) > frameMs / 2 + 1) {
      throw phaseError('result', `could not seek to ${target}ms via ruler+nudges (achieved ${ph}ms)`)
    }
    return ph
  }

  /** Click a timeline clip until the selection is confirmed. The rendered
   *  selection class is accepted as primary evidence (the full-coverage
   *  convention): the app's ui-state push to the server is debounced and can
   *  lag several hundred ms when the engine is busy (e.g. right after
   *  title.add renders its PNG), while the DOM class is the product state. */
  async function selectClip(clipId) {
    const loc = page.locator(`[data-cut-clip="${clipId}"]`).first()
    if (!(await loc.count())) throw phaseError('present', `timeline clip [data-cut-clip="${clipId}"] not in DOM`)
    for (let i = 0; i < 8; i++) {
      await loc.scrollIntoViewIfNeeded().catch(() => {})
      await loc.click().catch(() => loc.click({ force: true }).catch(() => {}))
      await sleep(350)
      if (await loc.evaluate((el) => el.classList.contains('tl-clip--selected')).catch(() => false)) return
      const sel = (await uiState()).selected_clip_ids || []
      if (sel.includes(clipId)) return
    }
    throw phaseError('result', `clip ${clipId} never confirmed selected (no tl-clip--selected class, not in ui.state)`)
  }

  /** Expand the right rail if collapsed (it defaults collapsed on a fresh
   *  browser profile — same guard as the full-coverage bootstrap). */
  async function ensureRail() {
    const expand = page.locator('[data-cut-action="expand-rail"]')
    if (await expand.count()) { await expand.click().catch(() => {}); await sleep(300) }
  }

  /** Open an Inspector section (they default collapsed). */
  async function openInspectorSection(key) {
    await ensureRail()
    await uiClick(page, '[data-cut-right-tab="properties"]', { label: 'Inspector tab' })
    await sleep(250)
    const section = page.locator(`[data-cut-section="${key}"]`).first()
    if (!(await section.count())) throw phaseError('present', `inspector section "${key}" not in DOM`)
    if ((await section.getAttribute('data-cut-section-collapsed')) === 'true') {
      await uiClick(page, `[data-cut-section-toggle="${key}"]`, { label: `inspector section toggle "${key}"` })
      await sleep(250)
    }
  }

  // ── bootstrap ───────────────────────────────────────────────────────────────
  try {
    await page.goto(appUrl, { waitUntil: 'domcontentloaded', timeout: 30_000 })
    await settleApp()
  } catch (e) {
    await browser.close()
    fail(`Mode B app did not load at ${appUrl}: ${e.message}`)
  }

  // ── the job, through the UI ────────────────────────────────────────────────
  await step('create-project', 'ui', 'project.create', async () => {
    await uiClick(page, '[data-cut-left-tab="projects"]', { label: 'Projects tab' })
    await uiFill(page, '[data-cut-projects-newname]', T.projectName, { label: 'new-project name input' })
    await uiClick(page, '[data-cut-projects-create]', { label: 'Create project button' })
    const opened = await waiters.waitForState((s) => s?.name === T.projectName, 15_000)
    if (!opened) throw phaseError('result', 'project did not open after Create click')
  }, { specArgs: { name: T.projectName } })

  // UI-COVERAGE GAP: media import is a NATIVE OS PICKER only. In the browser
  // build the "+ Import" button explicitly refuses ("Open the desktop app to
  // browse for files", Assets/index.tsx browseImport) and there is no DOM file
  // input to drive. The installed-app OS-action gate owns picker proof; here
  // the step falls back to the verb and RECORDS THE GAP as a product finding.
  const IMPORT_GAP = 'media import has no DOM affordance (native OS picker only; browser build refuses with "Open the desktop app to browse for files")'
  for (const [id, clip] of [['import-a', media.clipA], ['import-b', media.clipB]]) {
    await step(id, 'verb-fallback', 'media.import', async () => {
      const r = await verb('media.import', { path: clip })
      if (!r.ok || !r.result?.asset_id) throw new Error(JSON.stringify(r.error || r).slice(0, 200))
      importedAssetIds.push(r.result.asset_id)
      await jobs.awaitImportJobs(r)
    }, { uiGap: IMPORT_GAP })
  }
  await reloadApp() // reflect the verb-side imports in the UI (bootstrap-style)
  await uiClick(page, '[data-cut-left-tab="assets"]', { label: 'Assets tab' })
  await sleep(300)

  let achievedInsertAt = NaN
  await step('insert-b', 'ui', 'edit.insert', async () => {
    achievedInsertAt = await seekPlayhead(T.insertAtMs)
    const row = page.locator(`[data-cut-asset-card="${importedAssetIds[1]}"]`).first()
    if (!(await row.count())) throw phaseError('present', `asset card for ${importedAssetIds[1]} not in DOM`)
    const btn = row.locator('[data-cut-action="insert-asset"]').first()
    if (!(await btn.count())) throw phaseError('present', 'Add-at-playhead button not in asset card')
    await btn.click()
    const grown = await waiters.waitForState((s) => v1Clips(s).length >= 3, 12_000)
    if (!grown) throw phaseError('result', 'timeline did not grow to 3 clips after Add at playhead')
    return { detail: `inserted at achieved playhead ${achievedInsertAt}ms (target ${T.insertAtMs})` }
  })

  for (const [id, target] of [['split-1', T.split1Ms], ['split-2', T.split2Ms]]) {
    await step(id, 'ui', 'edit.split', async () => {
      const achieved = await seekPlayhead(target)
      await page.keyboard.press('Escape') // no selection → split targets video tracks
      await sleep(150)
      const before = v1Clips(await waiters.state()).length
      await page.keyboard.press('Control+b') // canonical "cut here"
      const grown = await waiters.waitForState((s) => v1Clips(s).length > before, 10_000)
      if (!grown) throw phaseError('result', `clip count did not grow after Ctrl+B at ${achieved}ms`)
      return { detail: `split at achieved ${achieved}ms (target ${target})` }
    })
  }

  await step('ripple-delete', 'ui', 'edit.ripple_delete', async () => {
    // The middle piece = the clip whose source window starts at split-1's cut
    // (source A, src_in ≈ split1 achieved). Resolve from live state.
    const clips = v1Clips(await waiters.state())
    const mid = clips.find((c) => Math.abs(c.src_in_ms - T.split1Ms) <= TOL && Math.abs(c.src_out_ms - T.split2Ms) <= TOL)
    if (!mid) throw phaseError('result', `could not locate the middle clip (~[${T.split1Ms},${T.split2Ms}]) in ${clips.map((c) => `[${c.src_in_ms},${c.src_out_ms}]`).join(' ')}`)
    await selectClip(mid.id)
    const before = v1Clips(await waiters.state()).length
    await page.keyboard.press('Delete')
    const shrunk = await waiters.waitForState((s) => v1Clips(s).length < before, 10_000)
    if (!shrunk) throw phaseError('result', 'clip count did not shrink after Delete')
  })

  for (const [id, seamIndex, dur] of [['xfade-1', 0, sab('xfade-1', 'b') ? 250 : T.xfade1Ms], ['xfade-2', 1, T.xfade2Ms]]) {
    await step(id, 'ui', 'edit.crossfade', async () => {
      const clips = v1Clips(await waiters.state())
      const left = clips[seamIndex]; const right = clips[seamIndex + 1]
      if (!left || !right) throw phaseError('result', `no seam ${seamIndex} — only ${clips.length} clips`)
      await page.keyboard.press('Escape')
      await sleep(150)
      await uiClick(page, `[data-cut-seam="${left.id}:${right.id}"]`, { label: `seam ${left.id}:${right.id}` })
      const pop = page.locator('[data-cut-xfade-pop]').first()
      try { await pop.waitFor({ state: 'visible', timeout: 8000 }) } catch { throw phaseError('render', 'crossfade popover did not open') }
      await uiFill(page, '[data-cut-xfade-input]', dur, { label: 'crossfade duration input' })
      await uiClick(page, '[data-cut-action="apply-xfade"]', { label: 'Apply crossfade' })
      await sleep(300)
    }, { specArgs: { track: 'v1', duration_ms: id === 'xfade-1' ? T.xfade1Ms : T.xfade2Ms, transition: T.xfadeTransition } })
  }

  /** Drive the right-rail Color tab for one clip (the UI's edit.grade path). */
  async function gradeViaColorTab(clipId, g, brokenContrast) {
    await selectClip(clipId)
    await ensureRail()
    await uiClick(page, '[data-cut-right-tab="color"]', { label: 'Color tab' })
    const panel = page.locator('[data-cut-grade-embed]').first()
    try { await panel.waitFor({ state: 'visible', timeout: 8000 }) } catch { throw phaseError('render', 'grade panel did not render') }
    await uiFill(page, '[data-cut-grade-input="contrast"]', brokenContrast ?? g.contrast, { label: 'contrast slider' })
    await uiFill(page, '[data-cut-grade-input="saturation"]', g.saturation, { label: 'saturation slider' })
    const tempOn = page.locator('[data-cut-grade-temp-on]').first()
    if (!(await tempOn.count())) throw phaseError('present', 'temperature toggle not in DOM')
    if ((await tempOn.getAttribute('aria-pressed')) !== 'true' && (await tempOn.getAttribute('data-cut-grade-temp-enabled')) !== 'true') {
      await tempOn.click()
      await sleep(200)
    }
    await uiFill(page, '[data-cut-grade-input="temperature_k"]', g.temperature_k, { label: 'temperature slider' })
    await uiClick(page, '[data-cut-grade-apply]', { label: 'Apply grade' })
    await sleep(400)
  }

  await step('grade-first', 'ui', 'edit.grade', async () => {
    const clips = v1Clips(await waiters.state())
    await gradeViaColorTab(clips[0].id, T.gradeFirst, sab('grade-first', 'b') ? 1.3 : undefined)
  }, { specArgs: { clip: 'video[0]@v1', ...T.gradeFirst } })
  await step('grade-inserted', 'ui', 'edit.grade', async () => {
    const clips = v1Clips(await waiters.state())
    await gradeViaColorTab(clips[2].id, T.gradeInserted)
  }, { specArgs: { clip: 'video[2]@v1', ...T.gradeInserted } })

  await step('speed-ramp', 'ui', 'edit.speed_ramp', async () => {
    const clips = v1Clips(await waiters.state())
    await selectClip(clips[2].id)
    await openInspectorSection('speed')
    const preset = page.locator('[data-cut-speed-ramp-preset]').first()
    if (!(await preset.count())) throw phaseError('present', 'speed-ramp preset select not in DOM (ramp blocked?)')
    await preset.selectOption(T.rampPreset)
    await uiClick(page, '[data-cut-action="speed-ramp"]', { label: 'Apply ramp' })
    await sleep(400)
  })

  await step('title', 'ui', 'title.add', async () => {
    await uiClick(page, '[data-cut-title-btn]', { label: 'Title button' })
    const drawer = page.locator('[data-cut-title]').first()
    try { await drawer.waitFor({ state: 'visible', timeout: 8000 }) } catch { throw phaseError('render', 'title drawer did not open') }
    await uiClick(page, '[data-cut-title-mode="preset"]', { label: 'title mode: preset' })
    const presetSel = page.locator('[data-cut-title-preset]').first()
    if (!(await presetSel.count())) throw phaseError('present', 'title preset select not in DOM')
    await presetSel.selectOption(T.title.preset)
    // TYPED text — a real keystroke sequence, not a programmatic value set.
    const text = page.locator('[data-cut-title-text]').first()
    if (!(await text.count())) throw phaseError('present', 'title text input not in DOM')
    await text.click()
    await text.fill('')
    await text.pressSequentially(T.title.text, { delay: 25 })
    await uiFill(page, '[data-cut-title-in]', T.title.range_ms[0] / 1000, { label: 'title in (s)' })
    await uiFill(page, '[data-cut-title-out]', T.title.range_ms[1] / 1000, { label: 'title out (s)' })
    await uiClick(page, '[data-cut-title-apply]', { label: 'Add title' })
    await sleep(500)
    await page.keyboard.press('Escape') // close the drawer
    await sleep(200)
  }, { specArgs: { text: T.title.text, range_ms: T.title.range_ms, preset: T.title.preset } })

  await step('fade-out', 'ui', 'edit.fade', async () => {
    const last = v1Clips(await waiters.state()).at(-1)
    await selectClip(last.id)
    await openInspectorSection('fades')
    const input = page.locator('[data-cut-prop-input="fade-out"]').first()
    if (!(await input.count())) throw phaseError('present', 'fade-out property input not in DOM')
    await input.fill(String(T.fadeOutMs / 1000))
    await input.press('Enter')
    await sleep(400)
  })

  let render = null
  await step('render', 'ui', null, async () => {
    const btn = page.locator('[data-cut-render-btn]').first()
    if (!(await btn.count())) throw phaseError('present', 'Render button not in DOM')
    if (await btn.isDisabled()) throw phaseError('click', 'Render button is disabled')
    await btn.click()
    await sleep(800)
    // A pre-render gate dialog may interpose (uninstrumented-asset warning) —
    // continuing through it is part of the real user flow.
    const cont = page.locator('[data-cut-pregate-continue]').first()
    if ((await cont.count()) && (await cont.isVisible().catch(() => false))) {
      await cont.click()
      await sleep(400)
    }
    const done = await awaitRender(verb)
    render = done
    if (!done.ok) throw new Error(`render job ${done.jobId || '?'}: ${done.jobState} ${done.error || ''}`)
  })

  let verify = null
  await step('verify', 'ui', null, async () => {
    // UI-COVERAGE NOTE (finding, not a fallback): verify.checks has NO
    // user-facing "run checks" control — the app only calls it on (re)connect
    // and surfaces the receipt in Review → Receipts. So the UI path here is:
    // reload (the app's own trigger), open Review, read the receipt rows —
    // then cross-check the DOM against the API's structured receipt.
    await reloadApp()
    // The Review panel lives in the RIGHT RAIL, which collapses on a fresh
    // load — expand it first, then pin the review rail if it is still closed
    // (the ensureReviewPanel pattern from the full-coverage suite).
    await ensureRail()
    const review = page.locator('[data-cut-panel="review"]').first()
    if (!(await review.count().catch(() => 0))) {
      const pin = page.locator('[data-cut-rail-pin]').first()
      if (await pin.count()) { await pin.click({ force: true }).catch(() => {}); await sleep(400) }
    }
    try { await review.waitFor({ state: 'visible', timeout: 6000 }) } catch { throw phaseError('present', 'Review panel not reachable (rail expanded, pin tried)') }
    const tab = review.locator('[data-cut-tab="receipts"]').first()
    if (await tab.count()) { await tab.click().catch(() => {}); await sleep(400) }
    const receiptEl = page.locator('[data-cut-receipt]').first()
    try { await receiptEl.waitFor({ state: 'visible', timeout: 10_000 }) } catch { throw phaseError('render', 'render receipt not shown in Review → Receipts') }
    const domChecks = await page.locator('[data-cut-check]').evaluateAll((els) => els.map((el) => el.getAttribute('data-cut-check')))
    // footage_profile intentionally renders as its own [data-cut-profile] row,
    // excluded from the [data-cut-check] set (Review/Receipts.tsx) — map it.
    if (await page.locator('[data-cut-profile]').count()) domChecks.push('footage_profile')
    const r = await verb('verify.checks', {})
    if (!r.ok || !Array.isArray(r.result?.checks)) throw new Error(`verify.checks read failed: ${JSON.stringify(r.error || r).slice(0, 200)}`)
    verify = { pass: Boolean(r.result.pass), checks: r.result.checks.map((c) => ({ name: c.name, pass: Boolean(c.pass) })) }
    // DOM ↔ API cross-check: the Review panel must show the same check set.
    const missing = verify.checks.filter((c) => !domChecks.includes(c.name)).map((c) => c.name)
    if (missing.length) throw phaseError('result', `Review panel misses check rows: ${missing.join(', ')} (DOM shows: ${domChecks.join(', ') || 'none'})`)
    return { detail: `Review panel shows ${domChecks.length} check rows, matching the API receipt` }
  })

  if (consoleErrors.length) log(`  (page errors observed: ${consoleErrors.length} — first: ${consoleErrors[0]})`)
  await browser.close()

  const run = {
    mode: 'ui',
    steps,
    ops: await waiters.ops(),
    state: await waiters.state(),
    importedAssetIds,
    verify,
    render,
  }
  return finalizeRun(run)
}

// ── main ─────────────────────────────────────────────────────────────────────
async function main() {
  const startedAt = new Date().toISOString()
  if (!['both', 'a', 'b'].includes(MODE)) fail(`DSJ_MODE must be both|a|b, got "${MODE}"`)
  const wantA = MODE !== 'b'
  const wantB = MODE !== 'a'
  if (wantA && !CUTD_A) fail('Mode A needs SWEEP_CUTD_A (or SWEEP_CUTD)')
  if (wantB && !CUTD_B) fail('Mode B needs SWEEP_CUTD_B (or SWEEP_CUTD)')
  if (process.env.DSJ_EXTERNAL_ISOLATED !== '1') {
    fail('DSJ_EXTERNAL_ISOLATED=1 required — the launcher must confirm the stack(s) run with an isolated SHELLX_CUT_HOME + SHELLX_CUT_PROJECTS_DIR (project.create is sent name-only, so isolation cannot be injected here). Use scripts/release/dual-surface-job-gate.mjs.')
  }
  if (wantA && wantB && CUTD_A === CUTD_B) {
    log('⚠ both modes share ONE stack (SWEEP_CUTD) — sequential runs in separate projects; a dedicated instance per mode is the stronger isolation (the gate wrapper provides it)')
  }
  if (BROKEN) log(`⚠ RED-PROOF RUN — deliberately sabotaging step "${BROKEN.step}" in mode ${BROKEN.mode.toUpperCase()}; the diff MUST catch this`)
  if (SKIP_STEPS.size) {
    const unknown = [...SKIP_STEPS].filter((id) => !JOB_STEPS.some((s) => s.id === id))
    if (unknown.length) fail(`DSJ_SKIP_STEPS names unknown step(s): ${unknown.join(', ')}`)
    log(`⚠ DIAGNOSTIC RUN — DSJ_SKIP_STEPS filters [${[...SKIP_STEPS].join(', ')}] in BOTH modes; the receipt is stamped diagnosticOnly and is NOT a release verdict`)
  }

  const media = ensureMedia()
  log(`test media: ${media.dir} (2 silent 720p clips — video-only by design, see header)`)

  let runA = null
  let runB = null
  if (wantA) { await stackUp(CUTD_A, 'Mode A'); runA = await runModeA(CUTD_A, media) }
  if (wantB) { await stackUp(CUTD_B, 'Mode B'); runB = await runModeB(CUTD_B, APP_B, media) }

  const modeBSkipped = wantB && runB === null
  const diff = runA && runB ? diffRuns(runA, runB, { toleranceMs: TOL }) : null
  const receipt = buildDualSurfaceReceipt({
    runA,
    runB,
    diff,
    stack: {
      modeA: wantA ? { cutd: CUTD_A } : null,
      modeB: wantB ? (modeBSkipped ? { skipped: 'browser unavailable' } : { cutd: CUTD_B, app: APP_B, driver: 'playwright-chromium-headless' }) : null,
      mediaDir: media.dir,
    },
    startedAt,
    tolerance: TOL,
    broken: BREAK,
    filtered: [...SKIP_STEPS],
  })
  mkdirSync(dirname(RECEIPT_PATH), { recursive: true })
  writeFileSync(RECEIPT_PATH, JSON.stringify(receipt, null, 2))
  log(`receipt → ${RECEIPT_PATH}`)

  // ── verdict ────────────────────────────────────────────────────────────────
  if (diff) {
    log(`DIFF VERDICT: ${diff.verdict.toUpperCase()}`)
    for (const f of diff.findings) {
      log(`  [${f.kind}] ${f.step} → layer=${f.layer}: ${f.detail}`)
    }
  }
  const gaps = (runB?.steps || []).filter((s) => s.uiGap)
  if (gaps.length) {
    log(`UI-COVERAGE GAPS (${gaps.length}):`)
    for (const g of gaps) log(`  – ${g.id}: ${g.uiGap}`)
  }
  const stepsOk = (run) => !run || run.steps.every((s) => s.ok)
  if (BROKEN) {
    // Red-proof semantics: the run SUCCEEDS (exit 0) only if the sabotage WAS
    // caught as a divergence attributed to the sabotaged mode's layer.
    const caught = diff && diff.verdict === 'divergent' && diff.findings.some((f) => f.step === BROKEN.step && f.kind === 'divergence')
    log(caught ? `RED-PROOF PASS — sabotage of ${BROKEN.step}:${BROKEN.mode} was caught and attributed` : 'RED-PROOF FAIL — the sabotage was NOT caught')
    process.exit(caught ? 0 : 1)
  }
  if (modeBSkipped) process.exit(4)
  const ok = stepsOk(runA) && stepsOk(runB) && (!diff || diff.verdict === 'convergent')
  log(ok
    ? (diff ? 'PASS — both surfaces ran the job and converged' : `PASS — single-surface run (mode ${MODE.toUpperCase()}) completed all steps (no diff without both modes)`)
    : 'FAIL — see findings above')
  process.exit(ok ? 0 : 1)
}

main().catch((e) => { console.error('[dsj] ✗ runner error:', e); process.exit(3) })
