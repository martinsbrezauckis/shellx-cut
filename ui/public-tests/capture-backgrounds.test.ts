import assert from 'node:assert/strict'
import {
  backgroundNotes,
  layerOffset,
  layerSize,
  repeatAxes,
  splitLayers,
} from '../src/lib/captureBackgrounds'

// Computed style lists layers at the top level; commas inside gradients do not split.
assert.deepEqual(
  splitLayers('linear-gradient(rgba(0, 0, 0, 0.28), rgba(0, 0, 0, 0.46)), url("http://x/filmstrip/a1.jpg?p=1,2")'),
  ['linear-gradient(rgba(0, 0, 0, 0.28), rgba(0, 0, 0, 0.46))', 'url("http://x/filmstrip/a1.jpg?p=1,2")'],
)
assert.deepEqual(splitLayers('100% 100%, auto 36px'), ['100% 100%', 'auto 36px'])

// auto-by-height keeps the aspect ratio: a 20480x72 strip at 36 px high is 10240 px wide
assert.deepEqual(layerSize('auto 100%', 20480, 72, 64, 36), [10240, 36])
assert.deepEqual(layerSize('20480px 100%', 20480, 72, 600, 50), [20480, 50])
assert.deepEqual(layerSize('100% 100%', 20480, 72, 600, 50), [600, 50])
assert.deepEqual(layerSize('auto', 100, 50, 600, 50), [100, 50])
const cover = layerSize('cover', 100, 50, 200, 200)
assert.equal(cover[1], 200)
assert.equal(layerSize('contain', 100, 50, 200, 200)[0], 200)

assert.equal(layerOffset('-1200px', 600, 20480, 'x'), -1200)
assert.equal(layerOffset('0%', 600, 100, 'x'), 0)
assert.equal(layerOffset('50%', 600, 100, 'x'), 250)
assert.equal(layerOffset('center', 600, 100, 'x'), 250)
assert.equal(layerOffset('bottom', 50, 10, 'y'), 40)

assert.deepEqual(repeatAxes('no-repeat'), [false, false])
assert.deepEqual(repeatAxes('repeat-x'), [true, false])
assert.deepEqual(repeatAxes('repeat'), [true, true])
assert.deepEqual(repeatAxes('repeat no-repeat'), [true, false])

// The notes say how many and which; nothing to say means no note.
assert.deepEqual(backgroundNotes({ redrawn: [], missing: [], skipped: [] }), [])
const notes = backgroundNotes({
  redrawn: ['http://127.0.0.1:6161/filmstrip/a1.jpg?p=2-abc'],
  missing: ['http://127.0.0.1:6161/filmstrip/a2.jpg?p=2-abc', 'http://127.0.0.1:6161/filmstrip/a3.jpg?p=2-abc'],
  skipped: [],
})
assert.equal(notes.length, 2)
assert.match(notes[0], /^1 CSS background image could not be rasterized by this web view's DOM capture and was drawn from the loaded image instead: \/filmstrip\/a1\.jpg/)
assert.match(notes[1], /^2 CSS background images could not be loaded and are missing from this capture: \/filmstrip\/a2\.jpg/)
console.log('capture backgrounds: layers, sizes, offsets and notes')
