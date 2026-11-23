/** Missing facts are unknown, never a definite no. No readiness/setup facts here. */
export interface AvailabilityFacts {
  desktop?: boolean
  os?: string
  pause?: boolean
  camera?: boolean
  microphoneChoice?: boolean
  liveTimer?: boolean
  sourcePreview?: boolean
  managedMotion?: boolean
}
export type AvailabilityAnswer =
  | { state: 'ready' }
  | { state: 'not-now'; reason: string }
  | { state: 'busy'; reason: string }
  | { state: 'absent'; family: 'kind' | 'system' | 'shell'; reason: string }

/** Local, explicit subject; do not turn no selection or an unprobed stream into false. */
export interface AvailabilitySubject {
  selectedKinds?: readonly ('video' | 'audio' | 'caption' | 'still' | 'title' | 'shape')[]
  streams?: { audio?: boolean; video?: boolean }
  outputShape?: string
  article?: { featureId?: string; editorTarget?: boolean }
  operation?: { timelinePosition?: number | null }
  embeddedManual?: boolean
}
export interface AvailabilityRule {
  id: string
  family: 'kind' | 'system' | 'shell'
  sentence: string
  /** Return true ONLY for definite inapplicability. */
  absent: (facts: Readonly<AvailabilityFacts>, subject: Readonly<AvailabilitySubject>) => boolean
}
