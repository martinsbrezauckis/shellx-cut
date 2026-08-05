#!/usr/bin/env node
import { execFileSync } from 'node:child_process'
import { mkdirSync, statSync, unlinkSync, writeFileSync } from 'node:fs'
import { dirname, extname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

function arg(name, fallback = '') {
  const index = process.argv.indexOf(name)
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback
}
const sleep = (ms) => new Promise((resolveSleep) => setTimeout(resolveSleep, ms))
const platform = arg('--platform', process.platform)
const actionId = arg('--action')
const mode = arg('--mode', 'cancel')
const selectedPath = arg('--path')
const windowsExpectedProcess = process.env.FCV_NATIVE_EXPECTED_PROCESS || 'shellx-cut'
const macExpectedProcess = process.env.FCV_NATIVE_EXPECTED_PROCESS || 'shellx-cut'
const osascriptBin = process.env.FCV_OSASCRIPT || '/usr/bin/osascript'
if (!actionId) throw new Error('--action is required')
if (!['accept', 'cancel', 'select'].includes(mode)) throw new Error(`unsupported --mode ${mode}`)
if (mode === 'select' && !selectedPath) throw new Error('--path is required for select mode')

function run(command, args, timeout = 10_000) {
  try {
    return execFileSync(command, args, {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
      timeout,
    }).trim()
  } catch (error) {
    const output = [error?.stdout, error?.stderr]
      .map((value) => String(value || '').trim())
      .filter(Boolean)
      .join(' | ')
    throw new Error(`${error?.message || String(error)}${output ? ` | ${output}` : ''}`)
  }
}

function linuxState() {
  let handles = ''
  try { handles = run('xdotool', ['search', '--onlyvisible', '--name', '.*']) } catch { /* no windows yet */ }
  const windows = handles.split(/\s+/).filter(Boolean).map((handle) => {
    let title = ''
    let className = ''
    try { title = run('xdotool', ['getwindowname', handle]) } catch { /* window closed */ }
    try { className = run('xdotool', ['getwindowclassname', handle]) } catch { /* optional */ }
    return { handle, title, className }
  })
  return { windows }
}

function linuxNewWindows(before, after = linuxState()) {
  const existing = new Set(before.windows.map((window) => window.handle))
  return after.windows.filter((window) => !existing.has(window.handle))
}

function linuxCurrentDialog(before, preferred = null, after = linuxState()) {
  const windows = linuxNewWindows(before, after)
  if (!windows.length) return null
  if (preferred) {
    const exact = windows.find((window) => window.handle === preferred.handle)
    if (exact) return exact
    const sameIdentity = windows.find((window) =>
      window.title === preferred.title && window.className === preferred.className)
    if (sameIdentity) return sameIdentity
  }
  const semanticDialog = windows.find((window) =>
    /file|open|save|select|choose|folder|import|relink/i.test(
      `${window.title} ${window.className}`,
    ))
  // While waiting for a known chooser to disappear, unrelated tooltips and
  // popovers can become new top-level X11 windows. They are not replacements
  // for the preferred chooser unless their identity or dialog semantics match.
  return semanticDialog || (preferred ? null : windows[0])
}

function linuxTypedPath(dialog) {
  const label = `${dialog?.title || ''} ${dialog?.className || ''}`
  const extension = extname(selectedPath)
  const autoExtensionDialog = /^save\b/i.test(dialog?.title || '') ||
    /(?:choose|select)\s+(?:an?\s+)?(?:recording\s+)?output\s+file/i.test(label)
  return autoExtensionDialog && extension
    ? selectedPath.slice(0, -extension.length)
    : selectedPath
}

function linuxDirectoryDialog(dialog) {
  return /folder|directory|package/i.test(
    `${dialog?.title || ''} ${dialog?.className || ''}`,
  )
}

async function linuxFocus(before, preferred) {
  let lastError = null
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const current = linuxCurrentDialog(before, preferred)
    if (!current) return null
    try {
      run('xdotool', ['windowfocus', '--sync', current.handle])
      return current
    } catch (error) {
      lastError = error
      await sleep(80)
    }
  }
  if (!linuxCurrentDialog(before, preferred)) return null
  throw lastError
}

