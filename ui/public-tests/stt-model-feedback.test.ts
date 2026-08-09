import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { strict as assert } from 'node:assert'

const root = resolve(import.meta.dirname, '..')
const control = readFileSync(resolve(root, 'src/panels/Environment/SttModelControl.tsx'), 'utf8')

assert.match(control, /Caption model set to/)
assert.match(control, /Caption model reset to/)
assert.match(control, /data-cut-env-stt-busy=/)

console.log('PASS STT model completion feedback contract')
