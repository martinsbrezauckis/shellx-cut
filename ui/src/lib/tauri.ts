// lib/tauri.ts — thin bridge to the native desktop shell.
//
// Role: the UI is otherwise PURE REST (it talks only to the loopback cutd
// engine). These helpers add the few desktop-only affordances a browser cannot
// provide: native file dialogs, OS drag-drop, window fullscreen, and one
// native-shell update preference that must be known before the web UI loads.
//
// All guarded by isTauri(): on a browser/remote build every helper degrades to
// a no-op so the existing path-input stays the fallback. withGlobalTauri:true
// (tauri.conf) exposes window.__TAURI__ for the small invoke/listen helpers;
// file drag/drop uses Tauri's official current-Webview subscription because
// native events target that Webview rather than the generic app event bus.

/* eslint-disable @typescript-eslint/no-explicit-any */
import { validShellUpdateState, type ShellUpdateState } from './updateState'

type TauriGlobal = {
  core: { invoke: <T = unknown>(cmd: string, args?: Record<string, unknown>) => Promise<T> }
  event: { listen: (event: string, cb: (e: { payload: unknown }) => void) => Promise<() => void> }
}

function tauri(): TauriGlobal | null {
  return (typeof window !== 'undefined' && (window as any).__TAURI__) || null
}

/** True when running inside the Tauri desktop shell (vs a browser/remote tab). */
export function isTauri(): boolean {
  return !!tauri()
}

export interface LaunchUpdatePreference {
  schema: 'shellx-cut/update-preferences/1'
  check_on_launch: boolean
}

function validLaunchUpdatePreference(value: unknown): value is LaunchUpdatePreference {
  if (!value || typeof value !== 'object') return false
  const candidate = value as Partial<LaunchUpdatePreference>
  return candidate.schema === 'shellx-cut/update-preferences/1'
    && typeof candidate.check_on_launch === 'boolean'
}

/** Read the installed shell's persisted launch-update preference. */
export async function getLaunchUpdatePreference(): Promise<LaunchUpdatePreference | null> {
  const t = tauri()
  if (!t) return null
  try {
    const value = await t.core.invoke<unknown>('get_update_preferences')
    return validLaunchUpdatePreference(value) ? value : null
  } catch {
    return null
  }
}

/** Replace the installed shell's launch-update preference and re-read it. */
export async function setLaunchUpdatePreference(
  checkOnLaunch: boolean,
): Promise<LaunchUpdatePreference | null> {
  const t = tauri()
  if (!t) return null
  try {
    const value = await t.core.invoke<unknown>('set_update_preferences', {
      checkOnLaunch,
    })
    return validLaunchUpdatePreference(value) ? value : null
  } catch {
    return null
  }
}

/**
 * Read the shell's live update snapshot (update_state.rs). Null in a browser
 * build or when the bridge/validation fails — callers render nothing then.
 */
export async function getShellUpdateState(): Promise<ShellUpdateState | null> {
  const t = tauri()
  if (!t) return null
  try {
    const value = await t.core.invoke<unknown>('get_update_state')
    return validShellUpdateState(value) ? value : null
  } catch {
    return null
  }
}

/**
 * Manual "Check for updates" (Settings > About). Deliberately independent of
 * the automatic-check preference — the shell command performs one check on
 * this explicit request and returns the post-check snapshot. Null = bridge
 * unavailable (browser build) or an invalid reply.
 */
export async function checkForUpdatesNow(): Promise<ShellUpdateState | null> {
  const t = tauri()
  if (!t) return null
  try {
    const value = await t.core.invoke<unknown>('update_check_now')
    return validShellUpdateState(value) ? value : null
  } catch {
    return null
  }
}

/** Reply shape of `update_install_now` (the flow either restarts the app or
 *  reports exactly why it did not). */
export interface UpdateInstallReply {
  ok: boolean
  /** True when the user chose "Later" in the native confirm. */
  cancelled?: boolean
  /** Honest failure text when the install could not complete. */
  error?: string
}

