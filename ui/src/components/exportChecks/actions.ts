// What the buttons of an export check row do. One place, used by the export
// warning and the finished-file list: go to a place, open the captions tools in
// Settings, or run the one check the person asked for ("Check now") and wait for
// it. Copy details is done by the list itself.
import type { VerbResult } from '../../lib/client'
import type { ExportCheckAction, ExportCheckEntry, ExportChecks } from '../../lib/exportChecks'
import type { ExportCheckActionHandler } from './ExportCheckList'

/** The captions tools live under Settings > AI & transcription. */
export function openCaptionsToolsSettings(): void {
  document.dispatchEvent(new CustomEvent('cut:open-ui-surface', { detail: { id: 'settings-ai-transcription' } }))
}

export interface ExportCheckActionDeps {
  /** Moves the playhead. */
  seek: (atMs: number) => void | Promise<unknown>
  /** Calls a verb by its wire name (the engine validates the arguments). */
  call: (verb: string, args: unknown) => Promise<VerbResult>
  /** Opens Settings at the captions tools. Defaults to the shared opener. */
  openTools?: (toolId: string) => void
  /** Runs after a Check now finished, so the list can be read again. */
  afterCommand?: () => void | Promise<unknown>
  /** Polling pause and the longest wait for one background check. */
  pollMs?: number
  maxWaitMs?: number
  wait?: (ms: number) => Promise<void>
  now?: () => number
}

const sleep = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms))

function jobIdOf(result: unknown): string | null {
  if (typeof result !== 'object' || result === null) return null
  const id = (result as { job_id?: unknown }).job_id
  return typeof id === 'string' && id ? id : null
}

/** Waits for a background job. Throws when it fails or takes too long. */
export async function waitForCheckJob(jobId: string, deps: Pick<ExportCheckActionDeps, 'call' | 'pollMs' | 'maxWaitMs' | 'wait' | 'now'>): Promise<void> {
  const pause = deps.wait ?? sleep
  const now = deps.now ?? Date.now
  const deadline = now() + (deps.maxWaitMs ?? 30 * 60_000)
  for (;;) {
    const status = await deps.call('jobs.status', { job_id: jobId })
    if (!status.ok) throw new Error(status.error?.message ?? 'The check could not be read back.')
    const state = (status.result as { state?: unknown } | undefined)?.state
    if (state === 'done') return
    if (state === 'failed' || state === 'cancelled') {
      const record = status.result as { error?: { message?: string } } | undefined
      throw new Error(record?.error?.message ?? 'The check did not finish.')
    }
    if (now() >= deadline) throw new Error('The check is still running.')
    await pause(deps.pollMs ?? 500)
  }
}

export function createExportCheckActionHandler(deps: ExportCheckActionDeps): ExportCheckActionHandler {
  return async (action: ExportCheckAction, _entry: ExportCheckEntry) => {
    if (action.kind === 'seek') {
      if (typeof action.at_ms === 'number') await deps.seek(action.at_ms)
      return
    }
    if (action.kind === 'open_tools') {
      ;(deps.openTools ?? (() => openCaptionsToolsSettings()))(action.tool_id ?? 'perception')
      return
    }
    if (action.kind === 'command') {
      if (!action.verb) throw new Error('This check has no command.')
      const started = await deps.call(action.verb, action.args ?? {})
      if (!started.ok) throw new Error(started.error?.message ?? 'The check could not start.')
      const jobId = jobIdOf(started.result)
      if (jobId) await waitForCheckJob(jobId, deps)
      await deps.afterCommand?.()
    }
  }
}

/** A finished export's list is a record of that export: a new check would not
 *  change it, so its rows offer no Check now. Everything else stays. */
export function withoutCommands(checks: ExportChecks): ExportChecks {
  return {
    ...checks,
    entries: checks.entries.map((entry) => (entry.action?.kind === 'command' ? { ...entry, action: undefined } : entry)),
  }
}