async function linuxKey(before, preferred, key) {
  const current = await linuxFocus(before, preferred)
  if (!current) return null
  try {
    run('xdotool', ['key', '--clearmodifiers', key])
    return current
  } catch (error) {
    // A chooser can disappear between the visibility probe and the key send.
    // That is success for the dismissal step, not a stale-handle failure.
    if (!linuxCurrentDialog(before, current)) return null
    throw error
  }
}

async function linuxDismiss(before, dialog, key, attempts = 3) {
  let current = dialog
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    current = await linuxKey(before, current, key)
    if (!current) return
    await sleep(250)
    current = linuxCurrentDialog(before, current)
    if (!current) return
  }
}

function linuxWindowGeometry(dialog) {
  const values = Object.fromEntries(
    run('xdotool', ['getwindowgeometry', '--shell', dialog.handle])
      .split(/\r?\n/)
      .map((line) => line.split('=', 2))
      .filter(([key, value]) => key && value !== undefined),
  )
  const geometry = {
    x: Number(values.X),
    y: Number(values.Y),
    width: Number(values.WIDTH),
    height: Number(values.HEIGHT),
  }
  if (Object.values(geometry).some((value) => !Number.isFinite(value))) {
    throw new Error(`could not resolve native dialog geometry: ${JSON.stringify(values)}`)
  }
  return geometry
}

function linuxFailureDiagnostics(error) {
  const receipt = String(process.env.FCV_RESULT_RECEIPT || '').trim()
  if (!receipt) return null
  const evidenceDir = dirname(receipt)
  const slug = actionId.replace(/[^a-z0-9_.-]+/gi, '-').replace(/^-+|-+$/g, '') || 'action'
  const jsonPath = join(evidenceDir, `native-action-${slug}-failure.json`)
  const screenshotPath = join(evidenceDir, `native-action-${slug}-failure.png`)
  try {
    mkdirSync(evidenceDir, { recursive: true })
    const snapshot = linuxState()
    const windows = snapshot.windows.map((window) => ({
      ...window,
      geometry: (() => {
        try { return linuxWindowGeometry(window) } catch { return null }
      })(),
    }))
    try { run('import', ['-window', 'root', screenshotPath], 15_000) } catch { /* optional diagnostic */ }
    writeFileSync(jsonPath, `${JSON.stringify({
      schema: 'shellx-cut/native-action-failure/1',
      action_id: actionId,
      mode,
      selected_path: selectedPath || null,
      error: error?.message || String(error),
      windows,
      screenshot: screenshotPath,
    }, null, 2)}\n`)
    return { json: jsonPath, screenshot: screenshotPath }
  } catch {
    return null
  }
}

