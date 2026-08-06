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

// STRUCTURE (measured live on macOS 26.5.2, 2026-08-06, AppKit panel probe):
//   window 1                      AXWindow   — the application window
//     sheet 1                     AXSheet    — the NSOpen/NSSavePanel itself
//       sheet 1                   AXSheet    — the "Go to the folder:" sheet
//         text field 1                       — the path field (prefilled with
//                                              the LAST value, globally
//                                              persisted by macOS, preselected)
//       splitter group 1
//         text field n                       — the save panel's file-name field
// A panel presented WITHOUT a parent window is an AXWindow instead of an
// AXSheet; the same expressions then apply to that window, so the query below
// resolves the panel either way and never assumes the sheet form.
const macPanelQueryScript = `
tell application "System Events"
  set targetProcesses to application processes whose name is ${JSON.stringify(macExpectedProcess)}
  if (count of targetProcesses) is 0 then return "missing"
  set targetProcess to first item of targetProcesses
  set windowCount to count of windows of targetProcess
  set panel to missing value
  repeat with currentWindow in windows of targetProcess
    try
      if (count of sheets of currentWindow) > 0 then
        set panel to sheet 1 of currentWindow
        exit repeat
      end if
    end try
  end repeat
  if panel is missing value and windowCount > 0 then set panel to window 1 of targetProcess
  if panel is missing value then return "none" & tab & "0" & tab & "false" & tab & ""
  set goToOpen to false
  set goToValue to ""
  try
    if (count of sheets of panel) > 0 then
      set goToOpen to true
      try
        set goToValue to (value of text field 1 of sheet 1 of panel) as text
      end try
    end if
  end try
  set fieldValues to ""
  try
    repeat with currentGroup in splitter groups of panel
      repeat with currentField in text fields of currentGroup
        try
          set fieldValues to fieldValues & tab & (value of currentField as text)
        end try
      end repeat
    end repeat
  end try
  return "panel" & tab & (windowCount as text) & tab & (goToOpen as text) & tab & goToValue & fieldValues
end tell`
function macPanelState() {
  const parts = run(osascriptBin, ['-e', macPanelQueryScript]).split('\t')
  return {
    kind: parts[0] || 'none',
    windows: Number(parts[1] || 0),
    goToOpen: parts[2] === 'true',
    goToValue: parts[3] || '',
    fieldValues: parts.slice(4).filter(Boolean),
  }
}
function macKeys(body) {
  run(osascriptBin, ['-e', `
tell application "System Events"
  tell first application process whose name is ${JSON.stringify(macExpectedProcess)}
    ${body}
  end tell
end tell`])
}
async function macWaitFor(predicate, timeoutMs, intervalMs = 150) {
  const deadline = Date.now() + timeoutMs
  for (;;) {
    const current = macPanelState()
    if (predicate(current)) return current
    if (Date.now() >= deadline) return null
    await sleep(intervalMs)
  }
}
// The Go to Folder field autocompletes, so an accepted value may carry a
// trailing separator. Anything else is a different destination and must fail.
function macGoToValueMatches(value) {
  const strip = (text) => String(text || '').replace(/\/+$/, '')
  return strip(value) === strip(selectedPath)
}