/**
 * Ask the shell to install the offered update. The SHELL still runs the
 * native confirm dialog and the signature-verified download+install — the
 * webview can only request. On success the app restarts, so this promise may
 * never resolve; on decline/failure it resolves with the honest reply. Null =
 * bridge unavailable (browser build).
 */
export async function installUpdateNow(): Promise<UpdateInstallReply | null> {
  const t = tauri()
  if (!t) return null
  try {
    const value = await t.core.invoke<unknown>('update_install_now')
    if (value && typeof value === 'object' && typeof (value as UpdateInstallReply).ok === 'boolean') {
      return value as UpdateInstallReply
    }
    return { ok: false, error: 'the desktop shell returned an unexpected reply' }
  } catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) }
  }
}

/**
 * Subscribe to shell update-state broadcasts (`cut:update-state`, emitted by
 * update_state.rs on every transition). Same `window.__TAURI__.event.listen`
 * path as onRecordHotkey (the engine-served remote origin holds
 * core:event:allow-listen). Invalid payloads are dropped, not surfaced.
 * Returns an unsubscribe fn; a no-op outside Tauri.
 */
export function onShellUpdateState(cb: (state: ShellUpdateState) => void): () => void {
  const t = tauri()
  if (!t) return () => {}
  let off: (() => void) | null = null
  let cancelled = false
  t.event
    .listen('cut:update-state', (event) => {
      if (validShellUpdateState(event.payload)) cb(event.payload)
    })
    .then((unlisten) => {
      // Unsubscribed before the listener registered ⇒ drop it immediately
      // (mirrors onRecordHotkey — avoids a leaked listener across re-subscribes).
      if (cancelled) unlisten()
      else off = unlisten
    })
    .catch(() => {})
  return () => {
    cancelled = true
    off?.()
  }
}

/**
 * Ask for destructive-action confirmation without using the dialog plugin's
 * injected `window.confirm` shim. In dialog 2.7.x that legacy shim invokes the
 * removed `plugin:dialog|confirm` command, while the supported module API
 * routes through `plugin:dialog|message`.
 *
 * Fail closed: an unavailable native dialog never authorizes the action.
 */
export async function confirmAction(
  text: string,
  opts?: { title?: string; okLabel?: string; cancelLabel?: string },
): Promise<boolean> {
  if (!isTauri()) {
    return typeof window !== 'undefined' ? window.confirm(text) : false
  }
  try {
    const { confirm } = await import('@tauri-apps/plugin-dialog')
    return await confirm(text, {
      title: opts?.title ?? 'ShellX Cut',
      kind: 'warning',
      okLabel: opts?.okLabel,
      cancelLabel: opts?.cancelLabel,
    })
  } catch {
    return false
  }
}

/** Show an actionable error through the supported dialog message command. */
export async function showMessage(
  text: string,
  opts?: { title?: string; kind?: 'info' | 'warning' | 'error' },
): Promise<void> {
  if (!isTauri()) {
    if (typeof window !== 'undefined') window.alert(text)
    return
  }
  try {
    const { message } = await import('@tauri-apps/plugin-dialog')
    await message(text, {
      title: opts?.title ?? 'ShellX Cut',
      kind: opts?.kind ?? 'info',
    })
  } catch {
    // The caller already owns its inline error state; a native-message failure
    // must not turn into an unhandled page error.
  }
}

/**
 * Toggle the native app window's fullscreen state.
 *
 * WebKit/WKWebView does not consistently expose the browser Fullscreen API.
 * The preview therefore uses this narrowly-scoped Tauri window command as its
 * installed-app fallback. The selected loopback origin receives only the
 * matching set/is permissions in the desktop capability.
 */
export async function setAppWindowFullscreen(fullscreen: boolean): Promise<boolean> {
  const t = tauri()
  if (!t) return false
  try {
    await t.core.invoke('plugin:window|set_fullscreen', {
      label: 'main',
      value: fullscreen,
    })
    return true
  } catch {
    return false
  }
}

