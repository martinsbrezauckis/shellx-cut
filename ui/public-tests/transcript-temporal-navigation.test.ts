import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { TemporalRange, formatTemporalTime } from '../src/components/TemporalNavigation'
import {
  activePhraseIndex,
  boundedPhraseWindow,
  formatTranscriptTime,
  groupTimelinePhrases,
  phraseTimelineSeek,
  phraseWindowStarts,
} from '../src/panels/Transcript/phraseGrouping'
import {
  boundedChapterWindow,
  chapterNavigationDisposition,
  chapterWindowStarts,
} from '../src/panels/Transcript/chapterNavigationModel'
import { chaptersOf } from '../src/panels/Transcript/model'
import type { TimelineWord } from '../src/lib/client'

const word = (
  word_index: number,
  wordText: string,
  start: number,
  end: number,
  overrides: Partial<TimelineWord> = {},
): TimelineWord => ({
  clip_id: 'clip-a',
  track: 'v1',
  track_kind: 'video',
  asset: 'asset-a',
  word_index,
  word: wordText,
  src_start_ms: start,
  src_end_ms: end,
  timeline_start_ms: start,
  timeline_end_ms: end,
  ...overrides,
})

const punctuationAndGap = groupTimelinePhrases([
  word(0, 'First', 0, 220),
  word(1, 'sentence.', 250, 600),
  word(2, 'Second', 700, 930),
  word(3, 'after-gap', 2_100, 2_400),
])
assert.equal(punctuationAndGap.length, 3, 'punctuation and a bounded-gap refusal split phrases')
assert.deepEqual(punctuationAndGap.map((phrase) => phrase.words.map((entry) => entry.word)), [
  ['First', 'sentence.'], ['Second'], ['after-gap'],
])
assert.equal(formatTranscriptTime(punctuationAndGap[0].startMs), '0:00.000')
assert.equal(formatTranscriptTime(punctuationAndGap[0].endMs), '0:00.600')

const occurrences = groupTimelinePhrases([
  word(3, 'Reused', 1_000, 1_200, { clip_id: 'clip-first' }),
  word(4, 'source', 1_240, 1_500, { clip_id: 'clip-first' }),
  word(3, 'Reused', 7_000, 7_200, { clip_id: 'clip-second' }),
  word(4, 'source', 7_240, 7_500, { clip_id: 'clip-second' }),
])
assert.equal(occurrences.length, 2, 'reused source words never merge Program occurrences')
assert.deepEqual(occurrences.map((phrase) => [phrase.clipId, phrase.startMs, phrase.endMs]), [
  ['clip-first', 1_000, 1_500], ['clip-second', 7_000, 7_500],
])
assert.equal(activePhraseIndex(occurrences, 7_250), 1, 'the playhead follows the later occurrence, not its reused source word')
assert.equal(phraseTimelineSeek(occurrences[1]), 7_000, 'a reused Program source seeks the later occurrence timestamp')
assert.equal(occurrences[1].clipId, 'clip-second', 'a reused Program source retains its later clip occurrence')
assert.equal(activePhraseIndex(occurrences, 1_600), -1, 'a silent gap does not retain the prior active phrase')
assert.equal(activePhraseIndex(occurrences, 7_500), -1, 'the phrase is inactive at its exclusive end boundary')

const speakers = groupTimelinePhrases([
  word(0, 'Hello', 0, 250, { speaker: 'S1' }),
  word(1, 'there', 280, 540, { speaker: 'S2' }),
])
assert.deepEqual(speakers.map((phrase) => phrase.speaker), ['S1', 'S2'], 'speaker changes remain visible phrase boundaries')

const longList = groupTimelinePhrases(Array.from({ length: 125 }, (_, index) =>
  word(index, `phrase-${index}.`, index * 2_000, index * 2_000 + 250),
))
const bounded = boundedPhraseWindow(longList, 9_999)
assert.equal(bounded.phrases.length, 120, 'the phrase DOM window stays bounded for long transcripts')
assert.equal(bounded.start, 5, 'the final window clamps to the last complete 120-phrase page')
assert.equal(bounded.phrases.at(-1)?.startMs, 248_000)
assert.deepEqual(phraseWindowStarts(longList.length, bounded.start), [0, 5], 'the final partial range remains selectable without a false 121-start label')
assert.deepEqual(phraseWindowStarts(400, 268), [0, 120, 240, 268, 280], 'playback-follow retains its exact non-page-aligned range among stable pages')

assert.equal(formatTemporalTime(62_345), '1:02.345', 'the shared presentation formats exact milliseconds')
assert.equal(typeof TemporalRange, 'function', 'the shared range control remains independently importable')