const linuxAtspiDirectoryScript = String.raw`
import sys
import os
import subprocess
import time
import gi
gi.require_version('Atspi', '2.0')
gi.require_version('Gtk', '3.0')
from pathlib import Path
from gi.repository import Atspi, GLib, Gtk

mode, dialog_title, selected_path = sys.argv[1], sys.argv[2], sys.argv[3]
recent_seed_path = sys.argv[4] if len(sys.argv) > 4 else selected_path
dialog_handle = sys.argv[5] if len(sys.argv) > 5 else ''

def settle_recent_manager():
    # RecentManager persists through an asynchronous GIO write. This helper is
    # intentionally short-lived, so pump its GLib context before exiting or
    # the URI never reaches recently-used.xbel for the chooser to consume.
    context = GLib.MainContext.default()
    deadline = time.monotonic() + 1.0
    while time.monotonic() < deadline:
        while context.pending():
            context.iteration(False)
        time.sleep(0.02)

def descendants(node):
    yield node
    for index in range(node.get_child_count()):
        child = node.get_child_at_index(index)
        if child:
            yield from descendants(child)

desktop = Atspi.get_desktop(0)
nodes = list(descendants(desktop))
dialog_candidates = [node for node in nodes
                     if node.get_name() == dialog_title
                     and node.get_role_name() in ('dialog', 'file chooser', 'frame', 'window')]
dialog = max(dialog_candidates, key=lambda node: node.get_child_count(), default=None)
if dialog is None:
    dialog = next((node for node in nodes
                   if node.get_role_name() == 'application'
                   and 'zenity' in node.get_name().lower()), None)
scope = list(descendants(dialog)) if dialog is not None else []
if mode == 'register':
    uri = Path(recent_seed_path).resolve().as_uri()
    if not Gtk.RecentManager.get_default().add_item(uri):
        raise SystemExit('GTK could not register the exact folder in Recent')
    settle_recent_manager()
    sidebar = next((node for node in scope
                    if node.get_role_name() == 'list'
                    and 'Selection' in node.get_interfaces()
                    and node.get_child_count() > 1
                    and node.get_child_at_index(0).get_role_name() == 'list item'
                    and node.get_child_at_index(1).get_role_name() == 'list item'), None)
    if sidebar is not None:
        # A mapped portal chooser caches its current Recent rows. Select Home
        # and then Recent through the real sidebar to force an exact reload.
        sidebar.select_child(1)
        time.sleep(0.25)
        sidebar.select_child(0)
        recent = sidebar.get_child_at_index(0)
        if recent.get_n_actions() > 0:
            recent.do_action(0)
    raise SystemExit(0)
if mode == 'unregister':
    uri = Path(recent_seed_path).resolve().as_uri()
    try:
        Gtk.RecentManager.get_default().remove_item(uri)
        settle_recent_manager()
    except Exception:
        # GTK may consume the transient directory entry when Open succeeds.
        # Cleanup is intentionally idempotent for that already-absent case.
        pass
    raise SystemExit(0)
basename = os.path.basename(selected_path)
selectable_roles = ('table cell', 'table row', 'list item', 'icon')
candidates = [node for node in scope
              if node.get_role_name() in selectable_roles
              and (node.get_name() == basename
                   or selected_path in node.get_name()
                   or node.get_name().startswith(basename + ','))]

def selection_owner(node):
    child = node
    parent = child.get_parent()
    while parent is not None:
        if 'Selection' in parent.get_interfaces():
            return parent, child
        child = parent
        parent = child.get_parent()
    return None, None

choice = None
selection = None
selection_child = None
for candidate in candidates:
    owner, child = selection_owner(candidate)
    if owner is not None:
        choice, selection, selection_child = candidate, owner, child
        break
if choice is None:
    rows = [(node.get_role_name(), node.get_name()) for node in scope
            if node.get_role_name() in selectable_roles]
    raise SystemExit('AT-SPI could not find the exact GTK folder row; rows=' + repr(rows[:100]))
button = next((node for node in scope
               if node.get_role_name() == 'push button'
               and node.get_name().strip().lower() in ('open', 'select', 'choose', 'ok')), None)
selected = selection.select_child(selection_child.get_index_in_parent())
if selected:
    time.sleep(0.1)
if not selected and not selection_child.get_state_set().contains(Atspi.StateType.SELECTED):
    # The XDG portal can expose a valid Selection interface yet reject its
    # programmatic select_child call. Click only the exact full-path row that
    # AT-SPI resolved, using that row's live window rectangle, then require its
    # selected state before the affirmative button is allowed to run.
    bounds = choice.get_extents(Atspi.CoordType.WINDOW)
    if not dialog_handle or bounds.width <= 2 or bounds.height <= 2:
        raise SystemExit('AT-SPI exact GTK folder row has no usable window rectangle')
    subprocess.run(['xdotool', 'mousemove', '--window', dialog_handle, '--sync',
                    str(bounds.x + bounds.width // 2),
                    str(bounds.y + bounds.height // 2), 'click', '1'],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    time.sleep(0.3)
if not selection_child.get_state_set().contains(Atspi.StateType.SELECTED):
    raise SystemExit('AT-SPI could not select the exact GTK folder row')
if button is None or button.get_n_actions() < 1 or not button.do_action(0):
    raise SystemExit('AT-SPI could not actuate the GTK folder action')
`

