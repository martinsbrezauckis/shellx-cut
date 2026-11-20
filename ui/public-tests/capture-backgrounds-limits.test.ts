// The screenshot's background repair is bounded: it loads only the app's own media
// routes, at most MAX_IMAGES distinct images, all within one time budget; a load
// still running at the end is cancelled and a late answer is ignored.
import assert from 'node:assert/strict'
import {
  MAX_IMAGES,
  backgroundNotes,
  repairBackgrounds,
  type BackgroundElement,
} from '../src/lib/captureBackgrounds'

const ORIGIN = 'http://127.0.0.1:6161'
const ctx = {} as CanvasRenderingContext2D // never touched: no image loads in these cases

const element = (...urls: string[]): BackgroundElement => ({
  x: 0, y: 0, w: 100, h: 40,
  layers: urls.map((url) => ({ url, raw: `url("${url}")`, size: 'auto', position: '0% 0%', repeat: 'repeat' })),
})

interface FakeImage {
  src: string
  onload: (() => void) | null
  onerror: (() => void) | null
  requested: string[]
}

/** An image factory: `behaviour` decides what a request does ('fail' errors at once,
 *  'stall' never answers). Records every URL ever requested and every image made. */
function factory(behaviour: (url: string) => 'fail' | 'stall') {
  const made: FakeImage[] = []
  const requested: string[] = []
  const make = () => {
    let src = ''
    const img: FakeImage = {
      get src() { return src },
      set src(v: string) {
        src = v
        if (v === '') return
        requested.push(v)
        if (behaviour(v) === 'fail') queueMicrotask(() => img.onerror?.())
      },
      onload: null,
      onerror: null,
      requested,
    }
    made.push(img)
    return img as unknown as HTMLImageElement
  }
  return { make, made, requested }
}

const repair = (els: BackgroundElement[], f: ReturnType<typeof factory>, o: Parameters<typeof repairBackgrounds>[4] = {}) =>
  repairBackgrounds(ctx, els, { x: 0, y: 0 }, 1, { pageOrigin: ORIGIN, makeImage: f.make, ...o })

// 1. Foreign and unsafe URLs are never requested, and the notes say so.
{
  const f = factory(() => 'fail')
  const bad = [
    'http://attacker.example/filmstrip/a1.jpg',
    'https://127.0.0.1:6161/filmstrip/a1.jpg',
    'http://127.0.0.1:6162/filmstrip/a1.jpg',
    'http://user:pw@127.0.0.1:6161/filmstrip/a1.jpg',
    'javascript:alert(1)',
    'data:image/png;base64,AAAA',
    `${ORIGIN}/api/verb/project.open`,
  ]
  const report = await repair([element(...bad)], f)
  assert.equal(f.made.length, 0, 'no image may be created for a refused URL')
  assert.deepEqual(report.skipped.map((s) => s.url), bad)
  assert.ok(report.skipped.every((s) => s.reason === 'foreign'))
  assert.deepEqual(report.missing, [])
  const notes = backgroundNotes(report)
  assert.equal(notes.length, 1)
  assert.match(notes[0], /^7 CSS background images were not checked or redrawn in this capture: 7 outside this app's own media routes/)
  assert.match(notes[0], /attacker\.example/)
}

// 2. The number of images is capped; the rest is named, not dropped.
{
  const f = factory(() => 'fail')
  const urls = Array.from({ length: 60 }, (_, i) => `${ORIGIN}/filmstrip/a${i}.jpg?p=1`)
  const report = await repair(urls.map((u) => element(u)), f)
  assert.equal(f.made.length, MAX_IMAGES)
  assert.equal(report.missing.length, MAX_IMAGES)
  assert.equal(report.skipped.length, 60 - MAX_IMAGES)
  assert.ok(report.skipped.every((s) => s.reason === 'limit'))
  assert.match(backgroundNotes(report).join('\n'), new RegExp(`${60 - MAX_IMAGES} over the limit of ${MAX_IMAGES} images`))
}

// 3. A stalled image ends at the total budget, is cancelled, and a late answer is ignored.
{
  const f = factory(() => 'stall')
  const urls = Array.from({ length: 5 }, (_, i) => `${ORIGIN}/filmstrip/s${i}.jpg`)
  const started = Date.now()
  const report = await repair(urls.map((u) => element(u)), f, { budgetMs: 150, imageTimeoutMs: 60_000 })
  const took = Date.now() - started
  assert.ok(took >= 140 && took < 1500, `the budget must end the run (took ${took} ms)`)
  assert.equal(f.made.length, 1, 'nothing new starts after the first stalled load')
  const img = f.made[0] as unknown as FakeImage
  assert.equal(img.src, '', 'the cancelled load has its source cleared')
  assert.equal(img.onload, null)
  assert.equal(img.onerror, null)
  assert.deepEqual(report.skipped.map((s) => s.reason), ['deadline', 'deadline', 'deadline', 'deadline', 'deadline'])
  assert.deepEqual(report.redrawn, [])
  assert.deepEqual(report.missing, [])
  const before = JSON.stringify(report)
  img.onload?.() // a late answer has nothing to call
  await new Promise((r) => setTimeout(r, 30))
  assert.equal(JSON.stringify(report), before, 'a late answer changes nothing')
  assert.match(backgroundNotes(report).join('\n'), /5 not reached within 4 s/)
}

// 4. One image that never answers is given up on after its own limit, with its source cleared.
{
  const f = factory((u) => (u.includes('slow') ? 'stall' : 'fail'))
  const report = await repair(
    [element(`${ORIGIN}/filmstrip/slow.jpg`), element(`${ORIGIN}/filmstrip/next.jpg`)],
    f,
    { budgetMs: 5000, imageTimeoutMs: 40 },
  )
  assert.equal((f.made[0] as unknown as FakeImage).src, '')
  assert.equal(f.made.length, 2, 'the run goes on after one image timed out')
  assert.equal(report.missing.length, 2)
  assert.deepEqual(report.skipped, [])
}
console.log('capture backgrounds: foreign urls, count cap and deadline')
