import { useEffect, useRef, useState } from 'react'
import {
  USER_ACTION_FEEDBACK_EVENT,
  type UserActionFeedbackDetail,
} from '../lib/userActionFeedback'

const DISPLAY_MS = 8_000

interface UserActionFeedbackNoticeProps {
  feedback: UserActionFeedbackDetail
  onOpenSetup: () => void
  onDismiss: () => void
}

/** Rendered notification shared by every human-triggered verb failure. */
export function UserActionFeedbackNotice({ feedback, onOpenSetup, onDismiss }: UserActionFeedbackNoticeProps) {
  return (
    <div className="user-action-feedback" data-cut-user-action-feedback role="alert" aria-live="assertive">
      <span data-cut-user-action-feedback-message>{feedback.message}</span>
      <span className="user-action-feedback__actions">
        {feedback.setupSurface && (
          <button type="button" data-cut-user-action-open-setup onClick={onOpenSetup}>Open setup</button>
        )}
        <button
          type="button"
          className="user-action-feedback__dismiss"
          data-cut-user-action-dismiss
          aria-label="Dismiss message"
          onClick={onDismiss}
        >
          ×
        </button>
      </span>
    </div>
  )
}

export default function UserActionFeedback() {
  const [feedback, setFeedback] = useState<UserActionFeedbackDetail | null>(null)
  const timer = useRef<number | null>(null)

  useEffect(() => {
    const clearTimer = () => {
      if (timer.current !== null) window.clearTimeout(timer.current)
      timer.current = null
    }
    const onFeedback = (event: Event) => {
      if (!(event instanceof CustomEvent) || !event.detail?.message) return
      clearTimer()
      setFeedback(event.detail as UserActionFeedbackDetail)
      timer.current = window.setTimeout(() => setFeedback(null), DISPLAY_MS)
    }
    document.addEventListener(USER_ACTION_FEEDBACK_EVENT, onFeedback)
    return () => {
      document.removeEventListener(USER_ACTION_FEEDBACK_EVENT, onFeedback)
      clearTimer()
    }
  }, [])

  if (!feedback) return null
  const openSetup = () => {
    if (!feedback.setupSurface) return
    document.dispatchEvent(new CustomEvent('cut:open-ui-surface', {
      detail: { id: feedback.setupSurface },
    }))
    setFeedback(null)
  }

  return <UserActionFeedbackNotice
    feedback={feedback}
    onOpenSetup={openSetup}
    onDismiss={() => setFeedback(null)}
  />
}
