import type { DoctorReport } from '../../lib/doctor'

export interface MediaReadinessProbe {
  kind?: string
  duration_ms?: number
  width?: number
  height?: number
  fps?: number
  has_audio?: boolean
  /** Set by the proxy job: how much of the source really decodes. `warning`
   *  is a plain sentence when the file is cut off or damaged. */
  decode_check?: { warning?: string }
  /** Nominal rate (ffprobe r_frame_rate) and whether it differs from the
   *  average: a variable-frame-rate clip, as phones record. */
  nominal_fps?: number
  variable_frame_rate?: boolean
}

export interface MediaReadinessAsset {
  id: string
  path: string
  film?: string
  proxy?: string
  transcript?: string
  perception?: string
  probe: MediaReadinessProbe
  offline: boolean
  used: number
}

export type MediaCapabilityState = 'ready' | 'unavailable' | 'unknown'
export type MediaDimensionState =
  | 'ready'
  | 'partial'
  | 'pending'
  | 'unavailable'
  | 'unknown'
  | 'source-fallback'
  | 'not-needed'

export interface MediaCapabilitySummary {
  speech: MediaCapabilityState
  perception: MediaCapabilityState
  optionalServicesReady: number
  optionalServicesTotal: number
}

export interface MediaHealthDimension {
  state: MediaDimensionState
  value: string
  detail: string
}

export type MediaReadinessLevel =
  | 'missing'
  | 'large-source'
  | 'source'
  | 'proxy-ready'
  | 'ready'

export interface ReadinessBadge {
  label: string
  tone: 'bad' | 'warn' | 'good' | 'neutral'
  title: string
}

export interface AssetReadiness {
  level: MediaReadinessLevel
  label: string
  hint: string
  needsAction: boolean
  badges: ReadinessBadge[]
}

export interface MediaHealthSummary {
  total: number
  videos: number
  offline: number
  usedOffline: number
  sourceOnly: number
  heavySource: number
  proxyReady: number
  filmstripMissing: number
  needsAction: number
  firstOffline: string | null
  level: 'missing' | 'source' | 'ready'
  title: string
  hint: string
  analysis: 'ready' | 'partial' | 'incomplete' | 'unavailable' | 'unknown' | 'not-needed'
  dimensions: {
    source: MediaHealthDimension
    edit: MediaHealthDimension
    proxy: MediaHealthDimension
    speech: MediaHealthDimension
    perception: MediaHealthDimension
    services: MediaHealthDimension
  }
}

const s = (count: number) => (count === 1 ? '' : 's')

/** What an asset is, for the labels a person reads:
 *  - `footage`: video that plays from an editing proxy, or from its original file
 *    until one is made (Proxy ready / Source playback / 4K source);
 *  - `title`: a title or shape movie Cut draws (`titles/title_*.mov`); it plays as
 *    drawn and never has a proxy;
 *  - `generated`: a clip made by a generator (`assets/generated/`); a video one
 *    plays like footage;
 *  - `image`, `audio`: stills and sound; no proxy is needed. */
export type AssetMediaKind = 'footage' | 'title' | 'generated' | 'image' | 'audio'

const DRAWN_TITLE = /[\\/]titles[\\/]title_[^\\/]+\.mov$/i
const GENERATED = /[\\/]assets[\\/]generated[\\/]/i

export function assetMediaKind(asset: Pick<MediaReadinessAsset, 'path' | 'probe'>): AssetMediaKind {
  const kind = asset.probe.kind ?? 'video'
  if (kind === 'image') return 'image'
  if (kind === 'audio') return 'audio'
  if (DRAWN_TITLE.test(asset.path)) return 'title'
  return GENERATED.test(asset.path) ? 'generated' : 'footage'
}

/** Video that plays from a proxy (footage and generated clips; never a title). */
export function isVideoAsset(asset: MediaReadinessAsset): boolean {
  return (asset.probe.kind ?? 'video') === 'video' && assetMediaKind(asset) !== 'title'
}

