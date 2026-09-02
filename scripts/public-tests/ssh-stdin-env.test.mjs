#!/usr/bin/env node
import { strict as assert } from 'node:assert'

import { buildSshEnvPayload, readEnvFirstLine } from '../lib/ssh-stdin-env.mjs'

assert.equal(readEnvFirstLine('TEST_TOKEN', { TEST_TOKEN: '\n  secret-value  \nignored\n' }), 'secret-value')
assert.equal(readEnvFirstLine('TEST_TOKEN', {}), '')
assert.throws(() => readEnvFirstLine('bad-name', {}), /invalid environment variable name/)

const secured = buildSshEnvPayload('secret-value', 'TEST_TOKEN', 'REMOTE_DIR=repo', 'echo test')
assert.match(secured.command, /IFS= read -r TEST_TOKEN/)
assert.doesNotMatch(secured.command, /secret-value/)
assert.equal(secured.input, 'secret-value\necho test')

const ordinary = buildSshEnvPayload('', 'TEST_TOKEN', 'REMOTE_DIR=repo', 'echo test')
assert.equal(ordinary.command, 'REMOTE_DIR=repo bash -s')
assert.equal(ordinary.input, 'echo test')

console.log('ssh stdin environment tests passed')
