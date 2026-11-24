import { useEffect, useRef, useState } from 'react'
import {
  USER_ACTION_FEEDBACK_EVENT,
  type UserActionFeedbackDetail,
} from '../lib/userActionFeedback'
import { usePublishedUiNotices, type UiNotice } from '../lib/uiNotices'

const DISPLAY_MS = 8_000
const PERSISTENT_FEEDBACK_CODES = new Set(['project_list_rebuilt'])

function isPersistentFeedback(detail: UserActionFeedbackDetail): boolean {
  return detail.persistent === true || (typeof detail.code === 'string' && PERSISTENT_FEEDBACK_CODES.has(detail.code))
}

/** The `ui.state.notices` entry for the shown message (path-free: no text). */
export function actionFeedbackNotice(feedback: UserActionFeedbackDetail | null): UiNotice[] {
  if (!feedback) return []
  return [{
    kind: 'action_feedback',
    source: feedback.source ?? 'app',
    code: feedback.code ?? null,
    setup_surface: feedback.setupSurface ?? null,
    ...(typeof feedback.count === 'number' ? { count: feedback.count } : {}),
  }]
}

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
  const [persistent, setPersistent] = useState<UserActionFeedbackDetail | null>(null)
  const [transient, setTransient] = useState<UserActionFeedbackDetail | null>(null)
  const timer = useRef<number | null>(null)

  useEffect(() => {
    const clearTimer = () => {
      if (timer.current !== null) window.clearTimeout(timer.current)
      timer.current = null
    }
    const onFeedback = (event: Event) => {
      if (!(event instanceof CustomEvent) || !event.detail?.message) return
      const detail = event.detail as UserActionFeedbackDetail
      if (isPersistentFeedback(detail)) {
        setPersistent((current) =>
          current?.code === detail.code && current?.message === detail.message ? current : detail,
        )
        return
      }
      clearTimer()
      setTransient(detail)
      timer.current = window.setTimeout(() => setTransient(null), DISPLAY_MS)
    }
    document.addEventListener(USER_ACTION_FEEDBACK_EVENT, onFeedback)
    return () => {
      document.removeEventListener(USER_ACTION_FEEDBACK_EVENT, onFeedback)
      clearTimer()
    }
  }, [])

  usePublishedUiNotices('action-feedback', [
    ...actionFeedbackNotice(persistent),
    ...actionFeedbackNotice(transient),
  ])

  if (!persistent && !transient) return null
  const openSetup = (item: UserActionFeedbackDetail, dismissItem: () => void) => {
    if (!item.setupSurface) return
    document.dispatchEvent(new CustomEvent('cut:open-ui-surface', {
      detail: { id: item.setupSurface },
    }))
    dismissItem()
  }
  const dismissTransient = () => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current)
      timer.current = null
    }
    setTransient(null)
  }

  return (
    <div className="user-action-feedback-stack">
      {persistent && (
        <UserActionFeedbackNotice
          feedback={persistent}
          onOpenSetup={() => openSetup(persistent, () => setPersistent(null))}
          onDismiss={() => setPersistent(null)}
        />
      )}
      {transient && (
        <UserActionFeedbackNotice
          feedback={transient}
          onOpenSetup={() => openSetup(transient, dismissTransient)}
          onDismiss={dismissTransient}
        />
      )}
    </div>
  )
}