export function isLargeSourcePlayback(asset: MediaReadinessAsset): boolean {
  if (!isVideoAsset(asset) || asset.proxy) return false
  const w = asset.probe.width ?? 0
  const h = asset.probe.height ?? 0
  const fps = asset.probe.fps ?? 0
  return w >= 3840 || h >= 2160 || w * h >= 3840 * 2160 || fps >= 50
}

export function mediaCapabilitiesFromDoctor(
  doctor: DoctorReport | null | undefined,
): MediaCapabilitySummary {
  const perception = doctor?.cards.find((card) => card.id === 'perception')
  const services = doctor?.cards.filter((card) => card.kind === 'service') ?? []
  const perceptionState: MediaCapabilityState = !perception || perception.status === 'unknown'
    ? 'unknown'
    : perception.status === 'missing'
      ? 'unavailable'
      : 'ready'
  const speechState: MediaCapabilityState = !perception || perception.status === 'unknown'
    ? 'unknown'
    : perception.details.stt_ready === true
      ? 'ready'
      : 'unavailable'

  return {
    speech: speechState,
    perception: perceptionState,
    optionalServicesReady: services.filter((card) => card.status === 'ok').length,
    optionalServicesTotal: services.length,
  }
}

/** The engine's warning that only part of the file can be read, if any. */
export function decodeWarning(asset: Pick<MediaReadinessAsset, 'probe'>): string | null {
  const warning = asset.probe.decode_check?.warning
  return typeof warning === 'string' && warning.trim() ? warning.trim() : null
}

/** A short neutral badge for a variable-frame-rate clip, or null. */
export function variableRateBadge(asset: Pick<MediaReadinessAsset, 'probe'>): ReadinessBadge | null {
  if (asset.probe.variable_frame_rate !== true) return null
  const avg = asset.probe.fps
  const nominal = asset.probe.nominal_fps
  const rates = avg && nominal
    ? ` (${avg.toFixed(2)} fps on average, ${Number(nominal.toFixed(2))} fps nominal)`
    : ''
  return {
    label: 'Variable frame rate',
    tone: 'neutral',
    title: `This clip has a variable frame rate${rates}, as phones record. Exports place each frame at its real time.`,
  }
}

export function assetReadiness(asset: MediaReadinessAsset): AssetReadiness {
  const base = sourceReadiness(asset)
  const vfr = asset.offline ? null : variableRateBadge(asset)
  const readiness = vfr ? { ...base, badges: [...base.badges, vfr] } : base
  const warning = asset.offline ? null : decodeWarning(asset)
  if (!warning) return readiness
  // A cut-off file still edits, but the person must know before exporting.
  return {
    ...readiness,
    hint: warning,
    needsAction: true,
    badges: [{ label: 'File cut off', tone: 'bad', title: warning }, ...readiness.badges],
  }
}

