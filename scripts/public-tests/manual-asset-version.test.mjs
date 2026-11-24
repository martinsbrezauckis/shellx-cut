import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

const root = resolve(fileURLToPath(new URL('../..', import.meta.url)))
const inputs = [
  'scripts/generate-manual-content.mjs',
  'docs/public/site/manual/cut/index.html',
  'docs/public/site/manual/manual.js',
  'docs/public/site/manual/manual.css',
  'ui/package.json',
  'ui/src/manual/content.generated.json',
  'ui/src/manual/readOnlyVerbs.generated.ts',
  'schema/verbs.json',
]

function run(root, ...args) {
  return spawnSync(process.execPath, ['scripts/generate-manual-content.mjs', ...args], { cwd: root, encoding: 'utf8' })
}

function version(bytes) {
  return createHash('sha256').update(bytes).digest('hex').slice(0, 10)
}

test('manual shared asset versions follow bytes and check catches stale references', async (t) => {
  const scratch = await mkdtemp(join(root, '.manual-version-test-'))
  try {
    for (const file of inputs) {
      await mkdir(dirname(join(scratch, file)), { recursive: true })
      await copyFile(join(root, file), join(scratch, file))
    }
    const page = join(scratch, 'docs/public/site/manual/cut/index.html')
    const generate = () => {
      const result = run(scratch)
      assert.equal(result.status, 0, result.stderr)
    }
    generate()
    for (const name of ['manual.js', 'manual.css']) {
      const asset = join(scratch, 'docs/public/site/manual', name)
      const assertVersion = async () => {
        const html = await readFile(page, 'utf8')
        assert.ok(html.includes(`../${name}?v=${version(await readFile(asset))}`), `${name} reference must match its content hash`)
      }
      await assertVersion()
      assert.equal(run(scratch, '--check').status, 0)
      await writeFile(asset, Buffer.concat([await readFile(asset), Buffer.from('\n/* changed asset */\n')]))
      const stale = run(scratch, '--check')
      assert.notEqual(stale.status, 0, `${name} change must fail --check`)
      assert.match(stale.stderr, /stale.*generate-manual-content\.mjs/)
      t.diagnostic(`${name}: stale check failed as expected; ${stale.stderr.match(/Error: .*stale.*/)[0]}`)
      generate()
      await assertVersion()
      const current = run(scratch, '--check')
      assert.equal(current.status, 0, current.stderr)
      t.diagnostic(`${name}: check passed after generation`)
    }
  } finally {
    await rm(scratch, { recursive: true, force: true })
  }
})
