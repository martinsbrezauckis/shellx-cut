// Timeline source-reveal actions — identity-only desktop and in-app routing.

import { requestSourceNavigation } from '../../app/sourceNavigation'
import { revealRegisteredSource } from '../../lib/tauri'
import { publishUserActionMessage } from '../../lib/userActionFeedback'
import type { SourceFrameMatch } from './layout'

interface SourceRevealActionsInput {
  allowsSourceEdits: boolean
  clipAssetId: string | undefined
  isRegistered: boolean
  offline: boolean
  onClose: () => void
}

export function sourceRevealActions({
  allowsSourceEdits,
  clipAssetId,
  isRegistered,
  offline,
  onClose,
}: SourceRevealActionsInput) {
  const sourceAssetId = allowsSourceEdits && clipAssetId && isRegistered ? clipAssetId : null
  const sourceRevealReason = 'This clip source is no longer registered in the open project'
  const sourceFileDisabledReason = sourceAssetId && offline ? 'Relink this source before revealing its file' : null
  const revealInSurface = (destination: 'project' | 'library') => {
    if (!sourceAssetId) return
    onClose()
    requestAnimationFrame(() => requestSourceNavigation(destination, sourceAssetId))
  }
  const revealSourceFile = () => {
    if (!sourceAssetId || sourceFileDisabledReason) return
    onClose()
    void revealRegisteredSource(sourceAssetId).then((reply) => publishUserActionMessage(reply.message))
  }
  return { sourceAssetId, sourceRevealReason, sourceFileDisabledReason, revealInSurface, revealSourceFile }
}

export function openMatchedSource(matchFrame: SourceFrameMatch | null, onClose: () => void): void {
  if (!matchFrame?.source) return
  const source = matchFrame.source
  onClose()
  requestAnimationFrame(() => {
    document.dispatchEvent(new CustomEvent('cut:open-source-monitor', {
      detail: { asset: source.asset, at_ms: source.srcMs },
    }))
  })
}
