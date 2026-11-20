import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { releaseMediaElement } from '../src/panels/Preview/mediaElementRelease'

// A removed preview <video> must give up its media pipeline at once, so
// repeated mounts (a project switch, poster/live surface changes) never pile up
// live pipelines.
const calls: string[] = []
const element = {
  pause: () => calls.push('pause'),
  removeAttribute: (name: string) => calls.push(`remove:${name}`),
  load: () => calls.push('load'),
} as unknown as HTMLMediaElement
releaseMediaElement(element)
assert.deepEqual(calls, ['pause', 'remove:src', 'load'])

const preview = readFileSync(new URL('../src/panels/Preview/index.tsx', import.meta.url), 'utf8')
const deck = readFileSync(new URL('../src/panels/Preview/BaseVideoDeck.tsx', import.meta.url), 'utf8')
const layers = readFileSync(new URL('../src/panels/Preview/PreviewLayers.tsx', import.meta.url), 'utf8')
// The base track renders through the two-element deck; each deck element
// releases its pipeline when the live stage goes away.
assert.match(preview, /<BaseVideoDeck\b/)
assert.doesNotMatch(preview, /<video\s+ref=/, 'the Preview panel renders no base <video> of its own')
assert.match(deck, /releaseMediaElement\(element\)/)
assert.match(deck, /<video\s+ref=\{setRef\}/)
assert.match(layers, /const releasedRef = useReleasedMediaRef\(ref\)/)
assert.match(layers, /<video\s+ref=\{releasedRef\}/)
// Item 98: the app's own playback audio (monitor mix, meter stems, Listen) is
// private working audio, never a file in the project's exports/ folder.
const mixer = readFileSync(new URL('../src/panels/Mixer/index.tsx', import.meta.url), 'utf8')
const audition = readFileSync(new URL('../src/components/TrackAuditionButton.tsx', import.meta.url), 'utf8')
assert.match(preview, /scratch: `monitor_\$\{buf\}`/)
assert.doesNotMatch(preview, /path: `exports\/_monitor_/)
assert.match(mixer, /scratch: `stem_/)
assert.match(audition, /scratch: `listen_/)
console.log('PASS preview media elements are released on unmount; playback audio is scratch')