// Path selection is driven through the Go to Folder sheet, and every stage is
// PROVEN before the next keystroke leaves. Typing blind is what produced the
// 2026-08-06 `export-save-as-video` failure: the sheet never opened, the path
// went into the panel's FILENAME field (macOS renders each '/' as ':' there and
// re-appends the extension), and `render.final` correctly refused
// "var:folders:…:save-as-3-video.mp4.mp4". A corrupted input that reads as a
// product defect is the worst possible outcome, so each step here either
// observes the state it needs or fails loudly naming the stage that broke.
const MAC_GO_TO_ATTEMPTS = 3
async function macSelect(before, dialog) {
  // A panel that already exists in the accessibility tree is not necessarily
  // key: its content is still being built, and a Cmd-Shift-G that lands first
  // is silently swallowed. Wait (bounded, non-fatal) for real panel content.
  await macWaitFor((state) => state.fieldValues.length > 0, 3_000)
  let opened = null
  for (let attempt = 1; attempt <= MAC_GO_TO_ATTEMPTS && !opened; attempt += 1) {
    macKeys('keystroke "g" using {command down, shift down}')
    // Re-sending Cmd-Shift-G is safe in a way that re-typing a path is not:
    // nothing has been typed yet, and a sheet that did open makes the retry a
    // no-op. This recovers the dropped-keystroke case without risking the
    // filename-field corruption that a blind act-level retry caused.
    opened = await macWaitFor((state) => state.goToOpen, 2_500)
  }
  if (!opened) {
    throw new Error(
      `Go to Folder sheet did not open for ${actionId} after ${MAC_GO_TO_ATTEMPTS} attempts; ` +
      `refusing to type the path into the panel blind: ${JSON.stringify(macPanelState())}`,
    )
  }
  // macOS restores the previous Go to Folder value and preselects it. Select
  // All first so a restored value can never be prefixed onto the typed path.
  macKeys(`keystroke "a" using {command down}
    keystroke ${JSON.stringify(selectedPath)}`)
  const typed = await macWaitFor(
    (state) => state.goToOpen && macGoToValueMatches(state.goToValue),
    4_000,
  )
  if (!typed) {
    const observed = macPanelState()
    throw new Error(
      `Go to Folder field did not receive the exact path for ${actionId}: ` +
      `field=${JSON.stringify(observed.goToValue)} expected=${JSON.stringify(selectedPath)} ` +
      `state=${JSON.stringify(observed)}`,
    )
  }
  macKeys('key code 36')
  const committed = await macWaitFor((state) => !state.goToOpen, 5_000)
  if (!committed) {
    throw new Error(
      `Go to Folder sheet stayed open after commit for ${actionId}: ` +
      `${JSON.stringify(macPanelState())}`,
    )
  }
  // A ':' in the panel's own filename field is the exact signature of a path
  // typed into it instead of the Go to Folder sheet. Catch it HERE, where it is
  // attributable to the harness, rather than letting the verb reject it and
  // read as a product defect.
  const corrupted = committed.fieldValues.find((value) => value.includes(':'))
  if (corrupted) {
    throw new Error(
      `panel file-name field holds a path-corrupted value for ${actionId}: ${JSON.stringify(corrupted)}`,
    )
  }
  macKeys('key code 36')
  // In a macOS directory chooser, the first Return accepts the Go to Folder
  // sheet and the second navigates into that directory. If the chooser is
  // still present, one more Return confirms the current folder. Ordinary file
  // choosers and directory choosers that already dismissed are untouched.
  if (macSelectedPathIsDirectory() && findDialog(before, macState(), dialog)) {
    macKeys('key code 36')
  }
}
async function macAct(before, dialog) {
  if (mode === 'select') return macSelect(before, dialog)
  macKeys(mode === 'accept' ? 'key code 36' : 'key code 53')
}

