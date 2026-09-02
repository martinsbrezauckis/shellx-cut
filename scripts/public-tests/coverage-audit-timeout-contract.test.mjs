#!/usr/bin/env node
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const source = readFileSync(resolve(import.meta.dirname, '../coverage-audit.sh'), 'utf8')

assert.match(source, /AUDIT_REST_TIMEOUT_SECONDS:-30/, 'ordinary structural routes keep a bounded 30-second default')
assert.match(source, /AUDIT_DOCTOR_TIMEOUT_SECONDS:-60/, 'the cold Doctor probe has one explicit 60-second budget')
assert.match(source, /\[\[ "\$v" == "system\.doctor" \]\] && timeout_seconds="\$DOCTOR_TIMEOUT_SECONDS"/,
  'only system.doctor selects the diagnostic cold-start timeout')
assert.match(source, /--max-time "\$timeout_seconds"/, 'curl consumes the selected bounded route timeout')

console.log('coverage audit timeout contract tests passed')
