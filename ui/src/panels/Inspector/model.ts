import type { Asset, Clip, ClipEffect, ClipGrade, ColorSpace, Project, VerbArgs } from '../../lib/client'
import type { EffectCatalogEntry } from '../../lib/catalogs'
import type { SourceDims } from '../../components/inspector/useClipProp'

/** One-click VIDEO effects (edit.effect) — the popular, param-free presets a human reaches for. */
export const VIDEO_EFFECTS: { eff: ClipEffect; label: string }[] = [
  { eff: { type: 'sepia' }, label: 'Sepia' },
  { eff: { type: 'invert' }, label: 'Invert' },
  { eff: { type: 'vignette', amount: 0.5 }, label: 'Vignette' },
  { eff: { type: 'sharpen', amount: 1 }, label: 'Sharpen' },
  { eff: { type: 'auto_color', amount: 1 }, label: 'Auto color' },
  { eff: { type: 'mirror' }, label: 'Mirror' },
  { eff: { type: 'flip' }, label: 'Flip' },
  { eff: { type: 'pixelize', size: 16 }, label: 'Pixelize' },
  { eff: { type: 'vhs', amount: 0.5 }, label: 'VHS' },
  { eff: { type: 'posterize', levels: 6 }, label: 'Posterize' },
]

/** One-click AUDIO cleanup effects (edit.effect, audio-track clips). */
export const AUDIO_EFFECTS: { eff: ClipEffect; label: string }[] = [
  { eff: { type: 'denoise', amount: 0.5 }, label: 'Denoise' },
  { eff: { type: 'compressor', amount: 0.5 }, label: 'Compress' },
  { eff: { type: 'gate', amount: 0.5 }, label: 'Noise gate' },
]

/** edit.eq named presets (resolved server-side to high/low-pass + bands). */
export const EQ_PRESETS: { preset: 'voice' | 'warmth' | 'de_rumble' | 'phone' | 'de_ess' | 'brighten'; label: string }[] = [
  { preset: 'voice', label: 'Voice' },
  { preset: 'warmth', label: 'Warmth' },
  { preset: 'de_rumble', label: 'De-rumble' },
  { preset: 'phone', label: 'Phone' },
  { preset: 'de_ess', label: 'De-ess' },
  { preset: 'brighten', label: 'Brighten' },
]

export const BLEND_MODES = ['normal', 'multiply', 'screen', 'overlay', 'darken', 'lighten', 'difference', 'addition', 'subtract', 'softlight', 'hardlight'] as const
export type BlendMode = (typeof BLEND_MODES)[number]

export function blendModeFromInput(value: string, fallback: BlendMode): BlendMode {
  for (const option of BLEND_MODES) {
    if (option === value) return option
  }
  return fallback
}

export const REDACT_PRESETS: { mode: 'blur' | 'pixelate'; label: string }[] = [
  { mode: 'blur', label: 'Blur centre' },
  { mode: 'pixelate', label: 'Pixelate centre' },
]

export const REDACT_CENTRE_POINTS: [number, number][] = [[0.33, 0.25], [0.67, 0.75]]
export type RedactMode = 'blur' | 'pixelate' | 'box'
export const REDACT_MODES: RedactMode[] = ['blur', 'pixelate', 'box']

export function redactModeFromInput(value: string, fallback: RedactMode): RedactMode {
  for (const option of REDACT_MODES) {
    if (option === value) return option
  }
  return fallback
}

export const CLEANUP_STRENGTHS: { strength: 'light' | 'medium' | 'strong'; label: string }[] = [
  { strength: 'light', label: 'Light' },
  { strength: 'medium', label: 'Medium' },
  { strength: 'strong', label: 'Strong' },
]
export type CleanupStrength = (typeof CLEANUP_STRENGTHS)[number]['strength']

export function cleanupStrengthFromInput(value: string, fallback: CleanupStrength): CleanupStrength {
  for (const option of CLEANUP_STRENGTHS) {
    if (option.strength === value) return option.strength
  }
  return fallback
}

export const CAPTION_CARD_MS = 2500
export const isObject = (v: unknown): v is object => v !== null && typeof v === 'object'
export const isRangeMs = (v: unknown): v is [number, number] =>
  Array.isArray(v) && v.length === 2 && typeof v[0] === 'number' && typeof v[1] === 'number'

