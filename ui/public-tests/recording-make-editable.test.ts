import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { OldRecordingZoomSection } from '../src/panels/Inspector/OldRecordingZoomSection'
import { createZoomMock } from '../src/panels/Review/recordingZoomMock'

test('OldRecordingZoomSection renders Make editable action under Zoom card', () => {
  const html = renderToStaticMarkup(createElement(OldRecordingZoomSection, {
    clip: 'c1',
    revision: 'rev1',
  }))
  assert.match(html, /data-cut-section="recording-zoom"/)
  assert.match(html, /data-cut-action="recording-make-editable"/)
  assert.match(html, />Make editable</)
  assert.match(html, /The original video stays\./)
})

test('zoom mock handles screen_record.make_editable and refusals', () => {
  const mock = createZoomMock()
  const reply = mock('screen_record.make_editable', { clip: 'c1' })
  assert.equal(reply?.ok, true)
  const result = reply?.result as { clip?: string; take?: string; segments?: unknown[] }
  assert.equal(result.clip, 'c1')
  assert.ok(result.take)
  assert.equal(result.segments?.length, 3)
  const list = mock('screen_record.zoom_list', { clip: 'c1' })
  assert.equal(list?.ok, true)
})
