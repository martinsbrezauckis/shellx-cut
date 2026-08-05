#!/usr/bin/env node
// installed-ai-workflows.mjs -- repeatable installed Windows AI workflow proof.
//
// Drives the installed ShellX Cut engine over loopback. The app should already
// be launched so the bundled cutd owns the project state. Media paths are passed
// by env to keep machine-specific paths out of source.
//
// Required env:
//   CUT_CANARY_MEDIA   Windows path to a short WAV for Canary STT timestamp proof
//   CUT_DUB_MEDIA      Windows path to speech video/audio for transcribe/diarize/dub
//
// Useful env:
//   CUT_EXPECTED_VERSION=0.6.59
//   CUT_ENGINE=http://127.0.0.1:6161
//   CUT_RECEIPT_DIR=/path/to/private/receipt
//   CUT_DUB_TRANSLATE_BACKEND=auto|cli|local   (default auto: CLI agent; local only when no CLI is installed)
//   CUT_DUB_SOURCE_LANG=en
//   CUT_DUB_TARGET_LANG=es
//   CUT_CLEANUP=0                            (default cleans test projects)

import { mkdirSync, writeFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { join } from 'node:path'

const ENGINE = process.env.CUT_ENGINE || `${'http'}://127.0.0.1:6161`
const EXPECTED_VERSION = process.env.CUT_EXPECTED_VERSION || ''
const CANARY_MEDIA = requiredEnv('CUT_CANARY_MEDIA')
const DUB_MEDIA = requiredEnv('CUT_DUB_MEDIA')
const VERSION_SLUG = (EXPECTED_VERSION || 'installed').replace(/\D+/g, '') || 'installed'
const RECEIPT_DIR = process.env.CUT_RECEIPT_DIR || join('/tmp', `shellx-cut-ai-workflows-${VERSION_SLUG}-${Date.now()}`)
const CLEANUP = process.env.CUT_CLEANUP !== '0'
const DUB_BACKEND = process.env.CUT_DUB_TRANSLATE_BACKEND || 'auto'
const DUB_SOURCE_LANG = process.env.CUT_DUB_SOURCE_LANG || 'en'
const DUB_TARGET_LANG = process.env.CUT_DUB_TARGET_LANG || 'es'

mkdirSync(RECEIPT_DIR, { recursive: true })

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const psLiteral = (value) => `'${String(value).replace(/'/g, "''")}'`

function powerShellPath(value) {
  const text = String(value)
  const uncPrefix = '\\\\?\\UNC\\'
  if (text.startsWith(uncPrefix)) return `\\\\${text.slice(uncPrefix.length)}`
  return text.replace(/^\\\\\?\\/, '')
}

function requiredEnv(name) {
  const value = process.env[name]
  if (!value || !value.trim()) {
    console.error(`FAIL missing ${name}`)
    process.exit(2)
  }
  return value.trim()
}

function save(name, value) {
  writeFileSync(join(RECEIPT_DIR, name), JSON.stringify(value, null, 2))
}

function trackSummary(project) {
  return (project?.tracks ?? []).map((track) => ({
    id: track.id,
    kind: track.kind,
    clips: (track.clips ?? []).length,
  }))
}

function removeWindowsDir(path) {
  if (!path) return { ok: true, skipped: true }
  const normalizedPath = powerShellPath(path)
  const script = [
    '$ErrorActionPreference = "Stop"',
    `$p = ${psLiteral(normalizedPath)}`,
    'if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Recurse -Force -ErrorAction Stop }',
    'if (Test-Path -LiteralPath $p) { exit 1 }',
  ].join('; ')
  const result = spawnSync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', script], {
    encoding: 'utf8',
  })
  return {
    ok: result.status === 0,
    status: result.status,
    path: normalizedPath,
    stderr: String(result.stderr || '').trim().slice(0, 500),
  }
}