export interface CaptionSelectionClip {
  id: string
  text: string
  range_ms: [number, number]
  style_ref?: string
}

export interface TitleSelectionClip {
  id: string
  title_text: string
}

export interface ShapeSelectionClip {
  id: string
  shape_kind: string
  shape_label?: string
  shape_color?: string
}

/** Inspector-only projection of render state already returned by project.state.
 * Keep these optional additions outside the shared legacy client-model backlog. */
export type InspectorMediaClip = Extract<Clip, { asset: string }> & {
  matte?: Record<string, unknown> | null
  mask?: Record<string, unknown> | null
  stabilize?: Record<string, unknown> | null
  speed_ramp?: Record<string, unknown> | null
}
export interface InspectorMediaSelection {
  clip: InspectorMediaClip
  trackKind: 'video' | 'audio'
  trackId: string
}

export const CAPTION_POSITIONS: { pos: 'bottom' | 'top' | 'center'; label: string }[] = [
  { pos: 'bottom', label: 'Bottom' },
  { pos: 'top', label: 'Top' },
  { pos: 'center', label: 'Center' },
]
export type CaptionPosition = (typeof CAPTION_POSITIONS)[number]['pos']

export function captionPositionFromInput(value: string, fallback: CaptionPosition): CaptionPosition {
  for (const option of CAPTION_POSITIONS) {
    if (option.pos === value) return option.pos
  }
  return fallback
}

export const ZOOM_INTENSITIES: { value: number; label: string }[] = [
  { value: 0.12, label: 'Subtle (12%)' },
  { value: 0.2, label: 'Medium (20%)' },
  { value: 0.35, label: 'Strong (35%)' },
]

export type AdjLook = { grade?: VerbArgs['edit.adjustment']['grade']; effects?: ClipEffect[] }
export const ADJ_LOOKS: { key: string; label: string; look: AdjLook }[] = [
  { key: 'vignette', label: 'Vignette', look: { effects: [{ type: 'vignette', amount: 0.5 }] } },
  { key: 'cinematic', label: 'Cinematic warm', look: { grade: { contrast: 1.12, saturation: 1.08, temperature_k: 5200 } } },
  { key: 'bw', label: 'Black & white', look: { grade: { saturation: 0 } } },
  { key: 'sepia', label: 'Sepia', look: { effects: [{ type: 'sepia' }] } },
]

export const COLOR_SPACES: { value: ColorSpace; label: string }[] = [
  { value: 'rec709', label: 'Rec.709 (HD/SDR)' },
  { value: 'rec2020', label: 'Rec.2020 (UHD/HDR)' },
  { value: 'srgb', label: 'sRGB' },
  { value: 'linear', label: 'Linear (scene)' },
]

export function colorSpaceFromInput(value: string): ColorSpace | null {
  for (const option of COLOR_SPACES) {
    if (option.value === value) return option.value
  }
  return null
}

export const GRADE_STACK_LAYERS: { key: string; label: string; grade: Partial<ClipGrade> }[] = [
  { key: 'contrast', label: 'Contrast +', grade: { contrast: 1.15 } },
  { key: 'brighten', label: 'Brighten', grade: { brightness: 0.1 } },
  { key: 'darken', label: 'Darken', grade: { brightness: -0.1 } },
  { key: 'desat', label: 'Desaturate', grade: { saturation: 0.6 } },
  { key: 'warm', label: 'Warm (temp)', grade: { temperature_k: 7000 } },
]

export type WinShape = 'rect' | 'ellipse'
export const WINDOW_REGIONS: { key: string; label: string; shape: WinShape; points: [number, number][] }[] = [
  { key: 'center', label: 'Center box', shape: 'rect', points: [[0.25, 0.25], [0.75, 0.75]] },
  { key: 'left', label: 'Left half', shape: 'rect', points: [[0, 0], [0.5, 1]] },
  { key: 'right', label: 'Right half', shape: 'rect', points: [[0.5, 0], [1, 1]] },
  { key: 'top', label: 'Top half', shape: 'rect', points: [[0, 0], [1, 0.5]] },
  { key: 'bottom', label: 'Bottom half', shape: 'rect', points: [[0, 0.5], [1, 1]] },
  { key: 'oval', label: 'Center oval', shape: 'ellipse', points: [[0.5, 0.5], [0.3, 0.3]] },
]

