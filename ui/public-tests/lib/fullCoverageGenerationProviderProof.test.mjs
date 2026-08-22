import assert from 'node:assert/strict'
import test from 'node:test'

import {
  classifyGenerationProviderExecution,
  verifyProductionGenerationOutcome,
} from './fullCoverageGenerationProviderProof.mjs'

function generated(provider = 'codex') {
  const id = `${provider}-asset`
  const generationId = `${provider}-generation`
  const final = {
    asset_id: id,
    generated: {
      provider, kind: 'image', schema: 'shellx-cut/generated-asset/2', generation_id: generationId,
      family_id: `${provider}-family`, provenance_path: `/tmp/${provider}.json`, reused: false,
    },
  }
  const history = [{
    asset_id: id, provider, kind: 'image', generation_id: generationId, family_id: `${provider}-family`,
    provenance_schema: 'shellx-cut/generated-asset/2', integrity: 'verified', content_hash: `sha256:${'a'.repeat(64)}`,
  }]
  return { final, history }
}

test('fixture-only generation false pass is ineligible for final success', () => {
  const execution = classifyGenerationProviderExecution({
    provider: 'codex',
    card: { status: 'ok', details: { chat: { resolved: '/tmp/fcv-fixtures/codex' } } },
    env: { FCV_AGENT_FIXTURE_PROVIDERS: 'claude,codex', FCV_AGENT_FIXTURE_ROOT: '/tmp/fcv-fixtures' },
  })
  const { final, history } = generated()
  assert.equal(execution.available, true)
  assert.equal(execution.fixture, true)
  assert.equal(execution.productionReady, false)
  assert.match(execution.reason, /deterministic fixture/)
  assert.equal(verifyProductionGenerationOutcome({ provider: 'codex', execution, response: final, history }).ok, false)
})

test('fixture-root executable is rejected even when its provider was not declared', () => {
  const execution = classifyGenerationProviderExecution({
    provider: 'grok', card: { status: 'ok', details: { chat: { resolved: '/repo/scripts/release/fixtures/grok' } } }, env: {},
  })
  assert.equal(execution.fixture, true)
  assert.equal(execution.productionReady, false)
})

test('every final generation provider rejects its declared deterministic fixture', () => {
  for (const provider of ['codex', 'grok', 'antigravity']) {
    const execution = classifyGenerationProviderExecution({
      provider, card: { status: 'ok', details: { chat: { resolved: `/tmp/fixture-bin/${provider}` } } },
      env: { FCV_AGENT_FIXTURE_PROVIDERS: 'claude,codex,grok,antigravity', FCV_AGENT_FIXTURE_ROOT: '/tmp/fixture-bin' },
    })
    assert.equal(execution.productionReady, false, `${provider} fixture must not satisfy FCV_REQUIRE_FULL`)
  }
})

test('resolved production executable requires a fresh verified provider outcome', () => {
  const execution = classifyGenerationProviderExecution({
    provider: 'antigravity', card: { status: 'ok', details: { chat: { resolved: '/usr/local/bin/agy' } } }, env: {},
  })
  const { final, history } = generated('antigravity')
  assert.equal(execution.productionReady, true)
  assert.equal(verifyProductionGenerationOutcome({ provider: 'antigravity', execution, response: final, history }).ok, true)
  final.generated.reused = true
  assert.equal(verifyProductionGenerationOutcome({ provider: 'antigravity', execution, response: final, history }).ok, false)
})
