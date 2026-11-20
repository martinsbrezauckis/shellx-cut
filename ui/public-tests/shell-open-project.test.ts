// A project named at launch, or by a second launch that the shell turned into
// "bring the running window forward and open this", opens the way the Projects
// panel opens one: running work someone asked for is never cancelled without
// asking, and a failure is told to the person.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { openProjectFromShell, type ShellOpenEffects } from '../src/app/useShellOpenProject'

function effects(replies: Array<{ ok: boolean; error?: Record<string, string>; warnings?: Array<{ code: string; message: string }> }>, answer = false) {
  const calls: Array<{ path: string; cancel: boolean }> = []
  const told: string[] = []
  const asked: string[] = []
  const fx: ShellOpenEffects = {
    open: async (path, cancel) => { calls.push({ path, cancel }); return replies.shift() ?? { ok: true } },
    confirm: async (text) => { asked.push(text); return answer },
    tell: (message) => { told.push(message) },
  }
  return { fx, calls, told, asked }
}

{
  const { fx, calls, told } = effects([{ ok: true, warnings: [{ code: 'format_reduced', message: 'The project format was reduced.' }] }])
  assert.equal(await openProjectFromShell('/p/Talk.cutproj', fx), 'opened')
  assert.deepEqual(calls, [{ path: '/p/Talk.cutproj', cancel: false }])
  assert.deepEqual(told, ['The project format was reduced.'], 'what the open changed is said')
}
{
  const running = { ok: false, error: { code: 'jobs_running', message: 'changing the project would cancel 1 running job(s)', cause: 'running: job_002 (render, 40% done)' } }
  const kept = effects([running], false)
  assert.equal(await openProjectFromShell('/p/Other.cutproj', kept.fx), 'kept', 'Keep working here keeps the export')
  assert.equal(kept.calls.length, 1)
  assert.match(kept.asked[0], /running: job_002/)
  const switched = effects([running, { ok: true }], true)
  assert.equal(await openProjectFromShell('/p/Other.cutproj', switched.fx), 'opened')
  assert.deepEqual(switched.calls.map((call) => call.cancel), [false, true], 'cancelled only after the person agreed')
}
{
  const { fx, told } = effects([{ ok: false, error: { code: 'invalid_args', message: 'this folder is not a ShellX Cut project', suggested_action: 'use project.create for a new one' } }])
  assert.equal(await openProjectFromShell('/p/Nope.cutproj', fx), 'failed')
  assert.match(told[0], /could not be opened: this folder is not a ShellX Cut project/)
}
{
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8')
  assert.match(app, /useShellOpenProject\(\)/, 'the app listens for a launched project')
  const hook = readFileSync(new URL('../src/app/useShellOpenProject.ts', import.meta.url), 'utf8')
  assert.match(hook, /takeLaunchProject\(\)/, 'the first launch is asked once')
  assert.match(hook, /onShellOpenProject\(/, 'a second launch is listened to')
}

console.log('PASS a launched project opens like one opened from Projects')
