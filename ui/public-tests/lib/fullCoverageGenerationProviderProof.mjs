import { delimiter } from 'node:path'

export const GENERATION_PROVIDER_IDS = Object.freeze(['codex', 'grok', 'antigravity'])

function normalizePath(value) {
  return String(value || '').trim().replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase()
}

export function generationFixtureProviders(env = process.env) {
  const configured = env.FCV_AGENT_FIXTURE_PROVIDERS
    || (env.FCV_AGENT_FIXTURES === '1' ? 'claude,codex,grok,antigravity' : '')
  return new Set(String(configured).split(',').map((name) => name.trim().toLowerCase()).filter(Boolean))
}

export function generationFixtureRoots(env = process.env) {
  return [...new Set([
    env.FCV_AGENT_FIXTURE_ROOT,
    ...(String(env.FCV_AGENT_FIXTURE_ROOTS || '').split(delimiter)),
  ].map(normalizePath).filter(Boolean))]
}

export function isFixtureGenerationExecutable(resolved, roots = []) {
  const executable = normalizePath(resolved)
  if (!executable) return false
  return roots.some((root) => executable === root || executable.startsWith(`${root}/`))
    || /(?:^|\/)scripts\/release\/fixtures(?:\/|$)/.test(executable)
}

export function classifyGenerationProviderExecution({ provider, card, env = process.env, fixtureProviders } = {}) {
  const id = String(provider || '').trim().toLowerCase()
  const resolved = String(card?.details?.chat?.resolved || '').trim()
  const ready = String(card?.status || '').toLowerCase() === 'ok'
  const declaredFixture = (fixtureProviders || generationFixtureProviders(env)).has(id)
  const fixtureExecutable = isFixtureGenerationExecutable(resolved, generationFixtureRoots(env))
  const fixture = declaredFixture || fixtureExecutable
  let reason = 'production executable resolved'
  if (declaredFixture) reason = `deterministic fixture declared for ${id}`
  else if (fixtureExecutable) reason = `resolved executable is under a fixture root: ${resolved}`
  else if (!ready) reason = `system.doctor judge.${id} is not ok`
  else if (!resolved) reason = `system.doctor judge.${id} did not report its resolved executable`
  return {
    provider: id,
    resolved,
    available: Boolean(ready && resolved),
    fixture,
    productionReady: Boolean(ready && resolved && !fixture),
    reason,
  }
}

function historyItems(history) {
  if (Array.isArray(history)) return history
  return Array.isArray(history?.result?.items) ? history.result.items : []
}

export function verifyProductionGenerationOutcome({ provider, kind = 'image', execution, response, job, history } = {}) {
  if (!execution?.productionReady) {
    return { ok: false, detail: `production generation is ineligible: ${execution?.reason || 'resolved non-fixture executable is missing'}` }
  }
  const final = job?.result || response?.result || response || {}
  const generated = final.generated || {}
  const assetId = String(final.asset_id || '').trim()
  const schema = 'shellx-cut/generated-asset/2'
  if (!assetId || generated.provider !== provider || generated.kind !== kind || generated.schema !== schema
    || !String(generated.generation_id || '').trim() || !String(generated.family_id || '').trim()
    || !String(generated.provenance_path || '').trim() || generated.reused !== false) {
    return { ok: false, detail: `provider outcome lacks new ${provider}/${kind} generated-asset provenance` }
  }
  const historical = historyItems(history).find((entry) => entry?.asset_id === assetId)
  if (!historical || historical.provider !== provider || historical.kind !== kind
    || historical.generation_id !== generated.generation_id || historical.family_id !== generated.family_id
    || historical.provenance_schema !== schema || historical.integrity !== 'verified'
    || !/^sha256:[a-f0-9]{64}$/i.test(String(historical.content_hash || ''))) {
    return { ok: false, detail: `assets.generated_list does not verify ${provider} outcome/provenance for ${assetId}` }
  }
  return {
    ok: true,
    assetId,
    generationId: generated.generation_id,
    contentHash: historical.content_hash,
    detail: `${provider}/${kind} production outcome verified: executable=${execution.resolved}; generation=${generated.generation_id}; provenance=${generated.provenance_path}; content=${historical.content_hash}`,
  }
}
