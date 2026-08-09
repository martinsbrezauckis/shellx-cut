import type { DoctorReport } from '../../lib/doctor'

export interface MediaReadinessProbe {
  kind?: string
  duration_ms?: number
  width?: number
  height?: number
  fps?: number
  has_audio?: boolean
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

export function isVideoAsset(asset: MediaReadinessAsset): boolean {
  return (asset.probe.kind ?? 'video') === 'video'
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

export function assetReadiness(asset: MediaReadinessAsset): AssetReadiness {
  const kind = asset.probe.kind ?? 'video'
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

  return {
    level: 'ready',
    label: 'Ready',
    hint: 'This asset is available for editing.',
    needsAction: false,
    badges: [{ label: 'Ready', tone: 'good', title: 'This asset is available for editing.' }],
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
  const offlineRows = assets.filter((asset) => asset.offline)
  const availableVideos = videos.filter((asset) => !asset.offline)
  const sourceOnlyRows = availableVideos.filter((asset) => !asset.proxy)
  const heavySourceRows = sourceOnlyRows.filter(isLargeSourcePlayback)
  const proxyReadyRows = availableVideos.filter((asset) => !!asset.proxy)
  const filmstripMissing = videos.filter((asset) => !asset.film).length
  const usedOffline = offlineRows.filter((asset) => asset.used > 0).length
  const needsAction = assets.filter((asset) => assetReadiness(asset).needsAction).length
  const available = total - offlineRows.length
  const analysisRows = assets.filter((asset) => (asset.probe.kind ?? 'video') !== 'image' && !asset.offline)
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
