import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { openAboutLink } from '../src/lib/tauri'
import { handleAboutLinkClick } from '../src/panels/Environment/aboutLinkClick'

const aboutSource = readFileSync(new URL('../src/panels/Environment/About.tsx', import.meta.url), 'utf8')
for (const [id, destination] of [
  ['data-cut-about-link="site"', 'site'],
  ['data-cut-about-link="github"', 'github'],
  ['data-cut-about-link="release_notes"', 'release_notes'],
] as const) {
  assert.ok(aboutSource.includes(id),
    `About exposes the ${destination} native link action`)
}
assert.match(aboutSource, /onClick=\{openLink\('site'\)\}/)
assert.match(aboutSource, /onClick=\{openLink\('github'\)\}/)
assert.match(aboutSource, /onClick=\{openLink\('release_notes',/)

const previousWindow = globalThis.window

try {
  Object.defineProperty(globalThis, 'window', { configurable: true, value: {} })
  assert.deepEqual(await openAboutLink('site'), {
    ok: false,
    message: 'Open the desktop app to launch this link in your default browser.',
  }, 'a browser build cannot claim to open an OS browser')

  const calls: Array<{ command: string; args: Record<string, unknown> | undefined }> = []
  let rejectWith: unknown = null
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {
      __TAURI__: {
        core: {
          invoke: async (command: string, args?: Record<string, unknown>) => {
            calls.push({ command, args })
            if (rejectWith !== null) throw rejectWith
          },
        },
      },
    },
  })

  assert.deepEqual(await openAboutLink('release_notes', '0.6.115'), { ok: true })
  assert.deepEqual(calls[0], {
    command: 'open_about_link',
    args: { destination: 'release_notes', version: '0.6.115' },
  }, 'the bridge passes a destination and version, never a caller URL or executable')
  assert.deepEqual(await openAboutLink('github'), { ok: true })
  assert.deepEqual(calls[1].args, { destination: 'github', version: null })

  rejectWith = 'The default browser is unavailable'
  assert.deepEqual(await openAboutLink('site'), {
    ok: false,
    message: 'The default browser is unavailable',
  }, 'native refusal is returned for visible About feedback')

  rejectWith = new Error('IPC unavailable')
  assert.deepEqual(await openAboutLink('site'), {
    ok: false,
    message: 'IPC unavailable',
  }, 'transport failure is returned for visible About feedback')

  let prevented = 0
  const errors: Array<string | null> = []
  const event = { preventDefault: () => { prevented += 1 } }
  await handleAboutLinkClick('site', undefined, event, (message) => errors.push(message), false)
  assert.equal(prevented, 0, 'browser click retains the ordinary anchor behavior')
  assert.deepEqual(errors, [])

  rejectWith = null
  await handleAboutLinkClick('release_notes', '0.6.115', event, (message) => errors.push(message), true)
  assert.equal(prevented, 1, 'desktop click prevents in-webview navigation')
  assert.deepEqual(errors, [null], 'new desktop attempt clears the previous error')
  assert.deepEqual(calls.at(-1)?.args, { destination: 'release_notes', version: '0.6.115' })

  rejectWith = 'The default browser is unavailable'
  await handleAboutLinkClick('github', undefined, event, (message) => errors.push(message), true)
  assert.equal(prevented, 2)
  assert.deepEqual(errors, [null, null, 'The default browser is unavailable'], 'native failure is shown by the About click path')
} finally {
  Object.defineProperty(globalThis, 'window', { configurable: true, value: previousWindow })
}

console.log('PASS About native-browser bridge')
