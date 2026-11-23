import type { AvailabilityFacts } from './types'

// Legacy optional source slots remain accepted for callers compiled against the
// foundation API, but are deliberately never read. Only system + desktop count.
interface FactSources { desktop?: boolean; system?: unknown; record?: unknown; preview?: unknown; motion?: unknown }
const object = (value: unknown): Record<string, unknown> => value !== null && typeof value === 'object' ? value as Record<string, unknown> : {}

/** The only wire-to-facts projection. An old engine's missing field is unknown. */
export function collectAvailabilityFacts(sources: FactSources): AvailabilityFacts {
  const facts: AvailabilityFacts = {}
  if (sources.desktop !== undefined) facts.desktop = sources.desktop
  const system = object(sources.system)
  if (typeof system.os === 'string') facts.os = system.os
  const fixed = object(system.availability)
  for (const [key, value] of [
    ['pause', fixed.pause], ['camera', fixed.camera], ['liveTimer', fixed.live_timer],
    ['microphoneChoice', fixed.microphone_choice], ['sourcePreview', fixed.source_preview],
    ['managedMotion', fixed.managed_motion],
  ] as const) if (typeof value === 'boolean') facts[key] = value
  return facts
}
