// panels/Transcript/TimelineView — the EDL-AWARE transcript body.
//
// Role: render the transcript as it maps onto the TIMELINE (transcript.timeline),
// not as raw per-asset word blobs. Two scopes:
//   - SELECTED-CLIP (default): the selected clip's words (its source window),
//     each at its timeline position; cutting a word trims THAT clip.
//   - PROGRAM: every clip's words in timeline order (the output line), grouped by
//     clip/line so it's always clear which line a word belongs to.
// What you read here is exactly what is on the timeline — cut words are simply
// absent (no strike-through bookkeeping), because the engine already removed them.
//
// Interactions: click a word → seek to its timeline position; drag / shift-click a
// range WITHIN one clip → a floating Cut button → transcript.cut_words{clip} (the
// clip-scoped cut, so a reused clip's other copies are untouched). Zero local
// mutation: the cut is a verb; words re-fetch from transcript.timeline on the next
// op. Callers: panels/Transcript. Deps: lib/client types only.

import { useCallback, useMemo, useRef, useState } from 'react'
import type { TimelineWord } from '../../lib/client'
import { Icon } from '../../icons'

export interface TimelineViewProps {
  entries: TimelineWord[]
  scope: 'clip' | 'program'
  playheadMs: number
  /** Seek the timeline to a word's position. */
  onSeek: (atMs: number) => void
  /** Cut a word range from one clip (clip-scoped). */
  onCut: (asset: string, wordRange: [number, number], clipId: string | null) => void
}

/** mm:ss.mmm short timecode for the floating cut chip. */
function tc(ms: number): string {
  const s = Math.floor(ms / 1000)
  const mm = Math.floor(s / 60)
  const ss = s % 60
  return `${mm}:${String(ss).padStart(2, '0')}`
}

export default function TimelineView({ entries, scope, playheadMs, onSeek, onCut }: TimelineViewProps) {
  // Selection by ENTRY INDEX (anchor..head). A valid cut needs the span to stay
  // inside ONE clip (same clip_id) — cross-clip selections can't map to a single
  // clip-scoped cut.
  const [anchor, setAnchor] = useState<number | null>(null)
  const [head, setHead] = useState<number | null>(null)
  const downRef = useRef(false)

  const range: [number, number] | null = useMemo(() => {
    if (anchor === null || head === null) return null
    return [Math.min(anchor, head), Math.max(anchor, head)]
  }, [anchor, head])

  // The selection is cuttable only if every entry in it shares one clip.
  const selClipId = useMemo(() => {
    if (!range) return null
    const id = entries[range[0]]?.clip_id ?? null
    for (let i = range[0]; i <= range[1]; i++) if (entries[i]?.clip_id !== id) return undefined
    return id
  }, [range, entries])

  const onWordDown = useCallback((i: number, e: React.MouseEvent) => {
    e.preventDefault()
    if (e.shiftKey && anchor !== null) {
      setHead(i)
    } else {
      downRef.current = true
      setAnchor(i)
      setHead(i)
    }
  }, [anchor])
  const onWordEnter = useCallback((i: number) => {
    if (downRef.current) setHead(i)
  }, [])
  const endDrag = useCallback(() => { downRef.current = false }, [])

  const doCut = useCallback(() => {
    if (!range) return
    const a = entries[range[0]]
    const b = entries[range[1]]
    if (!a || !b || selClipId === undefined) return
    // word_index range within the clip (contiguous for an in-clip drag).
    const lo = Math.min(a.word_index, b.word_index)
    const hi = Math.max(a.word_index, b.word_index)
    onCut(a.asset, [lo, hi], a.clip_id)
    setAnchor(null)
    setHead(null)
  }, [range, entries, selClipId, onCut])

  const clearSel = useCallback(() => { setAnchor(null); setHead(null) }, [])

  // Group consecutive entries by clip so PROGRAM view can show a per-line header.
  const groups = useMemo(() => {
    const out: Array<{ clipId: string | null; track: string; from: number; words: TimelineWord[] }> = []
    entries.forEach((w, i) => {
      const last = out[out.length - 1]
      if (last && last.clipId === w.clip_id) last.words.push(w)
      else out.push({ clipId: w.clip_id, track: w.track, from: i, words: [w] })
    })
    return out
  }, [entries])

  const playheadIdx = useMemo(() => {
    // The single word under the playhead (last whose start <= playhead).
    let idx = -1
    for (let i = 0; i < entries.length; i++) {
      if (entries[i].timeline_start_ms <= playheadMs && playheadMs < entries[i].timeline_end_ms) { idx = i; break }
      if (entries[i].timeline_start_ms <= playheadMs) idx = i
    }
    return idx
  }, [entries, playheadMs])

  if (entries.length === 0) {
    return (
      <div className="tx__empty" data-cut-timeline-empty>
        {scope === 'clip' ? 'Select a clip on the timeline to see its words' : 'No words on the timeline yet'}
      </div>
    )
  }

  return (
    <div className="txv" data-cut-timeline-view={scope} onMouseUp={endDrag} onMouseLeave={endDrag}>
      {groups.map((g) => (
        <div className="txv__clip" key={`${g.clipId}-${g.from}`} data-cut-timeline-clip={g.clipId ?? ''}>
          {scope === 'program' && (
            <div className="txv__clip-head" data-cut-timeline-clip-head>
              <span className="txv__clip-line">{g.track}</span>
              <span className="txv__clip-id">{g.clipId ?? '—'}</span>
            </div>
          )}
          <p className="txv__flow">
            {g.words.map((w, k) => {
              const i = g.from + k
              const inSel = range !== null && i >= range[0] && i <= range[1]
              return (
                <span
                  key={`${w.clip_id}-${w.word_index}-${i}`}
                  className={`txv__w${inSel ? ' txv__w--sel' : ''}${i === playheadIdx ? ' txv__w--play' : ''}`}
                  data-cut-action="timeline-word"
                  data-word-idx={w.word_index}
                  data-cut-timeline-word={i}
                  onMouseDown={(e) => onWordDown(i, e)}
                  onMouseEnter={() => onWordEnter(i)}
                  onClick={() => { if (anchor === head) onSeek(w.timeline_start_ms) }}
                  title={`${tc(w.timeline_start_ms)} · clip ${w.clip_id ?? '—'}`}
                >
                  {w.word}{' '}
                </span>
              )
            })}
          </p>
        </div>
      ))}
      {range && range[1] > range[0] - 1 && (
        <div className="txv__cut-bar" data-cut-timeline-cutbar>
          {selClipId === undefined ? (
            <span className="txv__cut-note">selection spans multiple clips — select within one clip to cut</span>
          ) : (
            <button
              type="button"
              className="txv__cut-btn"
              data-cut-action="timeline-cut-words"
              onMouseDown={(e) => e.preventDefault()}
              onClick={doCut}
            >
              Cut {range[1] - range[0] + 1} word{range[1] - range[0] === 0 ? '' : 's'}
            </button>
          )}
          <button type="button" className="txv__cut-x" data-cut-action="timeline-clear-sel" onClick={clearSel} title="clear selection"><Icon name="close" size={14} label="clear selection" /></button>
        </div>
      )}
    </div>
  )
}
