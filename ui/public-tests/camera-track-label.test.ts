import { test } from 'node:test'
import assert from 'node:assert/strict'
import { trackPresentation } from '../src/panels/Timeline/trackControlsModel'

test('Camera track uses a friendly label and camera icon; explicit names win', () => {
  const track = { id: 'v27', kind: 'video', clips: [{ id: 'cam1' }] }
  assert.deepEqual(trackPresentation(track, [{ camera_clip: 'cam1', camera_look: {} }]), { label: 'Camera', camera: true })
  assert.deepEqual(trackPresentation({ ...track, name: 'Presenter' }, [{ camera_clip: 'cam1', camera_look: {} }]), { label: 'Presenter', camera: true })
  assert.deepEqual(trackPresentation(track), { label: 'v27', camera: false })
})

test('baked compatibility track keeps its raw id and has no camera icon', () => {
  const id = 'v_camera_0123456789abcdef0123456789abcdef'
  const track = { id, kind: 'video', visible: false, clips: [{ id: 'baked' }] }
  for (const take of [{ camera_clip: 'baked' }, { camera_clip: 'baked', camera_look: null }]) {
    assert.deepEqual(trackPresentation(track, [take]), { label: id, camera: false })
  }
})

test('a track with the recorder\'s Camera id is a Camera track even when its first clip was cut away', () => {
  const id = 'Camera_0123456789abcdef0123456789abcdef'
  assert.deepEqual(trackPresentation({ id, kind: 'video', clips: [] }), { label: 'Camera', camera: true })
  const legacy = 'v_camera_0123456789abcdef0123456789abcdef'
  assert.deepEqual(trackPresentation({ id: legacy, kind: 'video', clips: [] }), { label: legacy, camera: false })
})
