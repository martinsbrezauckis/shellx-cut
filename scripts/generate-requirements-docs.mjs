#!/usr/bin/env node
// generate-requirements-docs.mjs — writes the "what needs what" section of
// skill/shellx-cut/SKILL.md and the requirements table of
// skill/shellx-cut/reference.md from schema/requirements.json, between
// generated markers, so the agent documents cannot drift from the registry
// the engine reads. `--check` fails when either file is out of date.

import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { loadRequirements, loadVerbs, verbRequirements } from './lib/requirements-contract.mjs'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
export const SKILL_PATH = 'skill/shellx-cut/SKILL.md'
export const REFERENCE_PATH = 'skill/shellx-cut/reference.md'
const START = '<!-- generated:requirements:start (node scripts/generate-requirements-docs.mjs) -->'
const END = '<!-- generated:requirements:end -->'

const cell = (text) => String(text).replace(/\|/g, '\\|').replace(/\s+/g, ' ').trim()

function names(registry, ids) {
  return ids.map((id) => registry.requirements.find((item) => item.id === id)?.name ?? id)
}

function wayForward(get) {
  const parts = []
  if (get.verb) parts.push(`\`${get.verb}${get.args ? ` ${JSON.stringify(get.args)}` : ''}\``)
  if (get.button) parts.push(`${get.button}${get.settings ? ` (\`ui.open {panel:"${get.settings}"}\`)` : ''}`)
  if (get.person) parts.push(get.person)
  return parts.join('; ')
}

/** The short SKILL.md section: each tool or condition, what uses it, how to get it. */
export function renderSkillSection(registry, verbs) {
  const lines = [
    START,
    '## What needs what',
    '',
    'Every verb needs only what the requirements registry lists for it (an open project follows the verb\'s',
    '`project_state`). Before a verb, `system.doctor {feature:"<verb>"}` says whether it can run and what blocks it;',
    'a refusal for something missing carries `error.requirement {id, name, get}`. A person\'s licence, sign-in or',
    'system permission is theirs: tell them what `get.person` says and never do it for them.',
    '',
    '| Tool or condition | Needed for | Get it |',
    '|---|---|---|',
  ]
  for (const item of registry.requirements) {
    const usedBy = registry.features
      .filter((feature) => feature.requires.includes(item.id)
        || (feature.any_of ?? []).flat().includes(item.id)
        || Object.values(feature.verb_extra ?? {}).flat().includes(item.id))
      .map((feature) => feature.name)
    if (item.id === 'open_project') {
      // Follows each verb's project_state, not the registry's feature lists.
      for (const feature of registry.features) {
        if (feature.verbs.some((verb) => verbRequirements(registry, verbs, verb).requires.includes('open_project'))) usedBy.push(feature.name)
      }
    }
    const optionalFor = registry.features
      .filter((feature) => (feature.optional ?? []).some((opt) => opt.id === item.id))
      .map((feature) => feature.name)
      .filter((name) => !usedBy.includes(name))
    const needed = [
      ...usedBy,
      ...optionalFor.map((name) => `${name} (optional)`),
    ]
    const id = item.platforms ? `\`${item.id}\` (${item.platforms.join(', ')})` : `\`${item.id}\``
    const license = item.license ? `; licence: ${item.license.name}${item.license.url ? ` (${item.license.url})` : ''}` : ''
    lines.push(`| ${cell(`${item.name} ${id}`)} | ${cell(needed.join('; ') || 'nothing yet')} | ${cell(wayForward(item.get) + license)} |`)
  }
  lines.push(END)
  return lines.join('\n')
}

/** The full reference.md table: every feature, its verbs and what each needs. */
export function renderReferenceSection(registry, verbs) {
  const lines = [
    START,
    '## Requirements (what each verb needs)',
    '',
    'The requirements registry, as `system.doctor` returns it (`requirements`, or',
    '`{feature:"<verb>"}` for one verb). "Open project" follows each verb\'s `project_state`. "Optional" items',
    'change the result as described; "one of" means any one alternative is enough.',
    '',
    '| Feature | Verbs | Needs | One of | Optional (effect) |',
    '|---|---|---|---|---|',
  ]
  for (const feature of registry.features) {
    const open = feature.verbs.map((verb) => verbRequirements(registry, verbs, verb).requires.includes('open_project'))
    const openNote = open.every(Boolean) ? ['An open project'] : open.some(Boolean) ? ['An open project (some verbs)'] : []
    const needs = [...openNote, ...names(registry, feature.requires)]
    const extra = Object.entries(feature.verb_extra ?? {})
      .filter(([, ids]) => ids.length > 0)
      .map(([verb, ids]) => `\`${verb}\` also: ${names(registry, ids).join(', ')}`)
    const anyOf = (feature.any_of ?? []).map((set) => names(registry, set).join(' + ')).join(' or ')
    const optional = (feature.optional ?? [])
      .map((item) => `${names(registry, [item.id])[0]}${item.verbs ? ` (${item.verbs.map((verb) => `\`${verb}\``).join(', ')})` : ''}: ${item.effect}`)
      .join('; ')
    lines.push(`| ${cell(feature.name)} | ${cell(feature.verbs.map((verb) => `\`${verb}\``).join(', '))} | ${cell([...needs, ...extra].join('; ') || 'nothing')} | ${cell(anyOf || '-')} | ${cell(optional || '-')} |`)
  }
  lines.push(END)
  return lines.join('\n')
}

/** `text` with the generated block replaced (or appended when absent). */
export function withSection(text, section) {
  const start = text.indexOf(START)
  const end = text.indexOf(END)
  if (start >= 0 && end > start) return `${text.slice(0, start)}${section}${text.slice(end + END.length)}`
  return `${text.replace(/\s*$/, '')}\n\n${section}\n`
}

export function expectedDocuments(root = ROOT) {
  const registry = loadRequirements(root)
  const verbs = loadVerbs(root)
  const skill = readFileSync(resolve(root, SKILL_PATH), 'utf8')
  const reference = readFileSync(resolve(root, REFERENCE_PATH), 'utf8')
  return [
    [SKILL_PATH, skill, withSection(skill, renderSkillSection(registry, verbs))],
    [REFERENCE_PATH, reference, withSection(reference, renderReferenceSection(registry, verbs))],
  ]
}

function main() {
  const check = process.argv.includes('--check')
  const unknown = process.argv.slice(2).filter((arg) => arg !== '--check')
  if (unknown.length) throw new Error(`unknown argument(s): ${unknown.join(', ')}`)
  const stale = []
  for (const [path, current, next] of expectedDocuments()) {
    if (current === next) continue
    if (check) stale.push(path)
    else writeFileSync(resolve(ROOT, path), next)
  }
  if (check && stale.length) {
    console.error(`requirements documents are stale: ${stale.join(', ')}; run node scripts/generate-requirements-docs.mjs`)
    process.exitCode = 1
    return
  }
  console.log(check ? 'PASS requirements documents are current' : 'wrote the requirements sections of SKILL.md and reference.md')
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) main()
