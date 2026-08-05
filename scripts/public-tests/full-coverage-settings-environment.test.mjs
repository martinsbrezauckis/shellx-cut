import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

const source = readFileSync(
  new URL('../../ui/public-tests/lib/fullCoverageSettingsEnvironment.mjs', import.meta.url),
  'utf8',
)

test('strict Settings actions wait for conditional controls instead of silently skipping them', () => {
  assert.match(source, /await auto[.]waitFor[(]\{ state: 'visible'/)
  assert.match(source, /await sttReset[.]waitFor[(]\{ state: 'visible'/)
  assert.match(source, /await changeFfmpeg[.]waitFor[(]\{ state: 'visible'/)
  assert.doesNotMatch(source, /if [(]await auto[.]count[(][)][)]/)
  assert.doesNotMatch(source, /if [(]await sttReset[.]count[(][)][)]/)
  assert.doesNotMatch(source, /if [(]await changeFfmpeg[.]count[(][)][)]/)
})
