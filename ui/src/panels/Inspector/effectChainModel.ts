import type { ClipEffect } from '../../lib/client'

export function effectType(effect: ClipEffect): ClipEffect['type'] {
  return effect.type
}

export function toggleClipEffect(effects: ClipEffect[], effect: ClipEffect): ClipEffect[] {
  const type = effectType(effect)
  return effects.some((item) => effectType(item) === type)
    ? effects.filter((item) => effectType(item) !== type)
    : [...effects, effect]
}

export function moveClipEffect(effects: ClipEffect[], index: number, delta: -1 | 1): ClipEffect[] {
  const target = index + delta
  if (index < 0 || index >= effects.length || target < 0 || target >= effects.length) return effects
  const next = [...effects]
  const [effect] = next.splice(index, 1)
  if (!effect) return effects
  next.splice(target, 0, effect)
  return next
}

export function effectParameterSummary(effect: ClipEffect): string {
  const entries = Object.entries(effect).filter(([key, value]) => key !== 'type' && value != null)
  if (entries.length === 0) return 'default'
  return entries
    .map(([key, value]) => `${key.replaceAll('_', ' ')} ${String(value)}`)
    .join(' · ')
}
