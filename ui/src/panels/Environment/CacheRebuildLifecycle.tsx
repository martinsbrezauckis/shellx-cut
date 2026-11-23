import { mediaBasename } from '../../lib/mediaPath'
import type { ProjectCacheRebuildResult } from '../../lib/clientResults'
import {
  attentionAssetCount,
  rebuildCostSummary,
  rebuildSkips,
  rebuildSummary,
  type RebuildSkippedAsset,
} from './cacheLifecyclePresentation'

interface CacheRebuildLifecycleProps {
  assetPaths?: Readonly<Record<string, string>>
  hasProject: boolean
  sessionCurrent: boolean
  hasCompleteInventory: boolean
  hasBatch: boolean
  remainingAssets: number
  activeJobLabel: string | null
  estimateLoading: boolean
  loading: boolean
  result: ProjectCacheRebuildResult | null
  success: string | null
  error: string | null
  onEstimate: () => Promise<void>
  onRebuild: () => Promise<void>
}

function skipReason(item: RebuildSkippedAsset): string {
  switch (item.reason) {
    case 'source_changed': return 'source changed'
    case 'source_unavailable': return 'source unavailable'
    case 'unowned_output': return 'cache output is not verified as Cut’s own'
    case 'legacy_output': return 'cache output has an older ownership record'
    case 'unsupported_asset': return 'media type is not supported for cache rebuild'
  }
}

export default function CacheRebuildLifecycle({
  assetPaths = {},
  hasProject,
  sessionCurrent,
  hasCompleteInventory,
  hasBatch,
  remainingAssets,
  activeJobLabel,
  estimateLoading,
  loading,
  result,
  success,
  error,
  onEstimate,
  onRebuild,
}: CacheRebuildLifecycleProps) {
  const skips = result ? rebuildSkips(result) : []
  const attentionCount = result ? attentionAssetCount(result) : 0
  const disabled = !sessionCurrent || !hasProject || !hasCompleteInventory || !hasBatch || loading || estimateLoading || activeJobLabel !== null
  const unavailable = !sessionCurrent
    ? null
    : !hasProject
      ? 'Open a project to rebuild its editing cache.'
      : !hasCompleteInventory
        ? 'Finish the Health & Recovery check before rebuilding cache.'
        : !hasBatch
          ? 'This completed check found no missing cache to rebuild.'
          : activeJobLabel
            ? `${activeJobLabel} is in progress.`
            : null

  return (
    <section className="settings-cache-lifecycle-rebuild" aria-labelledby="settings-cache-rebuild-title" data-cut-cache-rebuild-section>
      <div>
        <strong id="settings-cache-rebuild-title">Editing cache</strong>
        <p>Rebuild only missing or stale Cut-owned proxies and base filmstrips. Original media stays unchanged.</p>
      </div>
      <div className="settings-cache-lifecycle-actions">
        <button
          type="button"
          className="env-btn env-btn--ghost"
          data-cut-cache-rebuild-estimate
          onClick={() => void onEstimate()}
          disabled={disabled}
          aria-describedby="cut-cache-rebuild-status"
        >
          {estimateLoading ? 'Verifying…' : 'Estimate rebuild work'}
        </button>
        <button
          type="button"
          className="env-btn env-btn--ghost"
          data-cut-cache-rebuild
          onClick={() => void onRebuild()}
          disabled={disabled}
          aria-describedby="cut-cache-rebuild-status"
        >
          {loading ? 'Scheduling…' : 'Rebuild missing cache'}
        </button>
        {unavailable && <span data-cut-cache-rebuild-disabled>{unavailable}</span>}
      </div>
      {result && (
        <div
          id="cut-cache-rebuild-status"
          data-cut-cache-rebuild-status
          data-cut-cache-rebuild-state={result.status}
          data-cut-cache-rebuild-queued={result.scheduled_assets}
          data-cut-cache-rebuild-up-to-date={result.counts.fresh_assets}
          data-cut-cache-rebuild-attention-count={attentionCount}
          aria-live="polite"
        >
          <p>{rebuildSummary(result, remainingAssets)}</p>
          {result.estimate.assets > 0 && <p data-cut-cache-rebuild-cost>{rebuildCostSummary(result.estimate)}. Source identity was verified for this estimate; Cut checks it again before publishing output.</p>}
          {skips.length > 0 && (
            <details className="settings-cache-lifecycle-attention" data-cut-cache-rebuild-attention>
              <summary data-cut-cache-rebuild-attention-toggle>{attentionCount} skipped item{attentionCount === 1 ? '' : 's'}</summary>
              <ul>
                {skips.map((item, index) => (
                  <li key={`${item.asset_id}-${item.kind ?? 'source'}-${item.reason}-${index}`} data-cut-cache-rebuild-skipped-asset={item.asset_id}>
                    {assetPaths[item.asset_id] ? mediaBasename(assetPaths[item.asset_id]) : 'Unavailable file'}: {skipReason(item)}. {item.action}
                  </li>
                ))}
              </ul>
            </details>
          )}
        </div>
      )}
      {success && <p className="settings-cache-lifecycle-success" data-cut-cache-rebuild-success aria-live="polite">{success}</p>}
      {error && <p className="settings-cache-lifecycle-error" data-cut-cache-rebuild-error role="alert">{error}</p>}
    </section>
  )
}
