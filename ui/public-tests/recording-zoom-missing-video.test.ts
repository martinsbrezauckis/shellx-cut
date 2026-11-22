import { test } from 'node:test'
import assert from 'node:assert/strict'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { RecordingZoomRefresh } from '../src/panels/Inspector/RecordingZoomParts'

test('missing video offers rebuild even when the zoom plan is not stale', () => {
  const html = renderToStaticMarkup(createElement(RecordingZoomRefresh, {
    stale: false, missingVideo: true, busy: false, job: null, progress: 0, refresh() {},
  }))
  assert.match(html, /Video file missing\. Update video rebuilds it\./)
  assert.match(html, /Update video<\/button>/)
  assert.doesNotMatch(html, /Changes not in the video yet/)
})
