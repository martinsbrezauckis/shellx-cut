import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { dirname, resolve } from 'node:path'
import { promisify } from 'node:util'

import { MANUAL_CONTENT, MANUAL_FEATURES } from '../src/manual/content'
import {
  CUT_MANUAL_PROTOCOL,
  isManualFrontendMessage,
  manualPostMessageTargetOrigin,
} from '../src/manual/protocol'
import { manualFeatureTarget } from '../src/manual/targets'
import { uiSurface } from '../src/app/uiSurfaceRegistry'

const run = promisify(execFile)
const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const root = resolve(uiRoot, '..')
const manualRoot = resolve(uiRoot, 'src/manual')

async function source(path: string): Promise<string> {
  return readFile(path, 'utf8')
}

async function verifyGeneratedManual(): Promise<void> {
  await run(process.execPath, [resolve(root, 'scripts/generate-manual-content.mjs'), '--check'], { cwd: root })
  assert.equal(MANUAL_CONTENT.featureCount, 199, 'the generated manual retains its 199 indexed entries')
  assert.equal(MANUAL_FEATURES.length, 199, 'the generated feature array retains every indexed entry')
  assert.equal(new Set(MANUAL_FEATURES.map((feature) => feature.id)).size, 199, 'every indexed manual feature id is unique')
  assert.equal(MANUAL_CONTENT.unindexed.length, 0, 'every declared web-manual feature is indexed into the frontend content')
}

async function verifySchemaReferenceParity(): Promise<void> {
  const [schemaSource, reference] = await Promise.all([
    source(resolve(root, 'schema/verbs.json')),
    source(resolve(root, 'skill/shellx-cut/reference.md')),
  ])
  const schema = JSON.parse(schemaSource) as { verbs: Array<{ name: string }> }
  const schemaNames = schema.verbs.map((verb) => verb.name)
  const referenceNames = [...reference.matchAll(/^\| `([^`]+)`/gm)].map((match) => match[1])

  assert.equal(schemaNames.length, 306, 'the public verb schema remains the deliberate 306-verb contract')
  assert.equal(new Set(schemaNames).size, 306, 'schema verb names remain unique')
  assert.equal(referenceNames.length, 306, 'the full agent reference remains a 306-verb table')
  assert.equal(new Set(referenceNames).size, 306, 'agent-reference verb names remain unique')
  assert.deepEqual([...referenceNames].sort(), [...schemaNames].sort(), 'schema and full agent reference retain exact verb parity')
}

function verifyFeatureTargets(): { exactTargetCount: number; surfaceTargetCount: number; unavailableCount: number } {
  let exactTargetCount = 0
  let surfaceTargetCount = 0
  let unavailableCount = 0

  for (const feature of MANUAL_FEATURES) {
    const target = manualFeatureTarget(feature.id)
    assert.equal(target.featureId, feature.id, `${feature.id} resolves to itself`)
    assert.ok(target.precision === 'exact' || target.precision === 'surface', `${feature.id} declares a supported target precision`)

    if (target.unavailable) {
      unavailableCount += 1
      assert.equal(target.precision, 'surface', `${feature.id} is explicitly unavailable rather than mislabelled exact`)
      assert.equal(target.surface, undefined, `${feature.id} unavailable reason does not invent a UI surface`)
      assert.equal(target.selector, undefined, `${feature.id} unavailable reason does not invent a selector`)
      assert.match(target.unavailable, /do not correspond|unavailable|not available/i, `${feature.id} states a truthful unavailable reason`)
      continue
    }

    assert.ok(target.selector || target.surface, `${feature.id} resolves to a real editor target when it is available`)
    if (target.surface) {
      const surface = uiSurface(target.surface)
      assert.ok(surface, `${feature.id} surface exists in the UI registry`)
      if (target.precision === 'surface') {
        assert.equal(target.selector, surface.selector, `${feature.id} surface target selector comes from its actual UI surface`)
      }
    }
    if (target.precision === 'exact') {
      assert.ok(target.selector, `${feature.id} exact target identifies a concrete selector`)
      exactTargetCount += 1
    } else surfaceTargetCount += 1
  }

  assert.ok(exactTargetCount > 0, 'manual target coverage includes real exact-control targets')
  assert.ok(exactTargetCount < MANUAL_FEATURES.length, 'manual target coverage does not pretend every indexed entry is an exact control')
  assert.equal(exactTargetCount + surfaceTargetCount + unavailableCount, MANUAL_FEATURES.length, 'every indexed feature has either a target or an explicit unavailable state')
  return { exactTargetCount, surfaceTargetCount, unavailableCount }
}

function verifyProtocol(): void {
  const valid = [
    { schema: CUT_MANUAL_PROTOCOL, type: 'ready', mode: 'embedded' },
    { schema: CUT_MANUAL_PROTOCOL, type: 'ready', mode: 'local' },
    { schema: CUT_MANUAL_PROTOCOL, type: 'selected', featureId: 'cut.top.manual' },
    { schema: CUT_MANUAL_PROTOCOL, type: 'reveal', featureId: 'cut.top.manual', requestId: 'request-1' },
    { schema: CUT_MANUAL_PROTOCOL, type: 'reveal-result', featureId: 'cut.top.manual', requestId: 'request-1', status: 'shown' },
  ]
  const malformed: unknown[] = [
    null,
    'manual',
    {},
    { schema: 'shellx-cut/manual-frontend@0', type: 'ready', mode: 'embedded' },
    { schema: CUT_MANUAL_PROTOCOL, type: 'ready', mode: 'remote' },
    { schema: CUT_MANUAL_PROTOCOL, type: 'selected' },
    { schema: CUT_MANUAL_PROTOCOL, type: 'selected', featureId: 42 },
    { schema: CUT_MANUAL_PROTOCOL, type: 'reveal', featureId: 'cut.top.manual' },
    { schema: CUT_MANUAL_PROTOCOL, type: 'reveal', featureId: 'cut.top.manual', requestId: 42 },
    { schema: CUT_MANUAL_PROTOCOL, type: 'reveal-result', featureId: 'cut.top.manual', requestId: 'request-1', status: 'rendered' },
    { schema: CUT_MANUAL_PROTOCOL, type: 'unknown', featureId: 'cut.top.manual' },
  ]

  for (const message of valid) assert.equal(isManualFrontendMessage(message), true, `valid ${message.type} protocol message is accepted`)
  for (const message of malformed) assert.equal(isManualFrontendMessage(message), false, `malformed protocol message is rejected: ${JSON.stringify(message)}`)
}

async function verifyFrontendStructure(): Promise<void> {
  const [main, shell, bridge, css, localPanel, localCss] = await Promise.all([
    source(resolve(uiRoot, 'src/main.tsx')),
    source(resolve(manualRoot, 'ManualShell.tsx')),
    source(resolve(manualRoot, 'useManualFrontendBridge.ts')),
    source(resolve(manualRoot, 'manual-shell.css')),
    source(resolve(manualRoot, 'LocalManualPanel.tsx')),
    source(resolve(manualRoot, 'local-manual.css')),
  ])
  const frontend = `${shell}\n${bridge}\n${css}\n${localPanel}\n${localCss}`
  const interactiveEntries = `${shell}\n${bridge}\n${localPanel}`

  assert.match(main, /isManualShell/, 'the normal entrypoint selects the in-app manual shell')
  assert.match(main, /ManualShell/, 'the normal entrypoint mounts the real manual shell')
  assert.match(frontend, /searchParams\.set\(['"]manual['"],\s*['"]embed['"]\)/, 'embedded frontend URL declares manual=embed')
  assert.match(frontend, /searchParams\.set\(['"]mock['"],\s*['"]1['"]\)/, 'embedded frontend URL declares mock=1')
  assert.match(shell, /URLSearchParams\(window\.location\.search\)\.get\(['"]feature['"]\)/, 'the interactive shell retains established ?feature= deep links')
  assert.match(shell, /<iframe\b/, 'the shell embeds the real frontend instead of a screenshot')
  assert.match(shell, /sandbox=['"]allow-scripts allow-same-origin['"]/, 'the trusted read-only editor keeps its same origin in native WebViews')
  assert.doesNotMatch(frontend, /manual-highlight|data-manual-highlight|screenshot|<img\b|\.(?:png|jpe?g|webp|gif)\b/i, 'the new frontend manual entries contain no screenshot hotspot implementation')
  assert.doesNotMatch(interactiveEntries, /\b(?:left|top|right|bottom|width|height)\s*:\s*['"`]?[0-9]+(?:\.[0-9]+)?%/, 'the manual entries do not encode percentage-based hotspots')
}

