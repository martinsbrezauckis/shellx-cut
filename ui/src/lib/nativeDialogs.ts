// Native Save/Open/confirm dialogs live in the operating system, outside the
// webview, so nothing in the DOM shows that the app is waiting behind one. This
// store records every dialog the app opened until it returns (chosen, cancelled
// or failed) so `ui.state` can say which dialog is open and which action opened
// it. The OS dialog itself is never inspected or answered.

export type NativeDialogKind = 'open' | 'save' | 'confirm' | 'message'

export interface NativeDialogEntry {
  kind: NativeDialogKind
  /** The app action that opened it, e.g. `pickExportOutput`. */
  opened_by: string
  /** The dialog's title as the person sees it (for a question: which one),
   *  e.g. "Work is still running". Titles are fixed app text, never a path. */
  title?: string
}

interface Tracked extends NativeDialogEntry { id: number }

/** The `title` option of a dialog call: the first argument of open/save, the
 *  second of confirm/message. */
function titleOf(kind: NativeDialogKind, args: readonly unknown[]): string | undefined {
  const options = kind === 'open' || kind === 'save' ? args[0] : args[1]
  const title = options && typeof options === 'object' ? (options as { title?: unknown }).title : undefined
  return typeof title === 'string' && title.trim() ? title.trim() : undefined
}

let nextId = 1
let open: Tracked[] = []
let snapshot: readonly NativeDialogEntry[] = []
const listeners = new Set<() => void>()

function publish(): void {
  snapshot = open.map(({ kind, opened_by, title }) => (title ? { kind, opened_by, title } : { kind, opened_by }))
  for (const listener of listeners) listener()
}

/** Stable between changes, so it can back `useSyncExternalStore`. */
export function getOpenNativeDialogs(): readonly NativeDialogEntry[] {
  return snapshot
}

export function subscribeNativeDialogs(listener: () => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

/** Mark a dialog open for exactly as long as `run` is pending. The entry is
 * cleared when it settles, whether the person chose, cancelled, or it threw. */
export async function trackNativeDialog<T>(kind: NativeDialogKind, openedBy: string, run: () => Promise<T>, title?: string): Promise<T> {
  const entry: Tracked = { id: nextId++, kind, opened_by: openedBy, ...(title ? { title } : {}) }
  open = [...open, entry]
  publish()
  try {
    return await run()
  } finally {
    open = open.filter((item) => item.id !== entry.id)
    publish()
  }
}

type DialogPlugin = typeof import('@tauri-apps/plugin-dialog')

function tracked<F extends (...args: never[]) => Promise<unknown>>(kind: NativeDialogKind, openedBy: string, fn: F): F {
  return ((...args: Parameters<F>) => trackNativeDialog(kind, openedBy, () => fn(...args), titleOf(kind, args))) as F
}

/** The dialog plugin's functions, each registering itself in the store while
 * its dialog is open. `loadPlugin` is injectable for tests. */
export async function loadTrackedDialogs(
  openedBy: string,
  loadPlugin: () => Promise<DialogPlugin> = () => import('@tauri-apps/plugin-dialog'),
): Promise<Pick<DialogPlugin, 'open' | 'save' | 'confirm' | 'message'>> {
  const plugin = await loadPlugin()
  return {
    open: tracked('open', openedBy, plugin.open),
    save: tracked('save', openedBy, plugin.save),
    confirm: tracked('confirm', openedBy, plugin.confirm),
    message: tracked('message', openedBy, plugin.message),
  }
}