export const WINDOW_LOOKS: { key: string; label: string; grade: Partial<ClipGrade> }[] = [
  { key: 'brighten', label: 'Brighten', grade: { brightness: 0.12 } },
  { key: 'darken', label: 'Darken', grade: { brightness: -0.12 } },
  { key: 'punch', label: 'More contrast', grade: { contrast: 1.25 } },
  { key: 'desat', label: 'Desaturate', grade: { saturation: 0.4 } },
  { key: 'warm', label: 'Warm (temp)', grade: { temperature_k: 7000 } },
]

export function gradeSummary(g: Partial<ClipGrade>): string {
  const parts: string[] = []
  if (g.contrast != null && g.contrast !== 1) parts.push(`con ${g.contrast}`)
  if (g.brightness != null && g.brightness !== 0) parts.push(`bri ${g.brightness}`)
  if (g.saturation != null && g.saturation !== 1) parts.push(`sat ${g.saturation}`)
  if (g.gamma != null && g.gamma !== 1) parts.push(`gam ${g.gamma}`)
  if (g.temperature_k != null) parts.push(`${g.temperature_k}K`)
  if (g.lut) parts.push('LUT')
  return parts.length ? parts.join(' · ') : '—'
}

export type GradeLayerArg = VerbArgs['edit.grade_stack']['grades'][number]
export function toGradeLayer(g: ClipGrade | Partial<ClipGrade>): GradeLayerArg {
  const out: GradeLayerArg = {}
  if (g.contrast != null) out.contrast = g.contrast
  if (g.brightness != null) out.brightness = g.brightness
  if (g.saturation != null) out.saturation = g.saturation
  if (g.gamma != null) out.gamma = g.gamma
  if (g.temperature_k != null) out.temperature_k = g.temperature_k
  if (g.lut != null) out.lut = g.lut
  return out
}

export const TRANSLATE_LANGS: { code: string; label: string }[] = [
  { code: 'es', label: 'Spanish' },
  { code: 'fr', label: 'French' },
  { code: 'de', label: 'German' },
  { code: 'lv', label: 'Latvian' },
  { code: 'it', label: 'Italian' },
  { code: 'pt', label: 'Portuguese' },
  { code: 'nl', label: 'Dutch' },
  { code: 'pl', label: 'Polish' },
  { code: 'ru', label: 'Russian' },
  { code: 'ja', label: 'Japanese' },
  { code: 'zh', label: 'Chinese' },
]

export const fmtDur = (ms: number) => {
  const s = Math.max(0, Math.round(ms / 100) / 10)
  return `${s}s`
}

export function clipEffectFromCatalog(e: EffectCatalogEntry): ClipEffect | null {
  const numberDefault = (name: string) => e.params.find((p) => p.name === name && p.kind === 'number' && typeof p.default === 'number')?.default
  const amount = numberDefault('amount')
  switch (e.key) {
    case 'vignette': return amount == null ? { type: 'vignette' } : { type: 'vignette', amount }
    case 'sharpen': return amount == null ? { type: 'sharpen' } : { type: 'sharpen', amount }
    case 'grain': return amount == null ? { type: 'grain' } : { type: 'grain', amount }
    case 'denoise': return amount == null ? { type: 'denoise' } : { type: 'denoise', amount }
    case 'compressor': return amount == null ? { type: 'compressor' } : { type: 'compressor', amount }
    case 'gate': return amount == null ? { type: 'gate' } : { type: 'gate', amount }
    case 'auto_color': return amount == null ? { type: 'auto_color' } : { type: 'auto_color', amount }
    case 'vhs': return amount == null ? { type: 'vhs' } : { type: 'vhs', amount }
    case 'blur': {
      const radius = numberDefault('radius')
      return radius == null ? { type: 'blur' } : { type: 'blur', radius }
    }
    case 'hue_shift': {
      // The catalog/serde default is 0° (identity — keeps old projects
      // byte-identical), but a CHIP CLICK must visibly do something: seeding 0
      // made the effect land in state while the frame stayed pixel-identical
      // (the full-coverage gate flagged it: ssim=1.0000). Seed a half-turn;
      // the user dials the exact hue afterwards.
      const degrees = numberDefault('degrees')
      return { type: 'hue_shift', degrees: degrees == null || degrees === 0 ? 180 : degrees }
    }
    case 'rgb_split': return amount == null ? { type: 'rgb_split' } : { type: 'rgb_split', amount }
    case 'pixelize': {
      const size = numberDefault('size')
      return size == null ? { type: 'pixelize' } : { type: 'pixelize', size }
    }
    case 'posterize': {
      const levels = numberDefault('levels')
      return levels == null ? { type: 'posterize' } : { type: 'posterize', levels }
    }
    case 'invert': return { type: 'invert' }
    case 'emboss': return { type: 'emboss' }
    case 'mirror': return { type: 'mirror' }
    case 'flip': return { type: 'flip' }
    case 'sepia': return { type: 'sepia' }
    default: return null
  }
}

