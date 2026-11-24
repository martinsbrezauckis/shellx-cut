import { test } from 'node:test'
import assert from 'node:assert/strict'
import { callVerb, type VerbName } from '../src/lib/client'
import { isHumanDeliveryVerb, RECORDING_EXPORT_REFUSED, setDeliveryFeedbackSession } from '../src/topbar/exportRefusal'

test('all human delivery verbs share persistent refusal feedback, but dry-run planning does not', async () => {
  const fetchBefore = globalThis.fetch
  const documentBefore = Object.getOwnPropertyDescriptor(globalThis, 'document')
  const events = new EventTarget()
  const notices: string[] = []
  let reveals = 0
  events.addEventListener('cut:user-action-feedback', e => notices.push((e as CustomEvent).detail.message))
  events.addEventListener(RECORDING_EXPORT_REFUSED, () => reveals++)
  Object.defineProperty(globalThis, 'document', {configurable:true,value:events})
  globalThis.fetch = async () => new Response(JSON.stringify({ok:false,error:{code:'invalid_args',message:'Update video first'}}))
  const routes = ['render.final','render.queue','render.reframe','render.direct','render.qc','render.bundle','export.publish','export.range','export.audio','export.gif','export.frame','export.xml','screen_record.export','jobs.retry']
  try {
    for (const route of routes) {
      assert.equal(isHumanDeliveryVerb(route),true)
      await callVerb(route as VerbName, {} as never)
      assert.match(notices.at(-1)!,/^Export did not start:/)
    }
    assert.equal(reveals,routes.length)
    await callVerb('render.final',{dry_run:true})
    assert.equal(reveals,routes.length)
    assert.equal(notices.length,routes.length)
    await callVerb('export.audio',{scratch:'preview'})
    assert.equal(notices.length,routes.length,'internal preview audio is not a human export')
    globalThis.fetch = async () => new Response(JSON.stringify({ok:false,error:{code:'conflict',message:'Output is read-only.'}}))
    await callVerb('render.final',{})
    assert.equal(notices.at(-1),'Export failed: the export folder is read-only. Choose a writable export folder, then try again.')
    assert.equal(reveals,routes.length)
    const noticeCount=notices.length
    let release!: (response: Response) => void
    globalThis.fetch = () => new Promise(resolve => { release=resolve })
    const oldRequest=callVerb('render.final',{})
    setDeliveryFeedbackSession(1)
    release(new Response(JSON.stringify({ok:false,error:{code:'invalid_args',message:'Update video first'}})))
    await oldRequest
    assert.equal(notices.length,noticeCount,'obsolete project reply cannot show feedback')
  } finally {
    setDeliveryFeedbackSession(0)
    globalThis.fetch = fetchBefore
    if(documentBefore) Object.defineProperty(globalThis,'document',documentBefore)
    else Reflect.deleteProperty(globalThis,'document')
  }
})
