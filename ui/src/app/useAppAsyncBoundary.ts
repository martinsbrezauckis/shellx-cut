import { useCallback, useEffect, useMemo, useRef } from 'react'

interface RequestToken { lifetime: number; sequence: number }

/** App-owned async work may finish after a newer request or an unmount. */
export function useAppAsyncBoundary() {
  const state = useRef({ alive: false, lifetime: 0, resync: 0, doctor: 0 })
  const historyTail = useRef<Promise<void>>(Promise.resolve())

  useEffect(() => {
    state.current.alive = true
    state.current.lifetime += 1
    return () => {
      state.current.alive = false
      state.current.lifetime += 1
      state.current.resync += 1
      state.current.doctor += 1
    }
  }, [])

  const captureOwner = useCallback(() => state.current.lifetime, [])
  const isCurrentOwner = useCallback((lifetime: number) => state.current.alive && lifetime === state.current.lifetime, [])
  const beginResync = useCallback((): RequestToken => ({
    lifetime: state.current.lifetime,
    sequence: ++state.current.resync,
  }), [])
  const isCurrentResync = useCallback((token: RequestToken) => (
    state.current.alive && token.lifetime === state.current.lifetime && token.sequence === state.current.resync
  ), [])
  const beginDoctor = useCallback((): RequestToken => ({
    lifetime: state.current.lifetime,
    sequence: ++state.current.doctor,
  }), [])
  const supersedeDoctor = useCallback(() => { state.current.doctor += 1 }, [])
  const isCurrentDoctor = useCallback((token: RequestToken) => (
    state.current.alive && token.lifetime === state.current.lifetime && token.sequence === state.current.doctor
  ), [])

  /** Attach rejection handling immediately. The stored tail always settles so
   * one failed navigation cannot poison the next serialized action. */
  const enqueueHistory = useCallback((isCurrentProject: () => boolean, action: () => Promise<void>, onError: () => void) => {
    const lifetime = state.current.lifetime
    const task = historyTail.current.then(async () => {
      if (!state.current.alive || lifetime !== state.current.lifetime || !isCurrentProject()) return
      await action()
    })
    historyTail.current = task.then(
      () => undefined,
      () => {
        if (!state.current.alive || lifetime !== state.current.lifetime || !isCurrentProject()) return
        try { onError() } catch { /* feedback must never reject the queue */ }
      },
    )
  }, [])

  return useMemo(() => ({ captureOwner, isCurrentOwner, beginResync, isCurrentResync, beginDoctor, supersedeDoctor, isCurrentDoctor, enqueueHistory }),
    [captureOwner, isCurrentOwner, beginResync, isCurrentResync, beginDoctor, supersedeDoctor, isCurrentDoctor, enqueueHistory])
}
