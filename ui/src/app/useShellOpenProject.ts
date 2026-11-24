// A project named when the app is launched (`ShellX Cut <folder>.cutproj`), or
// by a second launch that the shell turned into "bring this window forward and
// open it" (one window per user). Opening follows the Projects panel's rule:
// work someone asked for is never cancelled without asking.

import { useEffect } from 'react'
import { callVerb } from '../lib/client'
import { confirmAction, onShellOpenProject, takeLaunchProject } from '../lib/tauri'
import { publishUserActionMessage } from '../lib/userActionFeedback'

type OpenReply = {
  ok: boolean
  error?: { code?: string; message?: string; cause?: string; suggested_action?: string } | null
  warnings?: Array<{ code: string; message: string }>
}

export interface ShellOpenEffects {
  open: (path: string, cancelRunningJobs: boolean) => Promise<OpenReply>
  confirm: (text: string, opts: { title: string; okLabel: string; cancelLabel: string }) => Promise<boolean>
  tell: (message: string, code?: string) => void
}

/** Open `path`; returns what happened, for tests and logs. */
export async function openProjectFromShell(path: string, effects: ShellOpenEffects): Promise<'opened' | 'kept' | 'failed'> {
  let reply = await effects.open(path, false)
  if (!reply.ok && reply.error?.code === 'jobs_running') {
    const go = await effects.confirm(
      `${reply.error.message}.\n\n${reply.error.cause}\n\nCancel them and continue? The cancelled work is lost and must be started again.`,
      { title: 'Work is still running', okLabel: 'Cancel it and continue', cancelLabel: 'Keep working here' },
    )
    if (!go) return 'kept'
    reply = await effects.open(path, true)
  }
  if (!reply.ok) {
    const reason = reply.error?.message ?? reply.error?.code ?? 'unknown error'
    effects.tell(`The project could not be opened: ${reason}.${reply.error?.suggested_action ? ` ${reply.error.suggested_action}` : ''}`)
    return 'failed'
  }
  for (const warning of reply.warnings ?? []) effects.tell(warning.message, warning.code)
  return 'opened'
}

const EFFECTS: ShellOpenEffects = {
  open: (path, cancel) => callVerb('project.open', cancel ? { path, cancel_running_jobs: true } : { path }) as Promise<OpenReply>,
  confirm: (text, opts) => confirmAction(text, opts),
  tell: (message, code) => publishUserActionMessage(message, { source: 'project.open', ...(code ? { code } : {}) }),
}

export function useShellOpenProject(): void {
  useEffect(() => {
    let cancelled = false
    void takeLaunchProject().then((path) => {
      if (path && !cancelled) void openProjectFromShell(path, EFFECTS)
    })
    const off = onShellOpenProject((path) => { void openProjectFromShell(path, EFFECTS) })
    return () => {
      cancelled = true
      off()
    }
  }, [])
}
