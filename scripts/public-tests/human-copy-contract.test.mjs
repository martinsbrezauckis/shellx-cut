import assert from 'node:assert/strict'
import { readdir, readFile } from 'node:fs/promises'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const repoRoot = fileURLToPath(new URL('../../', import.meta.url))
const uiRoot = path.join(repoRoot, 'ui/src')

async function filesUnder(root) {
  const entries = await readdir(root, { withFileTypes: true })
  const nested = await Promise.all(entries.map(async (entry) => {
    const target = path.join(root, entry.name)
    if (entry.isDirectory()) return filesUnder(target)
    return entry.name.endsWith('.tsx') ? [target] : []
  }))
  return nested.flat()
}

function titleRegions(source) {
  const regions = []
  for (const match of source.matchAll(/\btitle\s*=/g)) {
    const start = match.index ?? 0
    let cursor = start + match[0].length
    while (/\s/.test(source[cursor] ?? '')) cursor += 1
    const opener = source[cursor]
    if (opener === '"' || opener === "'") {
      const end = source.indexOf(opener, cursor + 1)
      regions.push(source.slice(cursor + 1, end < 0 ? cursor + 500 : end))
      continue
    }
    if (opener !== '{') continue
    let depth = 1
    let quote = ''
    let escaped = false
    let end = cursor + 1
    for (; end < source.length && depth > 0; end += 1) {
      const char = source[end]
      if (quote) {
        if (escaped) escaped = false
        else if (char === '\\') escaped = true
        else if (char === quote) quote = ''
      } else if (char === '"' || char === "'" || char === '`') quote = char
      else if (char === '{') depth += 1
      else if (char === '}') depth -= 1
    }
    regions.push(source.slice(cursor + 1, end - 1))
  }
  return regions
}

test('primary tooltips never expose public API verb names', async () => {
  const schema = JSON.parse(await readFile(path.join(repoRoot, 'schema/verbs.json'), 'utf8'))
  const verbs = schema.verbs.map((verb) => verb.name)
  const offenders = []
  for (const file of await filesUnder(uiRoot)) {
    const source = await readFile(file, 'utf8')
    for (const region of titleRegions(source)) {
      const leaked = verbs.filter((verb) => region.includes(verb))
      if (leaked.length) offenders.push(`${path.relative(repoRoot, file)}: ${leaked.join(', ')}`)
    }
  }
  assert.deepEqual(offenders, [])
})

test('first-run and primary chrome lead with outcomes', async () => {
  const [wizard, cards, assets, statusbar, topbar] = await Promise.all([
    readFile(path.join(uiRoot, 'panels/Environment/index.tsx'), 'utf8'),
    readFile(path.join(uiRoot, 'panels/Environment/EnvCards.tsx'), 'utf8'),
    readFile(path.join(uiRoot, 'panels/Assets/index.tsx'), 'utf8'),
    readFile(path.join(uiRoot, 'statusbar/index.tsx'), 'utf8'),
    readFile(path.join(uiRoot, 'topbar/index.tsx'), 'utf8'),
  ])
  assert.match(wizard, /groups=\{\['tools'\]\} showMeta=\{false\}/)
  assert.doesNotMatch(wizard, /system\.doctor \{refresh:true\}/)
  assert.match(cards, /showMeta = false/)
  assert.match(cards, /label="Video processing"/)
  assert.match(assets, /'Captions & transcription'/)
  assert.doesNotMatch(assets, /'Perception'/)
  assert.doesNotMatch(statusbar, /cutd :|env: ffmpeg|render judges/)
  assert.match(topbar, />Faster \{useGpu \? 'ON' : 'OFF'\}<\/span>/)
  assert.doesNotMatch(topbar, /render\.final \{preset:/)
})

test('review and drawer copy does not render raw API payloads', async () => {
  const files = [
    'panels/Comments/index.tsx',
    'panels/GenerateTemplates/PromptPanel.tsx',
    'panels/Layer/index.tsx',
    'panels/Shape/index.tsx',
    'panels/Matte/index.tsx',
    'panels/Grade/index.tsx',
    'panels/Title/index.tsx',
    'panels/Review/Receipts.tsx',
    'panels/Review/Scopes.tsx',
    'panels/Review/QC.tsx',
    'panels/Review/DiffView.tsx',
    'panels/Review/OpsFeed.tsx',
  ]
  const sources = await Promise.all(files.map((file) => readFile(path.join(uiRoot, file), 'utf8')))
  const visibleCopy = sources.join('\n')
  assert.doesNotMatch(visibleCopy, /JSON\.stringify\((?:v\.args|promptResult\.plan\.params)\)/)
  assert.doesNotMatch(visibleCopy, /Fires <code>|render\.final produces one|project\.checkpoint marks/)
  assert.doesNotMatch(visibleCopy, /<span className="qc__card-tag">(?:verify\.|project\.)/)
  assert.doesNotMatch(visibleCopy, /Create a checkpoint|every verb lands here/)
  assert.match(visibleCopy, /ask Agent Chat to save one/)
})
