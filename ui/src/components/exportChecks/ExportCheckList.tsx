// The rows of the export check list. One row per check: a state mark, the name
// and a short result. A row opens to one sentence and one action. The list is
// the message: there is no paragraph above or below it.
import { useCallback, useEffect, useId, useRef, useState, type ReactNode } from 'react'
import type { ExportCheckAction, ExportCheckEntry, ExportCheckPhase, ExportChecks } from '../../lib/exportChecks'
import { writeClipboardText } from '../../lib/clipboard'
import { Icon } from '../../icons'
import { buildExportCheckView, plural, MARK_WORDS, type CheckMark, type CheckRow } from './model'
import './exportChecks.css'

export type ExportCheckActionHandler = (action: ExportCheckAction, entry: ExportCheckEntry) => void | Promise<unknown>

export interface ExportCheckListProps {
  checks: ExportChecks
  /** Runs seek, open_tools and command actions. Copy details is done here. No handler: those buttons are not drawn. */
  onAction?: ExportCheckActionHandler
  /** Defaults to the app's clipboard helper; a test can replace it. */
  writeText?: (text: string) => Promise<void>
  /** Uses this phase instead of the one in `checks` (the finished file shows every applicable check). */
  phase?: ExportCheckPhase
  /** Accessible name of the list. */
  label?: string
  /** Rows or folded lines that start open (pictures and tests). */
  initiallyOpen?: { row?: string; passed?: boolean; onRequest?: boolean }
  className?: string
}

type Feedback = 'copied' | 'copy_failed' | 'busy' | 'failed'

const FEEDBACK_WORDS: Record<Feedback, string> = {
  copied: 'Copied',
  copy_failed: 'Could not copy, try again',
  busy: 'Working…',
  failed: 'Did not work, try again',
}

const isThenable = (value: unknown): value is PromiseLike<unknown> => !!value && typeof (value as PromiseLike<unknown>).then === 'function'

export function CheckMark({ mark }: { mark: CheckMark }) {
  // Shapes: triangle (found), cross in a ring (error), dashed ring (not checked), tick (fine).
  const glyph = mark === 'found' ? <Icon name="warning" size={16} />
    : mark === 'error' ? <Icon name="errorRing" size={16} />
      : mark === 'fine' ? <Icon name="check" size={16} />
        : <Icon name="minus" size={14} />
  return <span className={`ecl-mark ecl-mark--${mark}`} data-cut-export-mark={mark} title={MARK_WORDS[mark]} aria-hidden="true">{glyph}</span>
}

function Row({ row, open, feedback, onToggle, onRun, canRun }: {
  row: CheckRow
  open: boolean
  feedback?: Feedback
  onToggle: () => void
  onRun: (row: CheckRow) => void
  canRun: boolean
}) {
  const base = useId()
  const panelId = `${base}-open`
  const buttonId = `${base}-row`
  const action = row.action
  const showButton = !!action && (action.kind === 'copy_details' || canRun)
  const busy = feedback === 'busy'
  const note = feedback === 'busy' ? undefined : feedback ? FEEDBACK_WORDS[feedback] : undefined
  const label = `${row.stateWords}: ${row.name}${row.result ? `, ${row.result}` : ''}`
  return (
    <li className="ecl-row" data-cut-export-check={row.id} data-mark={row.mark} data-reason={row.entry.reason_code}>
      <button type="button" id={buttonId} className="ecl-rowbtn" data-cut-action="export-check-row" aria-expanded={open} aria-controls={panelId} aria-label={label} onClick={onToggle}>
        <CheckMark mark={row.mark} />
        {/* One run of text, so a long row takes a second line instead of cutting a word off. */}
        <span className="ecl-text">
          <span className="ecl-name">{row.name}</span>
          {row.shownResult && <><span className="ecl-sep" aria-hidden="true">·</span><wbr /><span className="ecl-result">{row.shownResult}</span></>}
        </span>
        <Icon name={open ? 'chevronUp' : 'chevronDown'} size={14} className="ecl-chev" />
      </button>
      <div id={panelId} className="ecl-live" aria-live="polite">
        {open && (
          <div className="ecl-open" role="group" aria-labelledby={buttonId} data-cut-export-check-open={row.id}>
            <p className="ecl-sentence">{row.sentence}</p>
            {row.extraLine && <p className="ecl-extra">{row.extraLine}</p>}
            {(showButton || note) && (
              <div className="ecl-act-row">
                {showButton && action && (
                  // Waiting keeps the button focusable (a keyboard user keeps their place) and ignores a second press.
                  <button type="button" className="ecl-act" data-cut-export-check-action={action.kind} aria-busy={busy || undefined} aria-disabled={busy || undefined} onClick={() => { if (!busy) onRun(row) }}>
                    {busy ? <><Icon name="spinner" size={14} />{FEEDBACK_WORDS.busy}</> : action.label}
                  </button>
                )}
                {note && <span className={`ecl-note${feedback === 'copied' ? ' ecl-note--ok' : ' ecl-note--bad'}`} data-cut-export-check-note={feedback}>{note}</span>}
              </div>
            )}
          </div>
        )}
      </div>
    </li>
  )
}

