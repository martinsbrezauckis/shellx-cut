// Migrated from the retained legacy aggregate: a small current contract for
// actionable screenshot-capture failures, owned by capture.ts and events.ts.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { CaptureError, captureFailureDetail, describeCaptureError } from '../src/lib/capture'

const fakeImgError = new Event('error')
Object.defineProperty(fakeImgError, 'target', {
  value: { tagName: 'IMG', currentSrc: 'http://127.0.0.1:6161/api/frame?at_ms=0' },
})
const detail = captureFailureDetail(fakeImgError)
assert.doesNotMatch(detail, /\[object Event\]/)
assert.match(detail, /error event/)
assert.match(detail, /<img>/)
assert.match(detail, /\/api\/frame/)

const staged = describeCaptureError(new CaptureError('dom-rasterize', detail))
assert.equal(staged.stage, 'dom-rasterize')
assert.match(staged.message, /dom-rasterize/)
assert.equal(describeCaptureError('plain failure').stage, 'unknown')
assert.equal(describeCaptureError(new Error('boom')).message, 'boom')

const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const eventsSrc = readFileSync(resolve(uiRoot, 'src/lib/events.ts'), 'utf8')
assert.equal((eventsSrc.match(/await capture\.captureApp\(\)/g) || []).length, 2)
assert.match(eventsSrc, /code: 'capture_failed'/)
assert.match(eventsSrc, /stage: described\.stage/)
assert.match(eventsSrc, /attempts: 2/)

const captureSrc = readFileSync(resolve(uiRoot, 'src/lib/capture.ts'), 'utf8')
assert.match(captureSrc, /new CaptureError\('dom-rasterize'/)
assert.match(captureSrc, /new CaptureError\('png-encode'/)

console.log('PASS capture error diagnostics contract')
