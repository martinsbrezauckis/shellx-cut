import { useCallback, useEffect, useRef, useState } from 'react'
import {
  countdownRemainingSeconds,
  RecordingCountdownGuard,
  type RecordingCountdownSeconds,
} from './recordingCountdown'

interface UseRecordingCountdownOptions {
  validate: () => string | null
  onPrepare: () => void
  /** Synchronous ownership admission before the visual countdown commits. */
  onCountdownStart: () => void
  /** Synchronous ownership admission before the zero handoff invokes Start. */
  onStarting: () => void
  onStart: () => Promise<void>
  onInvalid: (message: string) => void
  onCancel: () => void
}

/**
 * Owns only setup time. Its guarded zero handoff invokes `onStart` once; until
 * then it has no capture id, project creation, native reservation, or elapsed
 * recording clock to cancel.
 */
export function useRecordingCountdown({
  validate,
  onPrepare,
  onCountdownStart,
  onStarting,
  onStart,
  onInvalid,
  onCancel,
}: UseRecordingCountdownOptions) {
  const [seconds, setSeconds] = useState<RecordingCountdownSeconds>(3)
  const [remaining, setRemaining] = useState(0)
  const [active, setActive] = useState(false)
  const [starting, setStarting] = useState(false)
  const tickRef = useRef<number | null>(null)
  const guardRef = useRef(new RecordingCountdownGuard())
  const handedOffRef = useRef(false)
  // A Doctor result or project change can land while the visible countdown is
  // running. The timer itself belongs to the render that began it, so read the
  // latest admission and Start callbacks at zero rather than letting that old
  // closure create a capture with stale setup or project state.
  const validateRef = useRef(validate)
  const onInvalidRef = useRef(onInvalid)
  const onStartingRef = useRef(onStarting)
  const onStartRef = useRef(onStart)
  validateRef.current = validate
  onInvalidRef.current = onInvalid
  onStartingRef.current = onStarting
  onStartRef.current = onStart
  // State commits after an event handler returns. These refs close the window
  // where a second click/F9 could otherwise submit another Start before the
  // first request has changed the visible phase.
  const activeRef = useRef(false)
  const startingRef = useRef(false)

  const clearTick = useCallback(() => {
    if (tickRef.current !== null) window.clearInterval(tickRef.current)
    tickRef.current = null
  }, [])

  const beginStart = useCallback(async () => {
    if (startingRef.current) return
    startingRef.current = true
    onStartingRef.current()
    setStarting(true)
    try {
      await onStartRef.current()
    } finally {
      startingRef.current = false
      setStarting(false)
    }
  }, [])

  const cancel = useCallback(() => {
    if (!activeRef.current || handedOffRef.current) return
    guardRef.current.cancel()
    activeRef.current = false
    handedOffRef.current = false
    clearTick()
    setRemaining(0)
    setActive(false)
    onCancel()
  }, [clearTick, onCancel])

  const requestStart = useCallback(() => {
    if (activeRef.current || handedOffRef.current || startingRef.current) return
    const error = validateRef.current()
    if (error) {
      onInvalidRef.current(error)
      return
    }
    onPrepare()
    if (seconds === 0) {
      void beginStart()
      return
    }

    const generation = guardRef.current.begin()
    handedOffRef.current = false
    activeRef.current = true
    onCountdownStart()
    const deadline = Date.now() + seconds * 1_000
    clearTick()
    setRemaining(seconds)
    setActive(true)
    const tick = () => {
      if (!guardRef.current.isCurrent(generation)) return
      const nextRemaining = countdownRemainingSeconds(deadline)
      setRemaining(nextRemaining)
      if (nextRemaining !== 0) return
      clearTick()
      // `handoff` is the only transition into capture: an earlier Cancel
      // invalidates it, while repeated queued ticks cannot start twice.
      guardRef.current.handoff(generation, () => {
        handedOffRef.current = true
        activeRef.current = false
        setActive(false)
        const handoffError = validateRef.current()
        if (handoffError) {
          handedOffRef.current = false
          onInvalidRef.current(handoffError)
          return
        }
        void beginStart().finally(() => {
          handedOffRef.current = false
        })
      })
    }
    tick()
    tickRef.current = window.setInterval(tick, 100)
  }, [beginStart, clearTick, onCountdownStart, onPrepare, seconds])

  useEffect(() => () => {
    guardRef.current.cancel()
    handedOffRef.current = false
    activeRef.current = false
    startingRef.current = false
    clearTick()
  }, [clearTick])

  return {
    seconds,
    setSeconds,
    remaining,
    active,
    starting,
    requestStart,
    cancel,
  }
}
