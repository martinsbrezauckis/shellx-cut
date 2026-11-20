// Dropping a mix of files and folders imports the media files and tells the
// person plainly which items were left out and why.
import assert from 'node:assert/strict'
import { planDrop } from '../src/lib/projectBootstrap'

const drop = [
  'C:\\Users\\tester\\Videos\\clip one.mp4',
  'C:\\Users\\tester\\Videos\\Holiday photos',
  'C:\\Users\\tester\\Videos\\logo.png',
  'C:\\Users\\tester\\Videos\\Raw takes',
  'C:\\Users\\tester\\Videos\\Archive',
  'C:\\Users\\tester\\Videos\\notes.txt',
  '/home/tester/music/theme.mp3',
]
const plan = planDrop(drop)
assert.deepEqual(plan.media, [
  'C:\\Users\\tester\\Videos\\clip one.mp4',
  'C:\\Users\\tester\\Videos\\logo.png',
  '/home/tester/music/theme.mp3',
])
const notice = plan.skippedNotice ?? ''
assert.match(notice, /Holiday photos/)
assert.match(notice, /Raw takes/)
assert.match(notice, /Archive/)
assert.match(notice, /and 1 more/, 'long lists are shortened, not dropped')
assert.match(notice, /not supported media files/)
assert.match(notice, /Folders are not imported: open the folder and drop the files inside it/)

const one = planDrop(['C:\\media\\Takes'])
assert.equal(one.media.length, 0)
assert.match(one.skippedNotice ?? '', /“Takes”\. It is not a supported media file\./)

assert.equal(planDrop(['/a/b.mp4', '/a/c.wav']).skippedNotice, null, 'nothing skipped, nothing said')
console.log('drop: skipped items are named with the reason')
