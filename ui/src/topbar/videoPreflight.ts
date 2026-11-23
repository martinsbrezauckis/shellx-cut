import type { PregateReport, PregateRisk } from '../lib/client'
import { errorText } from '../lib/jobFailureText'

/** The TopBar owns this gate for single renders, video exports, and queue submission. */
export type VideoPreflightStatus = 'blocked' | 'warning' | 'started'
export async function runVideoPreflightAction(
  actionLabel: string,
  action: () => Promise<void>,
  deps: {
    ffmpegMissing: boolean
    /** `blocked`: the render plan itself was refused (e.g. an odd project
     *  size); the action is not started and the sentence is shown instead. */
    check: () => Promise<{ ok: boolean; result?: PregateReport; error?: { message?: string; code?: string; cause?: string; suggested_action?: string }; blocked?: string }>
    showWarning: (report: PregateReport, actionLabel: string, action: () => Promise<void>) => void
    note: (message: string) => void
  },
): Promise<VideoPreflightStatus> {
  if (deps.ffmpegMissing) {
    deps.note(`Install FFmpeg before ${actionLabel}.`)
    return 'blocked'
  }
  const unavailable = (reason: string): VideoPreflightStatus => {
    deps.showWarning({ pass: true, risks: [{
      kind: 'checks_unavailable', severity: 'low',
      // The reason often names Settings > AI & transcription already (missing captions tools); say where to look only once.
      detail: `Picture and sound were not checked. ${reason}${reason.includes('Settings') ? '' : ' Check the captions tools in Settings > AI & transcription.'} You can cancel or export anyway.`,
    }] }, actionLabel, action)
    return 'warning'
  }
  try {
    const r = await deps.check()
    if (r.blocked) {
      deps.note(r.blocked)
      return 'blocked'
    }
    if (r.ok && r.result) {
      const report = r.result
      if (report.pass === false || (report.risks ?? []).length > 0 || (report.uninstrumented_assets ?? []).length > 0) {
        deps.showWarning(report, actionLabel, action)
        return 'warning'
      }
    } else {
      return unavailable(errorText(r.error, 'No check results were returned.'))
    }
  } catch (error) {
    return unavailable(errorText(error instanceof Error ? { message: error.message, cause: typeof error.cause === 'string' ? error.cause : undefined } : null, 'The check request did not complete.'))
  }
  await action()
  return 'started'
}

/** Item 82: the render plan (render.final dry_run) warns or refuses when the
 *  render needs more memory or disk than this computer has free. Those come
 *  back as preflight risks so the person sees the numbers and the way out
 *  BEFORE the render starts; a refusal is shown as a risk too (the render
 *  itself then fails with the same message). */
export function resourceRisks(plan: unknown): PregateRisk[] {
  if (typeof plan !== 'object' || plan === null) return []
  const p = plan as { resources?: { warnings?: unknown }; refused?: unknown }
  const risks: PregateRisk[] = []
  const warnings = Array.isArray(p.resources?.warnings) ? p.resources.warnings : []
  for (const w of warnings) {
    if (typeof w !== 'object' || w === null) continue
    const { code, message } = w as { code?: unknown; message?: unknown }
    if (typeof code === 'string' && typeof message === 'string') risks.push({ kind: code, severity: 'high', detail: message })
  }
  if (typeof p.refused === 'object' && p.refused !== null) {
    const { code, message, suggested_action: action } = p.refused as { code?: unknown; message?: unknown; suggested_action?: unknown }
    if (typeof code === 'string' && typeof message === 'string') {
      risks.push({ kind: code, severity: 'high', detail: typeof action === 'string' ? `${message}. ${action}` : message })
    }
  }
  return risks
}

/** The pregate report with the render plan's resource risks added. */
export function withResourceRisks(report: PregateReport | undefined, plan: unknown): PregateReport | undefined {
  const risks = resourceRisks(plan)
  if (risks.length === 0) return report
  const base: PregateReport = report ?? { pass: true, risks: [] }
  return { ...base, pass: false, risks: [...(base.risks ?? []), ...risks] }
}
