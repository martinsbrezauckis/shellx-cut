import type { PregateReport } from '../lib/client'

/** The TopBar owns this gate for single renders, video exports, and queue submission. */
export type VideoPreflightStatus = 'blocked' | 'warning' | 'started'
export async function runVideoPreflightAction(
  actionLabel: string,
  action: () => Promise<void>,
  deps: {
    ffmpegMissing: boolean
    check: () => Promise<{ ok: boolean; result?: PregateReport; error?: { message?: string; code?: string } }>
    showWarning: (report: PregateReport, actionLabel: string, action: () => Promise<void>) => void
    note: (message: string) => void
  },
): Promise<VideoPreflightStatus> {
  if (deps.ffmpegMissing) {
    deps.note(`Install FFmpeg before ${actionLabel}.`)
    return 'blocked'
  }
  try {
    const r = await deps.check()
    if (r.ok && r.result) {
      const report = r.result
      if (report.pass === false || (report.risks ?? []).length > 0 || (report.uninstrumented_assets ?? []).length > 0) {
        deps.showWarning(report, actionLabel, action)
        return 'warning'
      }
    } else if (!r.ok) {
      deps.note(`preflight unavailable: ${r.error?.message ?? r.error?.code ?? 'continuing'}`)
    }
  } catch {
    deps.note('preflight unavailable; continuing')
  }
  await action()
  return 'started'
}
