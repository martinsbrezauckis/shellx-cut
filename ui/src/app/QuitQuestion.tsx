// Asked by the desktop shell when the app is closed or quit while an export
// runs (see quit_question.rs in the shell). The question stays in this window
// until the person answers; ui.state reports it as `overlays.quit_question`.

import { useEffect, useRef, useState } from 'react'
import { acknowledgeQuitQuestion, answerQuitQuestion, onQuitJobsQuestion, type QuitJobsQuestion } from '../lib/tauri'
import { callVerb } from '../lib/client'
import {
  KEEP_WORKING, LIVE_PROGRESS_FRESH_MS, QUIT_AND_STOP, STOPPING_BEFORE_QUIT, agentListing, answerForKey,
  liveFromStatus, questionJobs, quitQuestionView, statedPercent, type LiveProgress,
} from './quitQuestionModel'

type Reading = { value: LiveProgress; at: number }

export default function QuitQuestion() {
  const [question, setQuestion] = useState<QuitJobsQuestion | null>(null)
  const [stopping, setStopping] = useState(false)
  // The work keeps running while the person reads: its progress is read again
  // every second, and a number that is no longer current is not shown.
  const [live, setLive] = useState<Record<string, Reading>>({})
  const [now, setNow] = useState(() => Date.now())
  const keepRef = useRef<HTMLButtonElement | null>(null)

  useEffect(() => onQuitJobsQuestion(
    (asked) => {
      setStopping(false)
      const at = Date.now()
      setLive(Object.fromEntries(questionJobs(asked)
        .map((job) => [job.job_id, { value: statedPercent(job), at }] as const)))
      setNow(at)
      setQuestion(asked)
      void acknowledgeQuitQuestion()
    },
    // The shell ended the question (the work finished meanwhile): nothing
    // is asked or being stopped any more.
    () => {
      setQuestion(null)
      setStopping(false)
    },
  ), [])

  useEffect(() => {
    if (!question) return undefined
    const ids = questionJobs(question).map((job) => job.job_id)
    let cancelled = false
    const read = async () => {
      for (const id of ids) {
        const value = liveFromStatus(await callVerb('jobs.status', { job_id: id }).catch(() => ({ ok: false })))
        if (cancelled) return
        if (value !== null) setLive((current) => ({ ...current, [id]: { value, at: Date.now() } }))
      }
      if (!cancelled) setNow(Date.now())
    }
    const timer = window.setInterval(() => { void read() }, 1_000)
    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [question])

  useEffect(() => {
    if (!question) return undefined
    keepRef.current?.focus()
    // Esc keeps working wherever the focus is (the window may have just been
    // brought to the front with nothing focused inside the question).
    const onKey = (event: KeyboardEvent) => {
      if (answerForKey(event.key) !== 'keep') return
      event.preventDefault()
      setQuestion(null)
      void answerQuitQuestion(false)
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [question])

  if (stopping) {
    // The shell stops the work and then closes the app.
    return (
      <div className="quit-question-backdrop">
        <div className="quit-question" role="status" data-cut-quitting>
          <p>{STOPPING_BEFORE_QUIT}</p>
        </div>
      </div>
    )
  }
  if (!question) return null
  const progressOf = (id: string): LiveProgress => {
    const reading = live[id]
    return reading && now - reading.at <= LIVE_PROGRESS_FRESH_MS ? reading.value : null
  }
  const view = quitQuestionView(question, progressOf)
  // ui.state reads this attribute: the same live value as the visible lines,
  // one entry per job record (a JSON array, so no message is ever split).
  const listed = questionJobs(question).map((job) => agentListing(job, progressOf(job.job_id)))
  const answer = (stop: boolean) => {
    setQuestion(null)
    setStopping(stop)
    void answerQuitQuestion(stop)
  }
  return (
    <div
      className="quit-question-backdrop"
      onMouseDown={(event) => { if (event.target === event.currentTarget) answer(false) }}
    >
      <div
        className="quit-question"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="quit-question-title"
        data-cut-quit-question
        data-cut-quit-question-message={question.message}
        data-cut-quit-question-running={JSON.stringify(listed)}
      >
        <h2 id="quit-question-title">{view.title}</h2>
        <ul className="quit-question-running">
          {view.running.map((job, index) => <li key={`${index}:${job}`} data-cut-quit-question-job>{job}</li>)}
        </ul>
        <p>{view.consequence}</p>
        <div className="quit-question-actions">
          <button type="button" className="quit-question-stop" data-cut-quit-stop onClick={() => answer(true)}>
            {QUIT_AND_STOP}
          </button>
          <button
            type="button"
            ref={keepRef}
            className="quit-question-keep"
            data-cut-quit-keep
            onClick={() => answer(false)}
          >
            {KEEP_WORKING}
          </button>
        </div>
      </div>
    </div>
  )
}
