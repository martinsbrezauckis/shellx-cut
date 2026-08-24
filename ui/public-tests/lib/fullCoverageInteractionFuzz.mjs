// Bounded, replayable interaction stress coverage. This is deliberately a
// focused alias of the canonical full-coverage runner, not a second harness.
import { createHash } from 'node:crypto'
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'

export const INTERACTION_FUZZ_SCENARIO_ID = 'e2e-interaction-fuzz-01'
export const INTERACTION_FUZZ_RECEIPT_SCHEMA = 'shellx-cut/full-coverage-results@1'
export const INTERACTION_FUZZ_DEFAULT_STEPS = 12
export const INTERACTION_FUZZ_MIN_STEPS = 5
export const INTERACTION_FUZZ_MAX_STEPS = 24

const ACTIONS = Object.freeze([
  'select-primary',
  'select-secondary',
  'clipboard-roundtrip-primary',
  'clipboard-roundtrip-secondary',
  'playback-roundtrip',
])

const COMMIT = /^[a-f0-9]{40}$/
const SHA256 = /^[a-f0-9]{64}$/

function invariant(condition, message) {
  if (!condition) throw new Error(message)
}

function seedState(seed) {
  let value = 2166136261
  for (const character of String(seed)) {
    value ^= character.charCodeAt(0)
    value = Math.imul(value, 16777619)
  }
  return value >>> 0
}

function nextRandom(state) {
  let value = (state + 0x6D2B79F5) >>> 0
  value = Math.imul(value ^ (value >>> 15), value | 1)
  value ^= value + Math.imul(value ^ (value >>> 7), value | 61)
  return { state: value >>> 0, value: ((value ^ (value >>> 14)) >>> 0) / 4294967296 }
}

function shuffled(seed, values) {
  const result = [...values]
  let state = seedState(seed)
  for (let index = result.length - 1; index > 0; index -= 1) {
    const next = nextRandom(state)
    state = next.state
    const swap = Math.floor(next.value * (index + 1))
    ;[result[index], result[swap]] = [result[swap], result[index]]
  }
  return { values: result, state }
}

export function generateInteractionFuzzSequence(seed, steps = INTERACTION_FUZZ_DEFAULT_STEPS) {
  invariant(typeof seed === 'string' && seed.trim(), 'interaction fuzz seed is required')
  invariant(Number.isInteger(steps) && steps >= INTERACTION_FUZZ_MIN_STEPS && steps <= INTERACTION_FUZZ_MAX_STEPS,
    `interaction fuzz steps must be an integer in ${INTERACTION_FUZZ_MIN_STEPS}..${INTERACTION_FUZZ_MAX_STEPS}`)
  const leading = shuffled(seed, ACTIONS)
  const sequence = leading.values.slice()
  let state = leading.state
  while (sequence.length < steps) {
    const next = nextRandom(state)
    state = next.state
    sequence.push(ACTIONS[Math.floor(next.value * ACTIONS.length)])
  }
  return Object.freeze(sequence)
}

export function parseInteractionFuzzConfig(env = process.env) {
  const seed = String(env.FCV_INTERACTION_FUZZ_SEED || '').trim()
  invariant(seed, 'FCV_INTERACTION_FUZZ_SEED is required for e2e-interaction-fuzz-01')
  invariant(seed.length <= 128, 'FCV_INTERACTION_FUZZ_SEED must be at most 128 characters')
  const stepsRaw = String(env.FCV_INTERACTION_FUZZ_STEPS || INTERACTION_FUZZ_DEFAULT_STEPS)
  const steps = Number(stepsRaw)
  invariant(Number.isInteger(steps) && steps >= INTERACTION_FUZZ_MIN_STEPS && steps <= INTERACTION_FUZZ_MAX_STEPS,
    `FCV_INTERACTION_FUZZ_STEPS must be an integer in ${INTERACTION_FUZZ_MIN_STEPS}..${INTERACTION_FUZZ_MAX_STEPS}`)
  const gitCommit = String(env.FCV_SOURCE_GIT_COMMIT || '').trim()
  const contentManifestSha256 = String(env.FCV_SOURCE_CONTENT_MANIFEST_SHA256 || '').trim()
  const resultReceipt = String(env.FCV_RESULT_RECEIPT || '').trim()
  invariant(COMMIT.test(gitCommit), 'FCV_SOURCE_GIT_COMMIT must bind a full lowercase candidate commit')
  invariant(SHA256.test(contentManifestSha256), 'FCV_SOURCE_CONTENT_MANIFEST_SHA256 must bind the candidate content manifest')
  invariant(resultReceipt, 'FCV_RESULT_RECEIPT is required for candidate-bound interaction fuzz evidence')
  return Object.freeze({
    scenarioId: INTERACTION_FUZZ_SCENARIO_ID,
    seed,
    steps,
    resultReceipt,
    source: Object.freeze({ gitCommit, contentManifestSha256 }),
  })
}