// Failure evidence for macOS. Linux has had a window dump + screenshot since the
// GTK work; macOS had nothing, so a failed native row left only its one-line
// error and every diagnosis needed another rig run. The dump answers the two
// questions that actually decide product-vs-harness: did a panel EXIST at all,
// and what did the appearance poll observe while it waited.
const macDumpScript = `
tell application "System Events"
  set report to "foreground=" & (name of first application process whose frontmost is true)
  set targetProcesses to application processes whose name is ${JSON.stringify(macExpectedProcess)}
  if (count of targetProcesses) is 0 then return report & linefeed & "target=missing"
  set targetProcess to first item of targetProcesses
  set report to report & linefeed & "windows=" & (count of windows of targetProcess)
  repeat with currentWindow in windows of targetProcess
    set report to report & linefeed & "window name=" & (name of currentWindow) &
      " subrole=" & (subrole of currentWindow) & " sheets=" & (count of sheets of currentWindow)
    repeat with currentSheet in sheets of currentWindow
      set report to report & linefeed & "  sheet description=" & (description of currentSheet) &
        " subsheets=" & (count of sheets of currentSheet)
      try
        repeat with currentGroup in splitter groups of currentSheet
          repeat with currentField in text fields of currentGroup
            set report to report & linefeed & "    field=" & (value of currentField as text)
          end repeat
        end repeat
      end try
      try
        repeat with subSheet in sheets of currentSheet
          repeat with currentField in text fields of subSheet
            set report to report & linefeed & "    subsheet field=" & (value of currentField as text)
          end repeat
        end repeat
      end try
    end repeat
  end repeat
  return report
end tell`
// State observed BEFORE the trigger ran. Kept at module scope so the failure
// dump can name the one condition that makes a macOS native action IMPOSSIBLE
// rather than merely slow: a sheet that some EARLIER row left open. macOS will
// not present a second sheet on a window that already has one, so the app never
// gets to show this action's panel — and `findDialog`'s count delta
// (`after.sheets > before.sheets`) can never fire either. Both effects report as
// the same bland "no native dialog appeared", which on 2026-08-07 cost 49 rows
// and a full re-run to attribute. Say it in the error instead.
let macBeforeState = null
function macPreexistingPanelNote() {
  if (!macBeforeState || !(Number(macBeforeState.sheets) > 0)) return ''
  return ' — A NATIVE PANEL WAS ALREADY OPEN BEFORE THIS ACTION ' +
    `(before=${JSON.stringify(macBeforeState)}): an earlier row left a modal sheet up, so macOS ` +
    'cannot present this one. This is a HARNESS leak (an unpaired native picker), not a product ' +
    'defect — find the row that opened a panel without an OS-action controller to dismiss it.'
}
function macFailureDiagnostics(error, samples) {
  const receipt = String(process.env.FCV_RESULT_RECEIPT || '').trim()
  if (!receipt) return null
  const evidenceDir = dirname(receipt)
  const slug = actionId.replace(/[^a-z0-9_.-]+/gi, '-').replace(/^-+|-+$/g, '') || 'action'
  const jsonPath = join(evidenceDir, `native-action-${slug}-failure.json`)
  const screenshotPath = join(evidenceDir, `native-action-${slug}-failure.png`)
  try {
    mkdirSync(evidenceDir, { recursive: true })
    let dump = ''
    try { dump = run(osascriptBin, ['-e', macDumpScript], 15_000) } catch (dumpError) {
      dump = `dump failed: ${dumpError?.message || String(dumpError)}`
    }
    let panel = null
    try { panel = macPanelState() } catch { /* the process may already be gone */ }
    // Screen capture needs a TCC grant that only the Terminal-hosted runner
    // holds; a direct-from-ssh run legitimately has none, so this stays
    // best-effort and never masks the real failure.
    let screenshot = null
    try {
      run('/usr/sbin/screencapture', ['-x', screenshotPath], 15_000)
      screenshot = screenshotPath
    } catch { /* no ScreenCapture grant in this launch context */ }
    writeFileSync(jsonPath, `${JSON.stringify({
      schema: 'shellx-cut/native-action-failure/1',
      action_id: actionId,
      platform: 'macos',
      mode,
      selected_path: selectedPath || null,
      error: error?.message || String(error),
      before: macBeforeState,
      // TRUE ⇒ this action could never have succeeded: a leftover modal sheet
      // blocked it. Read `panel.fieldValues` to identify whose panel it is.
      preexisting_panel: !!(macBeforeState && Number(macBeforeState.sheets) > 0),
      appearance_samples: samples,
      panel,
      accessibility_dump: dump.split('\n'),
      screenshot,
    }, null, 2)}\n`)
    return { json: jsonPath, screenshot }
  } catch {
    return null
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

const appearanceSamples = []
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
  if (platform === 'darwin' || platform === 'macos') macBeforeState = before
  if ((platform === 'win32' || platform === 'windows') && !isExpectedWindowsProcess(before)) {
    throw new Error(
      `refusing native action ${actionId}: foreground process is ${before.processName || 'unknown'}, expected ${windowsExpectedProcess}`,
    )
  }
  process.stdout.write(`${JSON.stringify({ phase: 'ready', actionId, platform, before })}\n`)
  const deadline = Date.now() + Number(process.env.FCV_NATIVE_ACTION_TIMEOUT_MS || 20_000)
  let dialog = null
  const started = Date.now()
  while (Date.now() < deadline) {
    await sleep(100)
    const sampled = state()
    // Every appearance poll is recorded so a "no native dialog appeared"
    // failure can be read as either "the app never presented one" or "the poll
    // was too slow / too short to see it", instead of being re-derived on the
    // rig. Only the counting platforms have a cheap scalar snapshot.
    if (platform === 'darwin' || platform === 'macos') {
      appearanceSamples.push({ ms: Date.now() - started, windows: sampled.windows, sheets: sampled.sheets })
    }
    dialog = findDialog(before, sampled)
    if (dialog) break
  }
  if (!dialog) {
    throw new Error(
      `no native dialog appeared for ${actionId}` +
      (appearanceSamples.length
        ? ` (${appearanceSamples.length} polls over ${Date.now() - started}ms; last=${JSON.stringify(appearanceSamples.at(-1))})`
        : '') +
      macPreexistingPanelNote(),
    )
  }
  // ONE bounded retry of the whole act+wait. The keystroke sequences that drive
  // a native chooser (macOS Cmd-Shift-G, type path, Return) are fire-and-forget:
  // if the host drops or reorders an event under load, the dialog simply stays
  // open and the action fails with nothing retried. Linux (linuxDismiss
  // attempts=3) and Windows (windowsActOnce retry) already recover internally;
  // macOS had no recovery at any layer, which is how a rig-automation hiccup
  // deterministically failed the first two save-as rows on 2026-08-06 while the
  // product was never even reached (the verb never fired — the panel never
  // returned a path). Retrying only while the dialog is STILL open cannot leak
  // keystrokes into the app, and it masks no product behavior: a dialog that
  // never closes means the product got no chance to act.
  // The retry is only safe for IDEMPOTENT modes. 'accept'/'cancel' are a single
  // keypress, so repeating them cannot corrupt state. 'select' types a PATH,
  // and repeating that into a panel whose sheet did not reopen sends the text
  // to the FILENAME field instead — macOS renders each '/' as ':' there and the
  // panel appends the extension again, so the verb receives
  // "var:folders:…:name.mp4.mp4" and correctly rejects it. That is a corrupted
  // input masquerading as a product bug (observed 2026-08-06 on export-save-as
  // after the retry landed), and it is strictly worse than the honest failure
  // the retry was meant to prevent. Retry the drop-prone keystroke modes only.
  // 'select' recovers a dropped keystroke INSIDE macSelect instead, where each
  // retry is guarded by an observed sheet state and cannot type blind.
  const retryable = mode !== 'select'
  const maxAttempts = retryable ? 2 : 1
  let attempts = 0
  let after = null
  let dismissed = false
  while (attempts < maxAttempts && !dismissed) {
    attempts += 1
    if (attempts > 1) {
      process.stdout.write(`${JSON.stringify({ phase: 'retry', actionId, platform, attempt: attempts })}\n`)
    }
    await act(before, dialog)
    const dismissalDeadline = Date.now() + Math.min(
      15_000,
      Number(process.env.FCV_NATIVE_ACTION_TIMEOUT_MS || 20_000),
    )
    after = state()
    dismissed = !findDialog(before, after, dialog)
    while (!dismissed && Date.now() < dismissalDeadline) {
      await sleep(150)
      after = state()
      dismissed = !findDialog(before, after, dialog)
    }
  }
  if (!dismissed) {
    const geometry = platform === 'linux' && dialog
      ? (() => {
          try { return linuxWindowGeometry(dialog) } catch { return null }
        })()
      : null
    throw new Error(
      `native dialog remained open after ${mode} (${attempts} attempt(s)): ` +
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
  const diagnostics = platform === 'linux'
    ? linuxFailureDiagnostics(error)
    : (platform === 'darwin' || platform === 'macos')
      ? macFailureDiagnostics(error, appearanceSamples)
      : null
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
