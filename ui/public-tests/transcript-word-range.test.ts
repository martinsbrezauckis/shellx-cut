import assert from 'node:assert/strict'
import { activeCutSpans, effectsSummary, opSeekMs, type CutSpan } from '../src/panels/Review/shared'
import { wordIndexRangeFrom } from '../src/panels/Review/wordIndexRange'
import { cutSpansByAsset, cutWordsByIndex } from '../src/panels/Transcript/cutWordProjection'
import type { OpRecord, WordSpan } from '../src/lib/client'

const op = (op_id: string, args: unknown, extra: Partial<OpRecord> = {}): OpRecord => ({
  op_id, verb: 'transcript.cut_words', args, status: 'applied', ts: '',
  actor: { kind: 'human', name: 'test', via: 'test' }, ...extra,
})
const invalid: unknown[] = [
  null, {}, '0,1', [], [0], [0, 1, 2], ['0', 1], [0, '1'],
  [-1, 1], [0, -1], [2, 1], [0.5, 1], [0, 1.5],
  [NaN, 1], [0, NaN], [-Infinity, 1], [0, Infinity],
  [2 ** 53, 2 ** 53], [0, Number.MAX_SAFE_INTEGER + 1],
]
for (const range of invalid) {
  assert.equal(wordIndexRangeFrom(range), null)
  assert.deepEqual(activeCutSpans([op('args', { asset: 'a', word_range: range })]), [])
  assert.deepEqual(activeCutSpans([op('effect', {}, { effects: [{ asset: 'a', word_range: range }] })]), [])
}
assert.deepEqual(wordIndexRangeFrom([0, Number.MAX_SAFE_INTEGER]), [0, Number.MAX_SAFE_INTEGER])
assert.deepEqual(activeCutSpans([op('malformed-effects', {}, {
  effects: [null, 2, 'annotation', {}, { word_range: [0, 1] }] as unknown as OpRecord['effects'],
})]), [])
assert.deepEqual(activeCutSpans([op('non-array-effects', {}, {
  effects: {} as OpRecord['effects'],
})]), [])

const first = op('first', { asset: 'a', word_range: [2, 9] }, {
  effects: [{ asset: 'a', word_range: [10, 11] }], rationale: 'keep identity',
})
const huge = op('huge', { asset: 'a', word_range: [0, Number.MAX_SAFE_INTEGER] })
const fallback = op('fallback', { asset: 'a', word_range: [0, Infinity] }, {
  effects: [{ asset: 'a', word_range: [20, 25] }, { asset: 'b', word_range: [3, 4] }],
})
assert.deepEqual(activeCutSpans([first, fallback]), [
  { opId: 'first', asset: 'a', wordRange: [2, 9], rationale: 'keep identity' },
  { opId: 'fallback', asset: 'a', wordRange: [20, 25], rationale: undefined },
  { opId: 'fallback', asset: 'b', wordRange: [3, 4], rationale: undefined },
], 'valid args win; invalid args allow valid per-effect fallback with original identity')
const restore = op('restore', { op_id: 'first' }, { verb: 'edit.restore' })
assert.deepEqual(activeCutSpans([first, huge, restore, op('rejected', { asset: 'a', word_range: [0, 1] }, { status: 'rejected' })])
  .map(span => span.opId), ['huge'])
assert.equal(activeCutSpans([first, { ...restore, status: 'rejected' }]).length, 1)
assert.equal(activeCutSpans([op('other', { asset: 'a', word_range: [0, 1] }, { verb: 'clip.trim' })]).length, 0)

const keyedCuts = activeCutSpans([
  op('prototype', { asset: '__proto__', word_range: [0, 1] }),
  op('constructor', {}, { effects: [{ asset: 'constructor', word_range: [2, 3] }] }),
  first, huge,
])
const keyed = cutSpansByAsset(keyedCuts)
assert.deepEqual([...keyed.keys()], ['__proto__', 'constructor', 'a'])
assert.deepEqual(keyed.get('__proto__'), [keyedCuts[0]])
assert.deepEqual(keyed.get('constructor'), [keyedCuts[1]])
assert.deepEqual(keyed.get('a'), keyedCuts.slice(2), 'ordinary asset grouping preserves first-cut order')
assert.equal(keyed.get('toString'), undefined, 'absent inherited names have no invented annotations')

const words: WordSpan[] = [2, 9, 100_000, Number.MAX_SAFE_INTEGER].map(idx => ({
  idx, word: `word-${idx}`, start_ms: 0, end_ms: 1,
}))
const cuts = activeCutSpans([first, huge])
const projection = cutWordsByIndex(words, cuts)
assert.deepEqual([...projection.keys()], words.map(word => word.idx), 'work and allocation follow loaded words, including sparse indices')
assert.equal(projection.get(2), cuts[0])
assert.equal(projection.get(9), cuts[0], 'inclusive endpoints and first-cut-wins remain intact')
assert.equal(projection.get(100_000), cuts[1], 'indices beyond array length remain real word identities')
assert.equal(projection.get(Number.MAX_SAFE_INTEGER), cuts[1])
assert.equal(cutWordsByIndex([], cuts).size, 0)
assert.equal(cutWordsByIndex(words, []).size, 0)
assert.equal(cutWordsByIndex(words, invalid.map(wordRange => ({ opId: 'bad', asset: 'a', wordRange } as CutSpan))).size, 0,
  'projection also refuses invalid ranges from direct callers')
assert.deepEqual([...cutWordsByIndex([words[2]], cuts).keys()], [100_000], 'updated loaded words determine the projection')

const timing = op('timing', {}, { effects: [{ track: 'v1', removed_ms: [0.5, 1000.5] }] })
assert.equal(effectsSummary(timing), 'v1 −1.0s @ 00:00.0')
assert.equal(opSeekMs(timing), 0.5, 'time summaries/seeking retain fractional millisecond semantics')
console.log('PASS transcript word annotation bounds, sparse projection, precedence, overlap and restore')
