import { cardLabel, type DoctorCard } from '../../lib/doctor'
import { isAgentInstruction } from '../../lib/userActionFeedback'

export const SETTINGS_CATEGORY_IDS = [
  'overview',
  'health-recovery',
  'general',
  'editing',
  'video-performance',
  'ai-transcription',
  'recording',
  'services-integrations',
  'agent-control',
  'storage-privacy',
  'about',
] as const

export type SettingsCategoryId = (typeof SETTINGS_CATEGORY_IDS)[number]

// Keep the native category rail addressable through the same stable identity
// that macOS Accessibility exposes as AXDOMIdentifier. The data-cut attribute
// remains the browser-harness contract; this ID supplies a semantic native
// lookup without adding any visible UI metadata.
export function settingsCategoryDomId(id: SettingsCategoryId): `cut-settings-category-${SettingsCategoryId}` {
  return `cut-settings-category-${id}`
}

export interface SettingsCategory {
  id: SettingsCategoryId
  label: string
  description: string
  keywords: readonly string[]
}

export const SETTINGS_CATEGORIES: readonly SettingsCategory[] = [
  {
    id: 'overview',
    label: 'Overview',
    description: 'Readiness and the next useful setup actions.',
    keywords: ['status', 'ready', 'setup', 'health'],
  },
  {
    id: 'health-recovery',
    label: 'Health & recovery',
    description: 'Find missing media, recover recordings and retry failed work.',
    keywords: ['health', 'recovery', 'journal', 'offline', 'proxy', 'filmstrip', 'jobs', 'capture', 'repair'],
  },
  {
    id: 'general',
    label: 'General',
    description: 'Save location and interface appearance.',
    keywords: ['export', 'folder', 'destination', 'theme', 'light', 'dark'],
  },
  {
    id: 'editing',
    label: 'Editing',
    description: 'Keyboard shortcuts and editor behaviour.',
    keywords: ['keyboard', 'shortcut', 'keymap', 'remap', 'timeline'],
  },
  {
    id: 'video-performance',
    label: 'Video & performance',
    description: 'Video processing, media inspection and hardware acceleration.',
    keywords: ['ffmpeg', 'ffprobe', 'gpu', 'encode', 'import', 'render', 'performance'],
  },
  {
    id: 'ai-transcription',
    label: 'AI & transcription',
    description: 'Captions, speech models and background removal.',
    keywords: ['captions', 'transcription', 'speech', 'perception', 'matte', 'background', 'model'],
  },
  {
    id: 'recording',
    label: 'Recording',
    description: 'Capture readiness, permissions and fixed recording keys.',
    keywords: ['record', 'capture', 'screen', 'microphone', 'camera', 'f9', 'f10', 'f11', 'f12'],
  },
  {
    id: 'services-integrations',
    label: 'Services & integrations',
    description: 'Optional dubbing, speaker labels and review providers.',
    keywords: ['dub', 'diarize', 'speaker', 'judge', 'service', 'provider', 'cli'],
  },
  {
    id: 'agent-control',
    label: 'Agent control',
    description: 'Local Debug API and MCP access through the same Cut verbs.',
    keywords: ['agent', 'client', 'debug api', 'api', 'mcp', 'stdio', 'cutd', 'control'],
  },
  {
    id: 'storage-privacy',
    label: 'Storage & privacy',
    description: 'Download space, local data and network-activity boundaries.',
    keywords: ['disk', 'storage', 'cache', 'privacy', 'local', 'data', 'network', 'github', 'update check'],
  },
  {
    id: 'about',
    label: 'About',
    description: 'Version, updates, website and license.',
    keywords: ['version', 'update', 'license', 'github', 'shellx'],
  },
] as const

export interface SettingsSearchResult {
  category: SettingsCategory
  matched: string
}

export function isSettingsCategoryId(value: string): value is SettingsCategoryId {
  return SETTINGS_CATEGORY_IDS.includes(value as SettingsCategoryId)
}

export function settingsCategory(id: SettingsCategoryId): SettingsCategory {
  return SETTINGS_CATEGORIES.find((category) => category.id === id) ?? SETTINGS_CATEGORIES[0]
}

export function searchSettings(query: string): SettingsSearchResult[] {
  const needle = query.trim().toLocaleLowerCase()
  if (!needle) return []
  return SETTINGS_CATEGORIES.flatMap((category) => {
    const candidates = [category.label, category.description, ...category.keywords]
    const matched = candidates.find((candidate) => candidate.toLocaleLowerCase().includes(needle))
    return matched ? [{ category, matched }] : []
  })
}

const TOOLCHAIN_CARD_HINTS: Record<string, { missing: string; degraded: string }> = {
  perception: {
    missing: 'Captions and transcription are not installed yet. Open Settings > AI & transcription and choose Install captions.',
    degraded: 'Captions and transcription need attention. Open Settings > AI & transcription to install or repair them.',
  },
  matte: {
    missing: 'Background removal is not installed yet. Open Settings > AI & transcription and choose Install.',
    degraded: 'Background removal needs attention. Open Settings > AI & transcription to check or reinstall it.',
  },
  matte_premium: {
    missing: 'Cleaner background removal is not installed yet. Open Settings > AI & transcription and choose Install premium on a supported NVIDIA graphics card.',
    degraded: 'Cleaner background removal needs attention. Open Settings > AI & transcription to check it.',
  },
  ffmpeg: {
    missing: 'Video engine is missing. Open Settings > Video & performance and choose Download.',
    degraded: 'Video engine needs attention. Open Settings > Video & performance to check or repair it.',
  },
  ffprobe: {
    missing: 'Media inspection tool is missing. Open Settings > Video & performance and choose Download.',
    degraded: 'Media inspection tool needs attention. Open Settings > Video & performance to check or repair it.',
  },
  'vc-runtime': {
    missing: 'Microsoft C++ Redistributable is missing. Open Settings > Video & performance and choose Download and install.',
    degraded: 'Microsoft C++ Redistributable needs attention. Open Settings > Video & performance to check it.',
  },
  'gpu-encode': {
    missing: 'Hardware-accelerated export is not ready. Open Settings > Video & performance to check it.',
    degraded: 'Hardware-accelerated export needs attention. Open Settings > Video & performance to check it.',
  },
}

function safePersonHint(card: DoctorCard): string {
  const hint = card.hint?.trim()
  if (
    hint
    && !hint.includes('system.')
    && !hint.includes('{')
    && !hint.includes('Settings ->')
    && !isAgentInstruction(hint)
  ) {
    return hint
  }
  return `${cardLabel(card).title} needs attention. Open Settings to check it.`
}

/**
 * Formats a non-healthy DoctorCard hint as a plain sentence for people on the
 * Settings > Health & recovery Toolchain card without exposing agent verb syntax.
 */
export function toolchainCardHint(card: DoctorCard): string {
  const mapped = TOOLCHAIN_CARD_HINTS[card.id]
  if (mapped) return card.status === 'missing' ? mapped.missing : mapped.degraded
  if (card.kind === 'service') {
    return `${cardLabel(card).title} needs attention. Open Settings > Services & integrations to check it.`
  }
  if (card.kind === 'judge') {
    return `${cardLabel(card).title} cannot run until Captions and transcription are set up. Open Settings > AI & transcription or Settings > Services & integrations.`
  }
  return safePersonHint(card)
}