let linuxRegisteredDirectory = false
let linuxRecentSeedPath = ''
let linuxCreatedRecentSeed = false
function linuxSelectedPathIsDirectory() {
  try { return statSync(selectedPath).isDirectory() } catch { return false }
}
function linuxPrepareRecentSeed() {
  if (linuxRecentSeedPath) return
  const projectManifest = join(selectedPath, 'project.json')
  try {
    if (statSync(projectManifest).isFile()) {
      linuxRecentSeedPath = projectManifest
      return
    }
  } catch { /* an ordinary output folder has no project manifest */ }
  // A folder-only GTK chooser derives its Recent rows from the containing
  // folder of a recent item. Use a unique hidden fixture, then remove it in the
  // same controller process, so an empty exact output folder is addressable
  // without relying on its parent row or altering a user's existing history.
  linuxRecentSeedPath = join(selectedPath, `.shellx-cut-native-choice-${process.pid}`)
  writeFileSync(linuxRecentSeedPath, '', { flag: 'wx', mode: 0o600 })
  linuxCreatedRecentSeed = true
}
function linuxRegisterDirectory(dialogTitle = '') {
  if (linuxRegisteredDirectory) return
  linuxPrepareRecentSeed()
  run('python3', [
    '-c', linuxAtspiDirectoryScript, 'register', dialogTitle, selectedPath, linuxRecentSeedPath, '',
  ])
  linuxRegisteredDirectory = true
}
function linuxUnregisterDirectory() {
  if (!linuxRegisteredDirectory) return
  try {
    run('python3', [
      '-c', linuxAtspiDirectoryScript, 'unregister', '', selectedPath, linuxRecentSeedPath, '',
    ])
  } catch {
    // The exact transient URI may already have been consumed by GTK.
  } finally {
    linuxRegisteredDirectory = false
  }
}
function linuxRemoveRecentSeed() {
  if (!linuxCreatedRecentSeed || !linuxRecentSeedPath) return
  try { unlinkSync(linuxRecentSeedPath) } catch { /* best-effort test-fixture cleanup */ }
  linuxCreatedRecentSeed = false
}

async function linuxSelectDirectory(before, dialog) {
  let current = await linuxFocus(before, dialog)
  if (!current) return
  // Portal-backed GTK choosers do not commit programmatic Ctrl+L edits even
  // when their accessibility activation reports success. Register the exact
  // fixture in the run-isolated Recent model, then select its full-path row
  // through GTK's Selection interface and invoke the real Open/Select button.
  // This avoids keyboard-layout, row-order, search, and pointer assumptions.
  linuxRegisterDirectory(current.title)
  await sleep(1_500)
  current = linuxCurrentDialog(before, current)
  if (!current) return
  run('python3', [
    '-c', linuxAtspiDirectoryScript, 'select', current.title, selectedPath,
    linuxRecentSeedPath, current.handle,
  ])
  await sleep(500)
}

async function linuxAct(before, dialog) {
  // GTK file choosers ignore synthetic key events sent to the top-level X11
  // window when an internal child owns keyboard focus. Move X input focus to
  // the chooser, then let GTK route ordinary key events to its focused child.
  // This also avoids leaving a blank chooser behind after Ctrl+L appeared to
  // succeed only at the XSendEvent layer.
  if (mode === 'cancel') return linuxDismiss(before, dialog, 'Escape')
  if (mode === 'accept') return linuxDismiss(before, dialog, 'Return')
  let current = await linuxFocus(before, dialog)
  if (!current) return
  if (linuxDirectoryDialog(current)) return linuxSelectDirectory(before, current)
  run('xdotool', ['key', '--clearmodifiers', 'ctrl+l'])
  await sleep(120)
  run('xdotool', ['type', '--clearmodifiers', '--delay', '1', linuxTypedPath(current)])
  run('xdotool', ['key', '--clearmodifiers', 'Return'])
  await sleep(250)
  current = linuxCurrentDialog(before, current)
  if (current) {
    await linuxDismiss(before, current, 'Return')
  }
}

