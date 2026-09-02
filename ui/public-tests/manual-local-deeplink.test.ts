import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { externalCutManualFeatureUrl, openCutManual } from '../src/lib/manual'
import {
  INITIAL_LOCAL_MANUAL_STATE,
  openLocalManualArticle,
} from '../src/manual/localManualState'

const uiRoot = resolve(new URL('..', import.meta.url).pathname)
const source = (relative: string) => readFileSync(resolve(uiRoot, relative), 'utf8')
const articleIds = [
  'cut.preview.ffmpeg_setup',
  'cut.export.preflight',
  'cut.left.media_health',
]

const globalState = globalThis as typeof globalThis & { document?: Document; window?: Window }
const originalDocument = globalState.document
const originalWindow = globalState.window
const fakeDocument = new EventTarget()
const openedUrls: unknown[][] = []

try {
  Object.defineProperty(globalState, 'document', { configurable: true, value: fakeDocument })
  Object.defineProperty(globalState, 'window', {
    configurable: true,
    value: { open: (...args: unknown[]) => openedUrls.push(args) },
  })

  for (const featureId of articleIds) {
    let manualState = INITIAL_LOCAL_MANUAL_STATE
    const receiveManualRequest = (event: Event) => {
      const detail = (event as CustomEvent<{ feature?: string }>).detail
      manualState = openLocalManualArticle(manualState, detail?.feature)
    }
    fakeDocument.addEventListener('cut:open-manual', receiveManualRequest)
    openCutManual(featureId)
    fakeDocument.removeEventListener('cut:open-manual', receiveManualRequest)

    assert.equal(manualState.open, true, `${featureId} opens the bundled manual`)
    assert.equal(manualState.requestedFeatureId, featureId, `${featureId} selects its exact article`)
    assert.equal(manualState.requestId, 1, `${featureId} creates one local article request`)
  }

  assert.deepEqual(openedUrls, [], 'contextual manual help never calls window.open')
} finally {
  if (originalDocument === undefined) delete globalState.document
  else Object.defineProperty(globalState, 'document', { configurable: true, value: originalDocument })
  if (originalWindow === undefined) delete globalState.window
  else Object.defineProperty(globalState, 'window', { configurable: true, value: originalWindow })
}

assert.equal(
  externalCutManualFeatureUrl('cut.export.preflight'),
  'https://docs.theshellx.com/manual/cut/?feature=cut.export.preflight',
  'only the explicitly named external helper builds an online-manual URL',
)

const app = source('src/App.tsx')
const panel = source('src/manual/LocalManualPanel.tsx')
const state = source('src/manual/localManualState.ts')

assert.match(
  app,
  /onOpenManual:\s*\(featureId\)\s*=>\s*\{\s*if \(!embeddedManualFrontend\) setLocalManual\(\(state\) => openLocalManualArticle\(state, featureId\)\)/s,
  'App routes cut:open-manual article requests into local manual state',
)
assert.match(
  app,
  /requestedFeatureId=\{localManual\.requestedFeatureId\}[\s\S]*requestId=\{localManual\.requestId\}/,
  'App passes every requested article through to the bundled panel',
)
assert.match(
  panel,
  /useEffect\(\(\) => \{\s*if \(!open \|\| !requestedFeatureId\) return[\s\S]*setSelectedId\(featureId\)/,
  'the bundled panel selects a requested article only after it opens',
)
assert.doesNotMatch(state, /(?:document\.|window\.|cut:manual-reveal)/, 'opening an article is state-only')
assert.match(panel, /onClick=\{showInEditor\}/, 'Show in Cut is the explicit reveal control')
assert.match(
  panel,
  /const showInEditor = \(\) => \{[\s\S]*window\.requestAnimationFrame\(\(\) => revealFeature\(selected\.id\)\)\n  \}/,
  'reveal stays behind the explicit Show in Cut handler',
)
assert.equal((panel.match(/cut:manual-reveal/g) ?? []).length, 1, 'the panel has one reveal event path')

console.log(JSON.stringify({ result: 'PASS', articleIds, localOnly: true }, null, 2))