export function compactProjectFingerprint(project) {
  const tracks = (project?.tracks || []).map((track) => ({
    id: track.id,
    kind: track.kind,
    clips: (track.clips || []).map((clip) => ({
      id: clip.id,
      asset: clip.asset || '',
      in: clip.src_in_ms || 0,
      out: clip.src_out_ms || 0,
      at: clip.timeline_start_ms || clip.start_ms || 0,
      speed: clip.speed || 1,
    })),
  }))
  return JSON.stringify(tracks)
}

export function fingerprintHash(value) {
  return createHash('sha256').update(String(value)).digest('hex').slice(0, 16)
}

export function interactionFuzzTraceSha256(trace) {
  return createHash('sha256').update(JSON.stringify(trace)).digest('hex')
}

export function interactionFuzzFinalFingerprintSha256(project) {
  return createHash('sha256').update(compactProjectFingerprint(project)).digest('hex')
}

export function interactionFuzzClipIntegrity(items) {
  const rows = Array.isArray(items) ? items : []
  const gaps = rows.filter((item) => item?.kind === 'gap')
  const mediaClips = rows.filter((item) => item?.kind !== 'gap')
  const invalidMediaRange = mediaClips.some((clip) => !clip.id || !Number.isFinite(Number(clip.src_in_ms))
    || !Number.isFinite(Number(clip.src_out_ms)) || Number(clip.src_out_ms) <= Number(clip.src_in_ms))
  const invalidGap = gaps.some((gap) => !Number.isFinite(Number(gap.duration_ms)) || Number(gap.duration_ms) <= 0)
  return Object.freeze({ mediaClips, gaps, invalidMediaRange, invalidGap })
}

export function interactionFuzzFailurePath(resultReceipt) {
  return `${resultReceipt}.interaction-fuzz-failure.json`
}

export function writeInteractionFuzzFailure(config, failure) {
  const path = interactionFuzzFailurePath(config.resultReceipt)
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(path, `${JSON.stringify({
    schema: 'shellx-cut/interaction-fuzz-failure@1',
    scenarioId: config.scenarioId,
    seed: config.seed,
    source: config.source,
    ...failure,
  }, null, 2)}\n`, 'utf8')
  return path
}

export class InteractionFuzzInvariantError extends Error {
  constructor(message, failurePath = '') {
    super(message)
    this.name = 'InteractionFuzzInvariantError'
    this.failurePath = failurePath
  }
}

function normalizedInvariant(value) {
  if (value === true) return { ok: true, detail: '' }
  if (value === false || value == null) return { ok: false, detail: 'invariant returned false' }
  return { ok: value.ok === true, detail: String(value.detail || '') }
}

