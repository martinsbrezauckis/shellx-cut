import { useEffect, useMemo, useState } from 'react'
import type { Project } from '../../lib/client'
import { TemporalPoint, TemporalRange, formatTemporalTime } from '../../components/TemporalNavigation'
import { sourceTimelineOccurrences } from '../Timeline/layout'
import {
  boundedChapterWindow,
  CHAPTER_RENDER_LIMIT,
  chapterNavigationDisposition,
  chapterWindowStarts,
  type ChapterAssetAvailability,
  type TranscriptChapter,
} from './chapterNavigationModel'

interface ChapterNavigationProps {
  asset: string
  chapters: readonly TranscriptChapter[]
  availability: ChapterAssetAvailability
  project: Project | null
  onSeek: (atMs: number) => void
}

export default function ChapterNavigation({ asset, chapters, availability, project, onSeek }: ChapterNavigationProps) {
  const [start, setStart] = useState(0)
  const [choice, setChoice] = useState<number | null>(null)
  const window = useMemo(() => boundedChapterWindow(chapters, start), [chapters, start])

  useEffect(() => {
    setStart(0)
    setChoice(null)
  }, [asset, chapters])

  const starts = chapterWindowStarts(chapters.length, window.start)
  return (
    <section className="tx__chapters" aria-label="Chapter navigation" data-cut-chapter-list data-cut-chapter-asset={asset} data-cut-chapter-availability={availability}>
      <div className="tx__chapters-header">
        <span className="tx__chapters-title">Chapters</span>
        <span className="tx__chapters-count">{chapters.length}</span>
        {starts.length > 1 && (
          <select
            className="tx__chapters-page"
            aria-label="Chapter page"
            data-cut-action="chapter-window"
            value={String(window.start)}
            onChange={(event) => setStart(Number(event.target.value))}
          >
            {starts.map((offset) => (
              <option key={offset} value={offset}>{offset + 1}–{Math.min(chapters.length, offset + CHAPTER_RENDER_LIMIT)}</option>
            ))}
          </select>
        )}
      </div>
      <ol className="tx__chapters-list">
        {window.items.map((chapter, localIndex) => {
          const index = window.start + localIndex
          const title = chapter.title?.trim() || `Chapter ${index + 1}`
          const sourceEndMs = chapter.end_ms ?? chapter.start_ms
          const disposition = chapterNavigationDisposition(
            availability,
            sourceTimelineOccurrences(project, asset, chapter.start_ms),
          )
          const isChoiceOpen = choice === index
          return (
            <li
              key={`${index}:${chapter.start_ms}:${title}`}
              className="tx__chapter"
              data-cut-chapter
              data-cut-chapter-state={disposition.kind}
              data-cut-chapter-source-ms={String(Math.round(chapter.start_ms))}
            >
              <span className="tx__chapter-title" title={title}>{title}</span>
              <TemporalRange
                rangeMs={[chapter.start_ms, sourceEndMs]}
                className="tx__chapter-source"
                ariaLabel={`Source range for ${title}`}
                data={{ 'data-cut-chapter-source-range': `${Math.round(chapter.start_ms)}-${Math.round(sourceEndMs)}` }}
              >
                Source {formatTemporalTime(chapter.start_ms)}{sourceEndMs === chapter.start_ms ? '' : `–${formatTemporalTime(sourceEndMs)}`}
              </TemporalRange>
              {disposition.kind === 'direct' && (
                <TemporalPoint
                  atMs={disposition.occurrence.atMs}
                  className="tx__chapter-seek"
                  ariaLabel={`Seek ${title} at project time ${formatTemporalTime(disposition.occurrence.atMs)}`}
                  title="Exact current project occurrence"
                  data={{
                    'data-cut-chapter-seek': String(Math.round(disposition.occurrence.atMs)),
                    'data-cut-chapter-occurrence': disposition.occurrence.clipId,
                  }}
                  onActivate={onSeek}
                >
                  Project {formatTemporalTime(disposition.occurrence.atMs)}
                </TemporalPoint>
              )}
              {disposition.kind === 'choose' && (
                <>
                  <button
                    type="button"
                    className="tx__chapter-choose"
                    data-cut-action="choose-chapter-occurrence"
                    aria-expanded={isChoiceOpen}
                    aria-controls={`chapter-occurrences-${index}`}
                    onClick={() => setChoice((active) => active === index ? null : index)}
                  >
                    Used {disposition.occurrences.length} times
                  </button>
                  {isChoiceOpen && (
                    <div id={`chapter-occurrences-${index}`} className="tx__chapter-occurrences" data-cut-chapter-occurrences>
                      {disposition.occurrences.map((occurrence, occurrenceIndex) => (
                        <TemporalPoint
                          key={`${occurrence.trackId}:${occurrence.clipId}:${occurrence.atMs}`}
                          atMs={occurrence.atMs}
                          className="tx__chapter-occurrence"
                          ariaLabel={`Seek ${title}, project use ${occurrenceIndex + 1}, at project time ${formatTemporalTime(occurrence.atMs)}`}
                          title={`Project use ${occurrenceIndex + 1}`}
                          data={{
                            'data-cut-chapter-seek': String(Math.round(occurrence.atMs)),
                            'data-cut-chapter-occurrence': occurrence.clipId,
                          }}
                          onActivate={(atMs) => {
                            setChoice(null)
                            onSeek(atMs)
                          }}
                        >
                          Project {formatTemporalTime(occurrence.atMs)}
                        </TemporalPoint>
                      ))}
                    </div>
                  )}
                </>
              )}
              {'message' in disposition && (
                <span className="tx__chapter-status" data-cut-chapter-status>{disposition.message}</span>
              )}
            </li>
          )
        })}
      </ol>
    </section>
  )
}
