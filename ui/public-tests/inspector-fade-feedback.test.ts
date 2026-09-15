// Exercise the real FadesSection callbacks and typed client against an inert
// HTTP response. Native state/receipt assertions remain in the product matrix.
import assert from 'node:assert/strict'
import { register } from 'node:module'

register('./lib/css-module-loader.mjs', import.meta.url)
const { default: FadesSection } = await import('../src/panels/Inspector/FadesSection')

const savedFetch = globalThis.fetch
const savedDocument = Object.getOwnPropertyDescriptor(globalThis, 'document')
const events = new EventTarget()
const feedback: string[] = []
let composed = 0
let reply: object | Error = { ok: true, result: { op_id: 'fade-op' } }
const requests: Array<Record<string, unknown>> = []
events.addEventListener('cut:user-action-feedback', event => {
  feedback.push((event as CustomEvent<{ message: string }>).detail.message)
})
events.addEventListener('cut:show-composed', () => { composed += 1 })

try {
  Object.defineProperty(globalThis, 'document', { configurable: true, value: events })
  globalThis.fetch = async (url, init) => {
    assert.match(String(url), /\/api\/verb\/edit[.]fade$/)
    assert.equal(init?.method, 'POST')
    requests.push(JSON.parse(String(init?.body)))
    if (reply instanceof Error) throw reply
    return new Response(JSON.stringify(reply), { status: 200 })
  }
  const props = { clipId: 'selected-clip', fadeInMs: 700, fadeOutMs: 200, isVideo: true, clipDurMs: 5000 }
  const video = FadesSection(props)
  const [fadeIn, fadeOut] = video.props.children

  await fadeIn.props.onCommit(0.5)
  assert.equal(requests.at(-1)?.clip, 'selected-clip')
  assert.equal(requests.at(-1)?.in_ms, 500)
  assert.equal(composed, 1)
  assert.deepEqual(feedback, [])

  reply = { ok: false, error: { code: 'CONFLICT', message: 'Track is locked.', suggested_action: 'Unlock it first.' } }
  await fadeOut.props.onCommit(0.9)
  assert.equal(requests.at(-1)?.out_ms, 900)
  assert.match(feedback.at(-1)!, /Track is locked.*Unlock it first/)
  assert.equal(composed, 1, 'rejected fades do not select a successful composed preview')

  reply = new Error('inert connection failure')
  await video.props.onReset()
  assert.match(feedback.at(-1)!, /local engine is unreachable/)
  assert.equal(requests.at(-1)?.in_ms, 0)
  assert.equal(requests.at(-1)?.out_ms, 0)
  assert.equal(composed, 1)

  reply = { ok: true, result: { op_id: 'audio-fade-op' } }
  const audio = FadesSection({ ...props, isVideo: false })
  await audio.props.children[0].props.onCommit(0.3)
  assert.equal(requests.at(-1)?.in_ms, 300)
  assert.equal(composed, 1, 'audio fade success does not change the video preview')

  reply = { ok: false, error: { message: 'Track is locked.' } }
  await audio.props.onToggleBypass()
  assert.equal(requests.at(-1)?.in_ms, 0)
  assert.equal(requests.at(-1)?.out_ms, 0)
  assert.equal(feedback.at(-1), 'Track is locked.')
} finally {
  globalThis.fetch = savedFetch
  if (savedDocument) Object.defineProperty(globalThis, 'document', savedDocument)
  else Reflect.deleteProperty(globalThis, 'document')
}
console.log('PASS Inspector fade callbacks, failure feedback and preview behavior')