/** Read the native window state so an OS-level Esc exit cannot leave the UI lying. */
export async function isAppWindowFullscreen(): Promise<boolean | null> {
  const t = tauri()
  if (!t) return null
  try {
    return await t.core.invoke<boolean>('plugin:window|is_fullscreen', { label: 'main' })
  } catch {
    return null
  }
}

function mediaPickerFilters() {
  return [
    {
      name: 'Media (video, audio, image)',
      extensions: [
        'mp4', 'mov', 'mkv', 'webm', 'avi', 'm4v', 'm2ts', 'mts',
        'mp3', 'wav', 'm4a', 'aac', 'flac', 'ogg', 'opus',
        'png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp', 'tiff',
      ],
    },
  ]
}

/**
 * Open the native OS file picker (multi-select, media filter) and return the
 * chosen absolute paths — empty if cancelled or not in Tauri.
 *
 * Uses the dialog plugin's JS `open()` (command `plugin:dialog|open`) rather
 * than a custom app command: the Cut UI is served from the engine's loopback
 * http origin, where Tauri denies app-command IPC; the dialog plugin permission
 * `dialog:allow-open` CAN be granted to that remote origin (capabilities/
 * engine-remote.json), so this is the path that actually works from the
 * engine-served webview.
 */
export async function pickMedia(): Promise<string[]> {
  if (!isTauri()) return []
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const sel = await open({
      multiple: true,
      title: 'Import media — ShellX Cut',
      filters: mediaPickerFilters(),
    })
    if (!sel) return []
    const arr = Array.isArray(sel) ? sel : [sel]
    // Normalize: open() yields path strings (older builds yield {path}).
    return arr.map((s) => (typeof s === 'string' ? s : (s as { path?: string })?.path ?? '')).filter(Boolean)
  } catch {
    return []
  }
}

/** Pick one replacement path for a missing linked Library item. */
export async function pickLibraryRelinkMedia(): Promise<string | null> {
  if (!isTauri()) return null
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const sel = await open({
      multiple: false,
      title: 'Relink missing Library media — ShellX Cut',
      filters: mediaPickerFilters(),
    })
    if (!sel) return null
    return typeof sel === 'string' ? sel : (sel as { path?: string })?.path ?? null
  } catch {
    return null
  }
}

/**
 * Open the native OS folder picker and return the chosen absolute directory —
 * null if cancelled or not in Tauri. Used by the export menu's "Choose folder…"
 * (the destination for renders/exports). Same dialog-plugin path as pickMedia
 * (`dialog:allow-open` works from the engine-served remote origin); `directory:
 * true` makes it a folder picker.
 */
export async function pickFolder(): Promise<string | null> {
  if (!isTauri()) return null
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const sel = await open({ directory: true, multiple: false, title: 'Choose export folder — ShellX Cut' })
    if (!sel) return null
    return typeof sel === 'string' ? sel : (sel as { path?: string })?.path ?? null
  } catch {
    return null
  }
}

/** Pick the root of an existing local ShellX Motion package for clip relinking. */
export async function pickMotionPackage(): Promise<string | null> {
  if (!isTauri()) return null
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const sel = await open({ directory: true, multiple: false, title: 'Relink ShellX Motion package' })
    if (!sel) return null
    return typeof sel === 'string' ? sel : (sel as { path?: string })?.path ?? null
  } catch {
    return null
  }
}

/**
 * Open the native OS save dialog for any export/render action and return the
 * chosen absolute output file path. The caller supplies a friendly default name
 * and file filters; browser/remote builds get null and can fall back to the
 * configured default export folder.
 */
export async function pickExportOutput(opts?: {
  title?: string
  defaultPath?: string
  filters?: ReadonlyArray<{ name: string; extensions: readonly string[] }>
}): Promise<string | null> {
  if (!isTauri()) return null
  try {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const sel = await save({
      title: opts?.title ?? 'Choose output file — ShellX Cut',
      defaultPath: opts?.defaultPath ?? 'ShellX Cut export.mp4',
      filters: opts?.filters?.map((f) => ({ name: f.name, extensions: [...f.extensions] })) ?? [
        { name: 'Video render', extensions: ['mp4', 'mov', 'mkv', 'webm'] },
        { name: 'Audio render', extensions: ['wav', 'mp3', 'm4a'] },
        { name: 'Captions and sidecars', extensions: ['srt', 'vtt', 'ass', 'txt', 'md', 'xml', 'fcpxml', 'otio', 'edl'] },
      ],
    })
    if (!sel) return null
    return typeof sel === 'string' ? sel : (sel as { path?: string })?.path ?? null
  } catch {
    return null
  }
}