function Fold({ kind, text, mark, open, onToggle, children }: {
  kind: 'passed' | 'on_request'
  text: string
  mark?: CheckMark
  open: boolean
  onToggle: () => void
  children: ReactNode
}) {
  const id = `${useId()}-fold`
  return (
    <li className={`ecl-fold ecl-fold--${kind}`}>
      <button type="button" className="ecl-rowbtn ecl-foldbtn" data-cut-export-fold={kind} aria-expanded={open} aria-controls={id} onClick={onToggle}>
        {mark ? <CheckMark mark={mark} /> : <span className="ecl-mark ecl-mark--none" aria-hidden="true" />}
        <span className="ecl-text"><span className="ecl-name">{text}</span></span>
        <Icon name={open ? 'chevronUp' : 'chevronDown'} size={14} className="ecl-chev" />
      </button>
      <div id={id}>{open && <ul role="list" className="ecl ecl--nested" aria-label={text}>{children}</ul>}</div>
    </li>
  )
}

export default function ExportCheckList({ checks, onAction, writeText = writeClipboardText, phase, label = 'Export checks', initiallyOpen, className }: ExportCheckListProps) {
  const view = buildExportCheckView(checks, phase)
  const [openRow, setOpenRow] = useState<string | undefined>(initiallyOpen?.row)
  const [passedOpen, setPassedOpen] = useState(!!initiallyOpen?.passed)
  const [onRequestOpen, setOnRequestOpen] = useState(!!initiallyOpen?.onRequest)
  const [feedback, setFeedback] = useState<Record<string, Feedback>>({})
  const alive = useRef(true)
  const timers = useRef<number[]>([])
  useEffect(() => {
    alive.current = true
    return () => { alive.current = false; timers.current.forEach((timer) => window.clearTimeout(timer)) }
  }, [])

  const setNote = useCallback((id: string, value?: Feedback) => {
    if (!alive.current) return
    setFeedback((current) => {
      const next = { ...current }
      if (value) next[id] = value
      else delete next[id]
      return next
    })
  }, [])

  const run = useCallback(async (row: CheckRow) => {
    const action = row.action
    if (!action) return
    if (action.kind === 'copy_details') {
      try {
        await writeText(action.text ?? '')
        setNote(row.id, 'copied')
        timers.current.push(window.setTimeout(() => setNote(row.id), 2500))
      } catch {
        setNote(row.id, 'copy_failed')
      }
      return
    }
    try {
      const done = onAction?.(action, row.entry)
      if (isThenable(done)) {
        setNote(row.id, 'busy')
        await done
        setNote(row.id)
      }
    } catch {
      setNote(row.id, 'failed')
    }
  }, [onAction, setNote, writeText])

  const renderRow = (row: CheckRow) => (
    <Row
      key={row.id}
      row={row}
      open={openRow === row.id}
      feedback={feedback[row.id]}
      canRun={!!onAction}
      onToggle={() => setOpenRow((current) => (current === row.id ? undefined : row.id))}
      onRun={run}
    />
  )

  return (
    <ul role="list" className={`ecl${className ? ` ${className}` : ''}`} aria-label={label} data-cut-export-check-list data-phase={phase ?? checks?.phase}>
      {view.found.map(renderRow)}
      {view.errors.map(renderRow)}
      {view.suggestions.map(renderRow)}
      {view.passed.length > 0 && (
        <Fold kind="passed" mark="fine" text={`${plural(view.passed.length, 'check')} passed`} open={passedOpen} onToggle={() => setPassedOpen((current) => !current)}>
          {view.passed.map(renderRow)}
        </Fold>
      )}
      {view.onRequest.length > 0 && (
        <Fold kind="on_request" text={`${plural(view.onRequest.length, 'more check')} on request`} open={onRequestOpen} onToggle={() => setOnRequestOpen((current) => !current)}>
          {view.onRequest.map(renderRow)}
        </Fold>
      )}
    </ul>
  )
}