function sourceReadiness(asset: MediaReadinessAsset): AssetReadiness {
  const kind = asset.probe.kind ?? 'video'
  if (assetMediaKind(asset) === 'title') {
    // A title is never relinked: Cut draws its movie again from its settings.
    const hint = asset.offline
      ? 'Its picture file is missing; Cut draws it again from the title\'s settings before the next exact frame or render.'
      : 'A title Cut draws. It plays as drawn and needs no proxy; if its file goes missing, Cut draws it again.'
    return {
      level: 'ready',
      label: 'Title',
      hint,
      needsAction: false,
      badges: [{ label: 'Title', tone: 'neutral', title: hint }],
    }
  }

  if (asset.offline) {
    return {
      level: 'missing',
      label: 'Missing source',
      hint: 'Relink this file before preview or export.',
      needsAction: true,
      badges: [{ label: 'Missing source', tone: 'bad', title: 'The original file is missing. Relink it before preview or export.' }],
    }
  }

  if (kind === 'video' && asset.proxy) {
    return {
      level: 'proxy-ready',
      label: 'Proxy ready',
      hint: 'Smooth editing media is available. Final export still uses the original source.',
      needsAction: false,
      badges: [{ label: 'Proxy ready', tone: 'good', title: 'Smooth editing media is available for this clip.' }],
    }
  }

  if (isLargeSourcePlayback(asset)) {
    const w = asset.probe.width ?? 0
    const h = asset.probe.height ?? 0
    if (!(w >= 3840 || h >= 2160 || w * h >= 3840 * 2160)) {
      return {
        level: 'large-source',
        label: 'High frame rate',
        hint: 'This clip may stutter during preview. Turn on proxies for future imports, or re-import it with proxies if playback lags.',
        needsAction: true,
        badges: [{ label: 'High frame rate', tone: 'warn', title: 'High-frame-rate source playback can stutter without a proxy.' }],
      }
    }
    return {
      level: 'large-source',
      label: '4K source',
      hint: 'This clip may stutter during preview. Turn on proxies for future imports, or re-import it with proxies if playback lags.',
      needsAction: true,
      badges: [{ label: '4K source', tone: 'warn', title: 'High-resolution source playback can stutter without a proxy.' }],
    }
  }

  if (kind === 'video') {
    return {
      level: 'source',
      label: 'Source playback',
      hint: 'This clip is using the original file for preview. Final export quality is unchanged.',
      needsAction: false,
      badges: [{ label: 'Source playback', tone: 'neutral', title: 'This clip previews from the original source file.' }],
    }
  }

  const what = kind === 'image' ? 'A still image' : kind === 'audio' ? 'Sound only' : 'This asset'
  return {
    level: 'ready',
    label: 'Ready',
    hint: `${what}: ready for editing. No proxy is needed.`,
    needsAction: false,
    badges: [{ label: 'Ready', tone: 'good', title: `${what}: ready for editing. No proxy is needed.` }],
  }
}

