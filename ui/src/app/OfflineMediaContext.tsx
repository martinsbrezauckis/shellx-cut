import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import { callVerb, type Project } from '../lib/client'
import {
  offlineMediaMaps,
  type MediaCheckResult,
  type MediaCheckRow,
} from '../lib/offlineMedia'
import { mediaBasename } from '../lib/mediaPath'
import { isTauri, pickMedia } from '../lib/tauri'
import { publishUserActionMessage, runUserVerb } from '../lib/userActionFeedback'

interface OfflineMediaContextValue {
  offlineAssetIds: ReadonlySet<string>
  modifiedMs: ReadonlyMap<string, number>
  checking: boolean
  relinkingAssetId: string | null
  refresh: () => Promise<void>
  relinkAsset: (assetId: string) => Promise<boolean>
}

const EMPTY_IDS = new Set<string>()
const EMPTY_MODIFIED = new Map<string, number>()
const OfflineMediaContext = createContext<OfflineMediaContextValue>({
  offlineAssetIds: EMPTY_IDS,
  modifiedMs: EMPTY_MODIFIED,
  checking: false,
  relinkingAssetId: null,
  refresh: async () => undefined,
  relinkAsset: async () => false,
})

export function OfflineMediaProvider({
  project,
  onProjectChanged,
  children,
}: {
  project: Project | null
  onProjectChanged: () => void | Promise<void>
  children: ReactNode
}) {
  const [snapshot, setSnapshot] = useState<{
    project: Project | null
    rows: MediaCheckRow[]
  }>({ project: null, rows: [] })
  const [checking, setChecking] = useState(false)
  const [relinkingAssetId, setRelinkingAssetId] = useState<string | null>(null)
  const checkSequence = useRef(0)
  const assetCount = Object.keys(project?.assets ?? {}).length

  const refresh = useCallback(async () => {
    const request = ++checkSequence.current
    if (!project || assetCount === 0) {
      setSnapshot({ project, rows: [] })
      setChecking(false)
      return
    }
    setChecking(true)
    const response = await callVerb('media.check', {})
    if (request !== checkSequence.current) return
    setChecking(false)
    if (!response.ok) return
    const result = response.result as MediaCheckResult | undefined
    setSnapshot({ project, rows: result?.assets ?? [] })
  }, [assetCount, project])

  useEffect(() => {
    void refresh()
    return () => {
      checkSequence.current += 1
    }
  }, [refresh])

  const maps = useMemo(() => (
    snapshot.project === project ? offlineMediaMaps(snapshot.rows) : {
      offlineAssetIds: EMPTY_IDS,
      modifiedMs: EMPTY_MODIFIED,
    }
  ), [project, snapshot])

  const relinkAsset = useCallback(async (assetId: string) => {
    if (!project || relinkingAssetId) return false
    const asset = project.assets[assetId]
    if (!asset) return false
    if (!isTauri()) {
      publishUserActionMessage('Open the desktop app to browse for the moved source file.')
      return false
    }
    let paths: string[]
    try {
      paths = await pickMedia()
    } catch {
      publishUserActionMessage('Could not open the media picker.')
      return false
    }
    if (!paths.length) return false
    setRelinkingAssetId(assetId)
    const response = await runUserVerb(
      'media.relink',
      { asset: assetId, path: paths[0], rationale: 'user: relink offline source' },
      `Could not relink ${mediaBasename(asset.path)}.`,
    )
    setRelinkingAssetId(null)
    if (!response?.ok) return false

    setSnapshot((current) => ({
      ...current,
      rows: current.rows.map((row) => row.asset === assetId ? { ...row, exists: true } : row),
    }))
    const warning = response.warnings?.[0]?.message
    publishUserActionMessage(
      warning ? `Relinked ${mediaBasename(asset.path)}. ${warning}` : `Relinked ${mediaBasename(asset.path)}.`,
    )
    await onProjectChanged()
    await refresh()
    return true
  }, [onProjectChanged, project, refresh, relinkingAssetId])

  const value = useMemo<OfflineMediaContextValue>(() => ({
    ...maps,
    checking,
    relinkingAssetId,
    refresh,
    relinkAsset,
  }), [checking, maps, refresh, relinkAsset, relinkingAssetId])

  return <OfflineMediaContext.Provider value={value}>{children}</OfflineMediaContext.Provider>
}

export function useOfflineMedia(): OfflineMediaContextValue {
  return useContext(OfflineMediaContext)
}