const macStateScript = `
tell application "System Events"
  set frontProcess to first application process whose frontmost is true
  set foregroundName to name of frontProcess
  set targetProcesses to application processes whose name is ${JSON.stringify(macExpectedProcess)}
  if (count of targetProcesses) is 0 then return foregroundName & "|missing|0|0"
  set targetProcess to first item of targetProcesses
  set processName to name of targetProcess
  set windowCount to count of windows of targetProcess
  set sheetCount to 0
  repeat with currentWindow in windows of targetProcess
    try
      set sheetCount to sheetCount + (count of sheets of currentWindow)
    end try
  end repeat
  return foregroundName & "|" & processName & "|" & windowCount & "|" & sheetCount
end tell`
function macState() {
  const [foregroundProcessName, processName, windows, sheets] = run(osascriptBin, ['-e', macStateScript]).split('|')
  return { foregroundProcessName, processName, windows: Number(windows), sheets: Number(sheets) }
}
function isExpectedMacProcess(state) {
  return String(state?.foregroundProcessName || '').toLowerCase() === macExpectedProcess.toLowerCase()
    && String(state?.processName || '').toLowerCase() === macExpectedProcess.toLowerCase()
}
function macFocusExpected() {
  run(osascriptBin, ['-e', `
tell application "System Events"
  set targetProcesses to application processes whose name is ${JSON.stringify(macExpectedProcess)}
  if (count of targetProcesses) is 0 then error "ShellX Cut process is not running"
  set targetProcess to first item of targetProcesses
  set frontmost of targetProcess to true
end tell`])
}
async function macRequireExpectedFocus() {
  macFocusExpected()
  const deadline = Date.now() + 3_000
  let current = macState()
  while (!isExpectedMacProcess(current) && Date.now() < deadline) {
    await sleep(100)
    current = macState()
  }
  if (!isExpectedMacProcess(current)) {
    throw new Error(
      `ShellX Cut did not gain macOS foreground focus ` +
      `(foreground=${current.foregroundProcessName || 'unknown'}, target=${current.processName || 'missing'})`,
    )
  }
  return current
}
function macSelectedPathIsDirectory() {
  if (mode !== 'select') return false
  try { return statSync(selectedPath).isDirectory() } catch { return false }
}
function macAct(before, dialog) {
  const selection = mode === 'select'
    ? `
    keystroke "g" using {command down, shift down}
    delay 0.2
    keystroke ${JSON.stringify(selectedPath)}
    key code 36
    delay 0.4
    key code 36`
    : mode === 'accept' ? 'key code 36' : 'key code 53'
  run(osascriptBin, ['-e', `
tell application "System Events"
  tell first application process whose name is ${JSON.stringify(macExpectedProcess)}
    ${selection}
  end tell
end tell`])
  // In a macOS directory chooser, the first Return accepts the Go to Folder
  // sheet and the second navigates into that directory. If the chooser is
  // still present, one more Return confirms the current folder. Ordinary file
  // choosers and directory choosers that already dismissed are untouched.
  if (macSelectedPathIsDirectory() && findDialog(before, macState(), dialog)) {
    run(osascriptBin, ['-e', `
tell application "System Events"
  tell first application process whose name is ${JSON.stringify(macExpectedProcess)}
    key code 36
  end tell
end tell`])
  }
}

const windowsScriptPosix = resolve(dirname(fileURLToPath(import.meta.url)), 'native-os-action-windows.ps1')
const windowsScript = (platform === 'windows' || platform === 'win32') && process.platform === 'linux'
  ? run('wslpath', ['-w', windowsScriptPosix])
  : windowsScriptPosix
