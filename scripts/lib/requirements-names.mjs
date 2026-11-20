// requirements-names.mjs — every name a registry text gives must exist: the
// verbs and arguments of each way forward (`get.verb`, `get.args`, and any
// `domain.verb {arg}` written in a text), the Settings panel it opens
// (`get.settings`, a `ui.open` panel), and the control a person is told to
// choose (`get.button`, "choose X", "the X button", "Settings > X"), which
// must be a visible label in the UI sources. A text that names a control,
// verb or argument that does not exist sends the person or the agent nowhere.

import { readdirSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'

function filesUnder(dir) {
  const out = []
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = resolve(dir, entry.name)
    if (entry.isDirectory()) out.push(...filesUnder(path))
    else if (/\.(tsx|ts)$/.test(entry.name)) out.push(path)
  }
  return out
}

/** All UI source text, for label lookups. */
export function loadUiSources(root) {
  return filesUnder(resolve(root, 'ui/src')).map((file) => readFileSync(file, 'utf8')).join('\n')
}

const escape = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

/** A visible label: a whole string literal or JSX text, an optional trailing ellipsis. */
export function uiHasLabel(ui, label) {
  const body = `${escape(label)}(?:…|\\.\\.\\.)?`
  return new RegExp(`(['"\`>])${body}(['"\`<])`).test(ui)
}

/** Every string in the registry with where it is. */
function texts(registry) {
  const out = []
  const walk = (value, where) => {
    if (typeof value === 'string') out.push([where, value])
    else if (Array.isArray(value)) value.forEach((item, index) => walk(item, `${where}[${index}]`))
    else if (value && typeof value === 'object') {
      for (const [key, item] of Object.entries(value)) {
        // ids, verb lists and machine keys are checked elsewhere
        if (['id', 'verbs', 'requires', 'any_of', 'verb_extra', 'kind', 'platforms', 'url', 'schema'].includes(key)) continue
        walk(item, `${where}.${key}`)
      }
    }
  }
  registry.requirements.forEach((item) => walk(item, item.id))
  registry.features.forEach((feature) => walk(feature, `feature ${feature.id}`))
  return out
}

/** Control labels a text tells a person to use. */
export function namedControls(text) {
  const found = []
  for (const match of text.matchAll(/\bthe ([A-Z][\w&' -]*?) button\b/g)) found.push(match[1])
  for (const match of text.matchAll(/\b(?:[Cc]hoose|[Cc]lick|[Pp]ress) ([A-Z][^,.;:()]*?)(?=\s(?:in|on|to|and|or|again|then|from|first)\b|[,.;:()]|$)/g)) found.push(match[1].trim())
  // ShellX Cut's own Settings pages ("Settings > AI & transcription"); the
  // operating system's ("System Settings > ...", "Windows: Settings > ...") are not app labels.
  for (const match of text.matchAll(/(?<!System |Windows: )\bSettings > ([A-Z][A-Za-z]*(?: & [a-z]+)?)/g)) found.push(match[1])
  return found
}

export function registryNameProblems(registry, verbs, ui) {
  const problems = []
  const byName = new Map(verbs.map((verb) => [verb.name, verb]))
  const domains = new Set(verbs.map((verb) => verb.domain))
  const panels = new Set(byName.get('ui.open')?.args?.properties?.panel?.enum ?? [])
  const checkArgs = (where, verbName, keys) => {
    const properties = byName.get(verbName)?.args?.properties ?? {}
    for (const key of keys) {
      if (!(key in properties)) problems.push(`${where}: ${verbName} has no argument '${key}'`)
    }
  }
  for (const item of registry.requirements) {
    const get = item.get ?? {}
    if (get.verb) {
      const spec = byName.get(get.verb)
      if (!spec) problems.push(`${item.id}.get.verb: no verb ${get.verb}`)
      for (const [key, value] of Object.entries(get.args ?? {})) {
        const property = spec?.args?.properties?.[key]
        if (!property) problems.push(`${item.id}.get.args: ${get.verb} has no argument '${key}'`)
        else if (Array.isArray(property.enum) && !property.enum.includes(value)) problems.push(`${item.id}.get.args.${key}: ${JSON.stringify(value)} is not one of ${property.enum.join(', ')}`)
      }
    } else if (get.args) {
      problems.push(`${item.id}.get.args without get.verb`)
    }
    if (get.settings && !panels.has(get.settings)) problems.push(`${item.id}.get.settings: '${get.settings}' is not a ui.open panel`)
    if (get.button && !uiHasLabel(ui, get.button)) problems.push(`${item.id}.get.button: no control labelled '${get.button}' in ui/src`)
  }
  for (const [where, text] of texts(registry)) {
    for (const match of text.matchAll(/\b([a-z_]+)\.([a-z_]+(?:\.[a-z_]+)*)\b(\s*\{([^}]*)\})?/g)) {
      const [, domain, rest, , argText] = match
      if (!domains.has(domain)) continue
      const name = `${domain}.${rest}`
      if (!byName.has(name)) {
        problems.push(`${where}: names verb ${name}, which does not exist`)
        continue
      }
      if (argText !== undefined) {
        const keys = argText.split(',').map((part) => part.trim().split(/[:\s]/)[0].replace(/["'?]/g, '')).filter(Boolean)
        checkArgs(where, name, keys)
      }
    }
    for (const label of namedControls(text)) {
      if (!uiHasLabel(ui, label)) problems.push(`${where}: names a control '${label}' that is not a label in ui/src`)
    }
  }
  return problems
}
