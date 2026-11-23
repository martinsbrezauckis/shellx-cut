import type { AvailabilityRule } from './types'

// Owned by the desktop choosers lane; shared by all areas.
export const desktopRules: readonly AvailabilityRule[] = [{
  id: 'shell.desktop', family: 'shell', sentence: 'This action needs the desktop app.',
  absent: facts => facts.desktop === false,
}]