function windowsState() {
  const args = [
    '-NoProfile', '-NonInteractive', '-STA', '-ExecutionPolicy', 'Bypass',
    '-File', windowsScript, '-Command', 'state',
  ]
  let firstError = null
  for (let attempt = 0; attempt < 2; attempt += 1) {
    try { return JSON.parse(run('powershell.exe', args, 20_000)) } catch (error) {
      firstError ||= error
      if (!/ETIMEDOUT/.test(String(error?.message || error))) throw error
    }
  }
  throw firstError
}
function windowsDialogStillExists(state) {
  const current = windowsState()
  const windows = Array.isArray(current.windows) ? current.windows : [current]
  return windows.some((window) => String(window.handle) === String(state.handle))
}
function windowsActOnce(state) {
  return run('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-STA', '-ExecutionPolicy', 'Bypass',
    '-File', windowsScript, '-Command', 'act', '-Handle', String(state.handle),
    '-Mode', mode, '-Path', selectedPath, '-ExpectedProcessName', windowsExpectedProcess,
  ], 20_000)
}
function windowsAct(state) {
  try { return windowsActOnce(state) } catch (error) {
    if (!/ETIMEDOUT/.test(String(error?.message || error))) throw error
    // The helper can time out after PostMessage has already dismissed the
    // TaskDialog. Treat that as success only when the exact observed handle is
    // gone. If it remains, one bounded retry is safe and still targets the same
    // process/handle through the PowerShell guard.
    if (!windowsDialogStillExists(state)) return ''
    try { return windowsActOnce(state) } catch (retryError) {
      if (/ETIMEDOUT/.test(String(retryError?.message || retryError)) &&
          !windowsDialogStillExists(state)) return ''
      throw retryError
    }
  }
}
function windowsFocus(state) {
  return run('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-STA', '-ExecutionPolicy', 'Bypass',
    '-File', windowsScript, '-Command', 'focus', '-Handle', String(state.handle),
    '-ExpectedProcessName', windowsExpectedProcess,
  ], 10_000)
}

function isExpectedWindowsProcess(window) {
  return String(window?.processName || '').toLowerCase() === windowsExpectedProcess.toLowerCase()
}
function findDialog(before, after, preferred = null) {
  if (platform === 'linux') {
    // The Linux Tauri dialog backend gives confirmation and some import
    // choosers only the application title. Qualification runs in an isolated
    // Xvfb desktop, so any newly visible top-level window is the dialog while
    // the pre-existing application window remains excluded.
    return linuxCurrentDialog(before, preferred, after)
  }
  if (platform === 'darwin' || platform === 'macos') {
    return after.sheets > before.sheets || after.windows > before.windows ? after : null
  }
  const beforeWindows = Array.isArray(before.windows) ? before.windows : [before]
  const afterWindows = Array.isArray(after.windows) ? after.windows : [after]
  const existing = new Set(beforeWindows.map((window) => String(window.handle)))
  const newDialog = afterWindows.find((window) =>
    !existing.has(String(window.handle)) &&
    isExpectedWindowsProcess(window) &&
    (window.className === '#32770' || /open|save|select|choose|folder|import|relink/i.test(window.title)),
  )
  if (newDialog) return newDialog
  return after.handle !== before.handle &&
    isExpectedWindowsProcess(after) &&
    (after.className === '#32770' || /open|save|select|choose|folder|import|relink/i.test(after.title)) ? after : null
}
function state() {
  if (platform === 'linux') return linuxState()
  if (platform === 'darwin' || platform === 'macos') return macState()
  if (platform === 'win32' || platform === 'windows') return windowsState()
  throw new Error(`unsupported platform: ${platform}`)
}
async function act(before, dialog) {
  if (platform === 'linux') return linuxAct(before, dialog)
  if (platform === 'darwin' || platform === 'macos') return macAct(before, dialog)
  return windowsAct(dialog)
}

