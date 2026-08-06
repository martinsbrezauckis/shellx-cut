import assert from 'node:assert/strict'
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { createNativeOsActionController } from '../../ui/public-tests/lib/nativeOsActionController.mjs'

const controllerUrl = new URL('../release/native-os-action-controller.mjs', import.meta.url)
const windowsUrl = new URL('../release/native-os-action-windows.ps1', import.meta.url)

test('disabled native controller leaves the ordinary browser action path alone', async () => {
  const controller = createNativeOsActionController({ command: '' })
  let triggered = false
  const result = await controller.run({ actionId: 'picker', mode: 'cancel' }, async () => {
    triggered = true
  })
  assert.equal(result.controlled, false)
  assert.equal(triggered, false)
})

test('native controller drains a refusal proof before reporting child exit', {
  skip: process.platform === 'win32' ? 'the executable fixture uses a POSIX shebang' : false,
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-native-controller-'))
  const fixture = join(root, 'controller.mjs')
  await writeFile(fixture, `#!/usr/bin/env node
const actionIndex = process.argv.indexOf('--action')
const action = actionIndex >= 0 ? process.argv[actionIndex + 1] : ''
if (action === 'early-refusal') {
  process.stdout.write(JSON.stringify({
    phase: 'done',
    ok: false,
    error: 'fixture refused before ready',
  }) + '\\n')
  process.exitCode = 1
} else {
process.stdout.write(JSON.stringify({
  phase: 'ready',
  before: { processName: 'ShellX Cut', windows: 1, sheets: 0 },
}) + '\\n')
process.on('exit', () => {
  process.stdout.write(JSON.stringify({
    phase: 'done',
    ok: false,
    error: 'fixture foreground safety refusal',
  }) + '\\n')
})
}
`)
  await chmod(fixture, 0o755)
  try {
    const controller = createNativeOsActionController({
      command: fixture,
      platform: 'fixture',
      timeoutMs: 5_000,
    })
    await assert.rejects(
      controller.run({ actionId: 'picker', mode: 'cancel' }, () => {}),
      /fixture foreground safety refusal/,
    )
    await assert.rejects(
      controller.run({ actionId: 'early-refusal', mode: 'cancel' }, () => {}),
      /fixture refused before ready/,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test('native controller timeout stays an action failure instead of an unhandled rejection', {
  skip: process.platform === 'win32' ? 'the executable fixture uses a POSIX shebang' : false,
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-native-timeout-'))
  const fixture = join(root, 'controller.mjs')
  await writeFile(fixture, `#!/usr/bin/env node
process.stdout.write(JSON.stringify({ phase: 'ready' }) + '\\n')
setInterval(() => {}, 1000)
`)
  await chmod(fixture, 0o755)
  try {
    const controller = createNativeOsActionController({
      command: fixture,
      platform: 'fixture',
      timeoutMs: 40,
    })
    let triggered = false
    await assert.rejects(
      controller.run({ actionId: 'missing-picker', mode: 'cancel' }, () => {
        triggered = true
      }),
      /native action controller (timed out|exited before proof)/,
    )
    assert.equal(triggered, true)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test('native controller owns real host dialog detection and return actions', async () => {
  const source = await readFile(controllerUrl, 'utf8')
  const windowsSource = await readFile(windowsUrl, 'utf8')
  assert.match(source, /xdotool.+search.+--onlyvisible/)
  assert.match(source, /xdotool.+windowfocus.+--sync/)
  assert.match(source, /GTK file choosers ignore synthetic key events/)
  assert.match(source, /linuxCurrentDialog/)
  assert.match(source, /linuxTypedPath/)
  assert.match(source, /linuxAtspiDirectoryScript/)
  assert.match(source, /Gtk[.]RecentManager[.]get_default[(][)]/)
  assert.match(source, /statSync[(]selectedPath[)][.]isDirectory[(][)]/)
  assert.match(source, /Pre-register existing directory fixtures/)
  assert.match(source, /RecentManager persists through an asynchronous GIO write/)
  assert.match(source, /GLib[.]MainContext[.]default[(][)]/)
  assert.match(source, /A folder-only GTK chooser derives its Recent rows/)
  assert.match(source, /[.]shellx-cut-native-choice-/)
  assert.match(source, /unlinkSync[(]linuxRecentSeedPath[)]/)
  assert.match(source, /remove_item[(]uri[)]/)
  assert.match(source, /Cleanup is intentionally idempotent/)
  assert.match(source, /force an exact reload/)
  assert.match(source, /sidebar[.]select_child[(]1[)]/)
  assert.match(source, /sidebar[.]select_child[(]0[)]/)
  assert.match(source, /table cell/)
  assert.match(source, /selection_owner/)
  assert.match(source, /Atspi[.]CoordType[.]WINDOW/)
  assert.match(source, /'--window', dialog_handle/)
  assert.match(source, /selection_child[.]get_state_set[(][)][.]contains[(]Atspi[.]StateType[.]SELECTED[)]/)
  assert.match(source, /do_action/)
  assert.match(source, /getwindowgeometry/)
  assert.doesNotMatch(source, /mousemove', '--sync'/)
  assert.match(source, /autoExtensionDialog/)
  assert.match(source, /any newly visible top-level window is the dialog/)
  assert.match(source, /tell application "System Events"/)
  assert.match(source, /FCV_OSASCRIPT/)
  assert.match(source, /function macRequireExpectedFocus/)
  assert.match(source, /first application process whose name is/)
  assert.match(source, /ShellX Cut did not gain macOS foreground focus/)
  assert.match(source, /function macSelectedPathIsDirectory/)
  assert.match(source, /macSelectedPathIsDirectory[(][)] && findDialog[(]before, macState[(][)], dialog[)]/)
  assert.match(source, /one more Return confirms the current folder/)
  assert.match(source, /powershell[.]exe/)
  assert.match(source, /function windowsDialogStillExists/)
  assert.match(source, /if \(!windowsDialogStillExists\(state\)\) return ''/)
  assert.match(source, /one bounded retry is safe/)
  assert.match(source, /Array[.]isArray[(]before[.]windows[)]/)
  assert.match(source, /new Set[(]beforeWindows/)
  assert.match(source, /isExpectedWindowsProcess/)
  assert.match(source, /refusing native action/)
  assert.match(source, /windowsFocus[(]before[)]/)
  assert.match(source, /ShellX Cut did not regain foreground focus/)
  assert.match(windowsSource, /ValidateSet[(]"state", "act", "focus"[)]/)
  assert.match(windowsSource, /[$]Command -eq "focus"/)
  assert.match(windowsSource, /SetForegroundWindow/)
  assert.match(windowsSource, /ShellX Cut window [$]Handle did not regain foreground focus/)
  assert.match(source, /dismissalDeadline/)
  assert.match(source, /'accept', 'cancel', 'select'/)
  assert.match(source, /mode === 'accept'/)
  assert.match(source, /accepted the action/)
  assert.match(source, /no native dialog appeared/)
  assert.match(source, /native dialog remained open/)
  assert.doesNotMatch(source, /phase:\s*'done',\s*ok:\s*true[\s\S]+setTimeout/)
})

test('macOS controller activates, scopes input to, and restores the expected app', {
  skip: process.platform === 'win32' ? 'the osascript fixture uses a POSIX shebang' : false,
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-macos-dialog-'))
  const fixture = join(root, 'osascript')
  const statePath = join(root, 'state')
  const logPath = join(root, 'log')
  await writeFile(statePath, 'finder')
  await writeFile(logPath, '')
  await writeFile(fixture, `#!/usr/bin/env node
import { appendFileSync, readFileSync, writeFileSync } from 'node:fs'
const script = process.argv.at(-1) || ''
const statePath = process.env.FCV_FAKE_MAC_STATE
const logPath = process.env.FCV_FAKE_MAC_LOG
const state = readFileSync(statePath, 'utf8').trim()
if (script.includes('return foregroundName')) {
  const foreground = state === 'finder' ? 'Finder' : 'shellx-cut'
  const sheets = state === 'dialog' ? 1 : 0
  process.stdout.write(foreground + '|shellx-cut|1|' + sheets + '\\n')
} else if (script.includes('set frontmost')) {
  appendFileSync(logPath, 'focus-shellx-cut\\n')
  if (state !== 'dialog') writeFileSync(statePath, 'focused')
} else if (script.includes('first application process whose name is "shellx-cut"')) {
  appendFileSync(logPath, 'act-shellx-cut\\n')
  writeFileSync(statePath, 'closed')
} else {
  process.stderr.write('unexpected fake osascript request\\n')
  process.exitCode = 1
}
`)
  await chmod(fixture, 0o755)
  const old = {
    osascript: process.env.FCV_OSASCRIPT,
    expected: process.env.FCV_NATIVE_EXPECTED_PROCESS,
    state: process.env.FCV_FAKE_MAC_STATE,
    log: process.env.FCV_FAKE_MAC_LOG,
  }
  try {
    process.env.FCV_OSASCRIPT = fixture
    process.env.FCV_NATIVE_EXPECTED_PROCESS = 'shellx-cut'
    process.env.FCV_FAKE_MAC_STATE = statePath
    process.env.FCV_FAKE_MAC_LOG = logPath
    const controller = createNativeOsActionController({
      command: new URL('../release/native-os-action-controller.mjs', import.meta.url).pathname,
      platform: 'macos',
      timeoutMs: 5_000,
    })
    const proof = await controller.run(
      { actionId: 'mac-picker', mode: 'cancel' },
      () => writeFile(statePath, 'dialog'),
    )
    assert.equal(proof.ok, true)
    assert.equal(proof.readyProof.before.foregroundProcessName, 'shellx-cut')
    const log = await readFile(logPath, 'utf8')
    assert.match(log, /focus-shellx-cut/)
    assert.match(log, /act-shellx-cut/)
    assert.ok(log.match(/focus-shellx-cut/g)?.length >= 2, 'focus is restored after dismissal')
  } finally {
    for (const [name, value] of [
      ['FCV_OSASCRIPT', old.osascript],
      ['FCV_NATIVE_EXPECTED_PROCESS', old.expected],
      ['FCV_FAKE_MAC_STATE', old.state],
      ['FCV_FAKE_MAC_LOG', old.log],
    ]) {
      if (value === undefined) delete process.env[name]
      else process.env[name] = value
    }
    await rm(root, { recursive: true, force: true })
  }
})

test('Linux controller handles replaced, save, import, and confirmation dialogs', {
  skip: process.platform === 'win32' ? 'the xdotool fixture uses a POSIX shebang' : false,
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-linux-dialog-'))
  const bin = join(root, 'bin')
  const fixture = join(bin, 'xdotool')
  const atspiFixture = join(bin, 'python3')
  const statePath = join(root, 'state')
  const typedPath = join(root, 'typed')
  const focusPath = join(root, 'focus')
  await mkdir(bin)
  await writeFile(statePath, 'before')
  await writeFile(fixture, `#!/usr/bin/env node
import { appendFileSync, readFileSync, writeFileSync } from 'node:fs'
const [command, ...args] = process.argv.slice(2)
const statePath = process.env.FCV_FAKE_XDOTOOL_STATE
const state = readFileSync(statePath, 'utf8').trim()
const dialogHandle = state === 'replaced' ? '201' : state === 'tooltip' ? '300' : '200'
if (command === 'search') {
  process.stdout.write(state === 'before' || state === 'closed' ? '100\\n' : '100\\n' + dialogHandle + '\\n')
} else if (command === 'getwindowname') {
  process.stdout.write(args[0] === '100' ? 'ShellX Cut\\n' : args[0] === '300' ? 'shellx-cut\\n' : process.env.FCV_FAKE_XDOTOOL_TITLE + '\\n')
} else if (command === 'getwindowclassname') {
  process.stdout.write(args[0] === '100' ? 'shellx-cut\\n' : args[0] === '300' ? 'GtkTooltip\\n' : 'GtkFileChooserDialog\\n')
} else if (command === 'windowfocus') {
  if (state === 'closed' || args.at(-1) !== dialogHandle) process.exitCode = 1
  else appendFileSync(process.env.FCV_FAKE_XDOTOOL_FOCUS, dialogHandle + '\\n')
} else if (command === 'getwindowgeometry') {
  process.stdout.write('WINDOW=' + dialogHandle + '\\nX=10\\nY=20\\nWIDTH=815\\nHEIGHT=338\\nSCREEN=0\\n')
} else if (command === 'mousemove') {
  appendFileSync(process.env.FCV_FAKE_XDOTOOL_FOCUS, 'pointer=' + args.slice(-2).join(',') + '\\n')
} else if (command === 'click') {
  if (process.env.FCV_FAKE_XDOTOOL_DIRECTORY === '1') writeFileSync(statePath, 'closed')
} else if (command === 'type') {
  appendFileSync(process.env.FCV_FAKE_XDOTOOL_TYPED, args.at(-1) + '\\n')
} else if (command === 'key') {
  const key = args.at(-1)
  if (key === 'Escape') {
    if (process.env.FCV_FAKE_XDOTOOL_DIRECTORY === '1') {
      appendFileSync(process.env.FCV_FAKE_XDOTOOL_FOCUS, 'escape-location\\n')
    } else {
      writeFileSync(statePath, 'closed')
    }
  }
  if (key === 'Return') {
    if (process.env.FCV_FAKE_XDOTOOL_DIRECTORY === '1') writeFileSync(statePath, 'dialog')
    else if (process.env.FCV_FAKE_XDOTOOL_REPLACE === '1' && state === 'dialog') writeFileSync(statePath, 'replaced')
    else writeFileSync(statePath, 'closed')
  }
}
`)
  await chmod(fixture, 0o755)
  await writeFile(atspiFixture, `#!/usr/bin/env node
import { appendFileSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'
const args = process.argv.slice(2)
const modeIndex = args.findIndex((value) => ['register', 'unregister', 'select'].includes(value))
const mode = args[modeIndex]
const selectedPath = args[modeIndex + 2]
appendFileSync(process.env.FCV_FAKE_XDOTOOL_TYPED, (mode === 'parent' ? dirname(selectedPath) : selectedPath) + '\\n')
appendFileSync(process.env.FCV_FAKE_XDOTOOL_FOCUS, 'atspi-' + mode + '=' + selectedPath + '\\n')
if (mode === 'select') writeFileSync(
  process.env.FCV_FAKE_XDOTOOL_STATE,
  process.env.FCV_FAKE_XDOTOOL_TOOLTIP_AFTER === '1' ? 'tooltip' : 'closed',
)
`)
  await chmod(atspiFixture, 0o755)
  const oldEnv = {
    PATH: process.env.PATH,
    state: process.env.FCV_FAKE_XDOTOOL_STATE,
    title: process.env.FCV_FAKE_XDOTOOL_TITLE,
    typed: process.env.FCV_FAKE_XDOTOOL_TYPED,
    focus: process.env.FCV_FAKE_XDOTOOL_FOCUS,
    replace: process.env.FCV_FAKE_XDOTOOL_REPLACE,
    directory: process.env.FCV_FAKE_XDOTOOL_DIRECTORY,
    tooltipAfter: process.env.FCV_FAKE_XDOTOOL_TOOLTIP_AFTER,
  }
  try {
    process.env.PATH = `${bin}:${process.env.PATH}`
    process.env.FCV_FAKE_XDOTOOL_STATE = statePath
    process.env.FCV_FAKE_XDOTOOL_TITLE = 'Save Video (.mp4) — ShellX Cut'
    process.env.FCV_FAKE_XDOTOOL_TYPED = typedPath
    process.env.FCV_FAKE_XDOTOOL_FOCUS = focusPath
    process.env.FCV_FAKE_XDOTOOL_REPLACE = '1'
    process.env.FCV_FAKE_XDOTOOL_DIRECTORY = '0'
    const controller = createNativeOsActionController({
      command: new URL('../release/native-os-action-controller.mjs', import.meta.url).pathname,
      platform: 'linux',
      timeoutMs: 5_000,
    })
    const proof = await controller.run(
      { actionId: 'save-video', mode: 'select', path: '/tmp/final-video.mp4' },
      () => writeFile(statePath, 'dialog'),
    )
    assert.equal(proof.ok, true)
    assert.equal((await readFile(typedPath, 'utf8')).trim().split('\n').at(-1), '/tmp/final-video')
    assert.deepEqual(
      (await readFile(focusPath, 'utf8')).trim().split('\n'),
      ['200', '201'],
    )

    await writeFile(statePath, 'before')
    process.env.FCV_FAKE_XDOTOOL_TITLE = 'Import timeline (OpenTimelineIO) — ShellX Cut'
    process.env.FCV_FAKE_XDOTOOL_REPLACE = '0'
    const importProof = await controller.run(
      { actionId: 'import-otio', mode: 'select', path: '/tmp/timeline.otio' },
      () => writeFile(statePath, 'dialog'),
    )
    assert.equal(importProof.ok, true)
    assert.equal((await readFile(typedPath, 'utf8')).trim().split('\n').at(-1), '/tmp/timeline.otio')

    await writeFile(statePath, 'before')
    process.env.FCV_FAKE_XDOTOOL_TITLE = 'Relink Motion package — ShellX Cut'
    process.env.FCV_FAKE_XDOTOOL_DIRECTORY = '1'
    process.env.FCV_FAKE_XDOTOOL_TOOLTIP_AFTER = '1'
    const motionPackage = join(root, 'relinked.motionpkg')
    await mkdir(motionPackage)
    const motionRelinkProof = await controller.run(
      { actionId: 'motion-relink', mode: 'select', path: motionPackage },
      () => writeFile(statePath, 'dialog'),
    )
    assert.equal(motionRelinkProof.ok, true)
    assert.equal((await readFile(typedPath, 'utf8')).trim().split('\n').at(-1), motionPackage)

    await writeFile(statePath, 'before')
    process.env.FCV_FAKE_XDOTOOL_TITLE = 'ShellX Cut'
    process.env.FCV_FAKE_XDOTOOL_DIRECTORY = '0'
    const confirmProof = await controller.run(
      { actionId: 'delete-sequence', mode: 'accept' },
      () => writeFile(statePath, 'dialog'),
    )
    assert.equal(confirmProof.ok, true)

    await writeFile(statePath, 'before')
    process.env.FCV_FAKE_XDOTOOL_TITLE = 'Choose export folder — ShellX Cut'
    process.env.FCV_FAKE_XDOTOOL_DIRECTORY = '1'
    const exportFolder = join(root, 'project.cutproj')
    await mkdir(exportFolder)
    const directoryProof = await controller.run(
      { actionId: 'choose-folder', mode: 'select', path: exportFolder },
      () => writeFile(statePath, 'dialog'),
    )
    assert.equal(directoryProof.ok, true)
    assert.equal(
      (await readFile(typedPath, 'utf8')).trim().split('\n').at(-1),
      exportFolder,
      'directory selection sets the exact GTK accessible location',
    )
    assert.ok(
      (await readFile(focusPath, 'utf8')).includes(`atspi-select=${exportFolder}`),
      'directory selection actuates the accessible GTK folder action',
    )
    assert.equal(
      (await readFile(statePath, 'utf8')).trim(),
      'tooltip',
      'an unrelated transient can remain without being misidentified as the chooser',
    )
  } finally {
    process.env.PATH = oldEnv.PATH
    for (const [name, value] of [
      ['FCV_FAKE_XDOTOOL_STATE', oldEnv.state],
      ['FCV_FAKE_XDOTOOL_TITLE', oldEnv.title],
      ['FCV_FAKE_XDOTOOL_TYPED', oldEnv.typed],
      ['FCV_FAKE_XDOTOOL_FOCUS', oldEnv.focus],
      ['FCV_FAKE_XDOTOOL_REPLACE', oldEnv.replace],
      ['FCV_FAKE_XDOTOOL_DIRECTORY', oldEnv.directory],
      ['FCV_FAKE_XDOTOOL_TOOLTIP_AFTER', oldEnv.tooltipAfter],
    ]) {
      if (value === undefined) delete process.env[name]
      else process.env[name] = value
    }
    await rm(root, { recursive: true, force: true })
  }
})

test('native controller keeps trigger and host-controller failures together', {
  skip: process.platform === 'win32' ? 'the executable fixture uses a POSIX shebang' : false,
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-native-combined-failure-'))
  const fixture = join(root, 'controller.mjs')
  await writeFile(fixture, `#!/usr/bin/env node
process.stdout.write(JSON.stringify({
  phase: 'ready',
  before: { processName: 'ShellX Cut', windows: 1, sheets: 0 },
}) + '\\n')
setTimeout(() => {
  process.stdout.write(JSON.stringify({
    phase: 'done',
    ok: false,
    error: 'host picker stayed open',
  }) + '\\n')
  process.exitCode = 1
}, 20)
`)
  await chmod(fixture, 0o755)
  try {
    const controller = createNativeOsActionController({
      command: fixture,
      platform: 'fixture',
      timeoutMs: 5_000,
    })
    await assert.rejects(
      controller.run(
        { actionId: 'picker', mode: 'select', path: '/fixture/timeline.otio' },
        () => { throw new Error('preview never appeared') },
      ),
      /native action trigger failed: preview never appeared; native controller failed: host picker stayed open; controller ready state=\{"processName":"ShellX Cut","windows":1,"sheets":0\}/,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test('Windows controller enumerates visible dialogs and preserves path text', async () => {
  const source = await readFile(windowsUrl, 'utf8')
  assert.match(source, /GetForegroundWindow/)
  assert.match(source, /EnumWindows/)
  assert.match(source, /IsWindowVisible/)
  assert.match(source, /GetClassName/)
  assert.match(source, /ExpectedProcessName/)
  assert.match(source, /Assert-ForegroundTarget/)
  assert.match(source, /Refusing native input/)
  assert.match(source, /SendWait[(]"{ESC}"[)]/)
  assert.match(source, /[$]Mode -eq "accept"/)
  assert.match(source, /[$]TDM_CLICK_BUTTON = 0x0400 \+ 102/)
  assert.match(source, /[$]affirmativeButtonIds = @[(]1000, 1, 6[)]/)
  assert.match(source, /[$]TDM_CLICK_BUTTON/)
  assert.match(source, /SendKeys metacharacters/)
  assert.ok(source.includes('$pickerPath = $pickerPath.Replace("/", "\\")'))
  assert.match(source, /GetDlgItem/)
  assert.match(source, /BM_CLICK/)
  assert.match(source, /PostMessage[(]\s*[$]button,\s*[$]BM_CLICK/)
  assert.doesNotMatch(source, /SendMessage[(]\s*[$]button,\s*[$]BM_CLICK/)
  assert.match(source, /[$]Command -eq "focus"/)
  assert.match(source, /SetForegroundWindow[(][$]window[)]/)
  assert.match(source, /Assert-ForegroundTarget/)
})

// A native chooser is driven by fire-and-forget keystrokes; a host that drops
// one leaves the dialog open with nothing retried. Linux and Windows recover
// internally, macOS did not — which deterministically failed the first two
// save-as rows on 2026-08-06 without the product ever being reached. The
// controller now retries the act+wait ONCE while the dialog is still open.
// Red-proof: before the fix this test failed, because the first (dropped) act
// left the dialog open and the run threw instead of retrying.
test('a dropped native keystroke sequence is retried once instead of failing the action', {
  skip: process.platform === 'win32' ? 'the osascript fixture uses a POSIX shebang' : false,
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-macos-retry-'))
  const fixture = join(root, 'osascript')
  const statePath = join(root, 'state')
  const logPath = join(root, 'log')
  await writeFile(statePath, 'finder')
  await writeFile(logPath, '')
  // The fake host DROPS the first act (dialog stays open) and honors the second.
  await writeFile(fixture, `#!/usr/bin/env node
import { appendFileSync, readFileSync, writeFileSync } from 'node:fs'
const script = process.argv.at(-1) || ''
const statePath = process.env.FCV_FAKE_MAC_STATE
const logPath = process.env.FCV_FAKE_MAC_LOG
const state = readFileSync(statePath, 'utf8').trim()
if (script.includes('return foregroundName')) {
  const foreground = state === 'finder' ? 'Finder' : 'shellx-cut'
  const sheets = state === 'dialog' ? 1 : 0
  process.stdout.write(foreground + '|shellx-cut|1|' + sheets + '\\n')
} else if (script.includes('set frontmost')) {
  appendFileSync(logPath, 'focus-shellx-cut\\n')
  if (state !== 'dialog') writeFileSync(statePath, 'focused')
} else if (script.includes('first application process whose name is "shellx-cut"')) {
  const acts = (readFileSync(logPath, 'utf8').match(/act-shellx-cut/g) || []).length
  appendFileSync(logPath, 'act-shellx-cut\\n')
  if (acts >= 1) writeFileSync(statePath, 'closed')
} else {
  process.stderr.write('unexpected fake osascript request\\n')
  process.exitCode = 1
}
`)
  await chmod(fixture, 0o755)
  const old = {
    osascript: process.env.FCV_OSASCRIPT,
    expected: process.env.FCV_NATIVE_EXPECTED_PROCESS,
    state: process.env.FCV_FAKE_MAC_STATE,
    log: process.env.FCV_FAKE_MAC_LOG,
    timeout: process.env.FCV_NATIVE_ACTION_TIMEOUT_MS,
  }
  try {
    process.env.FCV_OSASCRIPT = fixture
    process.env.FCV_NATIVE_EXPECTED_PROCESS = 'shellx-cut'
    process.env.FCV_FAKE_MAC_STATE = statePath
    process.env.FCV_FAKE_MAC_LOG = logPath
    // Short dismissal window so the dropped attempt gives up quickly.
    process.env.FCV_NATIVE_ACTION_TIMEOUT_MS = '2000'
    const controller = createNativeOsActionController({
      command: new URL('../release/native-os-action-controller.mjs', import.meta.url).pathname,
      platform: 'macos',
      timeoutMs: 20_000,
    })
    const proof = await controller.run(
      { actionId: 'mac-picker-retry', mode: 'cancel' },
      () => writeFile(statePath, 'dialog'),
    )
    assert.equal(proof.ok, true, 'the retried action succeeds')
    const log = await readFile(logPath, 'utf8')
    assert.equal(log.match(/act-shellx-cut/g)?.length, 2, 'the act ran exactly twice — one bounded retry, not a loop')
  } finally {
    for (const [name, value] of [
      ['FCV_OSASCRIPT', old.osascript],
      ['FCV_NATIVE_EXPECTED_PROCESS', old.expected],
      ['FCV_FAKE_MAC_STATE', old.state],
      ['FCV_FAKE_MAC_LOG', old.log],
      ['FCV_NATIVE_ACTION_TIMEOUT_MS', old.timeout],
    ]) {
      if (value === undefined) delete process.env[name]
      else process.env[name] = value
    }
    await rm(root, { recursive: true, force: true })
  }
})

// The retry must NOT apply to 'select'. Re-typing a path into a save panel whose
// Go-to-Folder sheet did not reopen lands the text in the FILENAME field, where
// macOS renders '/' as ':' and the panel re-appends the extension — the verb then
// receives "var:folders:...:name.mp4.mp4" and correctly rejects it. A corrupted
// input that looks like a product bug is worse than an honest dismissal failure.
test('select mode never retries, because re-typing a path corrupts the filename field', () => {
  const source = readFileSync(new URL('../release/native-os-action-controller.mjs', import.meta.url), 'utf8')
  assert.match(source, /const retryable = mode !== 'select'/, 'select is excluded from the retry')
  assert.match(source, /const maxAttempts = retryable \? 2 : 1/, 'non-retryable modes get exactly one attempt')
  assert.match(source, /name\.mp4\.mp4|\.mp4\.mp4/, 'the corruption mode is documented at the guard')
})

test('macOS select drives the panel through observed state, never blind keystrokes', () => {
  const source = readFileSync(new URL('../release/native-os-action-controller.mjs', import.meta.url), 'utf8')
  assert.match(source, /macPanelQueryScript/, 'the panel is queried, not assumed')
  assert.match(source, /value of text field 1 of sheet 1 of panel/, 'the Go to Folder field is read back')
  assert.match(source, /splitter groups of panel/, 'the panel file-name field is observable')
  assert.match(source, /Go to Folder sheet did not open/, 'an unopened sheet is a loud failure')
  assert.match(source, /Go to Folder field did not receive the exact path/, 'a mistyped path is a loud failure')
  assert.match(source, /Go to Folder sheet stayed open after commit/, 'an uncommitted sheet is a loud failure')
  assert.match(source, /panel file-name field holds a path-corrupted value/, 'the ":" signature is caught at the harness')
  assert.match(source, /macFailureDiagnostics/, 'macOS failures leave evidence like Linux ones')
  assert.match(source, /appearance_samples/, 'the dialog-appearance poll is recorded for triage')
})

// A fake osascript stands in for the whole AppKit panel: it answers the state,
// panel-query and keystroke scripts the controller sends, and each scenario
// below reproduces one real failure shape observed on a macOS host.
async function macPanelFixture(scenario) {
  const root = await mkdtemp(join(tmpdir(), `shellx-cut-macos-${scenario}-`))
  const fixture = join(root, 'osascript')
  const statePath = join(root, 'state.json')
  const logPath = join(root, 'log')
  await writeFile(statePath, JSON.stringify({
    scenario, phase: 'idle', focused: false, goToOpen: false, goToValue: '', fields: [],
  }))
  await writeFile(logPath, '')
  await writeFile(fixture, `#!/usr/bin/env node
import { appendFileSync, readFileSync, writeFileSync } from 'node:fs'
const script = process.argv.at(-1) || ''
const statePath = process.env.FCV_FAKE_MAC_STATE
const logPath = process.env.FCV_FAKE_MAC_LOG
const state = JSON.parse(readFileSync(statePath, 'utf8'))
const save = () => writeFileSync(statePath, JSON.stringify(state))
const log = (line) => appendFileSync(logPath, line + '\\n')
if (script.includes('return foregroundName')) {
  const foreground = state.phase === 'idle' && !state.focused ? 'Finder' : 'shellx-cut'
  process.stdout.write(foreground + '|shellx-cut|1|' + (state.phase === 'dialog' ? 1 : 0) + '\\n')
} else if (script.includes('set goToOpen')) {
  process.stdout.write(['panel', '1', String(state.goToOpen), state.goToValue, ...state.fields].join('\\t') + '\\n')
} else if (script.includes('set frontmost')) {
  log('focus')
  if (state.phase !== 'dialog') state.focused = true
  save()
} else if (script.includes('first application process whose name is "shellx-cut"')) {
  if (script.includes('keystroke "g" using {command down, shift down}')) {
    log('go-to-folder')
    if (state.scenario !== 'no-goto') { state.goToOpen = true; state.goToValue = '/stale/previous/value' }
  } else if (script.includes('keystroke "a" using {command down}')) {
    const typed = (script.match(/keystroke "([^"]*)"\\s*$/m) || [])[1] || ''
    log('type=' + typed)
    state.goToValue = state.scenario === 'wrong-path' ? '/somewhere/else' : typed
  } else if (script.includes('key code 36')) {
    log('return')
    if (state.goToOpen) {
      state.goToOpen = false
      state.fields = [state.scenario === 'corrupt-name' ? 'var:folders:T:save.mp4.mp4' : 'save.mp4']
    } else {
      state.phase = 'closed'
    }
  } else if (script.includes('key code 53')) {
    log('escape')
    state.phase = 'closed'
  }
  save()
} else {
  process.stderr.write('unexpected fake osascript request\\n')
  process.exitCode = 1
}
`)
  await chmod(fixture, 0o755)
  return { root, fixture, statePath, logPath }
}

async function withMacPanelFixture(scenario, body) {
  const context = await macPanelFixture(scenario)
  const old = {
    osascript: process.env.FCV_OSASCRIPT,
    expected: process.env.FCV_NATIVE_EXPECTED_PROCESS,
    state: process.env.FCV_FAKE_MAC_STATE,
    log: process.env.FCV_FAKE_MAC_LOG,
    timeout: process.env.FCV_NATIVE_ACTION_TIMEOUT_MS,
  }
  try {
    process.env.FCV_OSASCRIPT = context.fixture
    process.env.FCV_NATIVE_EXPECTED_PROCESS = 'shellx-cut'
    process.env.FCV_FAKE_MAC_STATE = context.statePath
    process.env.FCV_FAKE_MAC_LOG = context.logPath
    process.env.FCV_NATIVE_ACTION_TIMEOUT_MS = '4000'
    const controller = createNativeOsActionController({
      command: new URL('../release/native-os-action-controller.mjs', import.meta.url).pathname,
      platform: 'macos',
      timeoutMs: 60_000,
    })
    const openDialog = async () => {
      const state = JSON.parse(await readFile(context.statePath, 'utf8'))
      state.phase = 'dialog'
      await writeFile(context.statePath, JSON.stringify(state))
    }
    await body({ controller, openDialog, ...context })
  } finally {
    for (const [name, value] of [
      ['FCV_OSASCRIPT', old.osascript],
      ['FCV_NATIVE_EXPECTED_PROCESS', old.expected],
      ['FCV_FAKE_MAC_STATE', old.state],
      ['FCV_FAKE_MAC_LOG', old.log],
      ['FCV_NATIVE_ACTION_TIMEOUT_MS', old.timeout],
    ]) {
      if (value === undefined) delete process.env[name]
      else process.env[name] = value
    }
    await rm(context.root, { recursive: true, force: true })
  }
}

test('macOS select proves the Go to Folder sheet, the typed path, and the commit', {
  skip: process.platform === 'win32' ? 'the osascript fixture uses a POSIX shebang' : false,
}, async () => {
  await withMacPanelFixture('happy', async ({ controller, openDialog, logPath }) => {
    const proof = await controller.run(
      { actionId: 'export-saveas-option', mode: 'select', path: '/tmp/fcv/save.mp4' },
      openDialog,
    )
    assert.equal(proof.ok, true)
    const log = (await readFile(logPath, 'utf8')).trim().split('\n')
    assert.equal(log.filter((line) => line === 'go-to-folder').length, 1, 'one sheet request when it opens first try')
    assert.ok(log.includes('type=/tmp/fcv/save.mp4'), 'the exact path is typed into the sheet')
    assert.equal(log.filter((line) => line === 'return').length, 2, 'one Return commits the sheet, one accepts the panel')
    assert.ok(
      log.indexOf('go-to-folder') < log.indexOf('type=/tmp/fcv/save.mp4'),
      'nothing is typed before the sheet is observed open',
    )
  })
})

// RED-PROOF for the 2026-08-06 export-save-as-video failure. Before this fix the
// controller sent Cmd-Shift-G, waited a fixed 0.2s and typed the path regardless:
// with the sheet closed the text went into the panel's FILENAME field, macOS
// rendered every '/' as ':' and re-appended the extension, and render.final
// refused "var:folders:…:save-as-3-video.mp4.mp4" — a harness-corrupted input
// that reads as a product defect. It must now fail loudly with nothing typed.
test('macOS select refuses to type when the Go to Folder sheet never opens', {
  skip: process.platform === 'win32' ? 'the osascript fixture uses a POSIX shebang' : false,
}, async () => {
  await withMacPanelFixture('no-goto', async ({ controller, openDialog, logPath }) => {
    await assert.rejects(
      controller.run(
        { actionId: 'export-saveas-option', mode: 'select', path: '/tmp/fcv/save.mp4' },
        openDialog,
      ),
      /Go to Folder sheet did not open for export-saveas-option after 3 attempts/,
    )
    const log = (await readFile(logPath, 'utf8')).trim().split('\n')
    assert.equal(log.filter((line) => line === 'go-to-folder').length, 3, 'the safe retry is bounded at 3')
    assert.equal(
      log.filter((line) => line.startsWith('type=')).length,
      0,
      'no path is ever typed into a panel whose sheet is not open',
    )
  })
})

test('macOS select fails loudly when the typed path lands somewhere else', {
  skip: process.platform === 'win32' ? 'the osascript fixture uses a POSIX shebang' : false,
}, async () => {
  await withMacPanelFixture('wrong-path', async ({ controller, openDialog, logPath }) => {
    await assert.rejects(
      controller.run(
        { actionId: 'export-saveas-option', mode: 'select', path: '/tmp/fcv/save.mp4' },
        openDialog,
      ),
      /Go to Folder field did not receive the exact path/,
    )
    const log = (await readFile(logPath, 'utf8')).trim().split('\n')
    assert.equal(log.filter((line) => line === 'return').length, 0, 'a mistyped path is never committed')
  })
})

test('macOS select catches a colon-corrupted panel file name at the harness', {
  skip: process.platform === 'win32' ? 'the osascript fixture uses a POSIX shebang' : false,
}, async () => {
  await withMacPanelFixture('corrupt-name', async ({ controller, openDialog }) => {
    await assert.rejects(
      controller.run(
        { actionId: 'export-saveas-option', mode: 'select', path: '/tmp/fcv/save.mp4' },
        openDialog,
      ),
      /panel file-name field holds a path-corrupted value.+var:folders/s,
    )
  })
})
