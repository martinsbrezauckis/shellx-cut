import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent } from 'react'
import type { TimelineWord } from '../../lib/client'
import { TemporalRange } from '../../components/TemporalNavigation'
import type { Sel } from './model'
import {
  activePhraseIndex,
  boundedPhraseWindow,
  formatTranscriptTime,
  groupTimelinePhrases,
  phraseWindowStarts,
  phraseTimelineSeek,
  TRANSCRIPT_PHRASE_RENDER_LIMIT,
  type TranscriptPhrase,
} from './phraseGrouping'
import './transcript-phrases.css'

interface TranscriptPhraseListProps {
  entries: TimelineWord[]
  scope: 'clip' | 'program'
  playheadMs: number
  selection: Sel | null
  onSeek: (atMs: number) => void
  onWordDown: (word: TimelineWord, event: MouseEvent) => void
  onWordEnter: (word: TimelineWord) => void
  onWordActivate: (word: TimelineWord) => void
  onSelectPhrase: (phrase: TranscriptPhrase) => void
}

function wordSelected(word: TimelineWord, selection: Sel | null): boolean {
  if (!selection || selection.location !== 'timeline') return false
  const [from, to] = selection.anchor <= selection.head
    ? [selection.anchor, selection.head]
    : [selection.head, selection.anchor]
  return selection.asset === word.asset
    && selection.clipId === word.clip_id
    && word.word_index >= from
    && word.word_index <= to
}

function PhraseRow({
  phrase,
  active,
  selection,
  onSeek,
  onWordDown,
  onWordEnter,
  onWordActivate,
  onSelectPhrase,
}: {
  phrase: TranscriptPhrase
  active: boolean
  selection: Sel | null
  onSeek: (atMs: number) => void
  onWordDown: (word: TimelineWord, event: MouseEvent) => void
  onWordEnter: (word: TimelineWord) => void
  onWordActivate: (word: TimelineWord) => void
  onSelectPhrase: (phrase: TranscriptPhrase) => void
}) {
  const [expanded, setExpanded] = useState(false)
  const range = `${formatTranscriptTime(phrase.startMs)} → ${formatTranscriptTime(phrase.endMs)}`
  const occurrence = phrase.clipId ?? 'unmapped occurrence'
  const seekLabel = `Seek ${range} in ${occurrence}`
  return (
    <article
      className={`tpx__phrase${active ? ' tpx__phrase--active' : ''}`}
      data-cut-transcript-phrase={phrase.id}
      data-cut-transcript-phrase-asset={phrase.asset}
      {...(active ? { 'data-cut-transcript-phrase-active': '' } : {})}
      data-cut-transcript-phrase-range={`${phrase.startMs}-${phrase.endMs}`}
      data-cut-transcript-occurrence={occurrence}
      {...(phrase.speaker ? { 'data-cut-transcript-phrase-speaker': phrase.speaker } : {})}
    >
      <TemporalRange
        rangeMs={[phrase.startMs, phrase.endMs]}
        format={formatTranscriptTime}
        className="tpx__time"
        data={{ 'data-cut-transcript-seek': phrase.id }}
        ariaLabel={seekLabel}
        title={seekLabel}
        onActivate={() => onSeek(phraseTimelineSeek(phrase))}
      >
        <span className="tpx__range">{formatTranscriptTime(phrase.startMs)} <span aria-hidden="true">→</span> {formatTranscriptTime(phrase.endMs)}</span>
      </TemporalRange>
      <div
        className="tpx__content"
        title={seekLabel}
        onClick={() => onSeek(phraseTimelineSeek(phrase))}
      >
        <p className="tpx__words">
          {phrase.words.map((word) => {
            const selected = wordSelected(word, selection)
            return (
              <button
                type="button"
                key={`${phrase.id}:${word.word_index}`}
                className={`tpx__word txv__w${selected ? ' txv__w--sel' : ''}`}
                data-cut-action="timeline-word"
                data-word-idx={word.word_index}
                data-asset={word.asset}
                data-cut-timeline-word={word.word_index}
                data-cut-transcript-word-time={word.timeline_start_ms}
                aria-label={`Select word ${word.word}, ${formatTranscriptTime(word.timeline_start_ms)}`}
                onMouseDown={(event) => onWordDown(word, event)}
                onMouseEnter={() => onWordEnter(word)}
                onClick={(event) => event.stopPropagation()}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault()
                    event.stopPropagation()
                    onWordActivate(word)
                  }
                }}
              >
                {word.word}
              </button>
            )
          })}
        </p>
        <div className="tpx__meta">
          {phrase.speaker && <span className="tpx__speaker">{phrase.speaker}</span>}
          <span>{phrase.track} · {occurrence}</span>
          <button
            type="button"
            className="tpx__expand"
            data-cut-transcript-phrase-expand={phrase.id}
            aria-expanded={expanded}
            aria-controls={`phrase-actions-${phrase.id}`}
            onClick={(event) => {
              event.stopPropagation()
              setExpanded((value) => !value)
            }}
          >
            {expanded ? 'Hide word actions' : 'Word actions'}
          </button>
        </div>
        {expanded && (
          <div id={`phrase-actions-${phrase.id}`} className="tpx__actions" data-cut-transcript-phrase-actions={phrase.id}>
            <span>Select individual words, or</span>
            <button
              type="button"
              data-cut-action="transcript-select-phrase"
              onClick={(event) => { event.stopPropagation(); onSelectPhrase(phrase) }}
            >
              select this phrase
            </button>
            <span>for Cut, Mute, Ignore, or Reel actions.</span>
          </div>
        )}
      </div>
    </article>
  )
}

