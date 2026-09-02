import assert from 'node:assert/strict'
import { readdir, readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'

const directory = dirname(new URL(import.meta.url).pathname)
const files = (await readdir(directory)).filter((file) => /^manual-highlight-.*\.mjs$/.test(file)).sort()
assert.ok(files.length >= 4, 'manual highlight verifier remains split into bounded modules')
const modules = await Promise.all(files.map(async (file) => ({ file, lines: (await readFile(resolve(directory, file), 'utf8')).split('\n').length - 1 })))
for (const module of modules) assert.ok(module.lines <= 350, `${module.file} exceeds the 350-line module limit (${module.lines})`)
console.log(JSON.stringify({ result: 'PASS', moduleLineLimit: 350, modules }))
