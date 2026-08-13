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
    description: 'Journal, media, jobs, capture and local-tool recovery evidence.',
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
