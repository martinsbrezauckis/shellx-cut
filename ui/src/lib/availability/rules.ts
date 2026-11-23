import { desktopRules } from './desktopRules'
import { editingRules } from './editingRules'
import { recordSettingsRules } from './recordSettingsRules'
import type { AvailabilityAnswer, AvailabilityFacts, AvailabilityRule, AvailabilitySubject } from './types'

export const rules: readonly AvailabilityRule[] = [...desktopRules, ...editingRules, ...recordSettingsRules]
function ruleFor(id: string): AvailabilityRule {
  const rule = rules.find(rule => rule.id === id)
  if (!rule) throw new Error(`Unknown availability rule: ${id}`)
  return rule
}
export function availabilitySentence(id: string): string { return ruleFor(id).sentence }
export function availability(id: string, facts: Readonly<AvailabilityFacts>, subject: Readonly<AvailabilitySubject> = {}): AvailabilityAnswer {
  const rule = ruleFor(id)
  return rule.absent(facts, subject)
    ? { state: 'absent', family: rule.family, reason: rule.sentence }
    : { state: 'ready' }
}
