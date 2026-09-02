import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { relative, resolve } from 'node:path'
import vm from 'node:vm'

export const root = resolve(new URL('../..', import.meta.url).pathname)
export const siteRoot = resolve(root, 'docs/public/site')
export const manualRoot = resolve(siteRoot, 'manual')
export const manualHtml = resolve(manualRoot, 'cut/index.html')
export const manualBehavior = resolve(manualRoot, 'manual.js')
export const manualStyle = resolve(manualRoot, 'manual.css')
export const targetAtlasPath = resolve(root, 'scripts/public-tests/fixtures/cut-manual-highlight-target-atlas.json')
export const liveUrl = 'https://docs.theshellx.com/manual/cut/'

export function sha256(value) {
  return createHash('sha256').update(value).digest('hex')
}

export function relativeToRoot(path) {
  return relative(root, path).replaceAll('\\', '/')
}

export function loadManualData(source) {
  const transformed = source
    .replace('const features = {', 'var features = {')
    .replace('const legacyFeatureAliases = {', 'var legacyFeatureAliases = {')
  assert.notEqual(transformed, source, 'manual feature definitions remain a literal object')
  const context = { document: { addEventListener() {} }, window: { addEventListener() {} } }
  vm.runInNewContext(transformed, context, { timeout: 500, codeGeneration: { strings: false, wasm: false } })
  return {
    features: JSON.parse(JSON.stringify(context.features)),
    legacyFeatureAliases: JSON.parse(JSON.stringify(context.legacyFeatureAliases || {})),
  }
}

function pngDimensions(bytes) {
  assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a', 'atlas capture is a PNG')
  return [bytes.readUInt32BE(16), bytes.readUInt32BE(20)]
}

export async function loadTargetAtlas() {
  const source = await readFile(targetAtlasPath, 'utf8')
  const atlas = JSON.parse(source)
  assert.equal(atlas.schemaVersion, 2, 'target atlas schema is current')
  assert.equal(atlas.status, 'complete', 'target atlas is a completed review, not a provisional target list')
  assert.ok(atlas.review?.reviewer && atlas.review?.reviewedAt, 'target atlas records its reviewer and review date')
  assert.ok(atlas.captures?.editor && atlas.captures?.recording, 'target atlas covers the editor and Record captures')
  assert.ok(Object.keys(atlas.targets || {}).length > 0, 'target atlas has reviewed targets')
  assert.equal(atlas.targetSet?.count, Object.keys(atlas.targets).length, 'target atlas declares its exact reviewed target count')
  return { atlas, sha256: sha256(source) }
}

export async function verifyAtlasAssets(atlas) {
  for (const capture of Object.values(atlas.captures)) {
    const bytes = await readFile(resolve(siteRoot, capture.path))
    assert.equal(sha256(bytes), capture.sha256, `${capture.path} still matches the reviewed capture hash`)
    assert.deepEqual(pngDimensions(bytes), [capture.width, capture.height], `${capture.path} still matches the reviewed capture dimensions`)
  }
}

export function verifyFeatureSet(features, atlas) {
  const ids = Object.keys(features).sort()
  assert.equal(ids.length, atlas.expectedFeatureCount, 'target atlas declares the expected feature count')
  assert.equal(ids.length, atlas.featureSet?.count, 'target atlas declares the exact manual feature count')
  assert.equal(sha256(ids.join('\n')), atlas.featureSet?.sha256, 'target atlas declares the exact manual feature set')
  assert.deepEqual(Object.keys(atlas.featureTargets || {}).sort(), ids, 'target atlas assigns every and only selectable manual feature ids')
  const targetLabels = new Set()
  const targetsByFeature = new Map()
  for (const [targetId, target] of Object.entries(atlas.targets)) {
    assert.ok(atlas.captures[target.capture], `${targetId} is anchored to a reviewed capture`)
    assert.ok(target.label && Number.isFinite(target.left) && Number.isFinite(target.top) && Number.isFinite(target.width) && Number.isFinite(target.height), `${targetId} has a complete reviewed target box`)
    assert.ok(!targetLabels.has(target.label), `target atlas label is unique: ${target.label}`)
    targetLabels.add(target.label)
  }
  for (const id of ids) {
    const targetId = atlas.featureTargets[id]
    const target = atlas.targets[targetId]
    assert.ok(target, `${id} maps to a declared independent target`)
    targetsByFeature.set(id, { id: targetId, ...target })
  }
  return { ids, targetsByFeature }
}

export function targetForFeature(id, feature, targetsByFeature) {
  const target = targetsByFeature.get(id)
  assert.ok(target, `manual feature has an independently reviewed target: ${id}`)
  assert.equal(feature.highlight.label, target.label, `${id} uses the atlas target label`)
  assert.equal(feature.highlight.surface || 'editor', target.capture, `${id} uses the atlas target surface`)
  return target
}

function hrefsFromHtml(html) {
  return [...html.matchAll(/(?:src|href)="([^"]+)"/g)].map((match) => match[1])
}

export async function verifyLiveAssets() {
  const [sourceHtml, sourceJs, sourceCss] = await Promise.all([readFile(manualHtml, 'utf8'), readFile(manualBehavior, 'utf8'), readFile(manualStyle, 'utf8')])
  const sourceUrls = hrefsFromHtml(sourceHtml).filter((href) => href.startsWith('../'))
  const assetPaths = ['../manual.js', '../manual.css', ...sourceUrls.filter((href) => href.includes('/assets/cut/'))]
  const failures = []
  for (const rawPath of [...new Set(assetPaths)]) {
    const localPath = resolve(resolve(manualHtml, '..'), rawPath.split('?')[0])
    const relativePath = relative(siteRoot, localPath).replaceAll('\\', '/')
    const source = await readFile(localPath)
    const response = await fetch(new URL(`/${relativePath}`, liveUrl))
    if (!response.ok) {
      failures.push({ asset: relativePath, issue: `live asset request failed with ${response.status}` })
      continue
    }
    const live = Buffer.from(await response.arrayBuffer())
    if (sha256(live) !== sha256(source)) failures.push({ asset: relativePath, issue: 'live asset SHA-256 differs from source', sourceSha256: sha256(source), liveSha256: sha256(live) })
  }
  assert.ok(sourceJs.length > 0 && sourceCss.length > 0, 'manual behavior and style sources are present')
  return failures
}
