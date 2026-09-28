// Transcript Tools speech-service contract. Run:
// npx tsx public-tests/transcript-speech-actions.test.ts

import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import {
  selectedSpeechAsset,
  settleDiarization,
  settleDubbing,
  speechServiceAvailability,
  startDiarization,
  startDubbing,
} from '../src/panels/Transcript/speechServiceModel'
import { createDiarizationPoller } from '../src/panels/Transcript/useSpeechServiceActions'
import { AGENT_PROMPT_LIBRARY } from '../src/panels/AgentChat/promptLibrary'

const root = resolve(import.meta.dirname, '..')
const transcript = readFileSync(resolve(root, 'src/panels/Transcript/index.tsx'), 'utf8')
const phraseList = readFileSync(resolve(root, 'src/panels/Transcript/TranscriptPhraseList.tsx'), 'utf8')
const serviceRuntime = readFileSync(resolve(root, 'src/panels/Environment/ServiceRuntime.tsx'), 'utf8')
const feedback = readFileSync(resolve(root, 'src/lib/userActionFeedback.ts'), 'utf8')
const schema = JSON.parse(readFileSync(resolve(root, '../schema/verbs.json'), 'utf8')) as {
  verbs: Array<{ name: string; behavior: { agent_chat: string } }>
}
const behavior = new Map(schema.verbs.map((verb) => [verb.name, verb.behavior.agent_chat]))
const identity = {
  schema: 'shellx-cut/project-identity/1' as const,
  origin_path_sha256: 'sha256:' + 'a'.repeat(64),
  project_name: 'speech-contract',
}
const changedIdentity = { ...identity, project_name: 'other-project' }

assert.equal(
  selectedSpeechAsset({
    selectedAsset: 'old-asset',
    selectedClipAsset: 'clip-asset',
    activeAsset: 'active-asset',
    assetIds: ['clip-asset', 'active-asset'],
  }),
  'clip-asset',
  'speech action ignores a stale selection before using the selected timeline occurrence',
)
assert.equal(
  selectedSpeechAsset({ selectedAsset: null, selectedClipAsset: null, activeAsset: 'active-asset', assetIds: ['active-asset'] }),
  'active-asset',
  'speech action can use the current transcript occurrence',
)
assert.equal(
  speechServiceAvailability({ id: 'diarize', kind: 'service', status: 'ok', details: {} }, 'asset-a', identity, 'diarize').ready,
  true,
  'only an ok Doctor diarize card enables the direct action',
)
assert.equal(
  speechServiceAvailability({ id: 'diarize', kind: 'service', status: 'unknown', details: {} }, 'asset-a', identity, 'diarize').ready,
  false,
  'unknown Doctor evidence never enables an external service action',
)

const calls: unknown[] = []
const diarize = await startDiarization(async (request) => {
  calls.push(request)
  return { ok: true, result: { job_id: 'job-diarize-1' } }
}, 'asset-a', identity)
assert.deepEqual(calls, [{
  verb: 'media.diarize',
  args: { asset: 'asset-a' },
  fallback: 'Could not start speaker labels.',
}], 'Label speakers dispatches the exact selected asset through the human callback')
assert.equal(diarize.state, 'running')
if (diarize.state !== 'running') throw new Error('diarize start fixture did not create a run')

assert.equal(
  settleDiarization(diarize.run, {
    job_id: 'job-other', kind: 'diarize', state: 'done', progress: 1,
    created_ts: '', updated_ts: '', result: {},
  }, identity).state,
  'stale',
  'a different job id cannot complete this user action',
)
assert.equal(
  settleDiarization(diarize.run, {
    job_id: 'job-diarize-1', kind: 'diarize', state: 'done', progress: 1,
    created_ts: '', updated_ts: '', result: { diarization: 'receipts/asset-b.diarize.json', num_speakers: 2, labeled_words: 12 },
  }, identity).state,
  'error',
  'a completed job with another asset receipt is refused',
)
assert.equal(
  settleDiarization(diarize.run, {
    job_id: 'job-diarize-1', kind: 'diarize', state: 'done', progress: 1,
    created_ts: '', updated_ts: '', result: { diarization: 'receipts/asset-a.diarize.json', num_speakers: 2, labeled_words: 12 },
  }, changedIdentity).state,
  'stale',
  'project identity change discards an otherwise valid asynchronous completion',
)
assert.equal(
  settleDiarization(diarize.run, {
    job_id: 'job-diarize-1', kind: 'diarize', state: 'done', progress: 1,
    created_ts: '', updated_ts: '', result: { diarization: 'receipts/asset-a.diarize.json', num_speakers: 2, labeled_words: 12 },
  }, identity).state,
  'success',
  'the exact returned job and expected asset receipt complete speaker labels',
)
assert.equal(
  settleDiarization(diarize.run, {
    job_id: 'job-diarize-1', kind: 'diarize', state: 'unknown' as never, progress: 1,
    created_ts: '', updated_ts: '', result: { diarization: 'receipts/asset-a.diarize.json', num_speakers: 2, labeled_words: 12 },
  }, identity).state,
  'error',
  'a receipt-shaped response cannot pass without the actual done job state',
)
assert.equal(
  settleDiarization(diarize.run, {
    job_id: 'job-diarize-1', kind: 'diarize', state: 'done', outcome: 'cancelled', progress: 1,
    created_ts: '', updated_ts: '', result: { diarization: 'receipts/asset-a.diarize.json', num_speakers: 2, labeled_words: 12 },
  }, identity).state,
  'error',
  'a non-success terminal outcome cannot surface a speaker-label result',
)

