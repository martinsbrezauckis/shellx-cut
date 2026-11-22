import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createZoomMock } from '../src/panels/Review/recordingZoomMock'
test('three automatic zooms, manual survives redo, refresh clears stale', () => {
 const mock = createZoomMock()
 const list = () => mock('screen_record.zoom_list',{clip:'recording'})!.result as any
 assert.equal(list().segments.length,3)
 mock('screen_record.zoom_add',{clip:'recording',at_ms:12000})
 mock('screen_record.zoom_auto',{clip:'recording',keep_manual:true})
 assert.equal(list().segments.filter((s:any) => s.origin === 'manual').length,1)
 assert.equal(list().stale,true)
 mock('screen_record.take_refresh',{clip:'recording'})
 assert.equal(list().stale,false)
})
