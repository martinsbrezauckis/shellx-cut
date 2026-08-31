import { useCallback, useEffect, useRef, useState } from 'react'
import {
  countdownRemainingSeconds,
  RecordingCountdownGuard,
  type RecordingCountdownSeconds,
} from './recordingCountdown'

interface UseRecordingCountdownOptions {
  validate: () => string | null
  onPrepare: () => void
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
  onStart,
  onInvalid,
  onCancel,
}: UseRecordingCountdownOptions) {
  const [seconds, setSeconds] = useState<RecordingCountdownSeconds>(3)
  const [remaining, setRemaining] = useState(0)
  const [active, setActive] = useState(false)
  const tickRef = useRef<number | null>(null)
  const guardRef = useRef(new RecordingCountdownGuard())
  const handedOffRef = useRef(false)

  const clearTick = useCallback(() => {
    if (tickRef.current !== null) window.clearInterval(tickRef.current)
    tickRef.current = null
  }, [])

  const cancel = useCallback(() => {
    if (!active || handedOffRef.current) return
    guardRef.current.cancel()
    clearTick()
    setRemaining(0)
    setActive(false)
    onCancel()
  }, [active, clearTick, onCancel])

  const requestStart = useCallback(() => {
    const error = validate()
    if (error) {
      onInvalid(error)
      return
    }
    onPrepare()
    if (seconds === 0) {
      void onStart()
      return
    }

    const generation = guardRef.current.begin()
    handedOffRef.current = false
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
        void onStart().finally(() => {
          handedOffRef.current = false
          if (guardRef.current.isCurrent(generation)) setActive(false)
        })
      })
    }
    tick()
    tickRef.current = window.setInterval(tick, 100)
  }, [clearTick, onInvalid, onPrepare, onStart, seconds, validate])

  useEffect(() => () => {
    guardRef.current.cancel()
    handedOffRef.current = false
    clearTick()
  }, [clearTick])

  return {
    seconds,
    setSeconds,
    remaining,
    active,
    requestStart,
    cancel,
  }
}
