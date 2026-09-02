// Record pointer-position disclosure contract. Run with `npx tsx`.

import { cursorCorrelationLabel, type CursorCorrelation } from '../src/panels/Record/studioTypes'

function correlation(
  state: CursorCorrelation['state'],
  approximate_clicks = 0,
  unavailable_clicks = 0,
): CursorCorrelation {
  return {
    source: 'wayland_pipewire_metadata',
    state,
    exact_clicks: 0,
    approximate_clicks,
    unavailable_clicks,
  }
}

const assertions: Array<[unknown, unknown, string]> = [
  [cursorCorrelationLabel(correlation('exact')), 'Pointer positions accurate', 'exact is disclosed'],
  [cursorCorrelationLabel(correlation('approximate', 1)), '1 pointer position approximate', 'one approximate click is singular'],
  [cursorCorrelationLabel(correlation('approximate', 2)), '2 pointer positions approximate', 'multiple approximate clicks are counted'],
  [cursorCorrelationLabel(correlation('approximate', 0, 1)), '1 pointer position unavailable', 'partial capture does not hide unavailable positions'],
  [cursorCorrelationLabel(correlation('unavailable')), 'Pointer positions unavailable', 'unavailable is never presented as exact'],
  [cursorCorrelationLabel(null), 'Pointer positions unavailable', 'missing legacy receipt stays truthful'],
]

for (const [actual, expected, label] of assertions) {
  if (actual !== expected) throw new Error(`${label}: got ${actual}, want ${expected}`)
  console.log(`PASS ${label}`)
}
