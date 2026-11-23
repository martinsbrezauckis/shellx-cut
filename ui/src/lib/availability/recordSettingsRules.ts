import type { AvailabilityRule } from './types'

// Owned by the Record/Settings lane. These are build facts, NOT device readiness.
export const recordSettingsRules: readonly AvailabilityRule[] = [
  { id: 'record.pause', family: 'system', sentence: 'Pause and resume are not available on this system.', absent: f => f.pause === false },
  { id: 'record.camera', family: 'system', sentence: 'Camera recording is not available on this system.', absent: f => f.camera === false },
  { id: 'record.microphone-choice', family: 'system', sentence: 'This system uses the default microphone.', absent: f => f.microphoneChoice === false },
  { id: 'record.live-timer', family: 'system', sentence: 'The on-video timer is unavailable on this system.', absent: f => f.liveTimer === false },
  { id: 'record.source-preview', family: 'system', sentence: 'Source preview is not available on this system.', absent: f => f.sourcePreview === false },
  { id: 'settings.motion-managed', family: 'system', sentence: 'Motion graphics is optional. Cut cannot install it yet.', absent: f => f.managedMotion === false },
]