const dub = await startDubbing(async (request) => {
  calls.push(request)
  return {
    ok: true,
    result: {
      asset: 'asset-a', target_lang: 'lv', track_id: 'dub1', n_clips: 4,
      receipt: 'receipts/asset-a.lv.dub.json',
    },
  }
}, 'asset-a', 'lv', identity)
assert.deepEqual(calls.at(-1), {
  verb: 'audio.dub',
  args: { asset: 'asset-a', target_lang: 'lv' },
  fallback: 'Could not create the dubbed track.',
}, 'Dub audio dispatches the exact selected asset and user-selected language')
assert.equal(dub.state, 'success', 'a matching synchronous dub result is surfaced as a result')
if (dub.state !== 'success') throw new Error('dub start fixture did not complete')
assert.equal(
  settleDubbing(dub.run, {
    asset: 'asset-a', target_lang: 'de', track_id: 'dub1', n_clips: 4,
    receipt: 'receipts/asset-a.de.dub.json',
  }, identity).state,
  'error',
  'a dub response for a different language is refused',
)
assert.equal(
  settleDubbing(dub.run, {
    asset: 'asset-a', target_lang: 'lv', track_id: 'dub1', n_clips: 4,
    receipt: 'receipts/asset-a.lv.dub.json',
  }, changedIdentity).state,
  'stale',
  'a dub response cannot update a later project',
)

function deferred<T>() {
  let resolve: (value: T) => void = () => undefined
  let reject: (reason?: unknown) => void = () => undefined
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve
    reject = nextReject
  })
  return { promise, resolve, reject }
}

function job(jobId: string, asset: string) {
  return {
    job_id: jobId,
    kind: 'diarize',
    state: 'done' as const,
    outcome: 'succeeded' as const,
    progress: 1,
    created_ts: '',
    updated_ts: '',
    result: { diarization: `receipts/${asset}.diarize.json`, num_speakers: 2, labeled_words: 12 },
  }
}

function scheduledCallbacks() {
  let next = 0
  const callbacks = new Map<number, () => void>()
  return {
    schedule(callback: () => void) {
      const id = ++next
      callbacks.set(id, callback)
      return id
    },
    clear(id: number) {
      callbacks.delete(id)
    },
    runNext() {
      const nextCallback = callbacks.entries().next().value as [number, () => void] | undefined
      if (!nextCallback) throw new Error('expected a scheduled poll')
      callbacks.delete(nextCallback[0])
      nextCallback[1]()
    },
    size() {
      return callbacks.size
    },
  }
}

async function flushAsync() {
  await Promise.resolve()
  await Promise.resolve()
  await Promise.resolve()
}

const oldStatus = deferred<{ ok: boolean; result: ReturnType<typeof job> }>()
const newStatus = deferred<{ ok: boolean; result: ReturnType<typeof job> }>()
const oldRun = { action: 'diarize' as const, asset: 'asset-old', projectIdentity: identity, jobId: 'job-old' }
const newRun = { action: 'diarize' as const, asset: 'asset-new', projectIdentity: identity, jobId: 'job-new' }
const oldNewScheduler = scheduledCallbacks()
const oldNewStates: string[] = []
let oldNewSuccesses = 0
const oldNewPoller = createDiarizationPoller({
  readStatus: (jobId) => jobId === 'job-old' ? oldStatus.promise : newStatus.promise,
  currentIdentity: () => identity,
  onState: (next) => oldNewStates.push(next.phase + ':' + next.message),
  onSuccess: () => { oldNewSuccesses += 1 },
  schedule: (callback) => oldNewScheduler.schedule(callback),
  clearSchedule: (timer) => oldNewScheduler.clear(timer),
})
oldNewPoller.start(oldRun, 0)
oldNewScheduler.runNext()
await flushAsync()
oldNewPoller.start(newRun, 0)
oldNewScheduler.runNext()
await flushAsync()
oldStatus.resolve({ ok: true, result: job('job-old', 'asset-old') })
await flushAsync()
assert.deepEqual(oldNewStates, [], 'an old status response cannot update the newer diarization run after its await')
assert.equal(oldNewSuccesses, 0, 'an old status response cannot refresh the project')
newStatus.resolve({ ok: true, result: job('job-new', 'asset-new') })
await flushAsync()
assert.deepEqual(oldNewStates, ['success:Labeled 12 words for 2 speakers.'], 'only the current diarization run can publish its result')
assert.equal(oldNewSuccesses, 1, 'only the current diarization run refreshes the project')

