// Find-media provider catalog parsing and presentation.
//
// The server's `assets.providers` result owns availability and supported media
// kinds. This module deliberately keeps the UI's small amount of presentation
// policy (friendly labels, local-folder requirements, and safe outgoing links)
// separate from the mounted panel.

export const ASSET_PROVIDER_NAMES = [
  'local_folder',
  'openverse',
  'archive_org',
  'wikimedia',
  'nasa',
  'stickers',
] as const

export type AssetProviderName = typeof ASSET_PROVIDER_NAMES[number]
export type AssetKind = 'audio' | 'image' | 'video'

export interface AssetProvider {
  name: AssetProviderName
  kinds: AssetKind[]
  needsKey: boolean
  network: boolean
  note: string
}

export interface ProviderHit {
  provider: AssetProviderName
  id: string
  title: string
  kind: AssetKind
  creator: string | null
  license: string
  licenseUrl: string | null
  sourceUrl: string | null
  filetype: string | null
  durationMs: number | null
  attribution: string
  requiresAttribution: boolean
}

const PROVIDER_LABELS: Record<AssetProviderName, string> = {
  local_folder: 'Local folder',
  openverse: 'Openverse',
  archive_org: 'Internet Archive',
  wikimedia: 'Wikimedia Commons',
  nasa: 'NASA Image and Video Library',
  stickers: 'Built-in stickers',
}

const ASSET_KINDS: AssetKind[] = ['audio', 'image', 'video']

const isRecord = (value: unknown): value is Record<string, unknown> => (
  typeof value === 'object' && value !== null && !Array.isArray(value)
)

export function isAssetProviderName(value: unknown): value is AssetProviderName {
  return typeof value === 'string' && (ASSET_PROVIDER_NAMES as readonly string[]).includes(value)
}

export function isAssetKind(value: unknown): value is AssetKind {
  return typeof value === 'string' && ASSET_KINDS.includes(value as AssetKind)
}

export function providerLabel(name: AssetProviderName): string {
  return PROVIDER_LABELS[name]
}

export function providerNeedsDirectory(name: AssetProviderName): boolean {
  return name === 'local_folder'
}

/** Local folders list matching files and stickers are an offline browse catalog. */
export function providerAllowsEmptyQuery(name: AssetProviderName): boolean {
  return name === 'local_folder' || name === 'stickers'
}

export function providerQueryLabel(name: AssetProviderName): string {
  if (name === 'local_folder') return 'Filename contains'
  if (name === 'stickers') return 'Find stickers'
  return 'Search'
}

export function providerQueryPlaceholder(name: AssetProviderName): string {
  if (name === 'local_folder') return 'Optional — leave blank to list this folder'
  if (name === 'stickers') return 'Optional — leave blank to browse all stickers'
  return 'e.g. city, rain, applause'
}

/**
 * Accept only the documented request vocabulary. A newer server cannot silently
 * acquire a UI path until this client can construct a schema-valid request for it.
 */
export function normalizeProviderCatalog(value: unknown): AssetProvider[] {
  if (!Array.isArray(value)) return []
  const seen = new Set<AssetProviderName>()
  const providers: AssetProvider[] = []
  for (const raw of value) {
    if (!isRecord(raw) || !isAssetProviderName(raw.name) || seen.has(raw.name)) continue
    if (!Array.isArray(raw.kinds) || typeof raw.needs_key !== 'boolean' || typeof raw.network !== 'boolean') continue
    const kinds = [...new Set(raw.kinds.filter(isAssetKind))]
    const note = typeof raw.note === 'string' ? raw.note.trim() : ''
    if (kinds.length === 0 || !note) continue
    seen.add(raw.name)
    providers.push({
      name: raw.name,
      kinds,
      needsKey: raw.needs_key,
      network: raw.network,
      note,
    })
  }
  return providers
}

export function preferredProvider(providers: AssetProvider[]): AssetProvider | null {
  return providers.find((provider) => provider.name === 'openverse') ?? providers[0] ?? null
}

function optionalString(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value : null
}

function optionalNonNegativeNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : null
}

/** Keep malformed or provider-crossed results out of the importable UI. */
export function normalizeProviderHits(value: unknown, provider: AssetProvider): ProviderHit[] {
  if (!Array.isArray(value)) return []
  const hits: ProviderHit[] = []
  for (const raw of value) {
    if (!isRecord(raw) || raw.provider !== provider.name || !isAssetKind(raw.kind)) continue
    if (!provider.kinds.includes(raw.kind) || typeof raw.id !== 'string' || !raw.id) continue
    if (typeof raw.title !== 'string' || !raw.title.trim()) continue
    if (typeof raw.license !== 'string' || !raw.license.trim()) continue
    if (typeof raw.attribution !== 'string' || !raw.attribution.trim()) continue
    if (typeof raw.requires_attribution !== 'boolean') continue
    hits.push({
      provider: provider.name,
      id: raw.id,
      title: raw.title,
      kind: raw.kind,
      creator: optionalString(raw.creator),
      license: raw.license,
      licenseUrl: optionalString(raw.license_url),
      sourceUrl: optionalString(raw.source_url),
      filetype: optionalString(raw.filetype),
      durationMs: optionalNonNegativeNumber(raw.duration_ms),
      attribution: raw.attribution,
      requiresAttribution: raw.requires_attribution,
    })
  }
  return hits
}

/** Do not turn provider metadata into a javascript:, file:, or data: navigation. */
export function safeExternalUrl(value: string | null): string | null {
  if (!value) return null
  try {
    const url = new URL(value)
    return url.protocol === 'https:' || url.protocol === 'http:' ? url.href : null
  } catch {
    return null
  }
}