export async function executeInteractionFuzzSequence({
  config,
  sequence = generateInteractionFuzzSequence(config?.seed, config?.steps),
  executeAction,
  assertInvariants,
  writeFailure = writeInteractionFuzzFailure,
}) {
  invariant(config?.scenarioId === INTERACTION_FUZZ_SCENARIO_ID, 'interaction fuzz config has the wrong scenario id')
  invariant(typeof executeAction === 'function', 'interaction fuzz executeAction is required')
  invariant(typeof assertInvariants === 'function', 'interaction fuzz assertInvariants is required')
  const trace = []
  try {
    for (const [index, action] of sequence.entries()) {
      const row = { step: index + 1, action }
      trace.push(row)
      const actionResult = await executeAction(action, index)
      if (actionResult?.detail) row.detail = String(actionResult.detail).slice(0, 240)
      const check = normalizedInvariant(await assertInvariants({ action, index, actionResult }))
      row.invariant = check.ok ? 'pass' : 'fail'
      if (check.detail) row.invariantDetail = check.detail.slice(0, 240)
      if (!check.ok) throw new Error(`invariant failed after ${action}: ${check.detail || 'no detail'}`)
    }
    return Object.freeze({
      seed: config.seed,
      source: config.source,
      sequence: Object.freeze([...sequence]),
      trace: Object.freeze(trace.map((row) => Object.freeze({ ...row }))),
      deterministic: JSON.stringify(sequence) === JSON.stringify(generateInteractionFuzzSequence(config.seed, config.steps)),
    })
  } catch (error) {
    const failure = {
      failedStep: trace.length || 1,
      error: String(error?.message || error).slice(0, 2000),
      trace: trace.slice(Math.max(0, trace.length - 12)),
    }
    let failurePath = ''
    let persistenceError = ''
    try { failurePath = writeFailure(config, failure) || '' } catch (writeError) { persistenceError = String(writeError?.message || writeError) }
    throw new InteractionFuzzInvariantError(
      `${failure.error}; seed=${config.seed}; failureTrace=${failurePath || 'unwritten'}${persistenceError ? `; failureTraceWriteError=${persistenceError}` : ''}`,
      failurePath,
    )
  }
}