try {
  // A separate RecentManager cannot reliably refresh a portal chooser that is
  // already mapped. Pre-register existing directory fixtures before the UI
  // trigger creates the chooser; ordinary file selections stay untouched.
  if (platform === 'linux' && mode === 'select' && linuxSelectedPathIsDirectory()) {
    linuxRegisterDirectory()
  }
  const before = (platform === 'darwin' || platform === 'macos')
    ? await macRequireExpectedFocus()
    : state()
  if ((platform === 'win32' || platform === 'windows') && !isExpectedWindowsProcess(before)) {
    throw new Error(
      `refusing native action ${actionId}: foreground process is ${before.processName || 'unknown'}, expected ${windowsExpectedProcess}`,
    )
  }
  process.stdout.write(`${JSON.stringify({ phase: 'ready', actionId, platform, before })}\n`)
  const deadline = Date.now() + Number(process.env.FCV_NATIVE_ACTION_TIMEOUT_MS || 20_000)
  let dialog = null
  while (Date.now() < deadline) {
    await sleep(100)
    dialog = findDialog(before, state())
    if (dialog) break
  }
  if (!dialog) throw new Error(`no native dialog appeared for ${actionId}`)
  await act(before, dialog)
  const dismissalDeadline = Date.now() + Math.min(
    15_000,
    Number(process.env.FCV_NATIVE_ACTION_TIMEOUT_MS || 20_000),
  )
  let after = state()
  let dismissed = !findDialog(before, after, dialog)
  while (!dismissed && Date.now() < dismissalDeadline) {
    await sleep(150)
    after = state()
    dismissed = !findDialog(before, after, dialog)
  }
  if (!dismissed) {
    const geometry = platform === 'linux' && dialog
      ? (() => {
          try { return linuxWindowGeometry(dialog) } catch { return null }
        })()
      : null
    throw new Error(
      `native dialog remained open after ${mode}: ` +
      `${JSON.stringify({
        selected: dialog?.title || dialog?.className || dialog?.handle || 'unknown',
        foreground: after?.title || after?.className || after?.handle || 'unknown',
        geometry,
      })}`,
    )
  }
  if (platform === 'win32' || platform === 'windows') {
    // A closed Common Item Dialog can leave the system PickerHost foreground
    // even though the exact ShellX Cut dialog is gone. Restore the exact app
    // window that owned this action before reporting success; otherwise the
    // next controller must safely refuse and a valid picker cascade fails.
    if (!isExpectedWindowsProcess(after)) {
      windowsFocus(before)
      const focusDeadline = Date.now() + 3_000
      do {
        await sleep(100)
        after = state()
      } while (!isExpectedWindowsProcess(after) && Date.now() < focusDeadline)
    }
    if (!isExpectedWindowsProcess(after)) {
      throw new Error(
        `native dialog closed but ShellX Cut did not regain foreground focus ` +
        `(foreground=${after?.processName || 'unknown'})`,
      )
    }
  }
  if (platform === 'darwin' || platform === 'macos') {
    after = await macRequireExpectedFocus()
  }
  process.stdout.write(`${JSON.stringify({
    phase: 'done',
    ok: true,
    actionId,
    evidence: `${platform} dialog opened (${dialog.title || dialog.className || 'native'}; process ${dialog.processName || 'unknown'}) and ${mode === 'select' ? 'selected the supplied path' : mode === 'accept' ? 'accepted the action' : 'returned via cancel'}`,
  })}\n`)
} catch (error) {
  const diagnostics = platform === 'linux' ? linuxFailureDiagnostics(error) : null
  process.stdout.write(`${JSON.stringify({
    phase: 'done',
    ok: false,
    actionId,
    error: error.message || String(error),
    diagnostics,
  })}\n`)
  process.exitCode = 1
} finally {
  linuxUnregisterDirectory()
  linuxRemoveRecentSeed()
}
