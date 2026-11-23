// An older project with an odd frame size: the person is told before an
// export, with one button that changes the project to the even size below.
// Renders the real notice and checks the helpers and the refusal hand-off
// from callVerb (any surface's refused export reopens the notice).

import { strict as assert } from 'node:assert'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import OddProjectSizeNotice from '../src/components/OddProjectSizeNotice'
import { callVerb } from '../src/lib/client'
import { errorText } from '../src/lib/jobFailureText'

// Sentence formatting must not rewrite case-sensitive paths or command names.
for (const [message, expected] of [
  ['x', 'X.'], ['x.', 'X.'], ['x..', 'X.'],
  ['clips/holiday.mov is missing', 'clips/holiday.mov is missing.'],
  ['system.set_ffmpeg', 'system.set_ffmpeg.'],
  ['ffmpeg_missing', 'ffmpeg_missing.'],
]) {
  assert.equal(errorText({ message }, 'Export did not start.'), expected)
}
import { runVideoPreflightAction } from '../src/topbar/videoPreflight'
import {
  changeSizeLabel,
  evenSizeFromAction,
  ODD_PROJECT_SIZE,
  ODD_PROJECT_SIZE_EVENT,
  oddProjectSize,
  oddSizeMessage,
} from '../src/lib/oddProjectSize'

// The helpers.
const odd = oddProjectSize({ width: 641, height: 361 })
assert.deepEqual(odd, { width: 641, height: 361, evenWidth: 640, evenHeight: 360 })
assert.equal(oddProjectSize({ width: 1920, height: 1080 }), null)
assert.equal(oddProjectSize(null), null)
assert.equal(changeSizeLabel(odd!), 'Change project to 640x360')
assert.equal(
  oddSizeMessage(odd!),
  'This project is 641x361, but video exports need an even width and height. Change it to 640x360 before exporting.',
)
assert.deepEqual(evenSizeFromAction('project.format {"width":640,"height":360}'), { evenWidth: 640, evenHeight: 360 })
assert.equal(evenSizeFromAction('use 640x360'), null)
assert.equal(evenSizeFromAction('project.format {"width":641,"height":360}'), null, 'never an odd target')

// On opening an odd project the notice is there, with exactly one fix button.
const html = renderToStaticMarkup(createElement(OddProjectSizeNotice, { settings: { width: 641, height: 361 } }))
assert.match(html, /data-cut-odd-size-notice="open"/)
assert.match(html, /This project is 641x361, but video exports need an even width and height\./)
assert.equal((html.match(/data-cut-action="odd-size-change"/g) ?? []).length, 1)
assert.match(html, /<button type="button" data-cut-action="odd-size-change">Change project to 640x360<\/button>/)
assert.match(html, /data-cut-action="odd-size-dismiss"/)
// An even project shows nothing.
assert.equal(renderToStaticMarkup(createElement(OddProjectSizeNotice, { settings: { width: 640, height: 360 } })), '')

// A refused export, from any surface, reaches the notice through callVerb.
const seen: Array<{ type: string; detail: unknown }> = []
const g = globalThis as unknown as Record<string, unknown>
g.document = { dispatchEvent: (event: { type: string; detail: unknown }) => { seen.push(event); return true } }
g.CustomEvent = class { type: string; detail: unknown; constructor(type: string, init?: { detail?: unknown }) { this.type = type; this.detail = init?.detail } }
const refusal = {
  ok: false,
  error: {
    code: ODD_PROJECT_SIZE,
    message: 'This project is 641x361, but video exports need an even width and height. Change the project to 640x360',
    cause: 'video is exported as 4:2:0',
    suggested_action: 'project.format {"width":640,"height":360}',
  },
}
g.fetch = async () => ({ json: async () => refusal })
const r = await callVerb('render.final', {})
assert.equal(r.ok, false)
assert.equal(seen.length, 1)
assert.equal(seen[0].type, ODD_PROJECT_SIZE_EVENT)
assert.equal((seen[0].detail as { code: string }).code, ODD_PROJECT_SIZE)
// Other refusals get general persistent feedback, not another odd-size notice.
g.fetch = async () => ({ json: async () => ({ ok: false, error: { code: 'invalid_args', message: 'x', cause: 'y' } }) })
await callVerb('render.final', {})
assert.equal(seen.filter(event => event.type === ODD_PROJECT_SIZE_EVENT).length, 1)
assert.equal(seen.at(-1)?.type, 'cut:user-action-feedback')
assert.equal((seen.at(-1)?.detail as { message: string }).message, 'export failed: x')

// The Render button's preflight stops at a refused plan: no quality dialog,
// no "Export anyway", the action is not started, the sentence is shown.
const notes: string[] = []
let started = false
let warned = false
const status = await runVideoPreflightAction('rendering video', async () => { started = true }, {
  ffmpegMissing: false,
  check: async () => ({ ok: false, error: refusal.error, blocked: `Not rendered: ${refusal.error.message}.` }),
  showWarning: () => { warned = true },
  note: (message) => { notes.push(message) },
})
assert.equal(status, 'blocked')
assert.equal(started, false)
assert.equal(warned, false)
assert.deepEqual(notes, [`Not rendered: ${refusal.error.message}.`])

console.log('odd project size notice: ok')
