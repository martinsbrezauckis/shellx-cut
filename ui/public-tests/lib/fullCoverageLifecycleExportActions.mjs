// Receipt-backed export lifecycle regression. Browser controls start/cancel the
// first render; the second render pins the source-level exact-output contract.

import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { spawnSync } from 'node:child_process'

export const EXPORT_LIFECYCLE_ACTION_NAMES = Object.freeze([
  'e2e-export-lifecycle-01-start-visible-progress',
  'e2e-export-lifecycle-01-targeted-cancel-terminal',
  'e2e-export-lifecycle-01-exact-new-output-identity',
])

// The first render plan can include cold media/FFmpeg discovery on a newly
// started candidate.  Keep this above the verifier's ordinary verb timeout so
// an aborted client request cannot leave the server finishing the plan while
// the browser starts the render whose identity that plan was meant to pin.
export const EXPORT_LIFECYCLE_PLAN_TIMEOUT_MS = 120_000

function digest(path) {
  try { return createHash('sha256').update(readFileSync(path)).digest('hex') } catch { return '' }
}

function terminalDetail(job) {
  if (!job) return 'terminal=timeout'
  const error = job.error?.code || job.error?.message || 'none'
  return `terminal=${job.state || '?'} error=${error}`
}

export function buildExportPredecessorProof({ path, sha256, unchangedAfterCancel }) {
  return { path, sha256, unchanged: unchangedAfterCancel === true }
}

async function activeJobEvidence(page, jobId, { verb, sleep }) {
  const deadline = Date.now() + 30_000
  let last = null
  while (Date.now() < deadline) {
    const status = await verb('jobs.status', { job_id: jobId })
    last = status.result || null
    const row = page.locator(`[data-cut-job="${jobId}"]`).first()
    const visible = (await row.count()) > 0 && await row.isVisible().catch(() => false)
    const progress = Number(last?.progress || 0)
    if (visible && last?.state === 'running' && progress > 0) {
      const text = (await row.textContent().catch(() => ''))?.replace(/\s+/g, ' ').trim() || ''
      return { ok: true, visible, progress, text, status: last }
    }
    if (last?.state === 'done' || last?.state === 'failed') break
    await sleep(200)
  }
  return { ok: false, visible: false, progress: Number(last?.progress || 0), text: '', status: last }
}

export function createExportLifecycleFixture({ driverDir, engineDir, ffmpeg, joinHostPath, nextName }) {
  const name = `e2e-export-lifecycle-${nextName()}.mp4`
  const driverPath = join(driverDir, name)
  const generated = spawnSync(ffmpeg, [
    '-hide_banner', '-loglevel', 'error', '-y',
    '-f', 'lavfi', '-i', 'testsrc2=size=854x480:rate=30:duration=48',
    '-f', 'lavfi', '-i', 'sine=frequency=523:sample_rate=48000:duration=48',
    '-shortest', '-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', '-c:a', 'aac',
    driverPath,
  ], { timeout: 120_000 })
  if (generated.status !== 0 || !existsSync(driverPath)) return null
  return joinHostPath(engineDir, name)
}

