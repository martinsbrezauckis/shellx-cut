// requirements-contract.mjs — reads schema/requirements.json (what each verb
// and feature needs) and checks it against the verb contract and the engine's
// up-front ffmpeg gate. The engine embeds the same file (app/server/src/
// requirements.rs), so the doctor answer, the refusals and the documents read
// one registry.

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

export const REQUIREMENTS_PATH = 'schema/requirements.json'
export const REQUIREMENTS_SCHEMA = 'shellx-cut/requirements/1'

const KINDS = new Set(['tool', 'condition'])
const STATE_KEYS = new Set([
  'doctor_card', 'doctor_cards_prefix', 'detail_true', 'tier_at_least', 'chat_wired',
  'engine', 'check_verb', 'check_args', 'per_asset', 'not_checked', 'reason', 'pending_owner',
])
const ENGINE_STATES = new Set(['open_project', 'export_folder', 'ui_client'])

export function loadRequirements(root) {
  return JSON.parse(readFileSync(resolve(root, REQUIREMENTS_PATH), 'utf8'))
}

export function loadVerbs(root) {
  return JSON.parse(readFileSync(resolve(root, 'schema/verbs.json'), 'utf8')).verbs
}

/** The verbs the engine refuses up front without ffmpeg (media_tool_gate.rs). */
export function mediaToolGateVerbs(root) {
  const source = readFileSync(resolve(root, 'app/server/src/media_tool_gate.rs'), 'utf8')
  const block = /pub\(crate\) const MEDIA_TOOL_VERBS: &\[&str\] = &\[([\s\S]*?)\];/.exec(source)
  if (!block) throw new Error('MEDIA_TOOL_VERBS not found in media_tool_gate.rs')
  return [...block[1].matchAll(/"([a-z_.]+)"/g)].map((match) => match[1])
}

/** Verb name -> its feature. */
export function featureOfVerb(registry) {
  const map = new Map()
  for (const feature of registry.features) {
    for (const verb of feature.verbs) map.set(verb, feature)
  }
  return map
}

/**
 * Everything one verb needs: its feature's `requires` plus the verb's own
 * `verb_extra`, plus an open project when the verb contract says so
 * (behavior.project_state "required"). `any_of` lists alternatives (one set
 * is enough); `optional` lists what improves the result, with the effect.
 */
export function verbRequirements(registry, verbs, name) {
  const feature = featureOfVerb(registry).get(name)
  if (!feature) return null
  const spec = verbs.find((verb) => verb.name === name)
  const required = [...feature.requires, ...(feature.verb_extra?.[name] ?? [])]
  if (spec?.behavior?.project_state === 'required') required.unshift('open_project')
  return {
    verb: name,
    feature: feature.id,
    feature_name: feature.name,
    requires: [...new Set(required)],
    any_of: feature.any_of ?? [],
    optional: (feature.optional ?? []).filter((item) => !item.verbs || item.verbs.includes(name)),
  }
}

/** Every problem with the registry; empty when it is complete and consistent. */
export function requirementProblems(registry, verbs, gateVerbs = []) {
  const problems = []
  if (registry.schema !== REQUIREMENTS_SCHEMA) problems.push(`schema must be ${REQUIREMENTS_SCHEMA}`)
  const ids = new Set()
  for (const item of registry.requirements) {
    if (!/^[a-z][a-z0-9_]*$/.test(item.id ?? '')) problems.push(`requirement id ${JSON.stringify(item.id)} is not a plain id`)
    if (ids.has(item.id)) problems.push(`requirement ${item.id} is declared twice`)
    ids.add(item.id)
    if (!KINDS.has(item.kind)) problems.push(`requirement ${item.id}: kind must be tool or condition`)
    for (const key of ['name', 'what']) {
      if (typeof item[key] !== 'string' || !item[key].trim()) problems.push(`requirement ${item.id}: ${key} is missing`)
    }
    const state = item.state ?? {}
    if (Object.keys(state).length === 0) problems.push(`requirement ${item.id}: state says nowhere how it is read`)
    for (const key of Object.keys(state)) {
      if (!STATE_KEYS.has(key)) problems.push(`requirement ${item.id}: unknown state key ${key}`)
    }
    if (state.engine && !ENGINE_STATES.has(state.engine)) problems.push(`requirement ${item.id}: unknown engine state ${state.engine}`)
    if (!item.get || (!item.get.verb && !item.get.person)) problems.push(`requirement ${item.id}: get must name a verb or what a person does`)
  }
  const verbNames = new Set(verbs.map((verb) => verb.name))
  const seen = new Map()
  const featureIds = new Set()
  for (const feature of registry.features) {
    if (featureIds.has(feature.id)) problems.push(`feature ${feature.id} is declared twice`)
    featureIds.add(feature.id)
    if (!Array.isArray(feature.requires)) problems.push(`feature ${feature.id}: requires must be a list ([] means it needs nothing)`)
    const referenced = [
      ...(feature.requires ?? []),
      ...(feature.any_of ?? []).flat(),
      ...(feature.optional ?? []).map((item) => item.id),
      ...Object.values(feature.verb_extra ?? {}).flat(),
    ]
    for (const id of referenced) {
      if (!ids.has(id)) problems.push(`feature ${feature.id} names unknown requirement ${id}`)
      if (id === 'open_project') problems.push(`feature ${feature.id}: open_project follows behavior.project_state, do not list it`)
    }
    for (const item of feature.optional ?? []) {
      if (typeof item.effect !== 'string' || !item.effect.trim()) problems.push(`feature ${feature.id}: optional ${item.id} has no effect`)
    }
    for (const verb of Object.keys(feature.verb_extra ?? {})) {
      if (!feature.verbs.includes(verb)) problems.push(`feature ${feature.id}: verb_extra names ${verb}, which is not in this feature`)
    }
    for (const verb of feature.verbs) {
      if (!verbNames.has(verb)) problems.push(`feature ${feature.id} names unknown verb ${verb}`)
      if (seen.has(verb)) problems.push(`verb ${verb} is in both ${seen.get(verb)} and ${feature.id}`)
      seen.set(verb, feature.id)
    }
  }
  for (const verb of verbNames) {
    if (!seen.has(verb)) problems.push(`verb ${verb} has no requirements declaration (add it to a feature in ${REQUIREMENTS_PATH}; a feature with "requires": [] is the declaration for "needs nothing")`)
  }
  // The engine's up-front ffmpeg gate and the registry must agree: a verb the
  // gate refuses without ffmpeg must declare ffmpeg (required, by argument or
  // optional), so the documents never say it needs nothing.
  for (const verb of gateVerbs) {
    const needs = verbRequirements(registry, verbs, verb)
    if (!needs) continue
    const all = [...needs.requires, ...needs.optional.map((item) => item.id)]
    if (!all.includes('ffmpeg')) {
      problems.push(`verb ${verb} is refused without ffmpeg by media_tool_gate but its requirements do not name ffmpeg`)
    }
  }
  return problems
}
