import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { strict as assert } from 'node:assert'

const root = resolve(import.meta.dirname, '..')
const control = readFileSync(resolve(root, 'src/panels/Environment/SttModelControl.tsx'), 'utf8')
const nativeCoverage = readFileSync(resolve(root, 'public-tests/lib/fullCoverageSettingsEnvironment.mjs'), 'utf8')

assert.match(control, /Caption model set to/)
assert.match(control, /Caption model reset to/)
assert.match(control, /data-cut-env-stt-busy=/)
assert.match(nativeCoverage, /waitSttConfirmation/)
assert.match(nativeCoverage, /Caption model reset to Parakeet v3/)

console.log('PASS STT model completion feedback and native action synchronization contract')