function windowsDirExists(path) {
  if (!path) return false
  const script = [
    `$p = ${psLiteral(powerShellPath(path))}`,
    'if (Test-Path -LiteralPath $p -PathType Container) { exit 0 }',
    'exit 1',
  ].join('; ')
  const result = spawnSync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', script], {
    encoding: 'utf8',
  })
  return result.status === 0
}

async function post(verb, args = {}, timeoutMs = 60000) {
  const response = await fetch(`${ENGINE}/api/verb/${verb}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', connection: 'close' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(timeoutMs),
  })
  return response.json()
}

async function pollJob(jobId, timeoutMs, label) {
  const started = Date.now()
  let last = null
  while (Date.now() - started < timeoutMs) {
    last = await post('jobs.status', { job_id: jobId }, 30000)
    const job = last?.result ?? last
    if (job?.state === 'done') return { ok: true, job }
    if (job?.state === 'failed') return { ok: false, job }
    await sleep(1000)
  }
  return { ok: false, timeout: true, label, last }
}

function assertOk(cond, message, detail = undefined) {
  if (!cond) {
    const suffix = detail === undefined ? '' : ` ${JSON.stringify(detail).slice(0, 600)}`
    throw new Error(`${message}${suffix}`)
  }
}

async function waitForEngine() {
  for (let i = 0; i < 45; i += 1) {
    try {
      const doctor = await post('system.doctor', { refresh: true }, 10000)
      if (doctor?.ok) return doctor
    } catch {
      // app still starting
    }
    await sleep(1000)
  }
  throw new Error(`engine did not answer at ${ENGINE}`)
}

function createdProjectPath(item) {
  return item?.project_path || item?.project_create?.result?.path || ''
}

async function cleanupCreatedProjects(summary) {
  const cleanup = {
    started_at: new Date().toISOString(),
    projects: [],
  }
  cleanup.close = await post('project.close', {}, 30000).catch((error) => ({ ok: false, error: String(error) }))

  const seen = new Set()
  for (const [label, source] of [
    ['canary', summary.canary],
    ['dub_diarize', summary.dub_diarize],
  ]) {
    const rawPath = createdProjectPath(source)
    if (!rawPath) continue
    const path = powerShellPath(rawPath)
    const key = path.toLowerCase()
    if (seen.has(key)) continue
    seen.add(key)

    const item = { label, path }
    item.project_delete = await post('project.delete', { path }, 60000).catch((error) => ({ ok: false, error: String(error) }))
    if (item.project_delete?.ok !== true) {
      item.fallback = removeWindowsDir(path)
    }
    item.exists_after = windowsDirExists(path)
    item.ok = (item.project_delete?.ok === true || item.fallback?.ok === true) && !item.exists_after
    if (source) source.cleanup = item
    cleanup.projects.push(item)
  }

  cleanup.ok = cleanup.projects.every((item) => item.ok)
  cleanup.finished_at = new Date().toISOString()
  return cleanup
}

async function runCanary(summary) {
  const item = {
    media: CANARY_MEDIA,
    started_at: new Date().toISOString(),
  }
  let projectPath = ''
  try {
    const set = await post('system.set_stt_model', {
      model: 'nemo-canary-1b-v2',
      language: 'en',
      rationale: `${EXPECTED_VERSION || 'installed'} Canary STT timestamp workflow proof`,
    })
    item.set_stt_model = set
    assertOk(set?.ok === true, 'system.set_stt_model failed', set?.error ?? set)

    const doctor = await post('system.doctor', { refresh: true })
    item.doctor_after_set = {
      app_version: doctor?.result?.app_version,
      perception: doctor?.result?.cards?.find((card) => card.id === 'perception'),
    }
    assertOk(
      item.doctor_after_set.perception?.details?.stt_model === 'nemo-canary-1b-v2',
      'doctor did not report Canary STT model',
      item.doctor_after_set.perception,
    )

    const projectName = `canary_stt_${VERSION_SLUG}_${Date.now()}`
    const created = await post('project.create', {
      name: projectName,
      settings: { width: 1280, height: 720, fps: 30 },
    })
    item.project_create = created
    assertOk(created?.ok === true, 'project.create failed', created?.error ?? created)
    projectPath = created?.result?.path || ''
    item.project_path = projectPath

    const imported = await post('media.import', {
      path: CANARY_MEDIA,
      rationale: `${EXPECTED_VERSION || 'installed'} Canary STT timestamp workflow proof`,
    })
    item.media_import = { initial: imported }
    assertOk(imported?.ok === true, 'media.import failed', imported?.error ?? imported)
    const importJob = await pollJob(imported.result.job_id, 120000, 'canary import')
    item.media_import.job = importJob.job
    assertOk(importJob.ok, 'media.import job failed or timed out', importJob)

    const asset = imported.result.asset_id || importJob.job?.result?.asset
    item.asset = asset
    assertOk(!!asset, 'media.import returned no asset')

    const transcribe = await post('media.transcribe', { asset })
    item.transcribe = { initial: transcribe }
    assertOk(transcribe?.ok === true, 'media.transcribe failed', transcribe?.error ?? transcribe)
    const transcribeJob = await pollJob(transcribe.result.job_id, 240000, 'canary transcribe')
    item.transcribe.job = transcribeJob.job
    assertOk(transcribeJob.ok, 'media.transcribe job failed or timed out', transcribeJob)

    const transcript = await post('transcript.get', { asset })
    save('canary.transcript.get.json', transcript)
    const words = transcript?.result?.words ?? []
    const timestamped = words.filter((word) =>
      Number.isFinite(word.start_ms) && Number.isFinite(word.end_ms) && word.end_ms >= word.start_ms
    )
    item.check = {
      ok: words.length > 0 && timestamped.length === words.length,
      word_count: words.length,
      timestamped_words: timestamped.length,
      model: transcript?.result?.model,
      language: transcript?.result?.language,
      first_words: words.slice(0, 8),
    }
    assertOk(item.check.ok, 'Canary transcript missing word timestamps', item.check)
    return item
  } finally {
    item.reset_stt_model = await post('system.set_stt_model', {
      clear: true,
      rationale: `${EXPECTED_VERSION || 'installed'} Canary STT workflow reset`,
    }).catch((error) => ({ ok: false, error: String(error) }))
    item.finished_at = new Date().toISOString()
    summary.canary = item
  }
}

async function runDubDiarize(summary) {
  const item = {
    media: DUB_MEDIA,
    started_at: new Date().toISOString(),
  }
  let projectPath = ''
  const projectName = `ai_dub_diarize_${VERSION_SLUG}_${Date.now()}`
  try {
    const created = await post('project.create', {
      name: projectName,
      settings: { width: 1280, height: 720, fps: 30 },
    })
    item.project_create = created
    assertOk(created?.ok === true, 'project.create failed', created?.error ?? created)
    projectPath = created?.result?.path || ''
    item.project_path = projectPath

    const imported = await post('media.import', {
      path: DUB_MEDIA,
      rationale: `${EXPECTED_VERSION || 'installed'} AI dub/diarize workflow proof`,
    })
    item.media_import = { initial: imported }
    assertOk(imported?.ok === true, 'media.import failed', imported?.error ?? imported)
    const importJob = await pollJob(imported.result.job_id, 180000, 'dub import')
    item.media_import.job = importJob.job
    assertOk(importJob.ok, 'media.import job failed or timed out', importJob)

    const asset = imported.result.asset_id || importJob.job?.result?.asset
    item.asset = asset
    assertOk(!!asset, 'media.import returned no asset')

    const transcribe = await post('media.transcribe', { asset })
    item.transcribe = { initial: transcribe }
    assertOk(transcribe?.ok === true, 'media.transcribe failed', transcribe?.error ?? transcribe)
    const transcribeJob = await pollJob(transcribe.result.job_id, 360000, 'dub transcribe')
    item.transcribe.job = transcribeJob.job
    assertOk(transcribeJob.ok, 'media.transcribe job failed or timed out', transcribeJob)

    const transcript = await post('transcript.get', { asset })
    save('dub.transcript.get.json', transcript)
    const words = transcript?.result?.words ?? []
    item.transcribe.check = {
      ok: words.length > 0,
      word_count: words.length,
      model: transcript?.result?.model,
    }
    assertOk(item.transcribe.check.ok, 'transcript.get returned no words', item.transcribe.check)

    const perception = await post('media.perception', { asset })
    item.perception = { initial: perception }
    assertOk(perception?.ok === true, 'media.perception failed', perception?.error ?? perception)
    const perceptionJob = await pollJob(perception.result.job_id, 360000, 'media perception')
    item.perception.job = perceptionJob.job
    assertOk(perceptionJob.ok, 'media.perception job failed or timed out', perceptionJob)

    const diarize = await post('media.diarize', { asset, max_speakers: 4 })
    item.diarize = { initial: diarize }
    assertOk(diarize?.ok === true, 'media.diarize failed', diarize?.error ?? diarize)
    const diarizeJob = await pollJob(diarize.result.job_id, 180000, 'media diarize')
    item.diarize.job = diarizeJob.job
    save('diarize.result.json', diarizeJob.job)
    assertOk(diarizeJob.ok, 'media.diarize job failed or timed out', diarizeJob)

    const transcriptAfter = await post('transcript.get', { asset })
    save('dub.transcript.after-diarize.json', transcriptAfter)
    const labeled = (transcriptAfter?.result?.words ?? []).filter((word) => typeof word.speaker === 'string').length
    const diarizeResult = diarizeJob.job?.result ?? {}
    item.diarize.check = {
      ok: diarizeResult.backend === 'sortformer'
        && diarizeResult.model === 'sortformer-v2'
        && diarizeResult.num_speakers >= 1
        && diarizeResult.n_turns > 0
        && labeled > 0,
      backend: diarizeResult.backend,
      model: diarizeResult.model,
      num_speakers: diarizeResult.num_speakers,
      n_turns: diarizeResult.n_turns,
      labeled_words: diarizeResult.labeled_words,
      transcript_labeled_words: labeled,
    }
    assertOk(item.diarize.check.ok, 'diarize did not produce speaker-labeled words', item.diarize.check)

    const stateBefore = await post('project.state', {})
    save('project.state.before-dub.json', stateBefore)
    const beforeTracks = trackSummary(stateBefore?.result)
    const dub = await post('audio.dub', {
      asset,
      target_lang: DUB_TARGET_LANG,
      source_lang: DUB_SOURCE_LANG,
      backend: DUB_BACKEND,
      timeout_ms: 600000,
      rationale: `${EXPECTED_VERSION || 'installed'} AI dubbing workflow proof`,
    }, 120000)
    item.dub = { initial: dub }
    save('dub.result.json', dub)
    assertOk(dub?.ok === true, 'audio.dub failed', dub?.error ?? dub)

    const stateAfter = await post('project.state', {})
    save('project.state.after-dub.json', stateAfter)
    const afterTracks = trackSummary(stateAfter?.result)
    const dubTracks = afterTracks.filter((track) => track.id.startsWith('dub') && track.kind === 'audio')
    const dubClipCount = dubTracks.reduce((sum, track) => sum + track.clips, 0)
    const dubResult = dub.result ?? {}
    const cliCards = (summary.doctor?.cards ?? []).filter((card) =>
      /^judge\.(claude|codex|grok)$/.test(card.id)
    )
    const cliInstalled = cliCards.some((card) =>
      card.status === 'ok' && (card.details?.found === true || card.details?.chat?.installed === true)
    )
    const cliReady = cliCards.some((card) =>
      card.status === 'ok' && card.details?.chat?.ready === true
    )
    const translateBackendOk = DUB_BACKEND === 'auto'
      ? (
          dubResult.translate_backend === 'cli'
          || (dubResult.translate_backend === 'local' && !cliInstalled)
        )
      : dubResult.translate_backend === DUB_BACKEND
    item.dub.check = {
      ok: dubResult.n_segments > 0
        && dubResult.n_clips > 0
        && !!dubResult.track_id
        && dubClipCount >= dubResult.n_clips
        && translateBackendOk,
      n_segments: dubResult.n_segments,
      n_clips: dubResult.n_clips,
      dub_asset: dubResult.dub_asset,
      dub_wav: dubResult.dub_wav,
      receipt: dubResult.receipt,
      requested_translate_backend: DUB_BACKEND,
      translate_backend: dubResult.translate_backend,
      translate_backend_ok: translateBackendOk,
      translate_warnings: dubResult.translate_warnings ?? [],
      cli_installed: cliInstalled,
      cli_ready: cliReady,
      translate_model: dubResult.translate_model,
      mean_fit_ratio: dubResult.mean_fit_ratio,
      before_tracks: beforeTracks,
      after_tracks: afterTracks,
      dub_tracks: dubTracks,
      dub_clip_count: dubClipCount,
    }
    assertOk(item.dub.check.ok, 'audio.dub did not create expected dub track/clips', item.dub.check)
    return item
  } finally {
    item.finished_at = new Date().toISOString()
    summary.dub_diarize = item
  }
}

const summary = {
  schema: 'shellx-cut/installed-ai-workflows/1',
  engine: ENGINE,
  expected_version: EXPECTED_VERSION || null,
  receipt_dir: RECEIPT_DIR,
  cleanup: CLEANUP,
  started_at: new Date().toISOString(),
  inputs: {
    canary_media: CANARY_MEDIA,
    dub_media: DUB_MEDIA,
    dub_backend: DUB_BACKEND,
    dub_source_lang: DUB_SOURCE_LANG,
    dub_target_lang: DUB_TARGET_LANG,
  },
}

let ok = false
try {
  const doctor = await waitForEngine()
  summary.doctor = {
    ok: doctor?.ok,
    app_version: doctor?.result?.app_version,
    cards: doctor?.result?.cards?.map((card) => ({ id: card.id, status: card.status, details: card.details })),
  }
  if (EXPECTED_VERSION) {
    assertOk(
      summary.doctor.app_version === EXPECTED_VERSION,
      `installed app_version mismatch: expected ${EXPECTED_VERSION}, got ${summary.doctor.app_version}`,
    )
  }
  assertOk(summary.doctor.cards?.find((card) => card.id === 'perception')?.status === 'ok', 'perception card is not ok')
  assertOk(summary.doctor.cards?.find((card) => card.id === 'dub')?.status === 'ok', 'dub service card is not ok')
  assertOk(summary.doctor.cards?.find((card) => card.id === 'diarize')?.status === 'ok', 'diarize service card is not ok')

  await runCanary(summary)
  await runDubDiarize(summary)
  ok = true
} catch (error) {
  summary.error = error?.stack || String(error)
} finally {
  if (CLEANUP) {
    summary.cleanup_projects = await cleanupCreatedProjects(summary).catch((error) => ({
      ok: false,
      error: error?.stack || String(error),
    }))
    if (ok && summary.cleanup_projects?.ok !== true) {
      ok = false
      summary.error = `cleanup failed: ${JSON.stringify(summary.cleanup_projects).slice(0, 1000)}`
    }
  }
  summary.finished_at = new Date().toISOString()
  summary.ok = ok
  summary.checks = {
    canary: summary.canary?.check,
    transcribe: summary.dub_diarize?.transcribe?.check,
    diarize: summary.dub_diarize?.diarize?.check,
    dub: summary.dub_diarize?.dub?.check,
  }
  save('summary.json', summary)
  console.log(JSON.stringify({
    receipt_dir: RECEIPT_DIR,
    ok,
    checks: summary.checks,
    cleanup: {
      projects: summary.cleanup_projects,
    },
    error: summary.error,
  }, null, 2))
}

process.exit(ok ? 0 : 1)
