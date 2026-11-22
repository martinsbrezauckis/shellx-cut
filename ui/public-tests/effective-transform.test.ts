import { test } from 'node:test'
import assert from 'node:assert/strict'
import { resolveOverlays } from '../src/panels/Preview/composite'
import type { Project, KfInterp, Keyframe } from '../src/lib/client'
import { keyframeValue, easeFraction, effectiveTransform, dragTransform } from '../src/panels/Preview/effectiveTransform'

const close = (actual: number | undefined, expected: number) => assert.ok(actual !== undefined && Math.abs(actual - expected) < 1e-10, `${actual} != ${expected}`)
const stored = { x: .76, y: .6, scale: .2, opacity: 1, shape: 'circle' as const }
const keys = [
  { param: 'pos_x' as const, interp: 'hold' as const, points: [{ t_ms: 0, value: .76 }, { t_ms: 1000, value: .04 }] },
  { param: 'scale' as const, interp: 'linear' as const, points: [{ t_ms: 0, value: 1 }, { t_ms: 2000, value: .5 }] },
]
test('overlay geometry at the playhead overrides static position and multiplies base scale in clip-local time', () => {
  const project = { settings: {}, assets: { a: { path: 'camera.png' } }, tracks: [
    { id: 'v1', kind: 'video', clips: [] },
    { id: 'v2', kind: 'video', clips: [{ kind: 'gap', duration_ms: 500 },
      { id: 'c', asset: 'a', src_in_ms: 9000, src_out_ms: 15000, speed: 2, transform: stored, keyframes: keys }] },
  ] } as unknown as Project
  const [layer] = resolveOverlays(project, 1500, 'v1', new Set()).overlays
  assert.equal(layer.transform.x, .04)
  assert.equal(layer.transform.y, .6)
  assert.ok(Math.abs(layer.transform.scale - .15) < 1e-9)
})
test('hold and linear clamp at ends, hold steps at exact keys, default interpolation is linear', () => {
  const key: Keyframe = { param: 'pos_x', points: [{ t_ms: 500, value: .1 }, { t_ms: 2500, value: .6 }] }
  close(keyframeValue(key, -1), .1)
  close(keyframeValue(key, 1500), .35)
  close(keyframeValue({ ...key, interp: 'hold' }, 2499), .1)
  close(keyframeValue({ ...key, interp: 'hold' }, 2500), .6)
  close(keyframeValue(key, 9999), .6)
  assert.equal(keyframeValue({ ...key, points: [] }, 100), undefined)
})
test('engine KfInterp known_midpoints tables and every eased endpoint', () => {
  // app/core/src/types.rs::easing_tests::known_midpoints (Penner reference).
  for (const [interp, expected] of Object.entries({ ease_in_quad: .25, ease_out_quad: .75,
    ease_in_cubic: .125, ease_in_out_cubic: .5, ease_in_out_quad: .5 })) {
    close(easeFraction(interp as KfInterp, .5), expected)
  }
  for (const family of ['quad', 'cubic', 'expo', 'back', 'elastic', 'bounce']) {
    for (const mode of ['in', 'out', 'in_out']) {
      const interp = `ease_${mode}_${family}` as KfInterp
      close(easeFraction(interp, 0), 0)
      close(easeFraction(interp, 1), 1)
      assert.ok(Number.isFinite(easeFraction(interp, .37)))
    }
  }
  assert.ok(easeFraction('ease_out_back', .6) > 1, 'overshoot must not be clamped away')
})
test('cut lead-in retains easing phase (engine keyframe_cut.rs fixture and split at 1000)', () => {
  const key: Keyframe = { param: 'pos_x', interp: 'ease_in_quad',
    lead_in: { t_ms: -500, value: .1 }, points: [{ t_ms: 0, value: .13125 }, { t_ms: 1500, value: .6 }] }
  close(keyframeValue(key, 0), .13125)
  close(keyframeValue(key, 500), .225)
  close(keyframeValue(key, 1500), .6)
  // Same engine fixture using ease-in cubic: .1 + .5 * (.5 cubed).
  close(keyframeValue({ ...key, interp: 'ease_in_cubic' }, 500), .1625)
})
test('camera move commits dropped position but base scale; resize scales both stretches without dropping movement', () => {
  const effective = effectiveTransform(stored, keys, 1000)
  const moved = dragTransform(stored, effective, { ...effective, x: .14, y: .3 }, true, false)
  close(moved.x, .14); close(moved.y, .3); close(moved.scale, .2)
  const resized = dragTransform(stored, effective, { ...effective, scale: .18 }, true, true)
  close(resized.x, stored.x); close(resized.y, stored.y); close(resized.scale, .24)
  const ordinary = dragTransform(stored, effective, { ...effective, x: .14 }, false, false)
  close(ordinary.x, .86); close(ordinary.scale, .2)
})
