// Transcript speech-service contracts. These helpers hold the durable bindings
// that the compact Transcript Tools controls must preserve: current project,
// selected asset, returned job, and receipt/result for the action that started it.

import type { JobRecord, ProjectIdentity, VerbResult } from '../../lib/client'
import type { DoctorCard } from '../../lib/doctor'

export type SpeechServiceAction = 'diarize' | 'dub'

export interface SpeechAssetCandidates {
  selectedAsset?: string | null
  selectedClipAsset?: string | null
  activeAsset?: string | null
  assetIds: readonly string[]
}

export interface DiarizeRun {
  action: 'diarize'
  asset: string
  projectIdentity: ProjectIdentity
  jobId: string
}

export interface DubRun {
  action: 'dub'
  asset: string
  targetLang: string
  projectIdentity: ProjectIdentity
}

export type SpeechRun = DiarizeRun | DubRun

export type SpeechActionRequest =
  | { verb: 'media.diarize'; args: { asset: string }; fallback: string }
  | { verb: 'audio.dub'; args: { asset: string; target_lang: string }; fallback: string }

export type SpeechActionDispatcher = (request: SpeechActionRequest) => Promise<VerbResult | null>

export type SpeechStart =
  | { state: 'running'; run: DiarizeRun }
  | { state: 'success'; run: DubRun; message: string }
  | { state: 'error'; message: string }

export type SpeechCompletion =
  | { state: 'running'; message: string }
  | { state: 'success'; message: string }
  | { state: 'error'; message: string }
  | { state: 'stale'; message: string }

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

function nonEmpty(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null
}

function nonNegativeInteger(value: unknown): number | null {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0 ? value : null
}

export function sameSpeechProject(left: ProjectIdentity | null | undefined, right: ProjectIdentity | null | undefined): boolean {
  return !!left
    && !!right
    && left.origin_path_sha256 === right.origin_path_sha256
    && left.project_name === right.project_name
}

export function selectedSpeechAsset(candidates: SpeechAssetCandidates): string | null {
  const current = new Set(candidates.assetIds)
  for (const candidate of [candidates.selectedAsset, candidates.selectedClipAsset, candidates.activeAsset]) {
    if (candidate && current.has(candidate)) return candidate
  }
  return null
}

export function speechServiceAvailability(
  card: DoctorCard | null | undefined,
  asset: string | null,
  identity: ProjectIdentity | null | undefined,
  action: SpeechServiceAction,
): { ready: boolean; message: string } {
  if (!identity) return { ready: false, message: 'Waiting for the open project identity.' }
  if (!asset) return { ready: false, message: 'Select a transcribed clip or transcript words first.' }
  if (card?.status === 'ok') return { ready: true, message: (action === 'diarize' ? 'Speaker labels' : 'AI dubbing') + ' is ready for this asset.' }
  const label = action === 'diarize' ? 'Speaker labels' : 'AI dubbing'
  return {
    ready: false,
    message: card?.hint?.trim() || label + ' is not ready. Open Services & integrations to check it.',
  }
}

function diarizationResult(result: unknown, asset: string): { speakers: number; labeledWords: number } | null {
  const value = record(result)
  if (!value || value.diarization !== 'receipts/' + asset + '.diarize.json') return null
  const speakers = nonNegativeInteger(value.num_speakers)
  const labeledWords = nonNegativeInteger(value.labeled_words)
  return speakers === null || labeledWords === null ? null : { speakers, labeledWords }
}

function dubbingResult(result: unknown, asset: string, targetLang: string): { trackId: string; clips: number } | null {
  const value = record(result)
  if (!value
    || value.asset !== asset
    || value.target_lang !== targetLang
    || value.receipt !== 'receipts/' + asset + '.' + targetLang + '.dub.json') return null
  const trackId = nonEmpty(value.track_id)
  const clips = nonNegativeInteger(value.n_clips)
  return !trackId || clips === null ? null : { trackId, clips }
}

function jobError(record: JobRecord): string {
  return record.error?.message?.trim()
    || (record.outcome === 'cancelled' ? 'Speaker-label job was cancelled.' : 'Speaker-label job failed.')
}

export async function startDiarization(
  dispatch: SpeechActionDispatcher,
  asset: string,
  projectIdentity: ProjectIdentity,
): Promise<SpeechStart> {
  const result = await dispatch({
    verb: 'media.diarize',
    args: { asset },
    fallback: 'Could not start speaker labels.',
  })
  if (!result?.ok) return { state: 'error', message: 'Could not start speaker labels.' }
  const jobId = nonEmpty(record(result.result)?.job_id)
  return jobId
    ? { state: 'running', run: { action: 'diarize', asset, projectIdentity, jobId } }
    : { state: 'error', message: 'Speaker labels did not return a job id.' }
}

export async function startDubbing(
  dispatch: SpeechActionDispatcher,
  asset: string,
  targetLang: string,
  projectIdentity: ProjectIdentity,
): Promise<SpeechStart> {
  const result = await dispatch({
    verb: 'audio.dub',
    args: { asset, target_lang: targetLang },
    fallback: 'Could not create the dubbed track.',
  })
  const run: DubRun = { action: 'dub', asset, targetLang, projectIdentity }
  if (!result?.ok) return { state: 'error', message: 'Could not create the dubbed track.' }
  const completion = settleDubbing(run, result.result, projectIdentity)
  return completion.state === 'success'
    ? { state: 'success', run, message: completion.message }
    : { state: 'error', message: completion.message }
}

export function settleDiarization(
  run: DiarizeRun,
  status: JobRecord,
  currentIdentity: ProjectIdentity | null | undefined,
): SpeechCompletion {
  if (!sameSpeechProject(run.projectIdentity, currentIdentity)) {
    return { state: 'stale', message: 'Speaker-label result was discarded after the project changed.' }
  }
  if (status.job_id !== run.jobId) {
    return { state: 'stale', message: 'Received a different speaker-label job; its result was discarded.' }
  }
  if (status.state === 'queued' || status.state === 'running') {
    return { state: 'running', message: status.message?.trim() || 'Labeling speakers…' }
  }
  if (status.state === 'failed' || (status.outcome && status.outcome !== 'succeeded')) {
    return { state: 'error', message: jobError(status) }
  }
  if (status.state !== 'done') {
    return { state: 'error', message: 'Speaker-label job returned an invalid terminal state.' }
  }
  const result = diarizationResult(status.result, run.asset)
  if (!result) return { state: 'error', message: 'Speaker-label job completed without the expected asset receipt.' }
  const speakerNoun = result.speakers === 1 ? 'speaker' : 'speakers'
  return {
    state: 'success',
    message: 'Labeled ' + result.labeledWords + ' words for ' + result.speakers + ' ' + speakerNoun + '.',
  }
}

export function settleDubbing(
  run: DubRun,
  result: unknown,
  currentIdentity: ProjectIdentity | null | undefined,
): SpeechCompletion {
  if (!sameSpeechProject(run.projectIdentity, currentIdentity)) {
    return { state: 'stale', message: 'Dub result was discarded after the project changed.' }
  }
  const accepted = dubbingResult(result, run.asset, run.targetLang)
  if (!accepted) return { state: 'error', message: 'Dubbing completed without the expected asset, language, track, and receipt.' }
  const clipNoun = accepted.clips === 1 ? 'clip' : 'clips'
  return { state: 'success', message: 'Added dubbed track ' + accepted.trackId + ' with ' + accepted.clips + ' ' + clipNoun + '.' }
}