/**
 * Open the native OS save dialog for a single render-queue delivery and return
 * the chosen absolute output file path. The manual text field remains the
 * fallback for browser/remote use and for engines on an unmatched ephemeral
 * loopback port.
 */
export async function pickRenderOutput(): Promise<string | null> {
  return pickExportOutput({
    title: 'Choose output file — ShellX Cut',
    defaultPath: 'ShellX Cut render.mp4',
    filters: [
      { name: 'Video render', extensions: ['mp4', 'mov', 'mkv', 'webm'] },
      { name: 'Audio render', extensions: ['wav', 'mp3', 'm4a'] },
    ],
  })
}

/**
 * Open the native OS file picker filtered to OpenTimelineIO (.otio) files and
 * return the chosen absolute path — null if cancelled or not in Tauri. Used by
 * the topbar's "Import timeline (OTIO)…" entry, the human counterpart to
 * export.otio. Same dialog-plugin path as pickMedia (`dialog:allow-open` works
 * from the engine-served remote origin); single-select, .otio filter. The
 * engine's import.otio reads the path server-side, exactly like media.import.
 */
export async function pickOtio(): Promise<string | null> {
  if (!isTauri()) return null
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const sel = await open({
      multiple: false,
      title: 'Import timeline (OpenTimelineIO) — ShellX Cut',
      filters: [{ name: 'OpenTimelineIO (.otio)', extensions: ['otio'] }],
    })
    if (!sel) return null
    return typeof sel === 'string' ? sel : (sel as { path?: string })?.path ?? null
  } catch {
    return null
  }
}

/**
 * Pick an existing SUBTITLE file (SRT / WebVTT / ASS/SSA) to import as caption
 * clips — the human counterpart to captions.import (the inverse of export.srt/
 * export.vtt/export.ass, so subtitles round-trip). Mirrors pickOtio: same
 * dialog-plugin path (works from the engine-served remote origin), single-select.
 * The engine's captions.import reads the path server-side, exactly like
 * media.import. Returns the chosen absolute path, or null (cancelled / not Tauri).
 */
export async function pickSubtitle(): Promise<string | null> {
  if (!isTauri()) return null
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const sel = await open({
      multiple: false,
      title: 'Import captions (SRT / VTT / ASS) — ShellX Cut',
      filters: [{ name: 'Subtitles (.srt .vtt .ass .ssa)', extensions: ['srt', 'vtt', 'ass', 'ssa'] }],
    })
    if (!sel) return null
    return typeof sel === 'string' ? sel : (sel as { path?: string })?.path ?? null
  } catch {
    return null
  }
}

/** Pick feedback JSON downloaded from a ShellX Cut offline review package. */
export async function pickReviewFeedback(): Promise<string | null> {
  if (!isTauri()) return null
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const sel = await open({
      multiple: false,
      title: 'Import review feedback — ShellX Cut',
      filters: [{ name: 'ShellX Cut review feedback (.json)', extensions: ['json'] }],
    })
    if (!sel) return null
    return typeof sel === 'string' ? sel : (sel as { path?: string })?.path ?? null
  } catch {
    return null
  }
}

/**
 * Pick a 3D LUT file (.cube) for the color grade — the human counterpart to the
 * edit.grade `lut` param (the engine fences it: must end .cube + exist). Mirrors
 * pickOtio/pickSubtitle: same dialog-plugin path (works from the engine-served
 * remote origin), single-select, .cube filter. The engine reads the path
 * server-side, exactly like media.import. Returns the chosen absolute path, or
 * null (cancelled / not in Tauri — the manual path input stays the fallback).
 */