async function verifyEmbeddedMockIsReadOnly(): Promise<void> {
  const globalState = globalThis as typeof globalThis & { window?: Window; location?: Location }
  const originalWindow = globalState.window
  const originalLocation = globalState.location
  const fallbackCalls: string[] = []
  const embeddedLocation = new URL('http://127.0.0.1/?manual=embed&mock=1')
  const fakeWindow = {
    location: embeddedLocation,
    fetch: async (input: RequestInfo | URL) => {
      fallbackCalls.push(typeof input === 'string' ? input : input.toString())
      return new Response('not found', { status: 404 })
    },
    WebSocket: class {},
  } as unknown as Window

  try {
    globalState.window = fakeWindow
    globalState.location = embeddedLocation as unknown as Location
    await import(`${pathToFileURL(resolve(uiRoot, 'src/panels/Review/mock.ts')).href}?manual-read-only-contract`)

    const readResponse = await fakeWindow.fetch('http://127.0.0.1/api/verb/system.doctor', { method: 'POST', body: '{}' })
    assert.equal(readResponse.ok, true, 'embedded manual mock permits generated read-only verbs')
    const mutationResponse = await fakeWindow.fetch('http://127.0.0.1/api/verb/project.delete', { method: 'POST', body: '{}' })
    const mutation = await mutationResponse.json() as { ok?: boolean; error?: { message?: string } }
    assert.equal(mutation.ok, false, 'embedded manual mock rejects a mutating verb')
    assert.match(mutation.error?.message ?? '', /read-only|manual/i, 'mutating-verb rejection explains the read-only manual boundary')
    assert.deepEqual(fallbackCalls, [], 'mock manual verb traffic never escapes to a real backend')
  } finally {
    if (originalWindow === undefined) delete globalState.window
    else globalState.window = originalWindow
    if (originalLocation === undefined) delete globalState.location
    else globalState.location = originalLocation
  }
}

await verifyGeneratedManual()
assert.equal(manualPostMessageTargetOrigin('null'), '*', 'opaque desktop origins use the valid postMessage target')
assert.equal(manualPostMessageTargetOrigin('https://docs.theshellx.com'), 'https://docs.theshellx.com', 'web manual messages retain an exact target origin')
await verifySchemaReferenceParity()
const targetCoverage = verifyFeatureTargets()
verifyProtocol()
await verifyFrontendStructure()
await verifyEmbeddedMockIsReadOnly()

console.log(JSON.stringify({
  result: 'PASS',
  indexedFeatureCount: MANUAL_FEATURES.length,
  schemaReferenceVerbCount: 306,
  ...targetCoverage,
  exactTargetCoverage: `${targetCoverage.exactTargetCount}/${MANUAL_FEATURES.length}`,
}, null, 2))
