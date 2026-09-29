import assert from 'node:assert/strict'
import { runOrderedTimelineDeletes, type TimelineDeleteRange } from '../src/panels/Timeline/orderedDelete'

const source = ['A', 'B', 'C', 'D']
let clips = [...source]
const history: string[][] = [[...clips]]
const deleted: string[] = []
const selected: TimelineDeleteRange[] = [
  { track: 'v1', start: 0, dur: 1000, id: 'A' },
  { track: 'v1', start: 2000, dur: 1000, id: 'C' },
]
assert.equal(await runOrderedTimelineDeletes(selected, true, async (range) => {
  const index = range.start / 1000
  deleted.push(clips[index] ?? '')
  clips.splice(index, 1)
  history.push([...clips])
  return true
}), true)
assert.deepEqual(deleted, ['C', 'A'])
assert.deepEqual(clips, ['B', 'D'], 'unselected D survives ripple deletion')
assert.deepEqual(history[0], source)
assert.deepEqual(history[history.length - 1], ['B', 'D'])
// One grouped history step: undo restores the original, redo restores the result.
let cursor = 1
const groupedHistory = [history[0], history[history.length - 1]]
clips = [...groupedHistory[--cursor]]
assert.deepEqual(clips, source)
clips = [...groupedHistory[++cursor]]
assert.deepEqual(clips, ['B', 'D'])

const calls: string[] = []
assert.equal(await runOrderedTimelineDeletes(selected, true, async (range) => {
  calls.push(range.id)
  return false
}), false)
assert.deepEqual(calls, ['C'], 'stop after the first refusal')

console.log('ordered delete: A/C retain D, grouped undo/redo model, stop on refusal')
