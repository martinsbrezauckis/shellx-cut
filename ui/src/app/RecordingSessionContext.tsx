import { createContext, useContext, type ReactNode } from 'react'
import type { RecordingSession } from './useRecordingSession'

const Context = createContext<RecordingSession | null>(null)

export function RecordingSessionProvider({ session, children }: { session: RecordingSession; children: ReactNode }) {
  return <Context.Provider value={session}>{children}</Context.Provider>
}

export function useAppRecordingSession(): RecordingSession {
  const session = useContext(Context)
  if (!session) throw new Error('RecordingSessionProvider is missing')
  return session
}
