import { useCallback, useEffect, useRef, useState } from 'react'
import { cannotDecodeProblem, MediaDecodeHealth, NO_VIDEO_PICTURE, type PreviewMediaProblem } from './mediaDecodeHealth'

/** Delay before the one retry of a proxy that failed to decode. */
const RETRY_DELAY_MS = 300

export interface DecodeReporter {
  /** A proxy element has data (loadeddata); it counts as decoded only with a picture. */
  onDecoded: (element: HTMLVideoElement, assetId: string) => void
  /** A proxy element failed; `assetId` is the clip's asset. */
  onDecodeError: (element: HTMLMediaElement, assetId: string) => void
}

/** Tracks whether the preview's proxy video can be decoded at all on this
 * system (see mediaDecodeHealth.ts) and gives the monitor the problem to show.
 * A first failure is retried once with a reload; a repeat with no proxy ever
 * decoded is reported. Reset when the project changes. */
export function usePreviewDecodeHealth(os: string | null | undefined, projectKey: string | null) {
  const health = useRef(new MediaDecodeHealth())
  const [assetIds, setAssetIds] = useState<string[]>([])
  useEffect(() => {
    health.current = new MediaDecodeHealth()
    setAssetIds([])
  }, [projectKey])
  const judge = useCallback((element: HTMLMediaElement, assetId: string, code: number | typeof NO_VIDEO_PICTURE | undefined) => {
    const src = element.currentSrc || element.getAttribute('src') || ''
    const verdict = health.current.failed(src, code)
    if (verdict === 'retry') {
      window.setTimeout(() => { if (element.isConnected) element.load() }, RETRY_DELAY_MS)
    } else if (verdict === 'cannot_decode') {
      setAssetIds((ids) => (ids.includes(assetId) ? ids : [...ids, assetId]))
    }
  }, [])
  const onDecoded = useCallback((element: HTMLVideoElement, assetId: string) => {
    if (element.videoWidth === 0) { judge(element, assetId, NO_VIDEO_PICTURE); return }
    if (health.current.decoded()) setAssetIds((ids) => (ids.length ? [] : ids))
  }, [judge])
  const onDecodeError = useCallback((element: HTMLMediaElement, assetId: string) => {
    judge(element, assetId, element.error?.code)
  }, [judge])
  const problem: PreviewMediaProblem | null = assetIds.length ? cannotDecodeProblem(os, assetIds) : null
  const reporter: DecodeReporter = { onDecoded, onDecodeError }
  return { problem, reporter }
}

/** The monitor notice: same card as the FFmpeg setup notice. `ui.state`
 * reads the same fact from these attributes (preview.media_problem). */
export function PreviewDecodeNotice({ problem }: { problem: PreviewMediaProblem }) {
  return (
    <div
      className="pv-setup"
      role="alert"
      data-cut-preview-media-problem={problem.kind}
      data-cut-preview-media-assets={problem.assetIds.join(' ')}
    >
      <div className="pv-setup-copy">
        <strong data-cut-preview-media-problem-title>{problem.title}</strong>
        <span data-cut-preview-media-problem-detail>{problem.detail}</span>
      </div>
    </div>
  )
}
