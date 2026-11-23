import { createContext, useContext, useRef, type ReactNode } from 'react'
import { isTauri } from '../tauri'
import { collectAvailabilityFacts } from './facts'
import { availability } from './rules'
import type { AvailabilityFacts, AvailabilitySubject } from './types'

const FactsContext = createContext<Readonly<AvailabilityFacts>>({})

/** Explicit facts make tests independent of window globals and engine transport. */
export function AvailabilityProvider({ facts, children }: { facts: Readonly<AvailabilityFacts>; children?: ReactNode }) {
  return <FactsContext.Provider value={facts}>{children}</FactsContext.Provider>
}

/** One app-wide collector: no requests, device enumeration or refresh reset.
 * Build/system facts cannot change during this process; retain known values if
 * a report is temporarily absent or comes from an older engine. */
export function AppAvailabilityProvider({ doctor, children }: { doctor: unknown; children?: ReactNode }) {
  const known = useRef<AvailabilityFacts>({})
  const incoming = collectAvailabilityFacts({ desktop: isTauri(), system: doctor })
  const facts = { ...known.current, ...incoming }
  known.current = facts
  return <AvailabilityProvider facts={facts}>{children}</AvailabilityProvider>
}

export function useAvailability(rule: string, subject: AvailabilitySubject = {}) {
  return availability(rule, useContext(FactsContext), subject)
}

/** Fragment only: absent controls leave no element or space. A whole empty group
 * may supply a sentence; an unchangeable single choice may supply its value. */
export function Availability({ rule, subject, replacement = null, children }: {
  rule: string; subject?: AvailabilitySubject; replacement?: ReactNode; children?: ReactNode
}) {
  const answer = useAvailability(rule, subject)
  // not-now/busy deliberately preserve today's controls until the next release.
  return <>{answer.state === 'absent' ? replacement : children}</>
}
