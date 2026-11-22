// Mirrors cut_core::KfInterp::sample and keyframe_window::points. Clip-local
// time is timeline time since the laid clip start, NOT source time or speed.
import type { ClipTransform, Keyframe, KfInterp } from '../../lib/client'

export type FullTransform = ClipTransform & { opacity: number }

export function easeFraction(interp: KfInterp, fraction: number): number {
  const f = Math.max(0, Math.min(1, fraction))
  if (interp === 'hold') return 0
  if (interp === 'linear') return f
  const c1 = 1.70158, c2 = c1 * 1.525
  if (interp === 'ease_in_out_back') {
    const g = f < .5 ? 2 * f : 2 * f - 2
    return f < .5 ? g * g * ((c2 + 1) * g - c2) / 2 : (g * g * ((c2 + 1) * g + c2) + 2) / 2
  }
  if (interp === 'ease_in_out_elastic') {
    if (f === 0 || f === 1) return f
    const c5 = 2 * Math.PI / 4.5
    return f < .5 ? -(2 ** (20 * f - 10) * Math.sin((20 * f - 11.125) * c5)) / 2
      : 2 ** (-20 * f + 10) * Math.sin((20 * f - 11.125) * c5) / 2 + 1
  }
  const both = interp.startsWith('ease_in_out_')
  const out = !both && interp.startsWith('ease_out_')
  const reverse = out || (both && f >= .5)
  const x = both ? (reverse ? 2 - 2 * f : 2 * f) : out ? 1 - f : f
  let value: number
  switch (interp.split('_').at(-1)) {
    case 'quad': value = x * x; break
    case 'cubic': value = x ** 3; break
    case 'expo': value = x === 0 ? 0 : 2 ** (10 * x - 10); break
    case 'back': value = (c1 + 1) * x ** 3 - c1 * x * x; break
    case 'elastic': value = x === 0 || x === 1 ? x
      : -(2 ** (10 * x - 10) * Math.sin((10 * x - 10.75) * (2 * Math.PI / 3))); break
    case 'bounce': {
      let b = 1 - x
      const n = 7.5625, d = 2.75
      value = 1 - (b < 1 / d ? n * b * b : b < 2 / d ? n * (b -= 1.5 / d) * b + .75
        : b < 2.5 / d ? n * (b -= 2.25 / d) * b + .9375 : n * (b -= 2.625 / d) * b + .984375)
      break
    }
    default: value = x
  }
  return both ? (reverse ? 1 - value / 2 : value / 2) : reverse ? 1 - value : value
}

export function keyframeValue(key: Keyframe, atMs: number): number | undefined {
  if (!key.points.length) return undefined
  // A cut's displayed zero key is NOT a new easing origin: retain its lead-in.
  const points = key.points.map((p, i) => i === 0 && key.lead_in ? key.lead_in : p)
  if (atMs < points[0].t_ms) return points[0].value
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1], b = points[i]
    if (atMs < b.t_ms) return a.value + (b.value - a.value)
      * easeFraction(key.interp ?? 'linear', (atMs - a.t_ms) / (b.t_ms - a.t_ms))
  }
  return points[points.length - 1].value
}

export function effectiveTransform(stored: FullTransform, keys: readonly Keyframe[], atMs: number): FullTransform {
  const next = { ...stored }
  for (const key of keys) {
    const value = keyframeValue(key, atMs)
    if (value === undefined) continue
    if (key.param === 'pos_x') next.x = value
    if (key.param === 'pos_y') next.y = value
    if (key.param === 'scale') next.scale = stored.scale * value
  }
  return next
}

/** Ordinary keyed positions still override static hand edits. Camera hand-place
 * removes position keys in the engine. Scale keys remain multiplicative for
 * either path: convert the visible resize back to the base Size control value. */
export function dragTransform(stored: FullTransform, effective: FullTransform, next: FullTransform,
  camera: boolean, resize: boolean): FullTransform {
  return {
    ...next,
    x: camera ? (resize ? stored.x : next.x) : stored.x + next.x - effective.x,
    y: camera ? (resize ? stored.y : next.y) : stored.y + next.y - effective.y,
    // Preserve untouched bases exactly: engine hand-place compares x/y, and a
    // move must not round or double-apply the recorded size multiplier.
    scale: next.scale === effective.scale ? stored.scale : stored.scale * next.scale / effective.scale,
  }
}