export default function TranscriptPhraseList({
  entries,
  scope,
  playheadMs,
  selection,
  onSeek,
  onWordDown,
  onWordEnter,
  onWordActivate,
  onSelectPhrase,
}: TranscriptPhraseListProps) {
  const rootRef = useRef<HTMLDivElement>(null)
  const followingRef = useRef(false)
  const [followPaused, setFollowPaused] = useState(false)
  const [windowStart, setWindowStart] = useState(0)
  const phrases = useMemo(() => groupTimelinePhrases(entries), [entries])
  const activeIndex = useMemo(() => activePhraseIndex(phrases, playheadMs), [phrases, playheadMs])
  const lastWindowStart = Math.max(0, phrases.length - TRANSCRIPT_PHRASE_RENDER_LIMIT)
  const { start: boundedStart, phrases: visiblePhrases } = useMemo(
    () => boundedPhraseWindow(phrases, windowStart),
    [phrases, windowStart],
  )
  const windowStarts = useMemo(
    () => phraseWindowStarts(phrases.length, boundedStart),
    [boundedStart, phrases.length],
  )

  useEffect(() => {
    if (activeIndex < 0 || followPaused) return
    if (activeIndex < boundedStart || activeIndex >= boundedStart + TRANSCRIPT_PHRASE_RENDER_LIMIT) {
      setWindowStart(Math.max(0, Math.min(lastWindowStart, activeIndex - 12)))
    }
  }, [activeIndex, boundedStart, followPaused, lastWindowStart])

  useEffect(() => {
    const scroller = rootRef.current?.closest<HTMLElement>('.panel__body')
    if (!scroller) return
    const onScroll = () => {
      if (!followingRef.current) setFollowPaused(true)
    }
    scroller.addEventListener('scroll', onScroll, { passive: true })
    return () => scroller.removeEventListener('scroll', onScroll)
  }, [])

  useEffect(() => {
    if (activeIndex < 0 || followPaused) return
    const active = rootRef.current?.querySelector<HTMLElement>('[data-cut-transcript-phrase-active]')
    if (!active) return
    followingRef.current = true
    active.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
    const release = window.setTimeout(() => { followingRef.current = false }, 180)
    return () => window.clearTimeout(release)
  }, [activeIndex, followPaused, boundedStart])

  const resumeFollow = useCallback(() => setFollowPaused(false), [])

  if (entries.length === 0) {
    return (
      <div className="tx__empty" data-cut-timeline-empty>
        {scope === 'clip' ? 'Select a clip on the timeline to see its words' : 'No words on the timeline yet'}
      </div>
    )
  }

  return (
    <div
      ref={rootRef}
      className="tpx"
      data-cut-timeline-view={scope}
      data-cut-transcript-phrase-list
      data-cut-transcript-follow={followPaused ? 'paused' : 'following'}
      data-cut-transcript-playhead={playheadMs}
      onWheel={() => setFollowPaused(true)}
      onTouchStart={() => setFollowPaused(true)}
    >
      <div className="tpx__toolbar">
        <span data-cut-transcript-window>{boundedStart + 1}–{Math.min(phrases.length, boundedStart + TRANSCRIPT_PHRASE_RENDER_LIMIT)} of {phrases.length} phrases</span>
        <label className="tpx__window-picker">
          <span>Phrase range</span>
          <select
            data-cut-action="transcript-phrase-window"
            aria-label="Choose transcript phrase range"
            value={boundedStart}
            onChange={(event) => setWindowStart(Number(event.currentTarget.value))}
          >
            {windowStarts.map((start) => (
              <option key={start} value={start}>
                {start + 1}–{Math.min(phrases.length, start + TRANSCRIPT_PHRASE_RENDER_LIMIT)}
              </option>
            ))}
          </select>
        </label>
        {followPaused && <button type="button" data-cut-transcript-resume-follow onClick={resumeFollow}>Resume follow</button>}
      </div>
      {visiblePhrases.map((phrase, index) => (
        <PhraseRow
          key={phrase.id}
          phrase={phrase}
          active={boundedStart + index === activeIndex}
          selection={selection}
          onSeek={onSeek}
          onWordDown={onWordDown}
          onWordEnter={onWordEnter}
          onWordActivate={onWordActivate}
          onSelectPhrase={onSelectPhrase}
        />
      ))}
    </div>
  )
}
