import { test } from 'node:test'
import assert from 'node:assert/strict'
import { cameraPreset, isTakeCamera, cameraTransform } from '../src/panels/Inspector/cameraSectionModel'

test('camera section matches only the take camera id, not filenames or other take clips', () => {
  const takes = [{ camera_clip: 'cam1', camera_look: {} }]
  assert.equal(isTakeCamera('cam1', takes), true)
  assert.equal(isTakeCamera('screen1', takes), false)
  assert.equal(isTakeCamera('camera.mp4', undefined), false)
  assert.equal(isTakeCamera({ id: 'right-half', recording: { camera: true } }, takes), true)
  assert.equal(isTakeCamera({ id: 'screen', recording: { camera: false } }, takes), false)
  assert.equal(isTakeCamera({ id: 'cam1' }, takes), true)
})
test('baked and legacy compatibility clip ids do not enable the Camera section', () => {
  for (const take of [{ camera_clip: 'baked' }, { camera_clip: 'baked', camera_look: null }]) {
    assert.equal(isTakeCamera('baked', [take]), false)
    assert.equal(isTakeCamera({ id: 'baked' }, [take]), false)
  }
})
test('camera presets use recorder margin and frame aspect for a square bubble', () => {
  assert.deepEqual(cameraPreset('top_left', .2, 1920, 1080), { x: .04, y: .04 })
  const bottom = cameraPreset('bottom_right', .2, 1920, 1080)
  assert.ok(Math.abs(bottom.x - .76) < 1e-9)
  assert.ok(Math.abs(bottom.y - (1 - .04 - .2 * 1920 / 1080)) < 1e-9)
  assert.deepEqual(cameraPreset('centre', .2, 1000, 1000), { x: .4, y: .4 })
})
test('camera edits preserve the entire SET transform and explicit ring removal', () => {
  const stored = { x: .1, y: .2, scale: .2, opacity: .7, shape: 'rounded' as const, radius: .15, ring: { width: .02, colour: '#ffffffe6' } }
  assert.deepEqual(cameraTransform(stored, { x: .04 }), { ...stored, x: .04 })
  assert.equal(cameraTransform(stored, { ring: undefined }).ring, undefined)
})
test('Rectangle clears incompatible ring and radius in the same SET; later shapes do not restore ring', () => {
  const stored = { x: .1, y: .2, scale: .2, opacity: .7, shape: 'rounded' as const,
    radius: .15, ring: { width: .02, colour: '#ffffffe6' } }
  const rectangle = cameraTransform(stored, { shape: 'rect', radius: undefined })
  assert.equal(rectangle.ring, undefined)
  assert.equal(rectangle.radius, undefined)
  assert.deepEqual(JSON.parse(JSON.stringify(rectangle)), { x: .1, y: .2, scale: .2, opacity: .7, shape: 'rect' })
  assert.equal(cameraTransform(rectangle, { shape: 'circle' }).ring, undefined)
  assert.equal(cameraTransform(rectangle, { shape: 'rounded', radius: .18 }).ring, undefined)
  assert.deepEqual(cameraTransform(rectangle, { shape: 'circle', ring: stored.ring }).ring, stored.ring)
})
