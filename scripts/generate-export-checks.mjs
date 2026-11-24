#!/usr/bin/env node
// Shared wire-only types/schema and deterministic examples. Does not run checks.
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const check = process.argv.includes('--check')
const catalog = JSON.parse(readFileSync(resolve(root, 'schema/export-checks.catalog.json'), 'utf8'))
const copy = JSON.parse(readFileSync(resolve(root, 'schema/export-checks.catalog.copy.json'), 'utf8'))
function output(path, text) {
  const full = resolve(root, path)
  if (check) {
    if (readFileSync(full, 'utf8') !== text) throw new Error(`${path} is stale`)
  } else { mkdirSync(dirname(full), { recursive: true }); writeFileSync(full, text) }
}
const str = { type: 'string' }, num = { type: 'integer', minimum: 0 }
const ref = name => ({ $ref: `#/$defs/${name}` })
const arr = items => ({ type: 'array', items })
const enumeration = values => ({ type: 'string', enum: values.split('|') })
const obj = (properties, required = Object.keys(properties)) => ({ type: 'object', properties, required, additionalProperties: false })
const states = ['checked', 'found', 'not_checked', 'error', 'not_applicable']
const reasons = ['clear','finding','optional_tool_missing','not_run_yet','unknown_older_analysis','after_export','on_request','check_failed','not_applicable']
const defs = {
  ExportCheckAction: obj({ kind: enumeration('seek|open_tools|copy_details|command'), label: str, at_ms: num, tool_id: str, text: str, verb: str, args: { type: 'object' } }, ['kind','label']),
  ExportCheckFinding: obj({ reason: str, asset_id: str, clip_id: str, range_ms: arr(num), evidence: {} }, ['reason','evidence']),
  ExportCheckProvider: obj({ id: str, installation_id: str, configured: { type: 'boolean' }, status: enumeration('ready|missing|damaged|unknown'), version: { type: ['string','null'] }, version_reason: str, attempted: { type: 'boolean' }, started_at: str, finished_at: str, error: {} }, ['id','configured','status','version','attempted']),
  ExportCheckCoverage: obj({ applicable_assets: arr(str), measured_assets: arr(str), unchecked_assets: arr(str), errors: arr(obj({ asset_id: str, provider_id: str, reason: str, error: {} }, ['provider_id','reason','error'])) }),
  ExportCheckReportDetails: obj({ check_id: str, providers: arr(ref('ExportCheckProvider')), error: {}, app_version: str, os: str, arch: str, at_op: str, render_id: str }, ['check_id','providers','error','app_version','os','arch','at_op']),
  ExportCheckEntry: obj({ id: str, name: str, when: enumeration('before_export|finished_file|both'), mode: enumeration('automatic|on_request'), state: enumeration(states.join('|')), reason_code: enumeration(reasons.join('|')), result: str, reason: str, suggested_action: str, action: ref('ExportCheckAction'), findings: arr(ref('ExportCheckFinding')), coverage: ref('ExportCheckCoverage'), providers: arr(ref('ExportCheckProvider')), report_details: ref('ExportCheckReportDetails') }, ['id','name','when','mode','state','reason_code','result','reason','suggested_action','findings','coverage','providers']),
  ExportChecks: obj({ schema_version: { const: 1, type: 'integer' }, catalog_version: { const: 1, type: 'integer' }, phase: enumeration('before_export|finished_file'), at_op: str, render_id: str, entries: arr(ref('ExportCheckEntry')) }, ['schema_version','catalog_version','phase','at_op','entries']),
}
const schema = { $schema: 'https://json-schema.org/draft/2020-12/schema', $id: 'shellx-cut://export-checks.schema.json', $ref: '#/$defs/ExportChecks', $defs: defs }
output('schema/export-checks.schema.json', `${JSON.stringify(schema, null, 2)}\n`)
// Generate both languages from exactly the same schema fields.
const enums = { when: 'ExportCheckWhen', mode: 'ExportCheckMode', state: 'ExportCheckState', reason_code: 'ExportCheckReason', status: 'ExportCheckProviderStatus', phase: 'ExportCheckPhase', kind: 'ExportCheckActionKind' }
const pascal = s => s.split('_').map(x => x[0].toUpperCase() + x.slice(1)).join('')
const enumValues = new Map()
function typeOf(rule, lang, field) {
  if (rule.enum) { enumValues.set(enums[field], rule.enum); return enums[field] }
  if (rule.$ref) return rule.$ref.split('/').at(-1)
  if (rule.type === 'array') return lang === 'ts' ? `Array<${typeOf(rule.items, lang)}>` : `Vec<${typeOf(rule.items, lang)}>`
  if (rule.type === 'object' && rule.properties) return lang === 'ts' ? `{ ${Object.entries(rule.properties).map(([k, v]) => `${k}${rule.required.includes(k) ? '' : '?'}: ${typeOf(v, lang, k)}`).join('; ')} }` : 'serde_json::Value'
  if (rule.type === 'object') return lang === 'ts' ? 'Record<string, unknown>' : 'serde_json::Value'
  if (Array.isArray(rule.type)) return lang === 'ts' ? 'string | null' : 'Option<String>'
  if (rule.type === 'string') return lang === 'ts' ? 'string' : 'String'
  if (rule.type === 'integer') return lang === 'ts' ? (rule.const ? '1' : 'number') : 'u64'
  if (rule.type === 'boolean') return lang === 'ts' ? 'boolean' : 'bool'
  return lang === 'ts' ? 'unknown' : 'serde_json::Value'
}
let ts = '// Generated by scripts/generate-export-checks.mjs; do not edit.\n'
let rust = '// Generated by scripts/generate-export-checks.mjs; do not edit.\nuse serde::{Deserialize, Serialize};\n'
for (const [name, def] of Object.entries(defs)) {
  ts += `export interface ${name} {\n`
  rust += `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\npub struct ${name} {\n`
  for (const [key, rule] of Object.entries(def.properties)) {
    const optional = !def.required.includes(key)
    ts += `  ${key}${optional ? '?' : ''}: ${typeOf(rule, 'ts', key)}\n`
    const type = typeOf(rule, 'rs', key)
    if (optional) rust += '    #[serde(default, skip_serializing_if = "Option::is_none")]\n'
    rust += `    pub ${key}: ${optional ? `Option<${type}>` : type},\n`
  }
  ts += '}\n'
  rust += '}\n'
}
for (const [name, values] of enumValues) {
  ts += `export type ${name} = ${values.map(v => `'${v}'`).join(' | ')}\n`
  rust += `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]\n#[serde(rename_all = "snake_case")]\npub enum ${name} {\n${values.map(v => `    ${pascal(v)},`).join('\n')}\n}\n`
}
// Typed access to the same catalog bytes; no separate engine/UI registry.
ts += `export interface ExportCheckDefinition { id: string; name: string; when: ExportCheckWhen; mode: ExportCheckMode; providers: string[]; command?: { verb: string; args: Record<string, unknown> } }\n`
rust += '#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\npub struct ExportCheckDefinition {\n    pub id: String,\n    pub name: String,\n    pub when: ExportCheckWhen,\n    pub mode: ExportCheckMode,\n    pub providers: Vec<String>,\n    pub command: Option<serde_json::Value>,\n}\npub fn export_check_catalog() -> Vec<ExportCheckDefinition> {\n    serde_json::from_str(include_str!("../../../schema/export-checks.catalog.json"))\n        .expect("built-in export check catalog")\n}\n'
output('ui/src/lib/exportChecks.ts', ts)
output('app/core/src/export_checks.rs', rust)
const error = { code: 'sidecar', message: 'Captions tools could not start', cause: 'Configured interpreter exited with code 1', suggested_action: 'Repair captions tools in Settings > AI & transcription.' }
const provider = { id: 'perception', configured: true, status: 'damaged', version: null, version_reason: 'Version probe failed', attempted: true, error }
const results = { checked: 'fine', found: '0:12 to 0:15', not_checked: 'not checked, tools not installed', error: 'could not run', not_applicable: 'does not apply' }
function entry(def, state) {
  const reason = { checked: 'The applicable check completed without findings.', found: 'A problem was found from 0:12 to 0:15.', not_checked: 'The optional tool is not installed.', error: error.cause, not_applicable: 'This export has nothing for this check.' }[state]
  const action = state === 'found' ? { kind: 'seek', label: 'Go to place', at_ms: 12000 } : state === 'not_checked' ? { kind: 'open_tools', label: 'Install captions', tool_id: 'perception' } : state === 'error' ? { kind: 'copy_details', label: 'Copy details', text: `${def.id}\nTool: perception; version unavailable (probe failed)\n${error.cause}\nCut: 0.6.121\nSystem: linux x86_64` } : undefined
  return { id: def.id, name: def.name, when: def.when, mode: def.mode, state, reason_code: { checked: 'clear', found: 'finding', not_checked: 'optional_tool_missing', error: 'check_failed', not_applicable: 'not_applicable' }[state], result: results[state], reason, suggested_action: action?.label ?? 'No action needed.', ...(action ? { action } : {}), findings: state === 'found' ? [{ reason, range_ms: [12000,15000], evidence: { count: 1 } }] : [], coverage: { applicable_assets: ['a1'], measured_assets: ['checked','found'].includes(state) ? ['a1'] : [], unchecked_assets: ['error','not_checked'].includes(state) ? ['a1'] : [], errors: [] }, providers: state === 'error' ? [provider] : [], ...(state === 'error' ? { report_details: { check_id: def.id, providers: [provider], error, app_version: '0.6.121', os: 'linux', arch: 'x86_64', at_op: 'op_000001' } } : {}) }
}
const envelope = (phase, entries) => {
  for (const entry of entries) {
    if (entry.name.trim().split(/\s+/u).length > copy.max_name_words) throw new Error(`${entry.id}: name exceeds ${copy.max_name_words} words`)
    if (entry.result.trim().split(/\s+/u).length > copy.max_result_words) throw new Error(`${entry.id}: result exceeds ${copy.max_result_words} words`)
  }
  return { schema_version: 1, catalog_version: 1, phase, at_op: 'op_000001', ...(phase === 'finished_file' ? { render_id: 'render_001' } : {}), entries }
}
// Cartesian examples test wire rendering only, NOT all reachable engine states.
output('schema/fixtures/export-checks/states.json', '[\n' + catalog.flatMap(c => ['before_export','finished_file'].flatMap(p => states.map(s => JSON.stringify(envelope(p, [entry(c,s)]))))).join(',\n') + '\n]\n')
const before = catalog.map((c, i) => entry(c, i === 1 ? 'found' : i === 3 ? 'error' : c.mode === 'on_request' || c.when === 'finished_file' ? 'not_checked' : 'checked'))
for (const e of before) {
  if (e.state === 'not_checked') { e.reason_code = e.mode === 'on_request' ? 'on_request' : 'after_export'; e.result = e.mode === 'on_request' ? 'on request' : 'checked after export'; e.reason = e.mode === 'on_request' ? 'Run this check only when requested.' : 'This check needs the finished file.'; e.suggested_action = e.mode === 'on_request' ? 'Choose this check when needed.' : 'Export to check the finished file.'; delete e.action }
}
for (const [id, code] of [['black_or_frozen','optional_tool_missing'],['cut_on_word','not_run_yet'],['cut_on_beat','unknown_older_analysis']]) {
  const e = before.find(e => e.id === id)
  Object.assign(e, entry(catalog.find(c => c.id === id), 'not_checked'), { reason_code: code, result: code === 'optional_tool_missing' ? results.not_checked : 'not checked yet' })
  if (code !== 'optional_tool_missing') { e.reason = code === 'not_run_yet' ? 'Analysis has not been requested.' : 'Older analysis did not record what ran.'; e.action = { kind: 'command', label: 'Check now', verb: 'media.perception', args: { asset: 'a1' } }; e.suggested_action = 'Run analysis for the unchecked files.' }
}
// Keep one found result with an independent coverage gap.
const mixed = before.find(e => e.id === 'uniform_border'); Object.assign(mixed, entry(catalog.find(c => c.id === mixed.id), 'found')); mixed.coverage.unchecked_assets = ['a2']; mixed.coverage.applicable_assets.push('a2')
for (const e of before.filter(e => e.mode === 'on_request')) {
  const command = catalog.find(c => c.id === e.id).command
  if (command) e.action = { kind: 'command', label: 'Check now', verb: command.verb, args: command.verb === 'verify.loudness' ? { asset: 'a1' } : command.verb === 'render.qc' ? { reframe_id: 'reframe_001' } : command.verb === 'verify.judge' ? { render_id: 'render_001' } : command.args }
}
const legacyPregate = { pass: true, risks: [], perception_assets: 0, uninstrumented_assets: ['a1'] }
const legacyReceipt = { render_id: 'render_001', ts: '2026-11-22T23:32:00Z', output_path: '/exports/render_001.mp4', output_hash: 'sha256:fixture', duration_ms: 20000, preset: 'h264_1080p30', at_op: 'op_000001', checks: [], pass: true }
for (const [name, data] of Object.entries({ 'before-export': { ...legacyPregate, export_checks: envelope('before_export', before) }, 'finished-file': { ...legacyReceipt, export_checks: envelope('finished_file', catalog.map((c,i) => c.mode === 'on_request' ? before.find(e => e.id === c.id) : entry(c, i % 5 === 0 ? 'found' : i % 5 === 1 ? 'error' : 'checked'))) }, 'legacy-pregate': legacyPregate, 'legacy-receipt': legacyReceipt })) output(`schema/fixtures/export-checks/${name}.json`, `${JSON.stringify(data, null, 2)}\n`)
console.log(`export checks wire ${check ? 'current' : 'generated'}: ${catalog.length} kinds, ${catalog.length * 10} state/phase examples`)