const unmountStatus = deferred<{ ok: boolean; result: ReturnType<typeof job> }>()
const unmountScheduler = scheduledCallbacks()
const unmountStates: string[] = []
let unmountSuccesses = 0
const unmountPoller = createDiarizationPoller({
  readStatus: () => unmountStatus.promise,
  currentIdentity: () => identity,
  onState: (next) => unmountStates.push(next.phase),
  onSuccess: () => { unmountSuccesses += 1 },
  schedule: (callback) => unmountScheduler.schedule(callback),
  clearSchedule: (timer) => unmountScheduler.clear(timer),
})
unmountPoller.start(oldRun, 0)
unmountScheduler.runNext()
await flushAsync()
// The hook's unmount cleanup calls cancel(), so this drives the same generation,
// active-run, and timer invalidation without a DOM test runtime.
unmountPoller.cancel()
unmountStatus.resolve({ ok: true, result: job('job-old', 'asset-old') })
await flushAsync()
assert.deepEqual(unmountStates, [], 'unmount invalidates an in-flight status response')
assert.equal(unmountSuccesses, 0, 'unmount cannot refresh a project from an old response')
assert.equal(unmountScheduler.size(), 0, 'unmount leaves no diarization timer behind')

const rejectedStatus = deferred<{ ok: boolean; result: ReturnType<typeof job> }>()
const rejectedScheduler = scheduledCallbacks()
const rejectedPoller = createDiarizationPoller({
  readStatus: () => rejectedStatus.promise,
  currentIdentity: () => identity,
  onState: () => { throw new Error('invalidated poller must not publish state') },
  onSuccess: () => { throw new Error('invalidated poller must not publish success') },
  schedule: (callback) => rejectedScheduler.schedule(callback),
  clearSchedule: (timer) => rejectedScheduler.clear(timer),
})
rejectedPoller.start(oldRun, 0)
rejectedScheduler.runNext()
await flushAsync()
rejectedPoller.cancel()
rejectedStatus.reject(new Error('temporary reconnect failure'))
await flushAsync()
assert.equal(rejectedScheduler.size(), 0, 'an invalidated polling exception cannot schedule a retry')

assert.equal(AGENT_PROMPT_LIBRARY.length, 8, 'the prompt library remains a deliberate compact catalog')
for (const preset of AGENT_PROMPT_LIBRARY) {
  assert.ok(preset.verbs.length > 0, preset.id + ' declares real contained verbs')
  assert.ok(
    preset.verbs.every((verb) => behavior.get(verb) === 'inspect' || behavior.get(verb) === 'edit'),
    preset.id + ' only advertises inspect/edit Chat policy verbs',
  )
}
assert.match(AGENT_PROMPT_LIBRARY.find((preset) => preset.id === 'label-speakers')?.prompt ?? '', /Transcript Tools > Label speakers/)
assert.match(AGENT_PROMPT_LIBRARY.find((preset) => preset.id === 'dub-latvian')?.prompt ?? '', /Transcript Tools > Dub audio/)
assert.match(AGENT_PROMPT_LIBRARY.find((preset) => preset.id === 'vertical-highlights')?.prompt ?? '', /Clips drawer/)
assert.match(AGENT_PROMPT_LIBRARY.find((preset) => preset.id === 'preflight-review')?.prompt ?? '', /Review > QC/)

for (const selector of [
  'data-cut-transcript-speech-actions',
  'data-cut-transcript-diarize',
  'data-cut-transcript-dub',
  'data-cut-transcript-dub-language',
  'data-cut-transcript-speech-status',
  'data-cut-transcript-speech-setup',
]) assert.match(transcript, new RegExp(selector), 'Transcript Tools exposes ' + selector)
assert.match(phraseList, /data-cut-transcript-phrase-asset/, 'rendered transcript phrases expose their exact owning asset')
assert.match(transcript, /speechRefreshRevision/, 'successful diarization has a dedicated transcript refresh revision')
assert.match(transcript, /setSpeechRefreshRevision\(\(revision\) => revision \+ 1\)/, 'a completed current speech action refreshes timeline labels even without an edit op')
assert.match(transcript, /ops, speechRefreshRevision/, 'timeline reads are invalidated by the completed speech refresh revision')
assert.match(serviceRuntime, /data-cut-env-service-transcript/, 'ready service cards route to the actual Transcript Tools surface')
assert.match(serviceRuntime, /cut:open-transcript-speech-actions/, 'service handoff opens the concrete direct-action menu')
assert.doesNotMatch(serviceRuntime, /chatPrompt/, 'ready service cards do not advertise a Chat service execution prompt')
assert.match(feedback, /'audio\.dub': 'settings-services-integrations'/)
assert.match(feedback, /'media\.diarize': 'settings-services-integrations'/)

console.log('PASS transcript speech actions, service handoff, and contained Chat presets')
