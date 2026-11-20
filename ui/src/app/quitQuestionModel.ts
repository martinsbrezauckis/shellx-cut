// The question the app asks when it is closed while an export (or other work
// someone asked for) runs, as large editors do ("An export is in progress.
// Quit anyway?"). Keeping the work is the default: "Keep working" has the
// focus (Enter), and Esc or clicking outside keep working too; only the
// explicit button quits and stops it.

import type { QuitJob, QuitJobsQuestion } from '../lib/tauri'
import { jobTaskName } from '../lib/jobPresentation'

export const QUIT_QUESTION_TITLE = 'An export is still running'
export const QUIT_QUESTION_TITLE_WORK = 'Work is still running'

/** Kinds that write an export file. Anything else is named as work. */
export function isExportKind(kind: string): boolean {
  return kind === 'render' || kind === 'render_queue' || kind === 'screen_record_export' || kind === 'reframe' || kind.startsWith('reframe-')
}
export const KEEP_WORKING = 'Keep working'
export const QUIT_AND_STOP = 'Quit and stop it'
export const STOPPING_BEFORE_QUIT = 'Stopping the work, then ShellX Cut closes…'

export interface QuitQuestionView {
  title: string
  /** What runs, in the person's words ("Render, 40% done"). */
  running: string[]
  /** What quitting now costs. */
  consequence: string
}

/** The running jobs of a question: one record each, from the engine. */
export function questionJobs(question: QuitJobsQuestion): QuitJob[] {
  return (question.running ?? []).filter((job) => typeof job?.job_id === 'string' && typeof job?.kind === 'string')
}

/** The progress the engine stated when it was asked, 0..100, or null. */
export function statedPercent(job: QuitJob): number | null {
  return typeof job.progress === 'number' && Number.isFinite(job.progress)
    ? Math.max(0, Math.min(100, job.progress * 100))
    : null
}

/** How far a job is now, as far as this window knows: a percent read from
 * the engine a moment ago, "finished", or null when nothing current is known
 * (then no number is shown rather than one that has gone stale). */
export type LiveProgress = number | 'finished' | 'stopped' | null

/** A running job in the person's words: "Render, 37% done". */
export function personJob(job: QuitJob, live: LiveProgress = statedPercent(job)): string {
  const name = jobTaskName(job.kind)
  if (live === 'finished' || live === 'stopped') return `${name}, ${live}`
  return live === null ? name : `${name}, ${Math.round(live)}% done`
}

/** A running job as ui.state lists it for agents, from its record, with the
 * progress the dialog shows: "job_005 (render, 37% done)". Names and numbers
 * only: a job's free-text message can carry a full path (a setup step prints
 * its tool's output), so it stays in the dialog and the job record. */
export function agentListing(job: QuitJob, live: LiveProgress): string {
  const progress = live === null ? '' : typeof live === 'number' ? `, ${Math.round(live)}% done` : `, ${live}`
  return `${job.job_id} (${job.kind}${progress})`
}

/** The running lines ui.state reports, from the dialog's JSON array; anything
 * that is not such a list gives none (it is never split). */
export function runningLines(value: string | undefined): string[] {
  try {
    const parsed: unknown = JSON.parse(value ?? '[]')
    return Array.isArray(parsed) ? parsed.filter((line): line is string => typeof line === 'string' && line.trim() !== '') : []
  } catch {
    return []
  }
}

/** The engine listing "job_005 (render, 8% done, rendering)" with the same
 * progress the dialog shows: the live percent, "finished"/"stopped", or no
 * number when nothing current is known. ui.state reports these lines, so an
 * agent reads what the person sees, never the number from when it was asked. */
export function liveListing(job: string, live: LiveProgress): string {
  const stated = /,?\s*\d+% done/
  if (!stated.test(job)) return job
  if (live === null) return job.replace(stated, '')
  return job.replace(/\d+% done/, typeof live === 'number' ? `${Math.round(live)}% done` : live)
}

/** `progressOf(jobId)` gives the live progress; without it the numbers the
 * engine stated when the question was asked are used. One line per job. */
export function quitQuestionView(
  question: QuitJobsQuestion,
  progressOf?: (jobId: string) => LiveProgress,
): QuitQuestionView {
  const jobs = questionJobs(question)
  const exportsOnly = jobs.length > 0 && jobs.every((job) => isExportKind(job.kind))
  const running = jobs.map((job) => personJob(job, progressOf ? progressOf(job.job_id) : statedPercent(job)))
  return {
    title: exportsOnly ? QUIT_QUESTION_TITLE : QUIT_QUESTION_TITLE_WORK,
    running: running.length > 0 ? running : ['work you started'],
    consequence: 'Quit anyway? The stopped work is lost and must be started again. Keep working to let it finish.',
  }
}

/** How long a progress reading stays current enough to show. */
export const LIVE_PROGRESS_FRESH_MS = 3_000

/** A `jobs.status` reply as live progress: a percent, finished, or unknown. */
export function liveFromStatus(reply: { ok: boolean; result?: unknown }): LiveProgress {
  if (!reply.ok) return null
  const record = reply.result as { state?: unknown; progress?: unknown } | undefined
  if (record?.state === 'done') return 'finished'
  if (record?.state === 'failed') return 'stopped'
  return typeof record?.progress === 'number' && Number.isFinite(record.progress)
    ? Math.max(0, Math.min(100, record.progress * 100))
    : null
}

/** Esc keeps working. Enter presses the focused button, which starts on
 * "Keep working". */
export function answerForKey(key: string): 'keep' | null {
  return key === 'Escape' ? 'keep' : null
}