function numberProp(v: object, name: string): number | undefined {
  const value = Reflect.get(v, name)
  return typeof value === 'number' ? value : undefined
}

function stringProp(v: object, name: string): string | undefined {
  const value = Reflect.get(v, name)
  return typeof value === 'string' ? value : undefined
}

export function clipEffectsOf(value: unknown): ClipEffect[] {
  if (!Array.isArray(value)) return []
  const out: ClipEffect[] = []
  for (const item of value) {
    if (!isObject(item) || !('type' in item) || typeof item.type !== 'string') continue
    switch (item.type) {
      case 'vignette': out.push({ type: 'vignette', amount: numberProp(item, 'amount') }); break
      case 'sharpen': out.push({ type: 'sharpen', amount: numberProp(item, 'amount') }); break
      case 'blur': out.push({ type: 'blur', radius: numberProp(item, 'radius') }); break
      case 'grain': out.push({ type: 'grain', amount: numberProp(item, 'amount') }); break
      case 'chroma_key': {
        const color = stringProp(item, 'color')
        if (color) out.push({ type: 'chroma_key', color, similarity: numberProp(item, 'similarity'), blend: numberProp(item, 'blend') })
        break
      }
      case 'denoise': out.push({ type: 'denoise', amount: numberProp(item, 'amount') }); break
      case 'compressor': out.push({ type: 'compressor', amount: numberProp(item, 'amount') }); break
      case 'gate': out.push({ type: 'gate', amount: numberProp(item, 'amount') }); break
      case 'mirror': out.push({ type: 'mirror' }); break
      case 'flip': out.push({ type: 'flip' }); break
      case 'hue_shift': out.push({ type: 'hue_shift', degrees: numberProp(item, 'degrees') }); break
      case 'rgb_split': out.push({ type: 'rgb_split', amount: numberProp(item, 'amount') }); break
      case 'pixelize': out.push({ type: 'pixelize', size: numberProp(item, 'size') }); break
      case 'sepia': out.push({ type: 'sepia' }); break
      case 'auto_color': out.push({ type: 'auto_color', amount: numberProp(item, 'amount') }); break
      case 'vhs': out.push({ type: 'vhs', amount: numberProp(item, 'amount') }); break
      case 'posterize': out.push({ type: 'posterize', levels: numberProp(item, 'levels') }); break
      case 'invert': out.push({ type: 'invert' }); break
      case 'emboss': out.push({ type: 'emboss' }); break
      default: break
    }
  }
  return out
}

export function sourceDims(project: Project | null, assetId: string | null): SourceDims | null {
  if (!project || !assetId) return null
  const probe = project.assets?.[assetId]?.probe
  if (!isObject(probe) || !('width' in probe) || !('height' in probe)) return null
  if (typeof probe.width !== 'number' || typeof probe.height !== 'number') return null
  return { w: probe.width, h: probe.height }
}

export function assetProbeKind(asset: Asset): string {
  const probe = asset.probe
  if (!isObject(probe) || !('kind' in probe) || typeof probe.kind !== 'string') return 'other'
  return probe.kind
}

export function assetBasename(asset: Asset, fallback: string): string {
  return asset.path?.split(/[\\/]/).filter(Boolean).pop() || fallback
}

export function replacementCandidates(project: Project | null, selectedAsset: string, trackKind: 'video' | 'audio' | 'caption') {
  if (!project || !selectedAsset || trackKind === 'caption') return []
  return Object.entries(project.assets ?? {})
    .filter(([id, asset]) => {
      if (id === selectedAsset) return false
      const kind = assetProbeKind(asset)
      if (trackKind === 'video') return kind === 'video' || kind === 'image' || kind === 'other'
      if (trackKind === 'audio') return kind === 'audio' || kind === 'other'
      return false
    })
    .map(([id, asset]) => ({ id, label: assetBasename(asset, id) }))
}