export function createInteractionFuzzCoverage({
  probe, verb, state, waitForState, sleep, freshProject, closeOverlays, selectClip, env = process.env, onAttestation = null,
}) {
  const surface = 'timeline-source-audit'
  const clips = (project) => (project?.tracks || []).flatMap((track) =>
    (track.clips || []).map((clip) => ({ ...clip, trackId: track.id, trackKind: track.kind })))
  const track = (project, id) => project?.tracks?.find((candidate) => candidate.id === id)

  async function copyClip(page, id) {
    await closeOverlays(page)
    const source = page.locator(`[data-cut-clip="${id}"]`).first()
    await source.waitFor({ state: 'visible', timeout: 10_000 })
    await source.click({ button: 'right' })
    const menu = page.locator('[data-cut-clip-menu]').first()
    await menu.waitFor({ state: 'visible', timeout: 8_000 })
    await menu.locator('[data-cut-ctx="copy"]').click()
    await menu.waitFor({ state: 'hidden', timeout: 4_000 })
  }

  async function addEmptyVideoTrack(tag) {
    const response = await verb('edit.add_track', { kind: 'video', rationale: `fcv: interaction fuzz ${tag}` })
    const id = response.result?.track_id || response.result?.id
    if (!response.ok || !id || !await waitForState((project) => !!track(project, id), 8_000)) {
      throw new Error(`interaction fuzz could not add ${tag} video track`)
    }
    return id
  }

  async function openEmptyLane(page, trackId, ratio) {
    await closeOverlays(page)
    const lane = page.locator(`[data-cut-track="${trackId}"] .tl-lane`).first()
    await lane.waitFor({ state: 'visible', timeout: 10_000 })
    await lane.scrollIntoViewIfNeeded()
    const box = await lane.boundingBox()
    if (!box) throw new Error(`interaction fuzz lane ${trackId} has no bounding box`)
    await lane.click({
      button: 'right',
      position: { x: Math.max(8, Math.min(box.width - 8, box.width * ratio)), y: Math.max(6, box.height / 2) },
    })
    const menu = page.locator('[data-cut-timeline-empty-menu]').first()
    await menu.waitFor({ state: 'visible', timeout: 8_000 })
    return menu
  }

  async function buildFixture(page) {
    await freshProject(page, 'interaction-fuzz')
    await closeOverlays(page)
    const initial = await waitForState((project) => clips(project).some((clip) => clip.trackKind === 'video' && clip.asset), 12_000)
    const source = clips(initial).find((clip) => clip.trackKind === 'video' && clip.asset)
    if (!source?.asset) throw new Error('interaction fuzz fixture lacks a video source')
    const rangeStart = Number(source.src_in_ms || 0)
    const sourceSpan = Math.max(0, Number(source.src_out_ms || 0) - rangeStart)
    const span = Math.min(1200, Math.floor(sourceSpan / 3))
    if (!Number.isFinite(span) || span < 400) throw new Error(`interaction fuzz source span is too short: ${sourceSpan}`)
    const sourceTrack = await addEmptyVideoTrack('source')
    const sourceRanges = [[rangeStart, rangeStart + span], [rangeStart + span, rangeStart + (span * 2)]]
    const sourceIds = []
    for (const [index, srcRange] of sourceRanges.entries()) {
      const response = await verb('edit.insert', {
        asset: source.asset,
        track: sourceTrack,
        at_ms: index * (span + 250),
        src_range_ms: srcRange,
        ripple: false,
        rationale: `fcv: interaction fuzz source ${index + 1}`,
      })
      const id = response.result?.clip_id
      if (!response.ok || !id) throw new Error(`interaction fuzz could not seed source ${index + 1}`)
      sourceIds.push(id)
    }
    const ready = await waitForState((project) => sourceIds.every((id) => !!clips(project).find((clip) => clip.id === id)), 10_000)
    if (!ready) throw new Error('interaction fuzz source clips did not land')
    return { primaryId: sourceIds[0], secondaryId: sourceIds[1] }
  }

  async function run(page) {
    const waitForSettledMutation = async (before, timeoutMs = 10_000) => {
      const deadline = Date.now() + timeoutMs
      let last = ''
      let stableSamples = 0
      while (Date.now() < deadline) {
        const project = await state()
        const fingerprint = compactProjectFingerprint(project)
        if (fingerprint === before) {
          last = ''
          stableSamples = 0
        } else if (fingerprint === last) {
          stableSamples += 1
          if (stableSamples >= 2) return project
        } else {
          last = fingerprint
          stableSamples = 0
        }
        await sleep(200)
      }
      return null
    }
    const waitForPlaying = async (panel, expected) => {
      for (let attempt = 0; attempt < 40; attempt += 1) {
        if ((await panel.getAttribute('data-cut-playing')) === expected) return true
        await sleep(50)
      }
      return false
    }
    await probe(page, {
      surface,
      name: INTERACTION_FUZZ_SCENARIO_ID,
      actionId: INTERACTION_FUZZ_SCENARIO_ID,
      rowKind: 'support',
      sel: page.locator('[data-cut-panel="timeline"]').first(),
      group: page.locator('[data-cut-panel="timeline"]').first(),
      groupName: INTERACTION_FUZZ_SCENARIO_ID,
      doClick: async () => {
        const config = parseInteractionFuzzConfig(env)
        const fixture = await buildFixture(page)
        const result = await executeInteractionFuzzSequence({
          config,
          executeAction: async (action, index) => {
            if (action === 'select-primary' || action === 'select-secondary') {
              const id = action === 'select-primary' ? fixture.primaryId : fixture.secondaryId
              const selected = await selectClip(page, id)
              if (!selected) throw new Error(`interaction fuzz could not select ${action}`)
              return { detail: `${action}=${id}` }
            }
            if (action === 'clipboard-roundtrip-primary' || action === 'clipboard-roundtrip-secondary') {
              const id = action.endsWith('primary') ? fixture.primaryId : fixture.secondaryId
              await copyClip(page, id)
              const targetTrack = await addEmptyVideoTrack(`paste-${index + 1}`)
              const before = compactProjectFingerprint(await state())
              const menu = await openEmptyLane(page, targetTrack, 0.18 + ((index % 4) * 0.17))
              await menu.locator('[data-cut-timeline-ctx="empty-paste"]').click()
              // edit.paste can persist a linked A/V group in successive engine
              // writes. Capture the completed UI action, not its first insert.
              const pasted = await waitForSettledMutation(before)
              if (!pasted) throw new Error(`interaction fuzz paste did not mutate ${targetTrack}`)
              const after = compactProjectFingerprint(pasted)
              await page.keyboard.press('Control+z')
              const undone = await waitForState((project) => compactProjectFingerprint(project) === before, 10_000)
              await page.keyboard.press('Control+Shift+z')
              const redone = await waitForState((project) => compactProjectFingerprint(project) === after, 10_000)
              if (!undone || !redone) throw new Error(`interaction fuzz undo/redo roundtrip failed: undo=${!!undone} redo=${!!redone}`)
              return { detail: `${action} target=${targetTrack} undo=${!!undone} redo=${!!redone}` }
            }
            if (action === 'playback-roundtrip') {
              const panel = page.locator('[data-cut-panel="preview"]').first()
              const control = page.locator('[data-cut-transport-btn="play"]').first()
              await control.waitFor({ state: 'visible', timeout: 10_000 })
              if ((await panel.getAttribute('data-cut-playing')) === 'true') await control.click()
              await control.click()
              const started = await waitForPlaying(panel, 'true')
              await control.click()
              const stopped = await waitForPlaying(panel, 'false')
              if (!started || !stopped) throw new Error(`interaction fuzz playback roundtrip failed: started=${started} stopped=${stopped}`)
              return { detail: `playback=${started}/${stopped}` }
            }
            throw new Error(`unknown interaction fuzz action ${action}`)
          },
          assertInvariants: async () => {
            const project = await state()
            const integrity = interactionFuzzClipIntegrity(clips(project))
            const clipIds = integrity.mediaClips.map((clip) => clip.id).filter(Boolean)
            const tracks = (project?.tracks || []).map((item) => item.id).filter(Boolean)
            const ui = await verb('ui.state', {})
            const selected = ui.result?.selected_clip_ids || []
            const valid = new Set(clipIds)
            const selectedLive = Array.isArray(selected) && selected.every((id) => valid.has(id))
            const ok = new Set(clipIds).size === clipIds.length
              && new Set(tracks).size === tracks.length
              && !integrity.invalidMediaRange
              && !integrity.invalidGap
              && selectedLive
              && valid.has(fixture.primaryId)
              && valid.has(fixture.secondaryId)
            return {
              ok,
              detail: `clips=${clipIds.length}; gaps=${integrity.gaps.length}; tracks=${tracks.length}; selected=${selected.join(',') || 'none'}; invalidMediaRange=${integrity.invalidMediaRange}; invalidGap=${integrity.invalidGap}`,
            }
          },
        })
        if (!result.deterministic) throw new Error('identical interaction fuzz seed produced a divergent sequence')
        probe._interactionFuzz = {
          ...result,
          traceSha256: interactionFuzzTraceSha256(result.trace),
          finalFingerprintSha256: interactionFuzzFinalFingerprintSha256(await state()),
        }
        onAttestation?.(probe._interactionFuzz)
      },
      assertResult: async () => {
        const result = probe._interactionFuzz
        return {
          ok: !!result?.deterministic && result.sequence?.length > 0,
          detail: `seed=${result?.seed || 'missing'} steps=${result?.sequence?.length || 0} deterministic=${result?.deterministic === true}; trace=${result?.traceSha256 || 'missing'} final=${result?.finalFingerprintSha256 || 'missing'}; candidate=${result?.source?.gitCommit || 'missing'}/${result?.source?.contentManifestSha256 || 'missing'}`,
        }
      },
    })
  }

  return { run }
}
