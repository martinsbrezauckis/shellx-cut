import { useMemo, type MouseEvent } from 'react'
import type { WordSpan } from '../../lib/client'
import type { CutSpan } from '../Review/shared'

interface AssetWordsProps {
  assetId: string
  words: WordSpan[]
  cuts: CutSpan[]
  /** Non-destructive mute ranges (SOURCE ms, union across the asset's audio
   *  clips) — words overlapping any range render dimmed with an amber dotted
   *  underline (distinct from the red strike of a CUT: the word still occupies
   *  its time, just silent). */
  muted: Array<[number, number]>
  /** Non-destructive ignored SOURCE word ranges. Ignored words stay readable in
   *  source view, but captions/assemble skip them. */
  ignored: Array<[number, number]>
  activeIdx: number
  sel: [number, number] | null
  pending: [number, number] | null
  onWordDown: (asset: string, idx: number, ev: MouseEvent) => void
  onWordEnter: (asset: string, idx: number) => void
  onRestore: (opId: string) => void
}

const FILLER_WORDS = new Set(['um', 'uh', 'uhm', 'umm', 'er', 'erm', 'ah', 'hmm', 'mhm', 'like'])

const normalize = (w: string) => w.toLowerCase().replace(/[^a-z']/g, '')

const inRange = (idx: number, r: [number, number] | null) => !!r && idx >= r[0] && idx <= r[1]

export default function AssetWords({
  assetId,
  words,
  cuts,
  muted,
  ignored,
  activeIdx,
  sel,
  pending,
  onWordDown,
  onWordEnter,
  onRestore,
}: AssetWordsProps) {
  const cutAt = useMemo(() => {
    const m = new Map<number, CutSpan>()
    for (const c of cuts) for (let i = c.wordRange[0]; i <= c.wordRange[1]; i++) if (!m.has(i)) m.set(i, c)
    return m
  }, [cuts])

  const groups = useMemo(() => {
    const out: Array<{ op: CutSpan | null; words: WordSpan[] }> = []
    for (const w of words) {
      const op = cutAt.get(w.idx) ?? null
      const last = out[out.length - 1]
      if (last && last.op === op) last.words.push(w)
      else out.push({ op, words: [w] })
    }
    return out
  }, [words, cutAt])

  const renderWord = (w: WordSpan, removed: boolean) => {
    const isMuted = !removed && muted.some((r) => w.start_ms < r[1] && w.end_ms > r[0])
    const isIgnored = !removed && ignored.some((r) => w.idx >= r[0] && w.idx <= r[1])
    const cls = [
      'tx-word',
      removed ? '' : FILLER_WORDS.has(normalize(w.word)) ? 'tx-word--filler' : '',
      isIgnored ? 'tx-word--ignored' : '',
      isMuted ? 'tx-word--muted' : '',
      w.idx === activeIdx ? 'tx-word--active' : '',
      inRange(w.idx, sel) ? 'tx-word--sel' : '',
      inRange(w.idx, pending) ? 'tx-word--pending' : '',
    ]
      .filter(Boolean)
      .join(' ')
    return (
      <span
        key={w.idx}
        className={cls}
        data-cut-action="word"
        data-word-idx={w.idx}
        data-asset={assetId}
        data-cut-word={`${assetId}:${w.idx}`}
        {...(isMuted ? { 'data-cut-word-muted': `${assetId}:${w.idx}` } : {})}
        {...(isIgnored ? { 'data-cut-word-ignored': `${assetId}:${w.idx}` } : {})}
        title={
          isIgnored
            ? 'ignored for captions/reels — select and Unignore to restore'
            : isMuted
              ? 'muted (non-destructive) — select and Unmute to restore'
              : undefined
        }
        onMouseDown={(e) => onWordDown(assetId, w.idx, e)}
        onMouseEnter={() => onWordEnter(assetId, w.idx)}
      >
        {w.word}{' '}
      </span>
    )
  }

  if (words.length === 0) {
    return (
      <div className="tx__asset" data-cut-transcript={assetId}>
        <div className="tx__asset-header">{assetId}</div>
        <p className="tx__flow tx__flow--empty" data-cut-transcript-empty={assetId}>
          No speech detected — run media.transcribe, or this clip has no narration.
        </p>
      </div>
    )
  }

  return (
    <div className="tx__asset" data-cut-transcript={assetId}>
      <div className="tx__asset-header">{assetId}</div>
      <p className="tx__flow">
        {groups.map((g, i) =>
          g.op ? (
            <span
              key={`${g.op.opId}-${i}`}
              className="tx-removed"
              data-op-id={g.op.opId}
              data-cut-removed={g.op.opId}
              title={g.op.rationale ? `${g.op.opId} — ${g.op.rationale}` : g.op.opId}
            >
              {g.words.map((w) => renderWord(w, true))}
              <button
                className="tx-restore"
                data-cut-action="restore"
                data-cut-op={g.op.opId}
                onMouseDown={(e) => e.stopPropagation()}
                onClick={() => onRestore(g.op!.opId)}
              >
                Restore
              </button>
            </span>
          ) : (
            g.words.map((w) => renderWord(w, false))
          ),
        )}
      </p>
    </div>
  )
}