const oneOccurrence = [{ clipId: 'c1', trackId: 'v1', atMs: 12_000 }]
const reusedOccurrences = [
  { clipId: 'c1', trackId: 'v1', atMs: 32_000 },
  { clipId: 'c9', trackId: 'v2', atMs: 40_000 },
]
assert.deepEqual(
  chapterNavigationDisposition('available', oneOccurrence),
  { kind: 'direct', occurrence: oneOccurrence[0] },
  'one exact current occurrence is the only direct chapter seek route',
)
assert.deepEqual(
  chapterNavigationDisposition('available', reusedOccurrences),
  { kind: 'choose', occurrences: reusedOccurrences },
  'a reused source chapter requires an explicit occurrence choice',
)
assert.equal(chapterNavigationDisposition('offline', oneOccurrence).kind, 'offline', 'offline media never exposes a stale timeline seek')
assert.equal(chapterNavigationDisposition('available', []).kind, 'edited-out', 'an edited-out source start has no invented timeline seek')
assert.equal(chapterNavigationDisposition('unknown', oneOccurrence).kind, 'unavailable', 'unverified availability stays fail-closed')
assert.equal(boundedChapterWindow(Array.from({ length: 15 }, (_, index) => index), 12).items.length, 12, 'chapter rendering is bounded')
assert.deepEqual(chapterWindowStarts(25, 13), [0, 12, 13], 'the final bounded chapter window remains reachable')
assert.deepEqual(chaptersOf({ chapters: [{ title: 'Topic', start_ms: 1_000, end_ms: 2_000 }] }), [
  { title: 'Topic', start_ms: 1_000, end_ms: 2_000 },
], 'chapter parsing retains server-owned source timing only')

const shared = readFileSync(new URL('../src/components/TemporalNavigation.tsx', import.meta.url), 'utf8')
const transcript = readFileSync(new URL('../src/panels/Transcript/TranscriptPhraseList.tsx', import.meta.url), 'utf8')
const captions = readFileSync(new URL('../src/panels/Inspector/CaptionEditSection.tsx', import.meta.url), 'utf8')
const markers = readFileSync(new URL('../src/panels/Timeline/MarkerContextMenu.tsx', import.meta.url), 'utf8')
const comments = readFileSync(new URL('../src/panels/Comments/index.tsx', import.meta.url), 'utf8')
const search = readFileSync(new URL('../src/panels/Search/SearchResults.tsx', import.meta.url), 'utf8')
const receipts = readFileSync(new URL('../src/panels/Review/Receipts.tsx', import.meta.url), 'utf8')
const chapterPanel = readFileSync(new URL('../src/panels/Transcript/ChapterNavigation.tsx', import.meta.url), 'utf8')
const chapterModel = readFileSync(new URL('../src/panels/Transcript/chapterNavigationModel.ts', import.meta.url), 'utf8')

assert.match(shared, /onActivate\(atMs\)/, 'the shared control gives the supplied timestamp back to its owner without selecting an occurrence')
assert.match(shared, /data-cut-temporal-point=/, 'shared point navigation has one stable action identity')
assert.match(shared, /data-cut-temporal-range=/, 'shared range navigation has one stable action identity')
assert.match(shared, /This range is displayed exactly as supplied/, 'the shared control does not normalize a consumer-owned range')
assert.match(transcript, /rangeMs=\{\[phrase\.startMs, phrase\.endMs\]\}/, 'Transcript retains its occurrence-owned phrase range')
assert.match(captions, /rangeMs=\{rangeMs\}[\s\S]*onActivate=\{onSeek\}/, 'captions retain the selected clip range and route its seek through the app owner')
assert.match(markers, /atMs=\{menu\.atMs\}[\s\S]*onActivate=/, 'markers retain Marker.at_ms as their seek owner')
assert.match(comments, /rangeMs=\{\[time\.atMs, time\.endMs \?\? time\.atMs\]\}/, 'comments retain resolved anchor timing')
assert.match(search, /rangeMs=\{\[hit\.source_start_ms, hit\.source_end_ms\]\}/, 'evidence search keeps its source range distinct')
assert.match(search, /nearestActiveOccurrence\(hit, project, playheadMs\)/, 'evidence search keeps occurrence selection in its existing owner')
assert.match(receipts, /atMs=\{s\.ms\}/, 'receipt evidence retains its existing measured timestamp owner')
assert.match(chapterPanel, /sourceTimelineOccurrences\(project, asset, chapter\.start_ms\)/, 'chapters derive their route from current clip occurrences')
assert.match(chapterPanel, /<TemporalPoint[\s\S]*onActivate=\{onSeek\}/, 'single chapter occurrences reuse the shared temporal control')
assert.match(chapterPanel, /<TemporalRange[\s\S]*rangeMs=\{\[chapter\.start_ms, sourceEndMs\]\}/, 'chapter source range stays exact and distinct from project time')
assert.match(chapterPanel, /Used \{disposition\.occurrences\.length\} times/, 'reused source chapters require an explicit human choice in novice language')
assert.match(chapterPanel, /data-cut-action="chapter-window"/, 'bounded chapter pages remain directly testable')
assert.match(chapterModel, /No exact occurrence is available/, 'edited-out chapters refuse rather than inventing a time')

console.log('PASS transcript temporal phrase grouping and shared navigation ownership')
