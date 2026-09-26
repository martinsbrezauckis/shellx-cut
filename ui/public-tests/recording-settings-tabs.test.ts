import { strict as assert } from 'node:assert'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { RecordingSettingsTabs } from '../src/panels/Record/RecordingSettingsTabs'
import { RecordingVideoTimerControl } from '../src/panels/Record/RecordingVideoTimerControl'

const html = renderToStaticMarkup(createElement(RecordingSettingsTabs, {
  pages: {
    camera: 'Camera fields',
    background: 'Background fields',
    timer: 'Timer fields',
    timing: 'Capture timing fields',
    quality: 'Quality fields',
  },
}))

const tabs = [...html.matchAll(/<button[^>]*role="tab"[^>]*>/g)].map(([tag]) => tag)
const panels = [...html.matchAll(/<section[^>]*role="tabpanel"[^>]*>/g)].map(([tag]) => tag)
assert.equal(tabs.length, 5)
assert.equal(panels.length, 5)
assert.deepEqual(tabs.map((tag) => tag.match(/data-cut-rec-settings-tab="([^"]+)"/)?.[1]),
  ['camera', 'background', 'timer', 'timing', 'quality'])
assert.match(tabs[0], /aria-selected="true"/)
assert.match(tabs[0], /tabindex="0"/)
assert.equal(tabs.slice(1).filter((tag) => tag.includes('aria-selected="false"')).length, 4)
assert.equal(panels.filter((tag) => tag.includes(' hidden=""')).length, 4)
for (const [index, tab] of tabs.entries()) {
  const panelId = tab.match(/aria-controls="([^"]+)"/)?.[1]
  assert.ok(panelId, 'every tab points to a panel')
  assert.ok(panels[index].includes(`id="${panelId}"`))
}
assert.ok(html.includes('Camera fields'))
assert.ok(html.includes('Quality fields'), 'inactive page content stays mounted so drafts survive tab changes')

const timerBase = {
  status: { state: 'not_started' as const, message: 'Starts with recording.' },
  recording: false,
  disabled: false,
  liveSupported: true,
  onTimerChange: () => {},
  onTimerControl: () => {},
}
const timerHtml = renderToStaticMarkup(createElement(RecordingVideoTimerControl, {
  ...timerBase, timer: { kind: 'elapsed' },
}))
assert.match(timerHtml, /data-cut-rec-video-timer-mode="elapsed"[^>]*aria-pressed="true"/)
assert.match(timerHtml, /Active on-video timer: Count up/)
assert.match(timerHtml, /Apply timer/)
assert.doesNotMatch(timerHtml, /data-cut-rec-scene=/, 'timer settings contain no scene cards')

const liveTimerHtml = renderToStaticMarkup(createElement(RecordingVideoTimerControl, {
  ...timerBase, timer: { kind: 'elapsed' }, recording: true, liveSupported: false,
}))
assert.match(liveTimerHtml, /data-cut-rec-video-timer-action="pause"[^>]*disabled/)
assert.match(liveTimerHtml, /Live video timer controls are unavailable/)

console.log('PASS Recording settings tabs and dedicated video timer contract')
