import assert from 'node:assert/strict'
import test from 'node:test'

import { resolveImmutableSourceIdentityRoot } from '../lib/immutable-source-sync.mjs'

test('a separate immutable identity root is test-control-only and absolute', () => {
  assert.equal(resolveImmutableSourceIdentityRoot({ repoRoot: '/source' }), '/source')
  assert.equal(resolveImmutableSourceIdentityRoot({
    repoRoot: '/harness',
    configured: '/frozen-product',
    testControl: true,
  }), '/frozen-product')
  assert.throws(
    () => resolveImmutableSourceIdentityRoot({ repoRoot: '/harness', configured: '/frozen-product' }),
    /requires test-control/,
  )
  assert.throws(
    () => resolveImmutableSourceIdentityRoot({ repoRoot: '/harness', configured: 'relative', testControl: true }),
    /absolute path/,
  )
})