export async function runExportLifecycleCoverage(page, {
  projectPath,
  fixturePath,
  probe,
  verb,
  awaitJob,
  captureVerbResp,
  continuePreflightIfPresent,
  sleep,
  record,
  resolveDriverPath,
  recordNativeAuditObservation,
}) {
  const plan = await verb(
    'render.final',
    { dry_run: true, preset: 'standard' },
    { timeoutMs: EXPORT_LIFECYCLE_PLAN_TIMEOUT_MS },
  )
  const staleOutput = plan.result?.out_path || ''
  const staleDriverPath = staleOutput ? resolveDriverPath(staleOutput) : ''
  const staleBytes = Buffer.from('stale predecessor must never be reported as a completed export\n')
  if (staleDriverPath) {
    mkdirSync(dirname(staleDriverPath), { recursive: true })
    writeFileSync(staleDriverPath, staleBytes)
  }
  const staleHash = digest(staleDriverPath)
  let started = null
  let active = null
  let preflight = null

  await page.locator('[data-cut-export-btn]').click()
  await sleep(200)
  await probe(page, {
    surface: 'export',
    name: EXPORT_LIFECYCLE_ACTION_NAMES[0],
    actionId: EXPORT_LIFECYCLE_ACTION_NAMES[0],
    sel: page.locator('[data-cut-export-option="video"]'),
    group: page.locator('[data-cut-export-menu]').first(),
    groupName: 'export-lifecycle-menu',
    doClick: async () => {
      started = await captureVerbResp(page, 'render.final', async () => {
        await page.locator('[data-cut-export-option="video"]').click()
        for (let i = 0; i < 80; i++) {
          preflight = await continuePreflightIfPresent(page, 1)
          if (preflight.seen) break
          await sleep(100)
        }
      }, 60_000)
      const jobId = started?.result?.job_id
      active = jobId ? await activeJobEvidence(page, jobId, { verb, sleep }) : null
    },
    assertResult: async () => {
      const jobId = started?.result?.job_id || ''
      const planned = started?.result?.render_id === plan.result?.render_id
      return {
        ok: !!plan.ok && !!staleHash && !!started?.ok && !!jobId && planned && !!active?.ok,
        detail: `dry-run=${plan.ok}${plan.ok ? '' : ` error=${String(plan.error?.message || plan.error?.code || 'unknown').slice(0, 160)}`} stale=${staleOutput || 'missing'} hash=${staleHash.slice(0, 12) || 'missing'}; render job=${jobId || 'missing'} renderIdMatchesPlan=${planned}; visible-running-progress=${active?.ok === true} progress=${active?.progress ?? '?'} preflight=${preflight?.seen === true}`,
      }
    },
  })

  const jobId = started?.result?.job_id || ''
  let cancelled = null
  let terminal = null
  let staleUnchangedAfterCancel = false
  await probe(page, {
    surface: 'export',
    name: EXPORT_LIFECYCLE_ACTION_NAMES[1],
    actionId: EXPORT_LIFECYCLE_ACTION_NAMES[1],
    sel: page.locator(`[data-cut-job-cancel="${jobId}"]`),
    group: page.locator('[data-cut-panel="statusbar"]').first(),
    groupName: 'export-lifecycle-statusbar',
    doClick: async () => {
      cancelled = await captureVerbResp(
        page,
        'jobs.cancel',
        () => page.locator(`[data-cut-job-cancel="${jobId}"]`).click(),
        60_000,
      )
      terminal = jobId ? await awaitJob(jobId, 60_000) : null
    },
    assertResult: async () => {
      staleUnchangedAfterCancel = !!staleHash && digest(staleDriverPath) === staleHash
      const nonSuccess = terminal?.state === 'failed' && terminal?.error?.code === 'job_cancelled'
      const noOutput = !terminal?.result?.path
      return {
        ok: cancelled?.ok === true && cancelled?.result?.cancelled === true && nonSuccess && noOutput && staleUnchangedAfterCancel,
        detail: `cancelled=${cancelled?.result?.cancelled === true}; ${terminalDetail(terminal)}; no-success-path=${noOutput}; stale-predecessor-unchanged=${staleUnchangedAfterCancel}`,
      }
    },
  })

  const fresh = staleOutput
    ? await verb('render.final', { path: staleOutput, preset: 'draft', hardware: 'off', profile: 'silent_screen_demo' })
    : { ok: false, error: { message: 'dry-run did not return an output path' } }
  const freshTerminal = fresh.result?.job_id ? await awaitJob(fresh.result.job_id, 180_000) : null
  const reportedPath = freshTerminal?.result?.path || ''
  const freshHash = digest(staleDriverPath)
  const exactPath = !!reportedPath && reportedPath === staleOutput && resolveDriverPath(reportedPath) === staleDriverPath
  const newOutput = !!freshHash && freshHash !== staleHash
  record('export', EXPORT_LIFECYCLE_ACTION_NAMES[2], {
    rowKind: 'support',
    actionId: EXPORT_LIFECYCLE_ACTION_NAMES[2],
    present: 'na', render: 'na', click: 'na',
    result: fresh.ok && freshTerminal?.state === 'done' && exactPath && newOutput ? 'pass' : 'fail',
  }, `project=${projectPath || 'missing'}; render.final explicit-path=${fresh.ok}; ${terminalDetail(freshTerminal)}; reported-exact=${exactPath}; fresh-hash=${freshHash.slice(0, 12) || 'missing'}; stale-hash-rejected=${newOutput}`)
  if (recordNativeAuditObservation) {
    const fixtureDriverPath = resolveDriverPath(fixturePath || '')
    recordNativeAuditObservation({
      scenarioId: 'NATIVE-EXPORT-01',
      fixture: { id: 'export-lifecycle-input', sha256: digest(fixtureDriverPath) },
      runtime: {
        kind: 'native', observedBy: 'installed-run', surface: process.env.FCV_TARGET_SURFACE || '',
        driver: process.env.FCV_UI_DRIVER || '', installedApp: process.env.FCV_INSTALLED_APP === '1',
        nativeAttached: process.env.FCV_NATIVE_ACTION_CONTROLLER ? true : false,
      },
      proof: {
        export: {
          progress: { rendered: active?.ok === true, jobId: jobId || '' },
          cancel: {
            requested: cancelled?.result?.cancelled === true, jobId,
            terminalState: terminal?.state || '', outputPath: terminal?.result?.path || '',
          },
          predecessor: buildExportPredecessorProof({
            path: staleOutput,
            sha256: staleHash,
            unchangedAfterCancel: staleUnchangedAfterCancel,
          }),
          fresh: {
            jobId: fresh.result?.job_id || '', terminalState: freshTerminal?.state || '',
            path: reportedPath, expectedPath: staleOutput, exactPath, sha256: freshHash,
          },
        },
      },
    })
  }
}
