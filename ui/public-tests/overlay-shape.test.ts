import assert from 'node:assert/strict'
import { overlayBoxStyle, resolveOverlays } from '../src/panels/Preview/composite'
import { effectiveTransform } from '../src/panels/Preview/effectiveTransform'

const rect = { x: 0.1, y: 0.2, scale: 0.25, opacity: 1 }
assert.deepEqual(overlayBoxStyle(rect), { left: '10.0000%', top: '20.0000%', width: '25.0000%', height: '25.0000%', opacity: 1 })
const circle = overlayBoxStyle({ ...rect, shape: 'circle' } as any)
assert.equal(circle.borderRadius, '50%')
assert.equal(circle.aspectRatio, '1')
assert.equal(circle.height, 'auto')
assert.equal(circle.objectFit, 'cover')
const rounded = overlayBoxStyle({ ...rect, shape: 'rounded', radius: 0.2 } as any)
assert.equal(rounded.borderRadius, '20%')
for (const [width, height] of [[320, 240], [320, 180], [180, 320]]) {
  const project = { assets: { camera: { path: '/camera.mp4', proxy: '/letterboxed.mp4', probe: { width, height } } }, tracks: [
    { id: 'base', kind: 'video', clips: [] },
    { id: 'camera', kind: 'video', clips: [{ id: 'c1', asset: 'camera', src_in_ms: 0, src_out_ms: 2000, transform: rect }] },
  ] } as any
  const layer = resolveOverlays(project, 500, 'base', new Set()).overlays[0]
  assert.equal(layer.sourceAspect, width / height)
  assert.equal(layer.storedTransform?.scale, 0.25, 'opening a saved overlay never changes its width')
  if (width / height !== 16 / 9) assert.equal(layer.kind, 'source', 'do not show baked-in proxy bars')
}
for (const output of [16 / 9, 9 / 16]) {
  for (const source of [4 / 3, 16 / 9, 9 / 16]) {
    const style = overlayBoxStyle(rect, source)
    assert.equal(style.width, '25.0000%', 'saved width is preserved')
    assert.equal(style.height, 'auto')
    assert.equal(Number(style.aspectRatio), source)
    const width = output * 1080 * 0.25
    assert.ok(Math.abs(width / Number(style.aspectRatio) - width / source) < 0.001)
  }
}
// Same three times/keys as the engine's multi-window camera regression.
for (const at of [4000, 20000, 34000]) {
  const transform = effectiveTransform({ ...rect, x: 0.836, y: 0.74, scale: 0.124 }, [
    { param: 'pos_x', interp: 'hold', points: [{ t_ms: 0, value: 0.836 }, { t_ms: 16296, value: 0.04 }] },
    { param: 'pos_y', interp: 'hold', points: [{ t_ms: 0, value: 0.74 }, { t_ms: 16296, value: 0.04 }] },
    { param: 'scale', interp: 'hold', points: [{ t_ms: 0, value: 1 }, { t_ms: 32830, value: 1.542 }] },
  ], at)
  const box = overlayBoxStyle(transform, 4 / 3)
  assert.equal(box.width, `${(0.124 * (at > 32830 ? 1.542 : 1) * 100).toFixed(4)}%`)
  assert.equal(box.left, at > 16296 ? '4.0000%' : '83.6000%')
  assert.equal(box.top, at > 16296 ? '4.0000%' : '74.0000%')
  assert.equal(box.aspectRatio, String(4 / 3))
}
assert.equal(overlayBoxStyle({ ...rect, shape: 'circle' } as any, 4 / 3).aspectRatio, '1')
console.log('PASS overlay-shape: source-shaped rectangles preserve saved width; circle/rounded unchanged')