export async function pickCube(): Promise<string | null> {
  if (!isTauri()) return null
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const sel = await open({
      multiple: false,
      title: 'Choose a LUT (.cube) — ShellX Cut',
      filters: [{ name: '3D LUT (.cube)', extensions: ['cube'] }],
    })
    if (!sel) return null
    return typeof sel === 'string' ? sel : (sel as { path?: string })?.path ?? null
  } catch {
    return null
  }
}

/**
 * Pick the ffmpeg EXECUTABLE for the manual override (Settings>Environment
 * "Change ffmpeg"). NO extension filter — on macOS/Linux the binary is just
 * "ffmpeg" with no extension (a filter would hide it); the engine
 * (system.set_ffmpeg) validates the pick actually runs `ffmpeg -version`.
 * Returns the chosen absolute path, or null (cancelled / not in Tauri).
 */
export async function pickFfmpeg(): Promise<string | null> {
  if (!isTauri()) return null
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const sel = await open({ multiple: false, title: 'Choose the ffmpeg executable — ShellX Cut' })
    if (!sel) return null
    return typeof sel === 'string' ? sel : (sel as { path?: string })?.path ?? null
  } catch {
    return null
  }
}

/**
 * Subscribe to OS file drops on the window (Tauri's built-in drag-drop events).
 * `onPaths` receives the dropped absolute paths; `onOver`/`onLeave` drive the
 * drop hint. Returns an unsubscribe fn (no-op outside Tauri).
 *
 * Tauri 2's official `getCurrentWebview().onDragDropEvent` adapter targets the
 * current Webview and normalizes native enter/over/drop/leave payloads.
 */
export function onFileDrop(handlers: {
  onPaths: (paths: string[]) => void
  onOver?: () => void
  onLeave?: () => void
}): () => void {
  if (!isTauri()) return () => {}
  let off: (() => void) | null = null
  let cancelled = false
  void import('@tauri-apps/api/webview')
    .then(({ getCurrentWebview }) => getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'enter' || event.payload.type === 'over') {
        handlers.onOver?.()
      } else if (event.payload.type === 'leave') {
        handlers.onLeave?.()
      } else {
        handlers.onLeave?.()
        if (event.payload.paths.length) handlers.onPaths(event.payload.paths)
      }
    }))
    .then((unlisten) => {
      if (cancelled) {
        unlisten()
      } else {
        off = unlisten
      }
    })
    .catch(() => {
      // A browser origin or denied native event capability keeps the ordinary
      // import button available; never leave an unhandled initialization error.
    })
  return () => {
    cancelled = true
    if (off) {
      off()
      off = null
    }
  }
}

/**
 * Subscribe to the GLOBAL record hotkey (OS-level F9). The desktop shell
 * (lib.rs) registers F9 as a global OS shortcut — it fires even while ANOTHER
 * app is focused, which is essential for a screen recorder (the Cut window is
 * backgrounded the whole time it records other apps, so a focused-window
 * keydown can never reliably STOP a capture). The Rust handler emits the
 * `cut:record-hotkey` Tauri event; this helper delivers it to the Record panel,
 * which toggles start⇄stop.
 *
 * Same `window.__TAURI__.event.listen` path as onFileDrop (withGlobalTauri ⇒ no
 * @tauri-apps/api bundle needed; the engine-served remote origin is granted
 * core:event:allow-listen). Returns an unsubscribe fn; a NO-OP outside Tauri
 * (the plain web/dev build has no shell, so there is no global hotkey there —
 * the Record panel keeps its in-page F9 keydown fallback for that case).
 */
export function onRecordHotkey(cb: () => void): () => void {
  const t = tauri()
  if (!t) return () => {}
  let off: (() => void) | null = null
  let cancelled = false
  t.event
    .listen('cut:record-hotkey', () => cb())
    .then((unlisten) => {
      // Unsubscribed before the listener registered ⇒ drop it immediately
      // (mirrors onFileDrop — avoids a leaked listener across re-subscribes).
      if (cancelled) unlisten()
      else off = unlisten
    })
    .catch(() => {})
  return () => {
    cancelled = true
    off?.()
  }
}