export function summarizeMediaReadiness(
  assets: MediaReadinessAsset[],
  capabilities: MediaCapabilitySummary = {
    speech: 'unknown',
    perception: 'unknown',
    optionalServicesReady: 0,
    optionalServicesTotal: 0,
  },
): MediaHealthSummary {
  const total = assets.length
  const videos = assets.filter(isVideoAsset)
  const offlineRows = assets.filter((asset) => asset.offline && assetMediaKind(asset) !== 'title')
  const availableVideos = videos.filter((asset) => !asset.offline)
  const sourceOnlyRows = availableVideos.filter((asset) => !asset.proxy)
  const heavySourceRows = sourceOnlyRows.filter(isLargeSourcePlayback)
  const proxyReadyRows = availableVideos.filter((asset) => !!asset.proxy)
  const filmstripMissing = videos.filter((asset) => !asset.film).length
  const usedOffline = offlineRows.filter((asset) => asset.used > 0).length
  const needsAction = assets.filter((asset) => assetReadiness(asset).needsAction).length
  const available = total - offlineRows.length
  // A title has no speech or picture to analyse: it is not counted as waiting for analysis.
  const analysisRows = assets.filter((asset) => (asset.probe.kind ?? 'video') !== 'image' && assetMediaKind(asset) !== 'title' && !asset.offline)
  const speechReady = analysisRows.filter((asset) => !!asset.transcript).length
  const perceptionReady = analysisRows.filter((asset) => !!asset.perception).length
  const analysisDimension = (
    ready: number,
    capability: MediaCapabilityState,
    name: string,
  ): MediaHealthDimension => {
    const eligible = analysisRows.length
    if (eligible === 0) {
      return { state: 'not-needed', value: 'Not needed', detail: `No available audio or video needs ${name}.` }
    }
    if (ready === eligible) {
      return { state: 'ready', value: `${ready}/${eligible}`, detail: `${name} is ready for every available audio and video asset.` }
    }
    if (ready > 0) {
      return { state: 'partial', value: `${ready}/${eligible}`, detail: `${name} exists for some available assets.` }
    }
    if (capability === 'unavailable') {
      return { state: 'unavailable', value: 'Unavailable', detail: `${name} tools are not available on this machine.` }
    }
    if (capability === 'unknown') {
      return { state: 'unknown', value: 'Unverified', detail: `${name} capability has not been verified.` }
    }
    return { state: 'pending', value: 'Not analyzed', detail: `${name} tools are ready, but no receipt exists yet.` }
  }
  const speech = analysisDimension(speechReady, capabilities.speech, 'Speech analysis')
  const perception = analysisDimension(perceptionReady, capabilities.perception, 'Perception analysis')
  const analysisStates = [speech.state, perception.state]
  const analysis = analysisRows.length === 0
    ? 'not-needed'
    : analysisStates.every((state) => state === 'ready')
      ? 'ready'
      : analysisStates.some((state) => state === 'partial' || state === 'ready')
        ? 'partial'
        : analysisStates.every((state) => state === 'unavailable')
          ? 'unavailable'
          : analysisStates.some((state) => state === 'pending')
            ? 'incomplete'
            : 'unknown'
  const analysisLabel = analysis === 'not-needed' ? 'no analysis needed' : `analysis ${analysis}`
  const level = offlineRows.length > 0 ? 'missing' : heavySourceRows.length > 0 ? 'source' : sourceOnlyRows.length > 0 ? 'source' : 'ready'
  const title =
    total === 0
      ? 'No media imported'
      : offlineRows.length > 0
        ? `${available === 0 ? 'Editing blocked' : 'Editing limited'} · ${offlineRows.length} source${s(offlineRows.length)} missing`
        : `Editing ready · ${analysisLabel}`
  const hint =
    total === 0
      ? 'Import media to begin editing.'
      : offlineRows.length > 0
        ? 'Relink missing files before preview or export.'
        : heavySourceRows.length > 0
          ? `Editing works, but ${heavySourceRows.length} large clip${s(heavySourceRows.length)} may preview more smoothly with a proxy.`
          : sourceOnlyRows.length > 0
            ? 'Editing uses the original files; analysis readiness is reported separately.'
            : 'Source, editing, and analysis readiness are reported separately below.'

  const proxyState: MediaHealthDimension = availableVideos.length === 0
    ? { state: 'not-needed', value: 'Not needed', detail: 'No video assets need editing proxies.' }
    : proxyReadyRows.length === availableVideos.length
      ? { state: 'ready', value: `${proxyReadyRows.length}/${availableVideos.length}`, detail: 'Every available video has an editing proxy.' }
      : proxyReadyRows.length > 0
        ? { state: 'partial', value: `${proxyReadyRows.length}/${availableVideos.length}`, detail: 'Some available videos use source playback.' }
        : { state: 'source-fallback', value: 'Source', detail: 'Videos remain editable from their original files.' }
  const services: MediaHealthDimension = capabilities.optionalServicesTotal === 0
    ? { state: 'unknown', value: 'Not reported', detail: 'Optional service cards are not available in the current doctor report.' }
    : capabilities.optionalServicesReady === capabilities.optionalServicesTotal
      ? { state: 'ready', value: `${capabilities.optionalServicesReady}/${capabilities.optionalServicesTotal}`, detail: 'All optional media services are reachable.' }
      : capabilities.optionalServicesReady > 0
        ? { state: 'partial', value: `${capabilities.optionalServicesReady}/${capabilities.optionalServicesTotal}`, detail: 'Some optional media services are reachable.' }
        : { state: 'unavailable', value: `0/${capabilities.optionalServicesTotal}`, detail: 'Optional media services are not reachable; core editing is unaffected.' }

  return {
    total,
    videos: videos.length,
    offline: offlineRows.length,
    usedOffline,
    sourceOnly: sourceOnlyRows.length,
    heavySource: heavySourceRows.length,
    proxyReady: proxyReadyRows.length,
    filmstripMissing,
    needsAction,
    firstOffline: offlineRows[0]?.id ?? null,
    level,
    title,
    hint,
    analysis,
    dimensions: {
      source: {
        state: offlineRows.length === 0 ? 'ready' : available > 0 ? 'partial' : 'unavailable',
        value: `${available}/${total}`,
        detail: offlineRows.length === 0 ? 'Every source file is available.' : 'Relink missing source files before preview or export.',
      },
      edit: {
        state: offlineRows.length === 0 ? 'ready' : available > 0 ? 'partial' : 'unavailable',
        value: `${available}/${total}`,
        detail: offlineRows.length === 0 ? 'Every asset is available for editing.' : 'Only available source files can be edited.',
      },
      proxy: proxyState,
      speech,
      perception,
      services,
    },
  }
}
