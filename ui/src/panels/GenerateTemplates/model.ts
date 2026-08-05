import type {
  GenerateKind,
  GenerateParam,
  GenerateSource,
  GenerateTemplateManifest,
  GenerateTemplateSummary,
} from '../../lib/client'

export type KindFilter = GenerateKind | 'all'
export type ParamValues = Record<string, unknown>
export type GenerateWorkspaceTab = 'templates' | 'prompt' | 'storyboard' | 'media'
export type PromptPolicy = 'plan' | 'preview' | 'insert'
export type PromptAgent = 'auto' | 'claude' | 'codex' | 'grok'
export type StoryboardMode = 'quick_prompt' | 'director_brief' | 'script' | 'existing_media'

export const KIND_FILTERS: KindFilter[] = ['all', 'title', 'caption', 'shape', 'motion', 'social', 'batch']
const GENERATE_KINDS: GenerateKind[] = ['title', 'caption', 'shape', 'motion', 'social', 'batch']
const GENERATE_SOURCES: GenerateSource[] = ['builtin', 'project', 'user']
export const PROMPT_AGENTS: PromptAgent[] = ['auto', 'claude', 'codex', 'grok']
export const PROMPT_POLICIES: PromptPolicy[] = ['plan', 'preview', 'insert']
export const STORYBOARD_MODES: StoryboardMode[] = ['quick_prompt', 'director_brief', 'script', 'existing_media']

export function optionValue<T extends string>(options: readonly T[], value: string, fallback: T): T {
  for (const option of options) {
    if (option === value) return option
  }
  return fallback
}

export function isBlank(value: unknown) {
  return value == null || (typeof value === 'string' && value.trim() === '')
}

export function seedParams(manifest: GenerateTemplateManifest): ParamValues {
  const out: ParamValues = {}
  for (const [name, param] of Object.entries(manifest.params)) {
    if (param.default !== undefined && param.default !== null) {
      out[name] = param.default
    } else if (param.type === 'boolean') {
      out[name] = false
    } else if (param.type === 'color') {
      out[name] = '#FFD24A'
    } else {
      out[name] = ''
    }
  }
  return out
}

function coerceParam(param: GenerateParam, value: unknown) {
  if (param.type === 'integer') {
    const n = Number(value)
    return Number.isFinite(n) ? Math.round(n) : value
  }
  if (param.type === 'number') {
    const n = Number(value)
    return Number.isFinite(n) ? n : value
  }
  if (param.type === 'boolean') return Boolean(value)
  if (typeof value === 'string') return value.trim()
  return value
}

export function serializeParams(manifest: GenerateTemplateManifest, values: ParamValues) {
  const out: Record<string, unknown> = {}
  for (const [name, param] of Object.entries(manifest.params)) {
    const value = values[name]
    if (!param.required && isBlank(value)) continue
    out[name] = coerceParam(param, value)
  }
  return out
}

export function missingRequired(manifest: GenerateTemplateManifest | null, values: ParamValues) {
  if (!manifest) return []
  return Object.entries(manifest.params)
    .filter(([, param]) => param.required)
    .filter(([name]) => isBlank(values[name]))
    .map(([name]) => name)
}

export function fieldLabel(name: string) {
  return name.replace(/_/g, ' ')
}

export function colorValue(value: unknown) {
  const s = typeof value === 'string' ? value : ''
  return /^#[0-9a-f]{6}$/i.test(s) ? s : '#FFD24A'
}

function recordOf(value: unknown): Record<string, unknown> | null {
  return value != null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
}

function stringArray(value: unknown): string[] | null {
  return Array.isArray(value) && value.every((item) => typeof item === 'string') ? value : null
}

function generateKind(value: unknown): GenerateKind | null {
  return typeof value === 'string' && GENERATE_KINDS.includes(value as GenerateKind) ? value as GenerateKind : null
}

function generateSource(value: unknown): GenerateSource | null {
  return typeof value === 'string' && GENERATE_SOURCES.includes(value as GenerateSource) ? value as GenerateSource : null
}

function generateParamRecord(value: unknown): Record<string, GenerateParam> | null {
  const obj = recordOf(value)
  if (!obj) return null
  const out: Record<string, GenerateParam> = {}
  for (const [name, raw] of Object.entries(obj)) {
    const param = recordOf(raw)
    if (!param || typeof param.type !== 'string' || typeof param.required !== 'boolean') return null
    out[name] = {
      type: param.type,
      required: param.required,
      ...(param.default !== undefined ? { default: param.default } : {}),
      ...(typeof param.description === 'string' || param.description === null ? { description: param.description } : {}),
      ...(Array.isArray(param.enum) || param.enum === null ? { enum: param.enum } : {}),
      ...(typeof param.minimum === 'number' || param.minimum === null ? { minimum: param.minimum } : {}),
      ...(typeof param.maximum === 'number' || param.maximum === null ? { maximum: param.maximum } : {}),
      ...(typeof param.step === 'number' || param.step === null ? { step: param.step } : {}),
    }
  }
  return out
}

function templateSummaryFrom(value: unknown): GenerateTemplateSummary | null {
  const obj = recordOf(value)
  if (!obj) return null
  const source = generateSource(obj.source)
  const kind = generateKind(obj.kind)
  const tags = stringArray(obj.tags)
  const capabilities = stringArray(obj.capabilities)
  const params = generateParamRecord(obj.params)
  if (
    typeof obj.id !== 'string' ||
    !source ||
    !kind ||
    typeof obj.title !== 'string' ||
    typeof obj.summary !== 'string' ||
    !tags ||
    !capabilities ||
    !params
  ) return null
  return { id: obj.id, source, kind, title: obj.title, summary: obj.summary, tags, params, capabilities }
}

export function templateListResultFrom(value: unknown): { templates: GenerateTemplateSummary[] } | null {
  const obj = recordOf(value)
  if (!obj || !Array.isArray(obj.templates)) return null
  const templates = obj.templates.map(templateSummaryFrom)
  return templates.every(Boolean) ? { templates: templates as GenerateTemplateSummary[] } : null
}

export function templateManifestFrom(value: unknown): GenerateTemplateManifest | null {
  const obj = recordOf(value)
  const summary = templateSummaryFrom(value)
  const lowering = recordOf(obj?.lowering)
  const defaults = recordOf(obj?.defaults)
  const verification = recordOf(obj?.verification)
  const loweringArgs = recordOf(lowering?.args)
  if (!summary || !defaults || !verification || !lowering || typeof lowering.verb !== 'string' || !loweringArgs) {
    return null
  }
  return { ...summary, defaults, verification, lowering: { verb: lowering.verb, args: loweringArgs } }
}

export function formatDuration(ms: number | undefined) {
  if (!Number.isFinite(ms)) return '0.0s'
  return `${((ms ?? 0) / 1000).toFixed(1)}s`
}
