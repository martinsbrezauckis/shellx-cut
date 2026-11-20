// The one gate for media URLs built from project or engine data: only a plain
// path on one of the engine's own media routes comes out; everything that could
// reach another host (or run script) gives nothing.
import assert from 'node:assert/strict'
import { engineMediaUrl, safeMediaPath, sameOriginMediaPath } from '../src/lib/mediaUrl'
import { scopeMediaUrl } from '../src/lib/mediaScope'

// Accepted: the shapes the UI builds today.
for (const ok of [
  '/filmstrip/a1.jpg',
  'filmstrip/a1.jpg',
  '/proxies/a1.mp4?p=2-abc',
  'proxies/a1.mp4',
  '/frames/g_1.contact.jpg',
  '/api/frame?at_ms=0&h=120&v=op_0004',
  '/api/source/a1',
  '/api/export/bundle/pack%20one.zip',
  '/api/export-file?path=C%3A%5CUsers%5Cme%5Cout.mp4',
  '/api/library-poster?id=x1',
  '/api/recording-rehearsal/h1',
]) assert.ok(safeMediaPath(ok), `should accept ${ok}`)
assert.equal(safeMediaPath('filmstrip/a1.jpg'), '/filmstrip/a1.jpg')

// Refused: nothing that can leave the engine origin or run code.
for (const bad of [
  '//attacker.example/x.png',
  '///attacker.example/x.png',
  '/attacker.example/x.png',                 // not a media route (the project field "/attacker.example/x.png" after one added slash is "//...")
  'http://attacker.example/x.png',
  'https://attacker.example/x.png',
  'HTTPS://attacker.example/x.png',
  'javascript:alert(1)',
  '/javascript:alert(1)',
  'data:image/png;base64,AAAA',
  'blob:http://x/1',
  'file:///etc/passwd',
  '\\\\attacker.example\\x.png',
  '/\\attacker.example/x.png',
  '\\/attacker.example/x.png',
  '/filmstrip\\..\\x.png',
  '/\tattacker.example/x.png',
  '/\nattacker.example/x.png',
  ' //attacker.example/x.png',
  '/%2f%2fattacker.example/x.png',
  '/filmstrip/%2e%2e/%2e%2e/x',
  '/filmstrip/../api/verb/x',
  '/filmstrip/..%2f..%2fx',
  '/filmstrip/a%5cb.png',
  '/filmstrip/a%00.png',
  '/filmstrip/%zz',
  '/api/verb/project.open',
  '/index.html',
  '/',
  '',
  'x'.repeat(5000),
]) assert.equal(safeMediaPath(bad), null, `should refuse ${JSON.stringify(bad)}`)
for (const bad of [null, undefined, 5, {}, ['/filmstrip/a.jpg']]) assert.equal(safeMediaPath(bad), null)

// The sink every scoped URL goes through gives '' for an unsafe field, and the
// exact form the UI builds from the project's filmstrip field is the attack form.
assert.equal(scopeMediaUrl('/attacker.example/seen.png'), '')
assert.equal(scopeMediaUrl(`/${'/attacker.example/seen.png'}`), '')
assert.equal(scopeMediaUrl('/filmstrip/a1.jpg', '2-abc'), '/filmstrip/a1.jpg?p=2-abc')
assert.equal(scopeMediaUrl('proxies/a1.mp4', '2-abc'), '/proxies/a1.mp4?p=2-abc')
assert.equal(scopeMediaUrl('/api/frame?at_ms=0', '2-abc'), '/api/frame?at_ms=0&p=2-abc')
assert.equal(scopeMediaUrl('//attacker.example/a.png', '2-abc'), '')
assert.equal(engineMediaUrl('//attacker.example/a.png'), undefined)
assert.equal(engineMediaUrl('/frames/a.png'), '/frames/a.png')

// Page-resolved CSS urls: the page's own origin only.
const page = 'http://127.0.0.1:6161'
assert.equal(sameOriginMediaPath(`${page}/filmstrip/a1.jpg?p=1`, page), '/filmstrip/a1.jpg?p=1')
assert.equal(sameOriginMediaPath('http://attacker.example/filmstrip/a1.jpg', page), null)
assert.equal(sameOriginMediaPath('http://127.0.0.1:6162/filmstrip/a1.jpg', page), null)
assert.equal(sameOriginMediaPath('https://127.0.0.1:6161/filmstrip/a1.jpg', page), null)
assert.equal(sameOriginMediaPath('http://user:pw@127.0.0.1:6161/filmstrip/a1.jpg', page), null)
assert.equal(sameOriginMediaPath(`${page}/api/verb/project.open`, page), null)
assert.equal(sameOriginMediaPath('data:image/png;base64,AAAA', page), null)
assert.equal(sameOriginMediaPath('not a url', page), null)
console.log('media-url ok')
