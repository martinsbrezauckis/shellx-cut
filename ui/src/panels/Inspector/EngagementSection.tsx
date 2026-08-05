import { Fragment, useState } from 'react'
import { callVerb } from '../../lib/client'
import InspectorSection from '../../components/inspector/InspectorSection'

const isObject = (v: unknown): v is object => v !== null && typeof v === 'object'

const SCORE_FACTOR_LABELS: Record<string, string> = {
  speech_density: 'Speech density',
  word_rate: 'Word rate',
  confidence: 'Transcript confidence',
  energy: 'Audio energy',
  visual_dynamics: 'Visual motion',
}

interface ClipScore {
  score: number
  duration_ms?: number
  factors?: Record<string, number>
  signals?: { words?: number; scenes?: number; silence_ms?: number; dead_ms?: number }
}

function clipScoreFrom(v: unknown): ClipScore | null {
  if (!isObject(v)) return null
  const score = Reflect.get(v, 'score')
  if (typeof score !== 'number') return null
  const duration = Reflect.get(v, 'duration_ms')
  const factors = Reflect.get(v, 'factors')
  const signals = Reflect.get(v, 'signals')
  return {
    score,
    duration_ms: typeof duration === 'number' ? duration : undefined,
    factors: isObject(factors) ? Object.fromEntries(Object.entries(factors).filter(([, value]) => typeof value === 'number')) : undefined,
    signals: isObject(signals) ? Object.fromEntries(Object.entries(signals).filter(([, value]) => typeof value === 'number')) : undefined,
  }
}

export default function EngagementSection({ clipId }: { clipId: string }) {
  const [busy, setBusy] = useState(false)
  const [score, setScore] = useState<ClipScore | null>(null)
  const [err, setErr] = useState<string | null>(null)

  const run = async () => {
    setBusy(true)
    setErr(null)
    try {
      const r = await callVerb('score.clip', { clip: clipId })
      const scoreResult = r.ok ? clipScoreFrom(r.result) : null
      if (scoreResult) {
        setScore(scoreResult)
      } else {
        setErr(r.error?.message ?? 'could not score this clip')
      }
    } catch {
      setErr('server unreachable')
    } finally {
      setBusy(false)
    }
  }

  return (
    <InspectorSection
      title="Short-form score"
      titleHint="Rates speech density, pace, audio energy, and visual motion for short-form potential"
      sectionKey="engagement"
      defaultCollapsed
    >
      <div className="insp__group" data-cut-inspector-group="engagement">
        {!score && (
          <button
            type="button"
            className="insp__btn insp__btn--accent"
            data-cut-action="score-clip"
            disabled={busy}
            title="Rate how engaging this clip is for short-form edits. Requires captions and content analysis."
            onClick={() => void run()}
          >
            {busy ? 'Scoring…' : 'Score this clip'}
          </button>
        )}
        {err && <p className="insp__hint" data-cut-inspector-score-error>{err}</p>}
        {score && (
          <>
            <div className="insp__score" data-cut-inspector-score={Math.round(score.score)}>
              <span className="insp__score-num">{Math.round(score.score)}</span>
              <span className="insp__score-max">/ 100</span>
            </div>
            {score.factors && Object.keys(score.factors).length > 0 && (
              <dl className="insp__props" data-cut-inspector-score-factors>
                {Object.entries(score.factors).map(([k, v]) => (
                  <Fragment key={k}>
                    <dt>{SCORE_FACTOR_LABELS[k] ?? k}</dt>
                    <dd>{Math.abs(v) <= 1 ? `${Math.round(v * 100)}%` : v.toFixed(2)}</dd>
                  </Fragment>
                ))}
              </dl>
            )}
            {score.signals && (typeof score.signals.words === 'number' || typeof score.signals.scenes === 'number') && (
              <p className="insp__hint" data-cut-inspector-score-signals>
                {typeof score.signals.words === 'number' && `${score.signals.words} words`}
                {typeof score.signals.words === 'number' && typeof score.signals.scenes === 'number' && ' · '}
                {typeof score.signals.scenes === 'number' && `${score.signals.scenes} scenes`}
              </p>
            )}
            <button
              type="button"
              className="insp__btn"
              data-cut-action="score-clip-again"
              disabled={busy}
              onClick={() => void run()}
              title="Re-score this clip"
            >
              {busy ? 'Scoring…' : 'Re-score'}
            </button>
          </>
        )}
      </div>
    </InspectorSection>
  )
}
